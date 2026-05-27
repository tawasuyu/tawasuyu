//! El ciclo del motor — un `tick` completo de la simulación.
//!
//! Orden fijo: difusión/entropía → evaluación de transiciones → acciones
//! de los agentes → envejecimiento y cosecha de muertos.

use crate::conceptos::{apply_conceptos, apply_hacks};
use crate::diffuse::{diffuse_with, regrow_materia};
use dominium_core::{select_action_argmax, ActionPolicy, SimParams, World};

/// Reelige la `accion` base de los lemmings libres según la política
/// psicológica. Cero costo cuando la política es `Fixed` o el periodo es 0
/// — el motor histórico no paga nada por esta fase.
///
/// Agentes capturados por un Concepto (`hack_lock > 0`) quedan blindados:
/// la captura externa siempre vence a la reelección psicológica. La
/// transición de desesperación (energía baja → pelear) se aplica *después*
/// de esta función, así que la supervivencia también vence a la psicología.
fn apply_psi_policy(world: &mut World, p: &SimParams) {
    if !matches!(p.action_policy, ActionPolicy::PsiArgmax) {
        return;
    }
    if p.policy_reeval_period == 0 {
        return;
    }
    // Reelige sólo en los ticks que son múltiplos del período. El reloj
    // global `tick_count` se incrementa al *final* de cada tick, así que
    // en el primer tick (tick_count == 0) la fase se ejecuta — eso es
    // intencional: deja a la psicología decidir antes de que la simulación
    // arranque a inercia.
    if (world.tick_count % p.policy_reeval_period as u64) != 0 {
        return;
    }
    let weights = &p.action_weights;
    for i in 0..world.lemmings.len() {
        if world.lemmings.hack_lock[i] > 0 {
            continue;
        }
        let psi = world.lemmings.vector_psi[i];
        world.lemmings.accion[i] = select_action_argmax(&psi, weights);
    }
}

