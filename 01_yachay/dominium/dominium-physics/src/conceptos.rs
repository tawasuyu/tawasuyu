//! Aplicación de Conceptos sobre la grilla y sobre los Lemmings.
//!
//! Dos pasos puros, sin estado interno, recorriendo la `Vec<Concepto>` en
//! el orden de inserción. Determinista bit-exacto.
//!
//! - [`apply_conceptos`] — emite/drena los modificadores de cada concepto
//!   sobre las celdas dentro de su radio, con falloff lineal.
//! - [`apply_hacks`] — decrementa los locks vivos y, para los lemmings
//!   libres dentro de un radio con `hack` cuyo `trigger` se cumple, fuerza
//!   `accion` y arranca el lock.

use dominium_core::{Trigger, World};

/// Suma los modificadores de cada concepto a las celdas dentro de su radio,
/// con falloff lineal (1 en el centro, 0 en el borde).
///
/// Recorre los conceptos en orden de inserción y las celdas en orden
/// `(y, x)` para que la simulación sea bit-exacta plataforma a plataforma.
pub fn apply_conceptos(world: &mut World) {
    let w = world.grid.width;
    let h = world.grid.height;
    for c in &world.conceptos.items {
        if c.radius <= 0.0 {
            continue;
        }
        let r2 = c.radius * c.radius;
        // Ventana acotada de celdas a inspeccionar.
        let xmin = ((c.pos_x - c.radius).floor() as i64).max(0) as usize;
        let xmax_raw = ((c.pos_x + c.radius).ceil() as i64).max(0) as usize;
        let xmax = xmax_raw.min(w.saturating_sub(1));
        let ymin = ((c.pos_y - c.radius).floor() as i64).max(0) as usize;
        let ymax_raw = ((c.pos_y + c.radius).ceil() as i64).max(0) as usize;
        let ymax = ymax_raw.min(h.saturating_sub(1));
        if xmin >= w || ymin >= h {
            continue;
        }
        for cy in ymin..=ymax {
            for cx in xmin..=xmax {
                let dx = cx as f32 - c.pos_x;
                let dy = cy as f32 - c.pos_y;
                let d2 = dx * dx + dy * dy;
                if d2 > r2 {
                    continue;
                }
                let falloff = 1.0 - libm::sqrtf(d2 / r2);
                let idx = world.grid.idx(cx, cy);
                world.grid.materia[idx] += c.mods.materia * falloff;
                world.grid.psique[idx] += c.mods.psique * falloff;
                world.grid.poder[idx] += c.mods.poder * falloff;
                world.grid.oro[idx] += c.mods.oro * falloff;
            }
        }
    }
}

