//! Sistema GR (García Rosas) — detección de *triggers* de rectificación.
//!
//! Un trigger GR es un cuerpo natal proyectado por dirección primaria
//! —directa o conversa— que cae cerca de un punto natal. La
//! rectificación horaria se valida observando estos contactos: un
//! evento real de la vida del sujeto debe coincidir con un trigger
//! ajustado si la hora natal es correcta.
//!
//! Cuando un mismo punto natal recibe a la vez un trigger directo y
//! otro converso dentro del micro-orbe de evento, hay una
//! **convergencia GR**: la señal fuerte de rectificación.
//!
//! Esta lógica es pura: el engine computa las longitudes dirigidas
//! (eso sí necesita `eternal-astrology`) y delega aquí el
//! emparejamiento contra los puntos natales. Así la parte que define
//! *qué cuenta como trigger* vive en un crate liviano y testeable.

use serde::{Deserialize, Serialize};

/// Dirección de una proyección primaria del Sistema GR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrDirection {
    /// Directa — rotación diurna hacia adelante en el tiempo.
    Direct,
    /// Conversa — rotación diurna inversa.
    Converse,
}

impl GrDirection {
    /// Etiqueta de una letra para el HUD (`D` / `C`).
    pub fn short(self) -> &'static str {
        match self {
            GrDirection::Direct => "D",
            GrDirection::Converse => "C",
        }
    }

    /// Etiqueta legible.
    pub fn label(self) -> &'static str {
        match self {
            GrDirection::Direct => "directa",
            GrDirection::Converse => "conversa",
        }
    }
}

/// Un contacto del Sistema GR: un cuerpo promisor dirigido que cae
/// cerca de un punto natal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrTrigger {
    /// Símbolo del cuerpo promisor (el que se dirige). Ej. `"mars"`.
    pub promissor: String,
    /// Si la proyección es directa o conversa.
    pub direction: GrDirection,
    /// Punto natal contactado: símbolo de cuerpo (`"sun"`) o ángulo
    /// (`"asc"`, `"mc"`, `"desc"`, `"ic"`).
    pub natal_target: String,
    /// Longitud eclíptica [0,360) del punto natal contactado.
    pub natal_deg: f32,
    /// Longitud eclíptica [0,360) donde cayó el promisor dirigido.
    pub directed_deg: f32,
    /// Orbe absoluto del contacto, en grados (separación circular).
    pub orb_deg: f32,
    /// `true` si el trigger forma parte de una convergencia GR
    /// (directo + converso sobre el mismo punto natal, ambos dentro
    /// del micro-orbe de evento). La UI lo resalta.
    #[serde(default)]
    pub event: bool,
}

