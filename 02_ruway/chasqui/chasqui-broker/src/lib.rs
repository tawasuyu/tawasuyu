//! `brahman-broker` — empareja productores y consumidores por tipo de flujo.
//!
//! El broker indexa [`card_core::Card`]s registradas por `SessionId` y,
//! para cada `flow.input` de un consumidor, busca el `flow.output`
//! compatible de mejor calidad entre los demás. Tres ejes:
//!
//! 1. **Estrategia de matching** ([`MatchStrategy`]):
//!    - `Exact`: igualdad estricta de [`card_core::TypeRef`].
//!    - `Structural`: misma forma (mismo `package` + `name` para Wit;
//!      ignora `interface`).
//!    - `ExactThenStructural`: prefiere exact; cae en structural si no hay.
//!
//! 2. **Override `pin_to`**: si el consumidor declara `pin_to = "label"`,
//!    el broker prefiere productores cuya Card tenga ese `label` (siempre
//!    que el tipo siga matcheando). Si la pista no resuelve, cae en
//!    matching por tipo normal.
//!
//! 3. **Prioridad**: empate de tipo se resuelve por
//!    [`card_core::Priority`] del productor (mayor gana). Empate de
//!    prioridad se resuelve lexicográficamente por `label` (estable y
//!    determinista).
//!
//! El broker es **stateless w.r.t. routes**: cada `find_producer_for` o
//! `all_matches` se calcula bajo demanda. La única persistencia es el
//! índice de Cards registradas. Esto permite re-evaluar matches cuando
//! cambia el set sin invalidar caches.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use card_core::{
    Card, CardKind, CardReference, ContextBias, DataFacet, Flow, Lifecycle, Priority, TypeRef,
    WitInterface,
};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Identificador de sesión emitido por el handshake. Idéntico al usado por
/// `brahman-handshake` (no es un re-export para evitar la dependencia).
pub type SessionId = Ulid;

/// Estrategia de matching de tipos.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchStrategy {
    /// Igualdad estricta de `TypeRef`.
    Exact,
    /// Misma forma: para `Wit`, mismo `package` + `name`; para
    /// `Primitive`, mismo `name`.
    Structural,
    /// Híbrido: intenta `Exact` primero; si no matchea, `Structural`.
    /// Reporta cuál estrategia ganó en [`Match::via`].
    #[default]
    ExactThenStructural,
}

/// Configuración del broker.
#[derive(Debug, Clone, Default)]
pub struct BrokerConfig {
    pub strategy: MatchStrategy,
    /// Contexto operativo activo. Si una Card declara un
    /// `priority_contexts.<this>`, ese bias se aplica durante el match.
    /// `None` = sin biases per-contexto, sólo se usa lo estático.
    pub current_context: Option<String>,
}

/// Vista mínima de una Card que el broker necesita.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokeredCard {
    pub session: SessionId,
    pub label: String,
    pub lifecycle: Lifecycle,
    pub priority: Priority,
    pub inputs: Vec<Flow>,
    pub outputs: Vec<Flow>,
    /// Interfaz WIT extraída si el módulo es "consciente"; `None` si agnóstico.
    pub wit: Option<WitInterface>,
    /// Biases per-contexto, propagados desde `Card.priority_contexts`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub priority_contexts: BTreeMap<String, ContextBias>,
    /// Naturaleza de la entidad. Diferencia procesos (Ente) de
    /// agrupaciones de datos (Data — p. ej. Mónadas Nouser).
    #[serde(default)]
    pub kind: CardKind,
    /// Faceta de datos cuando `kind != Ente`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<DataFacet>,
    /// Socket de servicio (data plane) si lo declara la Card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_socket: Option<PathBuf>,
    /// Referencias a otras Cards (relaciones declaradas por esta Card).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<CardReference>,
}

impl BrokeredCard {
    fn from_card(session: SessionId, card: &Card, wit: Option<WitInterface>) -> Self {
        Self {
            session,
            label: card.label.clone(),
            lifecycle: card.lifecycle,
            priority: card.priority,
            inputs: card.flow.input.clone(),
            outputs: card.flow.output.clone(),
            wit,
            priority_contexts: card.priority_contexts.clone(),
            kind: card.kind,
            data: card.data.clone(),
            service_socket: card.service_socket.clone(),
            references: card.references.clone(),
        }
    }
}

