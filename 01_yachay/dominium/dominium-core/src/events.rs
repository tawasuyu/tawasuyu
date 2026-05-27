//! Eventos discretos — Fase D.1 del simulador.
//!
//! Hasta ahora el mundo evolucionaba sólo por su dinámica interna
//! (difusión, agentes, Conceptos estáticos). Los eventos discretos son
//! **perturbaciones puntuales** que el experimentador inyecta en ticks
//! específicos para medir la respuesta poblacional: una sequía, una
//! noticia, una pandemia mental.
//!
//! Cada `Event` lleva el `tick` exacto en el que se dispara. El CLI carga
//! una *timeline* JSON (lista ordenada de eventos) y antes de cada
//! `tick()` aplica los que coinciden con el reloj global.
//!
//! Determinismo: la aplicación es lineal (sin random), los eventos se
//! procesan en orden de aparición en la lista. Mismas listas en x86 y
//! ARM → mismas trayectorias bit-exactas.

use crate::lemmings::{PSI_CORRUPTIBILIDAD, PSI_CURIOSIDAD, PSI_MIEDO, PSI_ORDEN};
use crate::world::World;
use serde::{Deserialize, Serialize};

/// Identificador semántico de una capa del Sustrato. Se serializa como
/// string (`"materia"`, `"psique"`, …) para que las timelines JSON sean
/// legibles a ojo, no como bytes opacos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayerId {
    Materia,
    Psique,
    Poder,
    Oro,
    Degradacion,
}

/// Variantes de evento. Diseñadas para ser ortogonales: cada una toca
/// exactamente un eje del mundo (capa de grilla, vector_psi de agentes, o
/// la lista de agentes mismos). Se evita el "mega-evento" porque rompe
/// la composabilidad.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum EventKind {
    /// Suma `amount` (con falloff lineal) a la capa indicada en una región
    /// circular. `amount` puede ser negativo (drenaje). Modela: sequía,
    /// descubrimiento de oro, plaga sobre la materia, contaminación.
    Shock {
        layer: LayerId,
        x: f32,
        y: f32,
        radius: f32,
        amount: f32,
    },
    /// Suma un delta a `vector_psi` de los agentes en una región circular,
    /// con falloff lineal en el centro→borde. Modela: noticia, manifiesto,
    /// shock cultural. Cero efecto sobre la grilla.
    PsiNudge {
        x: f32,
        y: f32,
        radius: f32,
        delta_psi: [f32; 4],
    },
}

/// Un evento etiquetado con el tick en que debe dispararse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Reloj global (`World::tick_count`) en el que se aplica.
    pub tick: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

/// Aplica un único evento al mundo. Funcionalmente puro respecto del tick
/// (no consulta `world.tick_count` — quién llame decide *cuándo* lo aplica
/// según su propia política).
pub fn apply_event(world: &mut World, ev: &EventKind) {
    match ev {
        EventKind::Shock { layer, x, y, radius, amount } => {
            apply_shock_on_layer(world, *layer, *x, *y, *radius, *amount);
        }
        EventKind::PsiNudge { x, y, radius, delta_psi } => {
            apply_psi_nudge(world, *x, *y, *radius, *delta_psi);
        }
    }
}

fn apply_shock_on_layer(
    world: &mut World,
    layer: LayerId,
    x: f32,
    y: f32,
    radius: f32,
    amount: f32,
) {
    if radius <= 0.0 {
        return;
    }
    let r2 = radius * radius;
    let w = world.grid.width;
    let h = world.grid.height;
    let xmin = ((x - radius).floor() as i64).max(0) as usize;
    let xmax_raw = ((x + radius).ceil() as i64).max(0) as usize;
    let xmax = xmax_raw.min(w.saturating_sub(1));
    let ymin = ((y - radius).floor() as i64).max(0) as usize;
    let ymax_raw = ((y + radius).ceil() as i64).max(0) as usize;
    let ymax = ymax_raw.min(h.saturating_sub(1));
    if xmin >= w || ymin >= h {
        return;
    }
    for cy in ymin..=ymax {
        for cx in xmin..=xmax {
            let dx = cx as f32 - x;
            let dy = cy as f32 - y;
            let d2 = dx * dx + dy * dy;
            if d2 > r2 {
                continue;
            }
            let falloff = 1.0 - libm::sqrtf(d2 / r2);
            let idx = world.grid.idx(cx, cy);
            let delta = amount * falloff;
            match layer {
                LayerId::Materia => world.grid.materia[idx] += delta,
                LayerId::Psique => world.grid.psique[idx] += delta,
                LayerId::Poder => world.grid.poder[idx] += delta,
                LayerId::Oro => world.grid.oro[idx] += delta,
                LayerId::Degradacion => world.grid.degradacion[idx] += delta,
            }
        }
    }
}