/// Separación circular mínima entre dos longitudes eclípticas, en
/// grados (rango `0..=180`).
fn circular_sep(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// Empareja cada posición dirigida contra cada punto natal y produce
/// la lista de triggers GR.
///
/// - `directed`: `(promisor, dirección, longitud_dirigida)`.
/// - `natal_targets`: `(nombre, longitud_natal)`.
/// - `hud_orb_deg`: orbe máximo para que un contacto entre a la lista.
/// - `event_orb_deg`: micro-orbe de convergencia (ver [`mark_events`]).
/// - `max_triggers`: tope de la lista tras ordenar por orbe.
///
/// El resultado va ordenado por `orb_deg` ascendente (los contactos
/// más cerrados primero) y truncado a `max_triggers`.
pub fn compute_gr_triggers(
    directed: &[(String, GrDirection, f32)],
    natal_targets: &[(String, f32)],
    hud_orb_deg: f32,
    event_orb_deg: f32,
    max_triggers: usize,
) -> Vec<GrTrigger> {
    let mut triggers = Vec::new();
    for (promissor, direction, raw_directed) in directed {
        let directed_deg = raw_directed.rem_euclid(360.0);
        for (name, raw_natal) in natal_targets {
            let natal_deg = raw_natal.rem_euclid(360.0);
            let orb = circular_sep(directed_deg, natal_deg);
            if orb <= hud_orb_deg {
                triggers.push(GrTrigger {
                    promissor: promissor.clone(),
                    direction: *direction,
                    natal_target: name.clone(),
                    natal_deg,
                    directed_deg,
                    orb_deg: orb,
                    event: false,
                });
            }
        }
    }

    mark_events(&mut triggers, event_orb_deg);

    triggers.sort_by(|a, b| {
        a.orb_deg
            .partial_cmp(&b.orb_deg)
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    triggers.truncate(max_triggers);
    triggers
}

/// Marca como `event` los triggers que forman una convergencia GR: un
/// mismo punto natal tocado por un trigger directo y otro converso,
/// ambos dentro de `event_orb_deg`.
fn mark_events(triggers: &mut [GrTrigger], event_orb_deg: f32) {
    use std::collections::HashSet;
    let mut has_direct: HashSet<String> = HashSet::new();
    let mut has_converse: HashSet<String> = HashSet::new();
    for t in triggers.iter() {
        if t.orb_deg <= event_orb_deg {
            match t.direction {
                GrDirection::Direct => {
                    has_direct.insert(t.natal_target.clone());
                }
                GrDirection::Converse => {
                    has_converse.insert(t.natal_target.clone());
                }
            }
        }
    }
    for t in triggers.iter_mut() {
        if t.orb_deg <= event_orb_deg
            && has_direct.contains(&t.natal_target)
            && has_converse.contains(&t.natal_target)
        {
            t.event = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(promissor: &str, dir: GrDirection, deg: f32) -> (String, GrDirection, f32) {
        (promissor.to_string(), dir, deg)
    }

    #[test]
    fn contact_within_hud_orb_becomes_a_trigger() {
        let directed = vec![d("mars", GrDirection::Direct, 101.5)];
        let targets = vec![("sun".to_string(), 100.0)];
        let out = compute_gr_triggers(&directed, &targets, 2.0, 0.083, 60);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].promissor, "mars");
        assert_eq!(out[0].natal_target, "sun");
        assert!((out[0].orb_deg - 1.5).abs() < 1e-3);
        assert!(!out[0].event);
    }

    #[test]
    fn contact_beyond_hud_orb_is_dropped() {
        let directed = vec![d("mars", GrDirection::Direct, 103.0)];
        let targets = vec![("sun".to_string(), 100.0)];
        assert!(compute_gr_triggers(&directed, &targets, 2.0, 0.083, 60).is_empty());
    }

    #[test]
    fn direct_and_converse_within_micro_orb_form_an_event() {
        // Marte directo y Venus converso, ambos sobre el Sol natal a
        // <5' de orbe: convergencia GR.
        let directed = vec![
            d("mars", GrDirection::Direct, 100.04),
            d("venus", GrDirection::Converse, 99.97),
        ];
        let targets = vec![("sun".to_string(), 100.0)];
        let out = compute_gr_triggers(&directed, &targets, 2.0, 5.0 / 60.0, 60);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|t| t.event), "ambos triggers son evento");
    }

    #[test]
    fn lone_direct_within_micro_orb_is_not_an_event() {
        // Un solo toque directo, sin converso: no hay convergencia.
        let directed = vec![
            d("mars", GrDirection::Direct, 100.02),
            d("venus", GrDirection::Direct, 99.98),
        ];
        let targets = vec![("sun".to_string(), 100.0)];
        let out = compute_gr_triggers(&directed, &targets, 2.0, 5.0 / 60.0, 60);
        assert!(out.iter().all(|t| !t.event), "sin converso no hay evento");
    }

    #[test]
    fn converging_pair_must_share_the_same_natal_target() {
        // Directo sobre el Sol, converso sobre la Luna: no convergen.
        let directed = vec![
            d("mars", GrDirection::Direct, 100.01),
            d("venus", GrDirection::Converse, 200.01),
        ];
        let targets = vec![("sun".to_string(), 100.0), ("moon".to_string(), 200.0)];
        let out = compute_gr_triggers(&directed, &targets, 2.0, 5.0 / 60.0, 60);
        assert!(out.iter().all(|t| !t.event));
    }

    #[test]
    fn results_are_sorted_by_orb_and_capped() {
        let directed = vec![
            d("a", GrDirection::Direct, 101.8),
            d("b", GrDirection::Direct, 100.3),
            d("c", GrDirection::Direct, 101.0),
        ];
        let targets = vec![("sun".to_string(), 100.0)];
        let out = compute_gr_triggers(&directed, &targets, 2.0, 0.083, 2);
        assert_eq!(out.len(), 2, "truncado a max_triggers");
        assert!(out[0].orb_deg <= out[1].orb_deg, "ordenado por orbe");
        assert_eq!(out[0].promissor, "b", "el más cerrado primero");
    }

    #[test]
    fn circular_sep_handles_wraparound() {
        // 359° y 1° están a 2°, no a 358°.
        let directed = vec![d("mars", GrDirection::Direct, 359.0)];
        let targets = vec![("asc".to_string(), 1.0)];
        let out = compute_gr_triggers(&directed, &targets, 3.0, 0.083, 60);
        assert_eq!(out.len(), 1);
        assert!((out[0].orb_deg - 2.0).abs() < 1e-3);
    }
}