/// Punto extremo de un flujo: qué sesión + nombre del flow dentro de su Card.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Endpoint {
    pub session: SessionId,
    pub flow_name: String,
}

/// Match concreto entre un consumidor y un productor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Match {
    pub consumer: Endpoint,
    pub consumer_label: String,
    pub producer: Endpoint,
    pub producer_label: String,
    /// Tipo del flow (lado consumidor — lado productor coincide en
    /// estrategia Exact, puede diferir en `interface` en Structural).
    pub ty: TypeRef,
    /// Estrategia que efectivamente matcheó.
    pub via: MatchStrategy,
    /// `true` si el match fue resuelto por `pin_to` y no por type-search.
    pub pinned: bool,
}

// =====================================================================
// Broker
// =====================================================================

/// El broker. Registra Cards por SessionId, computa matches bajo demanda.
#[derive(Debug, Clone, Default)]
pub struct Broker {
    cards: BTreeMap<SessionId, BrokeredCard>,
    config: BrokerConfig,
}

impl Broker {
    pub fn new(config: BrokerConfig) -> Self {
        Self {
            cards: BTreeMap::new(),
            config,
        }
    }

    /// Registra una Card con su WIT opcional. Devuelve `Some(prev)` si
    /// reemplazó una existente. Pasar `None` en `wit` indica módulo
    /// agnóstico (sin contrato WIT extraído).
    pub fn register(
        &mut self,
        session: SessionId,
        card: &Card,
        wit: Option<WitInterface>,
    ) -> Option<BrokeredCard> {
        self.cards
            .insert(session, BrokeredCard::from_card(session, card, wit))
    }

    /// Quita una Card por sesión.
    pub fn unregister(&mut self, session: SessionId) -> Option<BrokeredCard> {
        self.cards.remove(&session)
    }

    /// Cardinalidad del registro.
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// Iterador sobre las sesiones registradas.
    pub fn sessions(&self) -> impl Iterator<Item = SessionId> + '_ {
        self.cards.keys().copied()
    }

    /// Iterador sobre las Cards registradas (vista compartida).
    pub fn cards(&self) -> impl Iterator<Item = &BrokeredCard> + '_ {
        self.cards.values()
    }

    /// Busca el mejor productor para un input específico de un consumidor.
    ///
    /// Algoritmo:
    /// 1. Resuelve el flow input en el consumidor.
    /// 2. Si tiene `pin_to`, prefiere productores con ese `label` que
    ///    matcheen el tipo (cualquier estrategia configurada).
    /// 3. Si no hay pin_to o la pista falló, escanea todos los outputs
    ///    de las otras Cards. Filtra por compatibilidad de tipo.
    /// 4. Ordena por (priority desc, label asc) y devuelve el primero.
    pub fn find_producer_for(&self, consumer: SessionId, input_name: &str) -> Option<Match> {
        let cons = self.cards.get(&consumer)?;
        let input = cons.inputs.iter().find(|f| f.name == input_name)?;

        // pin_to efectivo: bias del contexto activo (si la Card declara
        // override consumer-side) > pin_to estático del Flow.
        let context_pin = self
            .context_bias(cons)
            .and_then(|b| b.pin_to.as_deref());
        let effective_pin = context_pin.or(input.pin_to.as_deref());

        if let Some(pin) = effective_pin {
            for prod in self.cards.values() {
                if prod.session == consumer || prod.label != pin {
                    continue;
                }
                for out in &prod.outputs {
                    if let Some(via) = self.types_match(&input.ty, &out.ty) {
                        return Some(self.make_match(cons, prod, input, out, via, true));
                    }
                }
            }
            // Fall through: pin no resuelto, type-search general.
        }

        let mut candidates: Vec<(&BrokeredCard, &Flow, MatchStrategy)> = Vec::new();
        for prod in self.cards.values() {
            if prod.session == consumer {
                continue;
            }
            for out in &prod.outputs {
                if let Some(via) = self.types_match(&input.ty, &out.ty) {
                    candidates.push((prod, out, via));
                }
            }
        }

        // Sort por (effective priority desc, label asc). El bias del
        // contexto puede subir o bajar la priority del productor.
        candidates.sort_by(|(a, _, _), (b, _, _)| {
            self.effective_priority(b)
                .cmp(&self.effective_priority(a))
                .then_with(|| a.label.cmp(&b.label))
        });

        let (prod, out, via) = candidates.into_iter().next()?;
        Some(self.make_match(cons, prod, input, out, via, false))
    }

    /// Devuelve el `ContextBias` que aplica a este Card en el contexto
    /// activo (si lo hay).
    fn context_bias<'a>(&self, card: &'a BrokeredCard) -> Option<&'a ContextBias> {
        self.config
            .current_context
            .as_ref()
            .and_then(|ctx| card.priority_contexts.get(ctx))
    }

    /// Priority efectiva del Card como productor, considerando el bias
    /// del contexto activo. El offset se clampa a `[Low=0, Critical=3]`.
    fn effective_priority(&self, card: &BrokeredCard) -> i16 {
        let base = priority_value(card.priority);
        let offset = self
            .context_bias(card)
            .map(|b| b.priority_offset as i16)
            .unwrap_or(0);
        (base + offset).clamp(0, 3)
    }

    /// Calcula todos los matches consumer→producer en el set actual.
    /// Útil para introspección o para que el Admin emita rutas en lote.
    pub fn all_matches(&self) -> Vec<Match> {
        let mut out = Vec::new();
        for cons in self.cards.values() {
            for input in &cons.inputs {
                if let Some(m) = self.find_producer_for(cons.session, &input.name) {
                    out.push(m);
                }
            }
        }
        out
    }

    fn types_match(&self, consumer_ty: &TypeRef, producer_ty: &TypeRef) -> Option<MatchStrategy> {
        match self.config.strategy {
            MatchStrategy::Exact => exact_match(consumer_ty, producer_ty).then_some(MatchStrategy::Exact),
            MatchStrategy::Structural => {
                structural_match(consumer_ty, producer_ty).then_some(MatchStrategy::Structural)
            }
            MatchStrategy::ExactThenStructural => {
                if exact_match(consumer_ty, producer_ty) {
                    Some(MatchStrategy::Exact)
                } else if structural_match(consumer_ty, producer_ty) {
                    Some(MatchStrategy::Structural)
                } else {
                    None
                }
            }
        }
    }

    fn make_match(
        &self,
        cons: &BrokeredCard,
        prod: &BrokeredCard,
        input: &Flow,
        output: &Flow,
        via: MatchStrategy,
        pinned: bool,
    ) -> Match {
        Match {
            consumer: Endpoint {
                session: cons.session,
                flow_name: input.name.clone(),
            },
            consumer_label: cons.label.clone(),
            producer: Endpoint {
                session: prod.session,
                flow_name: output.name.clone(),
            },
            producer_label: prod.label.clone(),
            ty: input.ty.clone(),
            via,
            pinned,
        }
    }
}

