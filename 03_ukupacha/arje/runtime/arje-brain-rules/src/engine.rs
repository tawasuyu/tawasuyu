//! Motor de inferencia. HashMap<EventKindDiscriminant, Vec<Arc<Rule>>> para
//! lookup O(1) por tipo de evento, luego filter lineal por scope + filtros
//! del payload (BusInvokeOf, Custom).
//!
//! Inmutabilidad fractal: `Arc<Rule>` es el unit de compartición. Clonar una
//! regla del motor para entregarla al dispatcher es un refcount bump, no copia.

use crate::rules::TimedEvent;
use crate::rules::{EventKind, EventPattern, Rule, Scope};
use arje_card::Capability;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use ulid::Ulid;

/// Discriminante barato de `EventKind` para indexar el HashMap. Sin payload —
/// el match de payload se hace en una segunda pasada lineal en O(k) donde k
/// es el número de reglas para ese tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKindDiscriminant {
    EnteSpawned,
    EnteDied,
    BusAnnounce,
    BusInvoke,
    BusInvokeOf,
    DeviceAdded,
    DeviceRemoved,
    Custom,
}

impl From<&EventKind> for EventKindDiscriminant {
    fn from(k: &EventKind) -> Self {
        match k {
            EventKind::EnteSpawned => Self::EnteSpawned,
            EventKind::EnteDied => Self::EnteDied,
            EventKind::BusAnnounce => Self::BusAnnounce,
            EventKind::BusInvoke => Self::BusInvoke,
            EventKind::BusInvokeOf(_) => Self::BusInvokeOf,
            EventKind::DeviceAdded => Self::DeviceAdded,
            EventKind::DeviceRemoved => Self::DeviceRemoved,
            EventKind::Custom(_) => Self::Custom,
        }
    }
}

/// Snapshot del Ente que disparó el evento. Necesario para evaluar `Scope`.
#[derive(Debug, Clone, Default)]
pub struct SubjectInfo {
    pub id: Option<Ulid>,
    pub label: Option<String>,
    pub capabilities: Vec<Capability>,
}

pub struct RuleEngine {
    rules: Vec<Arc<Rule>>,
    /// Reglas atómicas (Single, Sequence) indexadas por discriminante del
    /// kind que las dispara. Lookup O(1).
    by_kind: HashMap<EventKindDiscriminant, Vec<Arc<Rule>>>,
    /// Reglas compuestas (Either, All): se evalúan contra cada evento.
    /// Para fractales con N pequeño no afecta perf; con N grande, optimizar
    /// emitiendo a múltiples buckets en insert (fan-out).
    compound: Vec<Arc<Rule>>,
}

impl Default for RuleEngine {
    fn default() -> Self { Self::empty() }
}

impl RuleEngine {
    pub fn empty() -> Self {
        Self { rules: Vec::new(), by_kind: HashMap::new(), compound: Vec::new() }
    }

    /// Carga reglas desde JSON (lista de Rule).
    pub fn load_json(json: &str) -> anyhow::Result<Self> {
        let rules: Vec<Rule> = serde_json::from_str(json)?;
        let mut engine = Self::empty();
        for r in rules {
            r.validate().map_err(|e| anyhow::anyhow!("regla inválida: {e}"))?;
            engine.insert(r);
        }
        Ok(engine)
    }

    pub fn insert(&mut self, rule: Rule) {
        let arc = Arc::new(rule);
        // Atómicas → bucket por discriminante. Compuestas → bucket fallback.
        if let Some(trigger) = arc.when.trigger_kind() {
            let disc = EventKindDiscriminant::from(trigger);
            self.by_kind.entry(disc).or_default().push(arc.clone());
        } else {
            self.compound.push(arc.clone());
        }
        self.rules.push(arc);
    }

    pub fn remove(&mut self, id: Ulid) -> bool {
        let before = self.rules.len();
        self.rules.retain(|r| r.id != id);
        for v in self.by_kind.values_mut() {
            v.retain(|r| r.id != id);
        }
        self.compound.retain(|r| r.id != id);
        before != self.rules.len()
    }

