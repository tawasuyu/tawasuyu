//! Cristalización: del flujo observado a reglas explícitas.
//!
//! Detecta pares (a, b) donde:
//!   - support(a, b) ≥ min_support  (suficientes muestras para no ser ruido)
//!   - P(b|a) ≥ min_conditional_prob (a predice b con confianza)
//!   - PMI(a; b) ≥ min_pmi          (más correlacionados que random)
//!
//! Cada cristal se materializa como `Rule` ejecutable (`crystal_to_rule`).
//! Para persistencia/transporte, `crystal_to_json_pretty` serializa la Rule
//! resultante con serde — sin formatos intermedios.

use crate::observer::{GapStats, Observer};
use crate::rules::{Action, EventKind, EventPattern, LogLevel, Rule, Scope};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use ulid::Ulid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Crystal {
    pub antecedent: EventKind,
    pub consequent: EventKind,
    pub conditional_prob: f64,
    pub pmi: f64,
    pub support: u64,
    /// Estadísticas del gap temporal entre antecedent → consequent.
    /// None si no hay histograma. Habilita generación de reglas Sequence
    /// con `within_ms = (mean + 2σ) * 1000`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_stats: Option<GapStats>,
}

#[derive(Debug, Clone, Copy)]
pub struct CrystallizationParams {
    pub min_support: u64,
    pub min_conditional_prob: f64,
    pub min_pmi: f64,
}

impl Default for CrystallizationParams {
    fn default() -> Self {
        Self {
            min_support: 5,
            min_conditional_prob: 0.7,
            min_pmi: 0.5,
        }
    }
}