/// Decrementa los locks activos y arranca nuevos en los Lemmings que
/// caigan dentro del radio de un concepto con `hack` cuyo `trigger` se
/// cumple. La acción forzada vence cualquier transición posterior del
/// motor (incluida la desesperación → pelear).
///
/// Determinismo: orden `(lemming, concepto)` por índice; ante varios
/// conceptos que capturen al mismo lemming, gana el de menor índice.
pub fn apply_hacks(world: &mut World) {
    let n = world.lemmings.len();
    // 1. Decrementar locks vivos. El lemming sigue ejecutando la acción
    //    forzada porque su byte `accion` ya está fijado.
    for i in 0..n {
        if world.lemmings.hack_lock[i] > 0 {
            world.lemmings.hack_lock[i] -= 1;
        }
    }
    // 2. Capturar lemmings libres que entren al radio de un concepto.
    for i in 0..n {
        if world.lemmings.hack_lock[i] > 0 {
            continue;
        }
        for c in &world.conceptos.items {
            let Some(h) = &c.hack else { continue };
            let dx = world.lemmings.pos_x[i] - c.pos_x;
            let dy = world.lemmings.pos_y[i] - c.pos_y;
            if dx * dx + dy * dy > c.radius * c.radius {
                continue;
            }
            let fires = match h.trigger {
                Trigger::Always => true,
                Trigger::EnergiaBajo(e) => world.lemmings.energia[i] < e,
                Trigger::EdadSobre(a) => world.lemmings.edad[i] > a,
            };
            if fires {
                world.lemmings.accion[i] = h.forced_action;
                world.lemmings.hack_lock[i] = h.duration;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominium_core::{BehaviorHack, Concepto, LayerMods, Trigger, World};

    fn empty_world(w: usize, h: usize) -> World {
        World::new(w, h)
    }

    fn concepto(id: &str, x: f32, y: f32, r: f32, mods: LayerMods) -> Concepto {
        Concepto {
            id: id.into(),
            sprite_id: 0,
            pos_x: x,
            pos_y: y,
            radius: r,
            mods,
            hack: None,
        }
    }

    #[test]
    fn concepto_inyecta_psique_en_su_centro() {
        let mut w = empty_world(8, 8);
        w.conceptos.add(concepto(
            "iglesia",
            4.0,
            4.0,
            2.0,
            LayerMods { psique: 1.0, ..Default::default() },
        ));
        let center = w.grid.idx(4, 4);
        apply_conceptos(&mut w);
        assert!((w.grid.psique[center] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn falloff_decae_hacia_el_borde() {
        let mut w = empty_world(16, 16);
        w.conceptos.add(concepto(
            "fuente",
            8.0,
            8.0,
            4.0,
            LayerMods { materia: 1.0, ..Default::default() },
        ));
        apply_conceptos(&mut w);
        let center = w.grid.idx(8, 8);
        let halfway = w.grid.idx(10, 8);
        let edge = w.grid.idx(12, 8); // distancia 4 = radius → falloff = 0
        assert!(w.grid.materia[center] > w.grid.materia[halfway]);
        assert!(w.grid.materia[halfway] > 0.0);
        assert!(w.grid.materia[edge].abs() < 1e-5);
    }

    #[test]
    fn conceptos_no_afectan_celdas_fuera_del_radio() {
        let mut w = empty_world(20, 20);
        w.conceptos.add(concepto(
            "compacto",
            10.0,
            10.0,
            2.0,
            LayerMods { oro: 1.0, ..Default::default() },
        ));
        apply_conceptos(&mut w);
        let lejos = w.grid.idx(0, 0);
        assert!(w.grid.oro[lejos].abs() < 1e-6);
    }

    #[test]
    fn drenar_baja_el_campo() {
        let mut w = empty_world(8, 8);
        let center = w.grid.idx(4, 4);
        w.grid.materia[center] = 10.0;
        w.conceptos.add(concepto(
            "agujero",
            4.0,
            4.0,
            1.0,
            LayerMods { materia: -2.0, ..Default::default() },
        ));
        apply_conceptos(&mut w);
        assert!(w.grid.materia[center] < 10.0);
    }

    #[test]
    fn hack_captura_lemming_y_le_fija_accion() {
        let mut w = empty_world(20, 20);
        w.lemmings.spawn(10.0, 10.0, 30.0, [0.0; 4]);
        w.lemmings.accion[0] = 0; // Mover
        w.conceptos.add(Concepto {
            id: "iglesia".into(),
            sprite_id: 0,
            pos_x: 10.0,
            pos_y: 10.0,
            radius: 3.0,
            mods: LayerMods::default(),
            hack: Some(BehaviorHack {
                trigger: Trigger::Always,
                forced_action: 2, // Sincronizar
                duration: 10,
            }),
        });
        apply_hacks(&mut w);
        assert_eq!(w.lemmings.accion[0], 2);
        assert_eq!(w.lemmings.hack_lock[0], 10);
    }

    #[test]
    fn hack_con_trigger_no_cumplido_no_dispara() {
        let mut w = empty_world(20, 20);
        w.lemmings.spawn(10.0, 10.0, 30.0, [0.0; 4]); // energía 30
        w.lemmings.accion[0] = 0;
        w.conceptos.add(Concepto {
            id: "soup-kitchen".into(),
            sprite_id: 0,
            pos_x: 10.0,
            pos_y: 10.0,
            radius: 3.0,
            mods: LayerMods::default(),
            hack: Some(BehaviorHack {
                trigger: Trigger::EnergiaBajo(10.0),
                forced_action: 2,
                duration: 5,
            }),
        });
        apply_hacks(&mut w);
        assert_eq!(w.lemmings.accion[0], 0); // sigue moviéndose
        assert_eq!(w.lemmings.hack_lock[0], 0);
    }

    #[test]
    fn hack_lock_decrementa_y_no_resnatura_si_lock_vive() {
        let mut w = empty_world(20, 20);
        w.lemmings.spawn(10.0, 10.0, 30.0, [0.0; 4]);
        w.lemmings.accion[0] = 2;
        w.lemmings.hack_lock[0] = 3;
        // Concepto presente con hack — pero el lemming ya está locked.
        w.conceptos.add(Concepto {
            id: "x".into(),
            sprite_id: 0,
            pos_x: 10.0,
            pos_y: 10.0,
            radius: 3.0,
            mods: LayerMods::default(),
            hack: Some(BehaviorHack {
                trigger: Trigger::Always,
                forced_action: 5,
                duration: 10,
            }),
        });
        apply_hacks(&mut w);
        // El lock baja a 2, la acción se mantiene en 2 (no se re-evaluó).
        assert_eq!(w.lemmings.hack_lock[0], 2);
        assert_eq!(w.lemmings.accion[0], 2);
    }

    #[test]
    fn primer_concepto_gana_si_dos_capturan_al_mismo_lemming() {
        let mut w = empty_world(20, 20);
        w.lemmings.spawn(10.0, 10.0, 30.0, [0.0; 4]);
        let mk = |id: &str, action: u8| Concepto {
            id: id.into(),
            sprite_id: 0,
            pos_x: 10.0,
            pos_y: 10.0,
            radius: 5.0,
            mods: LayerMods::default(),
            hack: Some(BehaviorHack {
                trigger: Trigger::Always,
                forced_action: action,
                duration: 7,
            }),
        };
        w.conceptos.add(mk("a", 3));
        w.conceptos.add(mk("b", 5));
        apply_hacks(&mut w);
        assert_eq!(w.lemmings.accion[0], 3, "gana el primero por índice");
    }
}
