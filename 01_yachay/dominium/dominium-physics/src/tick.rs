//! El ciclo del motor — un `tick` completo de la simulación.
//!
//! Orden fijo: difusión/entropía → evaluación de transiciones → acciones
//! de los agentes → envejecimiento y cosecha de muertos.

use crate::conceptos::{apply_conceptos, apply_hacks};
use crate::diffuse::diffuse;
use dominium_core::{SimParams, World};

/// Evaluación de transiciones: un agente exhausto se fuerza a `Pelear`.
/// Un lemming bajo `hack_lock` está blindado: su acción ya está fijada por
/// un Concepto y no debe re-evaluarse hasta que el lock se agote.
fn apply_transitions(world: &mut World, p: &SimParams) {
    for i in 0..world.lemmings.len() {
        if world.lemmings.hack_lock[i] > 0 {
            continue;
        }
        if world.lemmings.energia[i] < p.desperation_threshold {
            world.lemmings.accion[i] = 5; // Degradar (Pelear)
        }
    }
}

/// Envejece a los agentes y cosecha a los muertos: la energía remanente
/// de un agente que muere se inyecta como fertilidad (`materia`) en su
/// celda. Devuelve cuántos murieron.
fn age_and_reap(world: &mut World, p: &SimParams) -> usize {
    for e in world.lemmings.edad.iter_mut() {
        *e += 1;
    }
    // Recolecta los índices muertos (energía agotada o edad excedida).
    let mut dead: Vec<usize> = (0..world.lemmings.len())
        .filter(|&i| {
            world.lemmings.energia[i] <= 0.0 || world.lemmings.edad[i] > p.max_edad
        })
        .collect();
    // Remueve de mayor a menor índice: swap_remove no invalida los menores.
    dead.sort_unstable_by(|a, b| b.cmp(a));
    let count = dead.len();
    for i in dead {
        let (cx, cy) = world
            .grid
            .clamp_cell(world.lemmings.pos_x[i], world.lemmings.pos_y[i]);
        let idx = world.grid.idx(cx, cy);
        // La energía remanente vuelve a la tierra como biomasa.
        let remnant = world.lemmings.energia[i].max(0.0);
        world.grid.materia[idx] += remnant;
        world.lemmings.remove(i);
    }
    count
}

/// Un paso completo de la simulación.
pub fn tick(world: &mut World, p: &SimParams) {
    // 1. Emisión/drenaje por Conceptos sobre las celdas (con falloff lineal).
    //    Va antes de la difusión para que las inyecciones se propaguen este tick.
    apply_conceptos(world);
    // 2. Difusión y entropía sobre los campos.
    diffuse(&mut world.grid, p);
    // 3. Transiciones de estado forzadas (desesperación → pelear).
    apply_transitions(world, p);
    // 4. Captura de acción por Conceptos. Vence cualquier transición previa:
    //    el `hack_lock` blindará al lemming hasta agotar su duración.
    apply_hacks(world);
    // 5. Acciones de los agentes. Se fija `n` antes del loop: los hijos
    //    que `Replicar` agrega al final NO actúan este tick.
    let n = world.lemmings.len();
    for i in 0..n {
        if i < world.lemmings.len() {
            world.step_lemming(i, p);
        }
    }
    // 6. Envejecer + cosechar muertos.
    age_and_reap(world, p);
}

/// Corre `steps` ticks seguidos.
pub fn run(world: &mut World, p: &SimParams, steps: usize) {
    for _ in 0..steps {
        tick(world, p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_agent_is_forced_to_fight() {
        let mut w = World::new(8, 8);
        w.lemmings.spawn(4.0, 4.0, 1.0, [0.0; 4]); // energía 1 < umbral 5
        w.lemmings.accion[0] = 0; // Mover
        let p = SimParams::default();
        apply_transitions(&mut w, &p);
        assert_eq!(w.lemmings.accion[0], 5); // forzado a Degradar
    }

    #[test]
    fn dead_agent_returns_energy_as_materia() {
        let mut w = World::new(8, 8);
        // Agente sin energía → muere este tick.
        w.lemmings.spawn(4.0, 4.0, 0.0, [0.0; 4]);
        let idx = w.grid.idx(4, 4);
        let p = SimParams::default();
        let reaped = age_and_reap(&mut w, &p);
        assert_eq!(reaped, 1);
        assert_eq!(w.lemmings.len(), 0);
        // (energía remanente 0 → materia no sube, pero no panickea)
        assert!(w.grid.materia[idx] >= 0.0);
    }

    #[test]
    fn tick_runs_without_panicking_on_a_populated_world() {
        let mut w = World::new(32, 32);
        for k in 0..20 {
            let x = (k % 8) as f32 + 2.0;
            let y = (k / 8) as f32 + 2.0;
            w.lemmings.spawn(x, y, 30.0, [1.0, 0.2, 0.5, 0.1]);
            w.lemmings.accion[k] = (k % 6) as u8;
        }
        // Sembrar algo de materia.
        for c in w.grid.materia.iter_mut() {
            *c = 5.0;
        }
        let p = SimParams::default();
        run(&mut w, &p, 50);
        // La sim avanzó 50 ticks sin romperse.
        assert!(w.lemmings.edad.iter().all(|&e| e <= 50));
    }

    #[test]
    fn run_is_deterministic() {
        let build = || {
            let mut w = World::new(16, 16);
            for k in 0..10 {
                w.lemmings.spawn(3.0 + k as f32, 8.0, 40.0, [1.0, 0.0, 0.3, 0.0]);
                w.lemmings.accion[k] = (k % 6) as u8;
            }
            for c in w.grid.materia.iter_mut() {
                *c = 3.0;
            }
            w
        };
        let p = SimParams::default();
        let mut a = build();
        let mut b = build();
        run(&mut a, &p, 30);
        run(&mut b, &p, 30);
        // Mismo input → mismo estado, bit a bit.
        assert_eq!(a.lemmings.pos_x, b.lemmings.pos_x);
        assert_eq!(a.lemmings.energia, b.lemmings.energia);
        assert_eq!(a.grid.materia, b.grid.materia);
    }
}