    pub fn rules(&self) -> impl Iterator<Item = &Arc<Rule>> { self.rules.iter() }

    pub fn len(&self) -> usize { self.rules.len() }
    pub fn is_empty(&self) -> bool { self.rules.is_empty() }

    /// Despacho determinista. Devuelve reglas que matchean, ordenadas por
    /// prioridad descendente. Cada Arc<Rule> se clona (refcount) — sin copiar
    /// los datos.
    ///
    /// `history` es el slice de eventos recientes (en orden cronológico,
    /// más reciente al final) usado para evaluar Sequence patterns.
    /// Para reglas Single, history se ignora.
    ///
    /// Si el evento es `BusInvokeOf(_)`, también consultamos el bucket
    /// `BusInvoke` (regla genérica que ignora la cap).
    pub fn dispatch(
        &self,
        event: &EventKind,
        subject: &SubjectInfo,
        history: &[TimedEvent],
    ) -> Vec<Arc<Rule>> {
        let primary = EventKindDiscriminant::from(event);
        let mut buckets: Vec<&Vec<Arc<Rule>>> = Vec::with_capacity(2);
        if let Some(v) = self.by_kind.get(&primary) {
            buckets.push(v);
        }
        if matches!(event, EventKind::BusInvokeOf(_)) {
            if let Some(v) = self.by_kind.get(&EventKindDiscriminant::BusInvoke) {
                buckets.push(v);
            }
        }
        let mut hits: Vec<Arc<Rule>> = buckets.into_iter()
            .flat_map(|v| v.iter())
            .filter(|r| matches_pattern(&r.when, event, history))
            .filter(|r| matches_scope(&r.scope, subject))
            .cloned()
            .collect();
        // Fallback: reglas compuestas (Either/All) se evalúan siempre.
        for r in &self.compound {
            if matches_pattern(&r.when, event, history) && matches_scope(&r.scope, subject) {
                hits.push(r.clone());
            }
        }
        hits.sort_by(|a, b| b.priority.cmp(&a.priority));
        hits
    }
}

/// Match recursivo del pattern. Atomic patterns evalúan contra el evento
/// actual + history. Compuestos (Either/All) recursan sobre sus children.
fn matches_pattern(pattern: &EventPattern, event: &EventKind, history: &[TimedEvent]) -> bool {
    match pattern {
        EventPattern::Single { kind } => matches_event_payload(kind, event),
        EventPattern::Sequence { kinds, within_ms } => {
            if kinds.is_empty() { return false; }
            let last_kind = kinds.last().unwrap();
            if !matches_event_payload(last_kind, event) { return false; }
            if history.len() < kinds.len() { return false; }
            let tail = &history[history.len() - kinds.len()..];
            for (t, k) in tail.iter().zip(kinds) {
                if !matches_event_payload(k, &t.kind) { return false; }
            }
            if *within_ms > 0 {
                let span = tail.last().unwrap().at.duration_since(tail.first().unwrap().at);
                if span > Duration::from_millis(*within_ms) { return false; }
            }
            true
        }
        EventPattern::Either { patterns } => {
            patterns.iter().any(|p| matches_pattern(p, event, history))
        }
        EventPattern::All { patterns } => {
            patterns.iter().all(|p| matches_pattern(p, event, history))
        }
    }
}

fn matches_event_payload(rule_kind: &EventKind, evt: &EventKind) -> bool {
    use EventKind::*;
    match (rule_kind, evt) {
        (EnteSpawned, EnteSpawned) => true,
        (EnteDied, EnteDied) => true,
        (BusAnnounce, BusAnnounce) => true,
        (BusInvoke, BusInvoke) | (BusInvoke, BusInvokeOf(_)) => true,
        (BusInvokeOf(want), BusInvokeOf(got)) => want == got,
        (DeviceAdded, DeviceAdded) => true,
        (DeviceRemoved, DeviceRemoved) => true,
        (Custom(want), Custom(got)) => want == got,
        _ => false,
    }
}