pub fn detect_crystals(obs: &Observer, params: &CrystallizationParams) -> Vec<Crystal> {
    let mut out = Vec::new();
    for ((a, b), &count) in obs.cooccurrences() {
        if count < params.min_support { continue; }
        let cp = obs.conditional_prob(a, b);
        if cp < params.min_conditional_prob { continue; }
        let mi = obs.pmi(a, b);
        if mi < params.min_pmi { continue; }
        // Stats del histograma si existen para este par.
        let gap_stats = obs.gap_histograms()
            .get(&(a.clone(), b.clone()))
            .map(|h| h.stats());
        out.push(Crystal {
            antecedent: a.clone(),
            consequent: b.clone(),
            conditional_prob: cp,
            pmi: mi,
            support: count,
            gap_stats,
        });
    }
    out.sort_by(|x, y| y.conditional_prob.partial_cmp(&x.conditional_prob).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Serializa la `Rule` derivada del cristal como JSON pretty-printed. Ese
/// JSON es el formato canónico de persistencia: el loader lo lee como una
/// línea de JSONL o como elemento de un array. Los stats del cristal (P, PMI,
/// support) viven en el audit log vía `AuditAction::PromoteCrystal`, no se
/// duplican aquí.
pub fn crystal_to_json_pretty(c: &Crystal) -> String {
    serde_json::to_string_pretty(&crystal_to_rule(c))
        .expect("Rule serialize should never fail")
}

/// Convierte un cristal a una `Rule` ejecutable. Si hay gap_stats con
/// muestras suficientes (≥ 4), genera una regla `Sequence` con
/// `within_ms = (mean + 2σ) * 1000`. 2σ cubre ~95% de la distribución
/// asumiendo normalidad — captura el "tiempo típico de respuesta" del
/// patrón observado. Si no hay stats, fallback a `Single { antecedent }`.
pub fn crystal_to_rule(c: &Crystal) -> Rule {
    let when = match &c.gap_stats {
        Some(s) if s.count >= 4 => {
            // Mínimo 1ms para evitar within_ms=0 cuando varianza colapsa.
            let bound_secs = (s.mean_secs + 2.0 * s.stddev_secs).max(0.001);
            EventPattern::Sequence {
                kinds: vec![c.antecedent.clone(), c.consequent.clone()],
                within_ms: (bound_secs * 1000.0).ceil() as u64,
            }
        }
        _ => EventPattern::Single { kind: c.antecedent.clone() },
    };
    let message = match &c.gap_stats {
        Some(s) if s.count >= 4 => format!(
            "crystal seq: {:?} → {:?} (P={:.2}, PMI={:.2}, gap={:.3}±{:.3}s)",
            c.antecedent, c.consequent, c.conditional_prob, c.pmi,
            s.mean_secs, s.stddev_secs,
        ),
        _ => format!(
            "crystal: {:?} → {:?} (P={:.2}, PMI={:.2}, n={})",
            c.antecedent, c.consequent, c.conditional_prob, c.pmi, c.support
        ),
    };
    Rule {
        id: Ulid::new(),
        priority: 5,
        when,
        scope: Scope::default(),
        then: vec![Action::Log { level: LogLevel::Info, message }],
    }
}


// ============================================================================
// Patrones extendidos: Burst (alta frecuencia) y Silence (ausencia prolongada).
// Estos cristales son sobre un único kind, no pares — capturan dinámicas
// temporales de eventos individuales.
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatternCrystal {
    /// Mismo evento aparece con frecuencia alta. `frequency_per_sec` se
    /// estima sobre el window de observación.
    Burst {
        kind: EventKind,
        count: u64,
        frequency_per_sec: f64,
    },
    /// Evento que dejó de aparecer. `since_secs` es el tiempo desde la
    /// última observación.
    Silence {
        kind: EventKind,
        last_count: u64,
        since_secs: f64,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct PatternParams {
    /// Mínimo de ocurrencias para considerar Burst.
    pub burst_min_count: u64,
    /// Frecuencia mínima (eventos por segundo) para considerar Burst.
    pub burst_min_freq_hz: f64,
    /// Tiempo desde última ocurrencia para considerar Silence.
    pub silence_min_secs: f64,
    /// Mínimo total previo para considerar Silence (eventos < N son ruido).
    pub silence_min_prior_count: u64,
}

impl Default for PatternParams {
    fn default() -> Self {
        Self {
            burst_min_count: 10,
            burst_min_freq_hz: 5.0,
            silence_min_secs: 30.0,
            silence_min_prior_count: 3,
        }
    }
}

/// Detecta Bursts y Silences sobre la distribución marginal del observer.
/// La frecuencia de un Burst se aproxima asumiendo que la observación cubre
/// el rango entre `last_seen` y `Instant::now()` para ese kind.
pub fn detect_pattern_crystals(obs: &Observer, params: &PatternParams) -> Vec<PatternCrystal> {
    let mut out = Vec::new();
    let now = Instant::now();
    for (kind, &count) in obs.marginals() {
        let last_seen = obs.last_seen_marginal(kind);
        // ---- Burst ----
        if count >= params.burst_min_count {
            // Aproximación: si vimos `count` eventos hasta `last_seen`, y el
            // primer evento sucedió en algún momento del window, la freq es
            // count / window_age. Sin tiempo del primer evento, usamos
            // last_seen → now como denominador (subestima freq) o asumimos
            // ventana fija de 60s. Usamos la última como aproximación.
            let elapsed = last_seen
                .map(|t| now.saturating_duration_since(t).as_secs_f64().max(0.001))
                .unwrap_or(60.0);
            // Estimación conservadora: count / max(window_age, 1s).
            // Si tenemos histograma, podríamos refinar — TODO.
            let freq = count as f64 / elapsed.max(1.0);
            if freq >= params.burst_min_freq_hz {
                out.push(PatternCrystal::Burst {
                    kind: kind.clone(),
                    count,
                    frequency_per_sec: freq,
                });
            }
        }
        // ---- Silence ----
        if count >= params.silence_min_prior_count {
            if let Some(t) = last_seen {
                let since = now.saturating_duration_since(t).as_secs_f64();
                if since >= params.silence_min_secs {
                    out.push(PatternCrystal::Silence {
                        kind: kind.clone(),
                        last_count: count,
                        since_secs: since,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::EventKind::*;

    #[test]
    fn detects_perfect_correlation() {
        let mut obs = Observer::new(100);
        for _ in 0..10 {
            obs.record(EnteSpawned);
            obs.record(EnteDied);
        }
        let crystals = detect_crystals(&obs, &CrystallizationParams {
            min_support: 3,
            min_conditional_prob: 0.5,
            min_pmi: 0.0,
        });
        assert!(crystals.iter().any(|c| matches!(c.antecedent, EnteSpawned)
                                       && matches!(c.consequent, EnteDied)));
    }

    #[test]
    fn rejects_below_threshold() {
        let mut obs = Observer::new(100);
        // Sin co-ocurrencia significativa.
        for _ in 0..3 { obs.record(EnteSpawned); }
        let crystals = detect_crystals(&obs, &CrystallizationParams::default());
        assert!(crystals.is_empty(), "no debería haber cristales: {:?}", crystals);
    }
}