fn apply_psi_nudge(world: &mut World, x: f32, y: f32, radius: f32, delta: [f32; 4]) {
    if radius <= 0.0 {
        return;
    }
    let r2 = radius * radius;
    for i in 0..world.lemmings.len() {
        let dx = world.lemmings.pos_x[i] - x;
        let dy = world.lemmings.pos_y[i] - y;
        let d2 = dx * dx + dy * dy;
        if d2 > r2 {
            continue;
        }
        let falloff = 1.0 - libm::sqrtf(d2 / r2);
        let psi = &mut world.lemmings.vector_psi[i];
        psi[PSI_ORDEN] += delta[PSI_ORDEN] * falloff;
        psi[PSI_MIEDO] += delta[PSI_MIEDO] * falloff;
        psi[PSI_CURIOSIDAD] += delta[PSI_CURIOSIDAD] * falloff;
        psi[PSI_CORRUPTIBILIDAD] += delta[PSI_CORRUPTIBILIDAD] * falloff;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shock_materia_inyecta_y_falloff_lineal() {
        let mut w = World::new(20, 20);
        apply_event(
            &mut w,
            &EventKind::Shock {
                layer: LayerId::Materia,
                x: 10.0,
                y: 10.0,
                radius: 4.0,
                amount: 100.0,
            },
        );
        let center = w.grid.idx(10, 10);
        let halfway = w.grid.idx(12, 10);
        let edge = w.grid.idx(14, 10); // distancia 4 = radius → falloff 0
        assert!((w.grid.materia[center] - 100.0).abs() < 1e-4);
        assert!(w.grid.materia[halfway] > 0.0);
        assert!(w.grid.materia[halfway] < 100.0);
        assert!(w.grid.materia[edge].abs() < 1e-5);
    }

    #[test]
    fn shock_negativo_drena() {
        let mut w = World::new(8, 8);
        for c in w.grid.materia.iter_mut() {
            *c = 50.0;
        }
        apply_event(
            &mut w,
            &EventKind::Shock {
                layer: LayerId::Materia,
                x: 4.0,
                y: 4.0,
                radius: 2.0,
                amount: -30.0,
            },
        );
        let center = w.grid.idx(4, 4);
        assert!(
            (w.grid.materia[center] - 20.0).abs() < 1e-4,
            "drenó {} en lugar de 30",
            50.0 - w.grid.materia[center]
        );
    }

    #[test]
    fn psi_nudge_empuja_vector_psi_de_agentes_en_radio() {
        let mut w = World::new(20, 20);
        w.lemmings.spawn(10.0, 10.0, 30.0, [0.0; 4]); // dentro
        w.lemmings.spawn(0.0, 0.0, 30.0, [0.5; 4]); // afuera
        let psi_pre_outside = w.lemmings.vector_psi[1];
        apply_event(
            &mut w,
            &EventKind::PsiNudge {
                x: 10.0,
                y: 10.0,
                radius: 5.0,
                delta_psi: [0.3, 0.0, 0.0, 0.0],
            },
        );
        // Agente en el centro: falloff = 1, psi[0] sube 0.3.
        assert!((w.lemmings.vector_psi[0][0] - 0.3).abs() < 1e-5);
        // Agente fuera del radio: sin cambios.
        assert_eq!(w.lemmings.vector_psi[1], psi_pre_outside);
    }

    #[test]
    fn timeline_json_roundtrip() {
        let events = vec![
            Event {
                tick: 50,
                kind: EventKind::Shock {
                    layer: LayerId::Materia,
                    x: 10.0,
                    y: 10.0,
                    radius: 5.0,
                    amount: -100.0,
                },
            },
            Event {
                tick: 100,
                kind: EventKind::PsiNudge {
                    x: 20.0,
                    y: 20.0,
                    radius: 8.0,
                    delta_psi: [0.0, 0.5, 0.0, 0.0],
                },
            },
        ];
        let s = serde_json::to_string(&events).expect("serializa");
        let back: Vec<Event> = serde_json::from_str(&s).expect("deserializa");
        assert_eq!(events, back);
    }
}