/// Evaluación de transiciones: un agente exhausto se fuerza a `Pelear`.
/// Un lemming bajo `hack_lock` está blindado: su acción ya está fijada por
/// un Concepto y no debe re-evaluarse hasta que el lock se agote.
///
/// Nota: la **abundancia** NO transiciona la acción base — eso convertiría
/// a los Extractores en Replicadores y secaría la fuente de energía del
/// sistema. En su lugar, la reproducción por abundancia se ejecuta como
/// *efecto colateral* dentro de `step_lemming` (ver `World::step_lemming`),
/// preservando la división del trabajo.
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
    // Costo metabólico basal: drena energía de TODOS los lemmings por
    // el simple hecho de estar vivos. Es el freno termodinámico que
    // estabiliza la población — sin él, los Extractores acumulan E sin
    // techo y la natalidad se descontrola.
    if p.metabolic_cost > 0.0 {
        for e in world.lemmings.energia.iter_mut() {
            *e -= p.metabolic_cost;
        }
    }
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
    // 2. Difusión y entropía sobre los campos — moduladas por el factor
    //    estacional del tick actual. Con season_period == 0 el factor es 1.0
    //    y la fase es bit-exactamente equivalente al motor sin estaciones.
    let season = p.season_factor(world.tick_count);
    diffuse_with(
        &mut world.grid,
        p.diffusion_rate * season,
        p.entropy_rate * season,
    );
    // 2b. Regrowth logístico de materia — cierre termodinámico que evita
    //     la extinción. Sub-fase del paso 2, no agrega fase nueva al §1.5.
    regrow_materia(&mut world.grid, p.regrowth_rate, p.carrying_capacity);
    // 2c. Política psicológica de acción (opt-in vía `ActionPolicy::PsiArgmax`).
    //     Reelige `accion` por argmax(W · psi) para lemmings libres. Sub-fase
    //     de la 2 — corre antes de las transiciones y los hacks, así la
    //     desesperación y la captura siempre ganan a la psicología tranquila.
    apply_psi_policy(world, p);
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
    // 7. Avanzar el reloj global — alimentación del ciclo estacional del
    //    próximo tick. Saturating para no entrar en UB en simulaciones
    //    eternas (~5.8e8 años a 1 tick/ns; suficiente).
    world.tick_count = world.tick_count.saturating_add(1);
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
    fn abundance_keeps_base_action_intact() {
        // La abundancia NO transiciona la acción base. Un Extractor saciado
        // sigue siendo Extractor — su rol funcional se preserva. La
        // reproducción ocurre como side-effect en step_lemming.
        let mut w = World::new(8, 8);
        w.lemmings.spawn(4.0, 4.0, 200.0, [0.0; 4]);
        w.lemmings.accion[0] = 1; // Extraer
        let p = SimParams::default();
        apply_transitions(&mut w, &p);
        assert_eq!(w.lemmings.accion[0], 1, "Extractor sigue extrayendo");
    }

    #[test]
    fn abundance_side_effect_spawns_child_via_step_lemming() {
        // Un Extractor con E > abundance_threshold replica como bonus
        // dentro de step_lemming, y luego ejecuta act_extraer normalmente.
        let mut w = World::new(8, 8);
        let idx = w.grid.idx(4, 4);
        w.grid.materia[idx] = 50.0;
        w.lemmings.spawn(4.0, 4.0, 200.0, [0.0; 4]);
        w.lemmings.accion[0] = 1; // Extraer
        let p = SimParams::default(); // abundance_threshold = 60
        let n_before = w.lemmings.len();
        let materia_before = w.grid.materia[idx];
        w.step_lemming(0, &p);
        // Replicó (hay hijo nuevo)
        assert_eq!(w.lemmings.len(), n_before + 1);
        // Y también extrajo (la materia bajó)
        assert!(w.grid.materia[idx] < materia_before);
    }

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
    fn tick_count_advances_one_per_step() {
        let mut w = World::new(4, 4);
        let p = SimParams::default();
        assert_eq!(w.tick_count, 0);
        tick(&mut w, &p);
        assert_eq!(w.tick_count, 1);
        run(&mut w, &p, 9);
        assert_eq!(w.tick_count, 10);
    }

    #[test]
    fn seasons_modulate_entropy_decay() {
        // Mismo campo uniforme + dos params: uno con estaciones que arrancan
        // en pico de verano (sin(π/2)=1, factor 1+amp), otro sin estaciones.
        // El de verano debe perder más por entropía en el primer tick.
        let mut a = World::new(4, 4);
        let mut b = World::new(4, 4);
        for v in a.grid.psique.iter_mut() {
            *v = 10.0;
        }
        for v in b.grid.psique.iter_mut() {
            *v = 10.0;
        }
        // Arrancamos en t=0; con period=4 el primer tick muestrea sin(0)=0 →
        // factor 1. Ajusto: empujamos el reloj al tick que muestrea el pico.
        let mut hot = SimParams::default();
        hot.season_period = 4;
        hot.season_amplitude = 0.5;
        a.tick_count = 1; // sin(2π·1/4) = sin(π/2) = 1 → factor 1.5
        let cold = SimParams::default();
        tick(&mut a, &hot);
        tick(&mut b, &cold);
        let avg_a: f32 = a.grid.psique.iter().sum::<f32>() / 16.0;
        let avg_b: f32 = b.grid.psique.iter().sum::<f32>() / 16.0;
        assert!(
            avg_a < avg_b,
            "el de verano debe perder más entropía: a={avg_a} b={avg_b}"
        );
    }

    #[test]
    fn seasons_disabled_by_default_keeps_old_behavior() {
        // Garantiza que el cambio de tick no movió el comportamiento default.
        let build = || {
            let mut w = World::new(8, 8);
            for c in w.grid.materia.iter_mut() {
                *c = 7.0;
            }
            for k in 0..5 {
                w.lemmings.spawn(2.0 + k as f32, 4.0, 30.0, [0.5, 0.0, 0.0, 0.0]);
            }
            w
        };
        let p = SimParams::default();
        let mut a = build();
        let mut b = build();
        run(&mut a, &p, 20);
        run(&mut b, &p, 20);
        assert_eq!(a.grid.materia, b.grid.materia);
        assert_eq!(a.lemmings.energia, b.lemmings.energia);
    }

    #[test]
    fn psi_policy_fixed_default_keeps_accion_intact() {
        // ActionPolicy::Fixed (default) NO debe tocar la `accion` aunque
        // la fase 2c esté presente en el tick. Aislamos el efecto desactivando
        // metabolic_cost (para que `desperation_threshold` no aplique) y
        // abundance (para que no haya replicación lateral). Excluimos
        // `Replicar` y `Degradar` del set probado porque consumen energía
        // propia / ajena y desestabilizan el test.
        let mut w = World::new(8, 8);
        for c in w.grid.materia.iter_mut() { *c = 5.0; }
        // Agentes con accion 0,1,2,3: Mover/Extraer/Sincronizar/Intercambiar.
        for k in 0..4u8 {
            let i = w.lemmings.spawn(4.0, 4.0, 200.0, [0.5; 4]);
            w.lemmings.accion[i] = k;
        }
        let mut p = SimParams::default();
        p.metabolic_cost = 0.0;
        p.abundance_threshold = 0.0;
        let acciones_antes = w.lemmings.accion.clone();
        run(&mut w, &p, 5);
        assert_eq!(w.lemmings.accion, acciones_antes);
    }

    #[test]
    fn psi_policy_argmax_reasigns_accion_segun_psi() {
        use dominium_core::ActionPolicy;
        // Tres agentes con psi extremos:
        // - psi=CORRUPTIBILIDAD → Degradar (5)
        // - psi=ORDEN → Intercambiar (3) por tie-break
        // - psi=CURIOSIDAD → Mover (0) por tie-break
        let mut w = World::new(8, 8);
        for c in w.grid.materia.iter_mut() { *c = 5.0; }
        w.lemmings.spawn(4.0, 4.0, 50.0, [0.0, 0.0, 0.0, 1.0]);
        w.lemmings.spawn(4.0, 4.0, 50.0, [1.0, 0.0, 0.0, 0.0]);
        w.lemmings.spawn(4.0, 4.0, 50.0, [0.0, 0.0, 1.0, 0.0]);
        // Acción inicial random (que NO coincide con lo esperado).
        w.lemmings.accion[0] = 0;
        w.lemmings.accion[1] = 0;
        w.lemmings.accion[2] = 5;
        let mut p = SimParams::default();
        p.action_policy = ActionPolicy::PsiArgmax;
        p.policy_reeval_period = 1; // reelige cada tick
        // Forzamos modulación 0 para que las acciones no cambien psi de paso
        // y el test mida sólo la reelección.
        p.psi_effect_modulation = 0.0;
        // Un solo tick basta: apply_psi_policy corre antes de step_lemming.
        tick(&mut w, &p);
        assert_eq!(w.lemmings.accion[0], 5, "corrupto → Degradar");
        assert_eq!(w.lemmings.accion[1], 3, "ordenado → Intercambiar (tie-break)");
        assert_eq!(w.lemmings.accion[2], 0, "curioso → Mover (tie-break)");
    }

    #[test]
    fn psi_policy_argmax_respeta_hack_lock() {
        use dominium_core::ActionPolicy;
        // Un agente bajo hack_lock no debe ser reelegido por psi.
        let mut w = World::new(8, 8);
        w.lemmings.spawn(4.0, 4.0, 50.0, [0.0, 0.0, 0.0, 1.0]); // psi → Degradar
        w.lemmings.accion[0] = 2; // pero está sincronizando bajo captura
        w.lemmings.hack_lock[0] = 50;
        let mut p = SimParams::default();
        p.action_policy = ActionPolicy::PsiArgmax;
        p.policy_reeval_period = 1;
        tick(&mut w, &p);
        // Sigue sincronizando: el hack_lock blinda contra la reelección psi.
        assert_eq!(w.lemmings.accion[0], 2);
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