fn matches_scope(scope: &Scope, subj: &SubjectInfo) -> bool {
    if scope.is_wildcard() { return true; }
    if let Some(id) = scope.subject_id {
        if subj.id != Some(id) { return false; }
    }
    if let Some(lbl) = &scope.subject_label {
        if subj.label.as_ref() != Some(lbl) { return false; }
    }
    if let Some(cap) = &scope.subject_has_cap {
        if !subj.capabilities.contains(cap) { return false; }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Action, EventPattern, LogLevel};
    use std::time::{Duration, Instant};

    fn rule_single(id_str: &str, kind: EventKind, prio: u8) -> Rule {
        Rule {
            id: id_str.parse().unwrap(),
            priority: prio,
            when: EventPattern::Single { kind },
            then: vec![Action::Log {
                level: LogLevel::Info,
                message: id_str.into(),
            }],
            scope: Scope::default(),
        }
    }

    fn empty_history() -> Vec<TimedEvent> { Vec::new() }

    #[test]
    fn dispatch_picks_only_matching_kind() {
        let mut e = RuleEngine::empty();
        e.insert(rule_single("01KQQ100000000000000000001", EventKind::EnteSpawned, 5));
        e.insert(rule_single("01KQQ100000000000000000002", EventKind::EnteDied, 5));
        let hits = e.dispatch(&EventKind::EnteSpawned, &SubjectInfo::default(), &empty_history());
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn priority_orders_descending() {
        let mut e = RuleEngine::empty();
        e.insert(rule_single("01KQQ100000000000000000003", EventKind::EnteSpawned, 1));
        e.insert(rule_single("01KQQ100000000000000000004", EventKind::EnteSpawned, 9));
        let hits = e.dispatch(&EventKind::EnteSpawned, &SubjectInfo::default(), &empty_history());
        assert_eq!(hits[0].priority, 9);
        assert_eq!(hits[1].priority, 1);
    }

    #[test]
    fn scope_filters_by_label() {
        let mut e = RuleEngine::empty();
        let mut r = rule_single("01KQQ100000000000000000005", EventKind::EnteSpawned, 5);
        r.scope = Scope { subject_label: Some("foo".into()), ..Default::default() };
        e.insert(r);
        let foo = SubjectInfo { label: Some("foo".into()), ..Default::default() };
        let bar = SubjectInfo { label: Some("bar".into()), ..Default::default() };
        assert_eq!(e.dispatch(&EventKind::EnteSpawned, &foo, &empty_history()).len(), 1);
        assert_eq!(e.dispatch(&EventKind::EnteSpawned, &bar, &empty_history()).len(), 0);
    }

    #[test]
    fn bus_invoke_generic_matches_specific() {
        let mut e = RuleEngine::empty();
        e.insert(rule_single("01KQQ100000000000000000006", EventKind::BusInvoke, 5));
        let hits = e.dispatch(
            &EventKind::BusInvokeOf(Capability::LegacyLogind),
            &SubjectInfo::default(),
            &empty_history(),
        );
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn sequence_pattern_matches_with_history() {
        let mut e = RuleEngine::empty();
        let r = Rule {
            id: "01KQQ100000000000000000007".parse().unwrap(),
            priority: 5,
            when: EventPattern::Sequence {
                kinds: vec![EventKind::EnteSpawned, EventKind::BusAnnounce],
                within_ms: 1000,
            },
            then: vec![Action::Log { level: LogLevel::Info, message: "seq".into() }],
            scope: Scope::default(),
        };
        e.insert(r);

        let now = Instant::now();
        let history = vec![
            TimedEvent { kind: EventKind::EnteSpawned, at: now },
            TimedEvent { kind: EventKind::BusAnnounce, at: now + Duration::from_millis(50) },
        ];
        let hits = e.dispatch(&EventKind::BusAnnounce, &SubjectInfo::default(), &history);
        assert_eq!(hits.len(), 1, "esperaba match secuencia, got {}", hits.len());
    }

    #[test]
    fn sequence_rejects_outside_time_window() {
        let mut e = RuleEngine::empty();
        let r = Rule {
            id: "01KQQ100000000000000000008".parse().unwrap(),
            priority: 5,
            when: EventPattern::Sequence {
                kinds: vec![EventKind::EnteSpawned, EventKind::BusAnnounce],
                within_ms: 100,
            },
            then: vec![Action::Log { level: LogLevel::Info, message: "seq".into() }],
            scope: Scope::default(),
        };
        e.insert(r);
        let now = Instant::now();
        let history = vec![
            TimedEvent { kind: EventKind::EnteSpawned, at: now },
            TimedEvent { kind: EventKind::BusAnnounce, at: now + Duration::from_millis(500) },
        ];
        let hits = e.dispatch(&EventKind::BusAnnounce, &SubjectInfo::default(), &history);
        assert!(hits.is_empty(), "no debería matchear fuera de la ventana");
    }

    #[test]
    fn either_matches_any_branch() {
        let mut e = RuleEngine::empty();
        let r = Rule {
            id: "01KQQ100000000000000000010".parse().unwrap(),
            priority: 5,
            when: EventPattern::Either { patterns: vec![
                EventPattern::Single { kind: EventKind::EnteSpawned },
                EventPattern::Single { kind: EventKind::EnteDied },
            ]},
            then: vec![Action::Log { level: LogLevel::Info, message: "either".into() }],
            scope: Scope::default(),
        };
        e.insert(r);
        assert_eq!(e.dispatch(&EventKind::EnteSpawned, &SubjectInfo::default(), &[]).len(), 1);
        assert_eq!(e.dispatch(&EventKind::EnteDied, &SubjectInfo::default(), &[]).len(), 1);
        assert_eq!(e.dispatch(&EventKind::BusAnnounce, &SubjectInfo::default(), &[]).len(), 0);
    }

    #[test]
    fn all_requires_every_branch() {
        let mut e = RuleEngine::empty();
        // All: matchear sólo si el evento actual es BusAnnounce Y la
        // secuencia EnteSpawned→BusAnnounce ocurrió en history.
        let r = Rule {
            id: "01KQQ100000000000000000011".parse().unwrap(),
            priority: 5,
            when: EventPattern::All { patterns: vec![
                EventPattern::Single { kind: EventKind::BusAnnounce },
                EventPattern::Sequence {
                    kinds: vec![EventKind::EnteSpawned, EventKind::BusAnnounce],
                    within_ms: 0,
                },
            ]},
            then: vec![Action::Log { level: LogLevel::Info, message: "all".into() }],
            scope: Scope::default(),
        };
        e.insert(r);

        let now = Instant::now();
        let history = vec![
            TimedEvent { kind: EventKind::EnteSpawned, at: now },
            TimedEvent { kind: EventKind::BusAnnounce, at: now + Duration::from_millis(10) },
        ];
        // Single y Sequence ambos matchean → All matches.
        assert_eq!(e.dispatch(&EventKind::BusAnnounce, &SubjectInfo::default(), &history).len(), 1);
        // Sólo Single matchea (history vacío) → All no matches.
        assert!(e.dispatch(&EventKind::BusAnnounce, &SubjectInfo::default(), &[]).is_empty());
    }

    #[test]
    fn sequence_requires_correct_order() {
        let mut e = RuleEngine::empty();
        let r = Rule {
            id: "01KQQ100000000000000000009".parse().unwrap(),
            priority: 5,
            when: EventPattern::Sequence {
                kinds: vec![EventKind::EnteSpawned, EventKind::BusAnnounce],
                within_ms: 0,
            },
            then: vec![Action::Log { level: LogLevel::Info, message: "seq".into() }],
            scope: Scope::default(),
        };
        e.insert(r);
        let now = Instant::now();
        // Orden invertido en el history.
        let history = vec![
            TimedEvent { kind: EventKind::BusAnnounce, at: now },
            TimedEvent { kind: EventKind::EnteSpawned, at: now + Duration::from_millis(10) },
        ];
        // El evento actual es EnteSpawned, pero el último de la secuencia
        // requerida es BusAnnounce — no debería matchear.
        let hits = e.dispatch(&EventKind::EnteSpawned, &SubjectInfo::default(), &history);
        assert!(hits.is_empty());
    }
}