// =====================================================================
// Predicados de matching (libres, testeables aislados)
// =====================================================================

fn priority_value(p: Priority) -> i16 {
    match p {
        Priority::Low => 0,
        Priority::Normal => 1,
        Priority::High => 2,
        Priority::Critical => 3,
    }
}

fn exact_match(a: &TypeRef, b: &TypeRef) -> bool {
    a == b
}

fn structural_match(a: &TypeRef, b: &TypeRef) -> bool {
    match (a, b) {
        (TypeRef::Primitive { name: na }, TypeRef::Primitive { name: nb }) => na == nb,
        (
            TypeRef::Wit {
                package: pa, name: na, ..
            },
            TypeRef::Wit {
                package: pb, name: nb, ..
            },
        ) => pa == pb && na == nb,
        _ => false,
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use card_core::{Card, Flows, Payload, Supervision, CARD_SCHEMA_VERSION};

    fn card(label: &str, priority: Priority, flows: Flows) -> Card {
        Card {
            schema_version: CARD_SCHEMA_VERSION,
            id: Ulid::new(),
            label: label.into(),
            payload: Payload::Virtual,
            supervision: Supervision::OneShot,
            priority,
            flow: flows,
            ..Default::default()
        }
    }

    fn prim(name: &str) -> TypeRef {
        TypeRef::Primitive { name: name.into() }
    }

    fn wit(pkg: &str, iface: Option<&str>, name: &str) -> TypeRef {
        TypeRef::Wit {
            package: pkg.into(),
            interface: iface.map(|s| s.into()),
            name: name.into(),
        }
    }

    fn flow(name: &str, ty: TypeRef, pin: Option<&str>) -> Flow {
        Flow {
            name: name.into(),
            ty,
            pin_to: pin.map(|s| s.into()),
        }
    }

    #[test]
    fn exact_match_same_typeref() {
        let mut b = Broker::new(BrokerConfig {
            strategy: MatchStrategy::Exact,
            current_context: None,
        });
        let producer = card(
            "dht",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("results", prim("string"), None)],
            },
        );
        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow("query", prim("string"), None)],
                output: vec![],
            },
        );
        let s_prod = Ulid::new();
        let s_cons = Ulid::new();
        b.register(s_prod, &producer, None);
        b.register(s_cons, &consumer, None);

        let m = b.find_producer_for(s_cons, "query").expect("match");
        assert_eq!(m.producer_label, "dht");
        assert_eq!(m.via, MatchStrategy::Exact);
        assert!(!m.pinned);
    }

    #[test]
    fn structural_ignores_interface() {
        let mut b = Broker::new(BrokerConfig {
            strategy: MatchStrategy::Structural,
            current_context: None,
        });
        let producer = card(
            "dht",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow(
                    "out",
                    wit("brahman:dht", Some("v1"), "entity-result"),
                    None,
                )],
            },
        );
        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow(
                    "in",
                    wit("brahman:dht", Some("v2"), "entity-result"),
                    None,
                )],
                output: vec![],
            },
        );
        let s_prod = Ulid::new();
        let s_cons = Ulid::new();
        b.register(s_prod, &producer, None);
        b.register(s_cons, &consumer, None);

        let m = b.find_producer_for(s_cons, "in").expect("match");
        assert_eq!(m.via, MatchStrategy::Structural);
    }

    #[test]
    fn exact_strategy_rejects_interface_mismatch() {
        let mut b = Broker::new(BrokerConfig {
            strategy: MatchStrategy::Exact,
            current_context: None,
        });
        let producer = card(
            "dht",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow(
                    "out",
                    wit("brahman:dht", Some("v1"), "entity-result"),
                    None,
                )],
            },
        );
        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow(
                    "in",
                    wit("brahman:dht", Some("v2"), "entity-result"),
                    None,
                )],
                output: vec![],
            },
        );
        b.register(Ulid::new(), &producer, None);
        let s_cons = Ulid::new();
        b.register(s_cons, &consumer, None);

        assert!(b.find_producer_for(s_cons, "in").is_none());
    }

    #[test]
    fn exact_then_structural_prefers_exact() {
        let mut b = Broker::new(BrokerConfig {
            strategy: MatchStrategy::ExactThenStructural,
            current_context: None,
        });
        // Productor 1: match estructural (interface diferente)
        let p_struct = card(
            "dht-cache",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow(
                    "out",
                    wit("brahman:dht", Some("v2"), "entity-result"),
                    None,
                )],
            },
        );
        // Productor 2: match exact (interface igual)
        let p_exact = card(
            "dht",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow(
                    "out",
                    wit("brahman:dht", Some("v1"), "entity-result"),
                    None,
                )],
            },
        );
        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow(
                    "in",
                    wit("brahman:dht", Some("v1"), "entity-result"),
                    None,
                )],
                output: vec![],
            },
        );
        b.register(Ulid::new(), &p_struct, None);
        b.register(Ulid::new(), &p_exact, None);
        let s_cons = Ulid::new();
        b.register(s_cons, &consumer, None);

        let m = b.find_producer_for(s_cons, "in").expect("match");
        // El exact gana incluso si tiene priority igual: por estrategia.
        assert_eq!(m.producer_label, "dht");
        assert_eq!(m.via, MatchStrategy::Exact);
    }

    #[test]
    fn pin_to_overrides_type_search() {
        let mut b = Broker::new(BrokerConfig::default());
        // Dos productores que producen el mismo tipo.
        let p1 = card(
            "dht-prod",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let p2 = card(
            "dht-test",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow("in", prim("string"), Some("dht-test"))],
                output: vec![],
            },
        );
        b.register(Ulid::new(), &p1, None);
        b.register(Ulid::new(), &p2, None);
        let s_cons = Ulid::new();
        b.register(s_cons, &consumer, None);

        let m = b.find_producer_for(s_cons, "in").expect("match");
        assert_eq!(m.producer_label, "dht-test");
        assert!(m.pinned);
    }

    #[test]
    fn pin_to_unresolvable_falls_back_to_type_match() {
        let mut b = Broker::new(BrokerConfig::default());
        let p = card(
            "real-dht",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow("in", prim("string"), Some("nonexistent"))],
                output: vec![],
            },
        );
        b.register(Ulid::new(), &p, None);
        let s_cons = Ulid::new();
        b.register(s_cons, &consumer, None);

        let m = b.find_producer_for(s_cons, "in").expect("match");
        assert_eq!(m.producer_label, "real-dht");
        assert!(!m.pinned);
    }

    #[test]
    fn priority_breaks_ties() {
        let mut b = Broker::new(BrokerConfig::default());
        let p_low = card(
            "z-dht",
            Priority::Low,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let p_high = card(
            "a-dht",
            Priority::High,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow("in", prim("string"), None)],
                output: vec![],
            },
        );
        b.register(Ulid::new(), &p_low, None);
        b.register(Ulid::new(), &p_high, None);
        let s_cons = Ulid::new();
        b.register(s_cons, &consumer, None);

        let m = b.find_producer_for(s_cons, "in").expect("match");
        assert_eq!(m.producer_label, "a-dht"); // priority High > Low
    }

    #[test]
    fn label_alpha_breaks_priority_ties() {
        let mut b = Broker::new(BrokerConfig::default());
        let p1 = card(
            "z-dht",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let p2 = card(
            "a-dht",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow("in", prim("string"), None)],
                output: vec![],
            },
        );
        b.register(Ulid::new(), &p1, None);
        b.register(Ulid::new(), &p2, None);
        let s_cons = Ulid::new();
        b.register(s_cons, &consumer, None);

        let m = b.find_producer_for(s_cons, "in").expect("match");
        assert_eq!(m.producer_label, "a-dht"); // alfabético gana
    }

    #[test]
    fn unregister_removes_producer() {
        let mut b = Broker::new(BrokerConfig::default());
        let p = card(
            "dht",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow("in", prim("string"), None)],
                output: vec![],
            },
        );
        let s_p = Ulid::new();
        b.register(s_p, &p, None);
        let s_c = Ulid::new();
        b.register(s_c, &consumer, None);

        assert!(b.find_producer_for(s_c, "in").is_some());
        b.unregister(s_p);
        assert!(b.find_producer_for(s_c, "in").is_none());
    }

    #[test]
    fn no_self_loops() {
        let mut b = Broker::new(BrokerConfig::default());
        let same = card(
            "echo",
            Priority::Normal,
            Flows {
                input: vec![flow("in", prim("string"), None)],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let s = Ulid::new();
        b.register(s, &same, None);

        // Solo una Card registrada — no hay otra que produzca string.
        assert!(b.find_producer_for(s, "in").is_none());
    }

    #[test]
    fn all_matches_lists_pairs() {
        let mut b = Broker::new(BrokerConfig::default());
        let dht = card(
            "dht",
            Priority::Normal,
            Flows {
                input: vec![flow("query", prim("string"), None)],
                output: vec![flow("results", prim("bytes"), None)],
            },
        );
        let ui = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow("data", prim("bytes"), None)],
                output: vec![flow("user-input", prim("string"), None)],
            },
        );
        b.register(Ulid::new(), &dht, None);
        b.register(Ulid::new(), &ui, None);

        let matches = b.all_matches();
        assert_eq!(matches.len(), 2);
        // dht.query ← ui.user-input  y  ui.data ← dht.results
        let pairs: Vec<_> = matches
            .iter()
            .map(|m| (m.consumer_label.as_str(), m.producer_label.as_str()))
            .collect();
        assert!(pairs.contains(&("dht", "ui")));
        assert!(pairs.contains(&("ui", "dht")));
    }

    // ===========================================================
    // Priority contexts
    // ===========================================================

    #[test]
    fn context_priority_offset_lifts_producer_above_alphabetic_winner() {
        // Sin contexto, "a-prod" gana contra "b-prod" (alfabético).
        // En contexto "test", b-prod tiene offset +1 → debería ganar.
        let mut a_prod = card(
            "a-prod",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        a_prod.priority_contexts = std::collections::BTreeMap::new(); // explícito vacío

        let mut b_prod = card(
            "b-prod",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        b_prod.priority_contexts.insert(
            "test".into(),
            ContextBias {
                pin_to: None,
                priority_offset: 1,
            },
        );

        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow("in", prim("string"), None)],
                output: vec![],
            },
        );

        let s_cons = Ulid::new();

        // Caso 1: sin contexto → a-prod gana (alfabético).
        let mut b = Broker::new(BrokerConfig {
            strategy: MatchStrategy::default(),
            current_context: None,
        });
        b.register(Ulid::new(), &a_prod, None);
        b.register(Ulid::new(), &b_prod, None);
        b.register(s_cons, &consumer, None);
        let m = b.find_producer_for(s_cons, "in").unwrap();
        assert_eq!(m.producer_label, "a-prod");

        // Caso 2: contexto "test" → b-prod gana por offset +1.
        let mut b = Broker::new(BrokerConfig {
            strategy: MatchStrategy::default(),
            current_context: Some("test".into()),
        });
        b.register(Ulid::new(), &a_prod, None);
        b.register(Ulid::new(), &b_prod, None);
        b.register(s_cons, &consumer, None);
        let m = b.find_producer_for(s_cons, "in").unwrap();
        assert_eq!(m.producer_label, "b-prod");
    }

    #[test]
    fn context_pin_to_overrides_static_pin() {
        // Consumer pinea estático a "real-dht", pero en contexto "test"
        // declara override a "mock-dht".
        let real = card(
            "real-dht",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let mock = card(
            "mock-dht",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let mut consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow("in", prim("string"), Some("real-dht"))],
                output: vec![],
            },
        );
        consumer.priority_contexts.insert(
            "test".into(),
            ContextBias {
                pin_to: Some("mock-dht".into()),
                priority_offset: 0,
            },
        );

        let s_cons = Ulid::new();

        // Caso 1: sin contexto → static pin gana ("real-dht").
        let mut b = Broker::new(BrokerConfig::default());
        b.register(Ulid::new(), &real, None);
        b.register(Ulid::new(), &mock, None);
        b.register(s_cons, &consumer, None);
        let m = b.find_producer_for(s_cons, "in").unwrap();
        assert_eq!(m.producer_label, "real-dht");
        assert!(m.pinned);

        // Caso 2: contexto "test" → context override gana ("mock-dht").
        let mut b = Broker::new(BrokerConfig {
            strategy: MatchStrategy::default(),
            current_context: Some("test".into()),
        });
        b.register(Ulid::new(), &real, None);
        b.register(Ulid::new(), &mock, None);
        b.register(s_cons, &consumer, None);
        let m = b.find_producer_for(s_cons, "in").unwrap();
        assert_eq!(m.producer_label, "mock-dht");
        assert!(m.pinned);
    }

    #[test]
    fn unknown_context_no_op() {
        // Si la Card declara biases para "test" pero el broker está en
        // "prod", los biases no aplican.
        let mut b_prod = card(
            "b-prod",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        b_prod.priority_contexts.insert(
            "test".into(),
            ContextBias {
                pin_to: None,
                priority_offset: 5,
            },
        );
        let a_prod = card(
            "a-prod",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        let consumer = card(
            "ui",
            Priority::Normal,
            Flows {
                input: vec![flow("in", prim("string"), None)],
                output: vec![],
            },
        );

        let mut b = Broker::new(BrokerConfig {
            strategy: MatchStrategy::default(),
            current_context: Some("prod".into()),
        });
        let s_cons = Ulid::new();
        b.register(Ulid::new(), &a_prod, None);
        b.register(Ulid::new(), &b_prod, None);
        b.register(s_cons, &consumer, None);

        // En contexto "prod" sin biases declarados, gana por alfabético.
        let m = b.find_producer_for(s_cons, "in").unwrap();
        assert_eq!(m.producer_label, "a-prod");
    }

    #[test]
    fn priority_offset_clamps_to_critical() {
        // Offset enorme no debe hacer overflow ni saltar fuera del rango.
        let mut prod = card(
            "p",
            Priority::Normal,
            Flows {
                input: vec![],
                output: vec![flow("out", prim("string"), None)],
            },
        );
        prod.priority_contexts.insert(
            "x".into(),
            ContextBias {
                pin_to: None,
                priority_offset: 100,
            },
        );

        let b = Broker::new(BrokerConfig {
            strategy: MatchStrategy::default(),
            current_context: Some("x".into()),
        });
        let bc = BrokeredCard::from_card(Ulid::new(), &prod, None);
        // effective_priority debe estar clampada a 3 (Critical), no 101.
        assert_eq!(b.effective_priority(&bc), 3);
    }
}
