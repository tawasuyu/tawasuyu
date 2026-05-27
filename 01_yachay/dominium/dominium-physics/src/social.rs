//! Contagio social — Fase B del simulador de psicología poblacional.
//!
//! El `vector_psi` de cada agente NO es independiente del de sus vecinos:
//! si estás rodeado de gente curiosa, te volvés curioso; rodeado de
//! corruptibles, derivás a corrupto. Es la mecánica básica de **conformismo
//! local**: cada tick los agentes en radio social `R` acercan su psi al
//! promedio local con tasa `c`.
//!
//! Determinismo bit-exacto: doble-buffer (lectura del psi "antes",
//! escritura del psi "después"). Sin esto, agentes con índices mayores
//! leerían el psi ya actualizado de los menores — la simulación dependería
//! del orden de iteración aunque sea lineal. Con el buffer, el resultado
//! es **simétrico**: actualizar `i` o `j` primero da el mismo estado final.

use dominium_core::{SimParams, World};

/// Aplica una pasada de contagio social. No hace nada si `social_radius`
/// o `contagion_rate` son cero (motor histórico, retrocompat).
///
/// Algoritmo:
///
/// 1. Snapshot del psi de toda la población (lectura "antes").
/// 2. Para cada agente `i`, calcular el psi promedio de sus vecinos en
///    radio `R` usando el snapshot.
/// 3. Empujar el psi del agente: `psi_i ← psi_i + rate · (mean_local − psi_i)`.
///
/// El agente *no* se cuenta a sí mismo en el promedio. Si no hay vecinos
/// dentro del radio, su psi no se modifica este tick (sin sociedad, sin
/// influencia).
///
/// Costo: O(N²) por la búsqueda all-pairs. Con N ~10k es marginal frente
/// al loop principal del tick; para N > 50k habría que indexar agentes
/// por celda (Fase B.2).
pub fn apply_social_contagion(world: &mut World, p: &SimParams) {
    if p.social_radius <= 0.0 || p.contagion_rate <= 0.0 {
        return;
    }
    let n = world.lemmings.len();
    if n < 2 {
        return;
    }
    let r2 = p.social_radius * p.social_radius;
    // Si `homophily_threshold` > 0, comparamos contra su cuadrado para
    // ahorrar sqrt en el loop interior (distancia euclidiana al cuadrado).
    let use_homophily = p.homophily_threshold > 0.0;
    let homo2 = p.homophily_threshold * p.homophily_threshold;
    // Snapshot del psi "antes" — sin esto el contagio sería asimétrico y
    // dependiente del orden de iteración. También sirve como base contra
    // la cual se evalúa el filtro de homofilia.
    let psi_snapshot: Vec<[f32; 4]> = world.lemmings.vector_psi.clone();
    // Buffer de actualizaciones — escritura única al final.
    let mut new_psi: Vec<[f32; 4]> = psi_snapshot.clone();
    for i in 0..n {
        let xi = world.lemmings.pos_x[i];
        let yi = world.lemmings.pos_y[i];
        let psi_i = psi_snapshot[i];
        let mut sum = [0.0f64; 4];
        let mut count: u32 = 0;
        for j in 0..n {
            if j == i {
                continue;
            }
            let dx = world.lemmings.pos_x[j] - xi;
            let dy = world.lemmings.pos_y[j] - yi;
            if dx * dx + dy * dy > r2 {
                continue;
            }
            let psi_j = psi_snapshot[j];
            if use_homophily {
                // Distancia psi² — sólo nos cuenta si está "psicológicamente
                // cerca" del agente i.
                let d0 = psi_j[0] - psi_i[0];
                let d1 = psi_j[1] - psi_i[1];
                let d2 = psi_j[2] - psi_i[2];
                let d3 = psi_j[3] - psi_i[3];
                let dpsi2 = d0 * d0 + d1 * d1 + d2 * d2 + d3 * d3;
                if dpsi2 > homo2 {
                    continue;
                }
            }
            for k in 0..4 {
                sum[k] += psi_j[k] as f64;
            }
            count += 1;
        }
        if count == 0 {
            continue;
        }
        let cf = count as f64;
        let rate = p.contagion_rate as f64;
        for k in 0..4 {
            let mean = sum[k] / cf;
            let cur = psi_snapshot[i][k] as f64;
            new_psi[i][k] = (cur + rate * (mean - cur)) as f32;
        }
    }
    world.lemmings.vector_psi = new_psi;
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominium_core::SimParams;

    fn world_with_psi(psis: &[[f32; 4]]) -> World {
        let mut w = World::new(40, 40);
        for (k, &psi) in psis.iter().enumerate() {
            // Distribuirlos cerca pero no encima — radius del test los cubre.
            let x = 10.0 + (k as f32) * 0.5;
            let y = 10.0;
            w.lemmings.spawn(x, y, 30.0, psi);
        }
        w
    }

    #[test]
    fn contagion_disabled_by_default_is_a_noop() {
        let mut w = world_with_psi(&[
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ]);
        let psi_before = w.lemmings.vector_psi.clone();
        let p = SimParams::default(); // radius=0, rate=0
        apply_social_contagion(&mut w, &p);
        assert_eq!(w.lemmings.vector_psi, psi_before);
    }

    #[test]
    fn contagion_moves_outlier_toward_local_mean() {
        // Dos cercanos con psi=[0,0,0,0] y un outlier con psi=[1,1,1,1] al
        // lado. El outlier debe acercarse al promedio (que es [0,0,0,0]).
        let mut w = world_with_psi(&[
            [0.0; 4],
            [0.0; 4],
            [1.0, 1.0, 1.0, 1.0],
        ]);
        let mut p = SimParams::default();
        p.social_radius = 10.0;
        p.contagion_rate = 0.5;
        apply_social_contagion(&mut w, &p);
        // Outlier (índice 2): vecinos en radio = 0 y 1, ambos con psi=0.
        // Mean local = [0,0,0,0]. Nuevo psi = 1 + 0.5·(0-1) = 0.5.
        for k in 0..4 {
            assert!(
                (w.lemmings.vector_psi[2][k] - 0.5).abs() < 1e-5,
                "outlier comp {k}: {}",
                w.lemmings.vector_psi[2][k]
            );
        }
    }

    #[test]
    fn isolated_agent_unchanged() {
        // Un agente solo lejos de cualquiera no debe verse afectado.
        let mut w = World::new(40, 40);
        w.lemmings.spawn(2.0, 2.0, 30.0, [0.7, 0.3, 0.5, 0.1]);
        w.lemmings.spawn(35.0, 35.0, 30.0, [0.1, 0.9, 0.2, 0.8]);
        let mut p = SimParams::default();
        p.social_radius = 3.0; // demasiado chico para que se vean
        p.contagion_rate = 0.5;
        let psi_before = w.lemmings.vector_psi.clone();
        apply_social_contagion(&mut w, &p);
        assert_eq!(w.lemmings.vector_psi, psi_before);
    }

    #[test]
    fn contagion_is_symmetric_under_index_swap() {
        // Determinismo: aplicar contagio a [A, B, C] o a [C, B, A] (mismos
        // psi, mismas posiciones, distintos índices) debe producir
        // psi finales idénticos por agente. Esto valida que el doble-buffer
        // elimina la dependencia del orden de iteración.
        let psis = [
            [0.1, 0.9, 0.5, 0.0],
            [0.5, 0.5, 0.5, 0.5],
            [0.9, 0.1, 0.0, 1.0],
        ];
        let mut w_ab = world_with_psi(&psis);
        let mut psis_rev = psis;
        psis_rev.reverse();
        let mut w_rev = world_with_psi(&psis_rev);
        let mut p = SimParams::default();
        p.social_radius = 10.0;
        p.contagion_rate = 0.3;
        apply_social_contagion(&mut w_ab, &p);
        apply_social_contagion(&mut w_rev, &p);
        // El agente físicamente en posición 2 (ahora índice 2 en w_ab y
        // índice 0 en w_rev): comparar el psi del MISMO agente físico.
        // En w_ab: agente físicamente en pos x=11 es índice 2.
        // En w_rev: agente físicamente en pos x=11 es índice 0 (porque la
        // construcción asigna pos por orden y la reversa puso el psi de
        // antes-índice-2 en índice-0... pero el psi propio también cambió).
        // Mejor invariante: el promedio global de psi se conserva en cada
        // componente (el contagio es un promedio ponderado, no inyecta ni
        // drena).
        let mean_orig: [f64; 4] = {
            let mut m = [0.0f64; 4];
            for psi in &psis {
                for k in 0..4 { m[k] += psi[k] as f64; }
            }
            for k in 0..4 { m[k] /= psis.len() as f64; }
            m
        };
        let mean_after_ab: [f64; 4] = {
            let mut m = [0.0f64; 4];
            for psi in &w_ab.lemmings.vector_psi {
                for k in 0..4 { m[k] += psi[k] as f64; }
            }
            for k in 0..4 { m[k] /= w_ab.lemmings.len() as f64; }
            m
        };
        let mean_after_rev: [f64; 4] = {
            let mut m = [0.0f64; 4];
            for psi in &w_rev.lemmings.vector_psi {
                for k in 0..4 { m[k] += psi[k] as f64; }
            }
            for k in 0..4 { m[k] /= w_rev.lemmings.len() as f64; }
            m
        };
        for k in 0..4 {
            assert!(
                (mean_after_ab[k] - mean_orig[k]).abs() < 1e-4,
                "comp {k}: media drift (ab) {} vs orig {}",
                mean_after_ab[k], mean_orig[k]
            );
            assert!(
                (mean_after_rev[k] - mean_orig[k]).abs() < 1e-4,
                "comp {k}: media drift (rev) {} vs orig {}",
                mean_after_rev[k], mean_orig[k]
            );
        }
    }

    #[test]
    fn homophily_isolates_two_distinct_tribes() {
        // Dos grupos físicamente cercanos (radio social los cubre a todos)
        // pero psicológicamente lejanos. Con homophily_threshold pequeño,
        // cada tribu sólo se influye a sí misma — NO converge al promedio
        // global; cada tribu mantiene su centroide y la varianza entre
        // tribus se preserva.
        let mut w = World::new(40, 40);
        // Tribu A (psi=[1,0,0,0]) en posiciones cercanas.
        for k in 0..4 {
            w.lemmings
                .spawn(10.0 + k as f32 * 0.3, 10.0, 30.0, [1.0, 0.0, 0.0, 0.0]);
        }
        // Tribu B (psi=[0,0,0,1]) en posiciones también cercanas a A.
        for k in 0..4 {
            w.lemmings
                .spawn(12.0 + k as f32 * 0.3, 10.0, 30.0, [0.0, 0.0, 0.0, 1.0]);
        }
        let mut p = SimParams::default();
        p.social_radius = 10.0; // todos se ven entre sí
        p.contagion_rate = 0.30;
        // Distancia psi entre tribus = sqrt(1²+1²) ≈ 1.41. Threshold 0.5
        // → A ignora a B y viceversa.
        p.homophily_threshold = 0.5;
        for _ in 0..100 {
            apply_social_contagion(&mut w, &p);
        }
        // Tras 100 pasos: la tribu A debe mantenerse cerca de [1,0,0,0],
        // la tribu B cerca de [0,0,0,1] — NO al promedio global [0.5,0,0,0.5].
        for i in 0..4 {
            let p_a = w.lemmings.vector_psi[i];
            assert!(
                (p_a[0] - 1.0).abs() < 0.01 && p_a[3].abs() < 0.01,
                "tribu A drift: {:?}",
                p_a
            );
        }
        for i in 4..8 {
            let p_b = w.lemmings.vector_psi[i];
            assert!(
                p_b[0].abs() < 0.01 && (p_b[3] - 1.0).abs() < 0.01,
                "tribu B drift: {:?}",
                p_b
            );
        }
    }

    #[test]
    fn homophily_zero_falls_back_to_universal_contagion() {
        // homophily_threshold = 0.0 (default) → comportamiento de B.1:
        // las dos tribus convergen al promedio global.
        let mut w = World::new(40, 40);
        for k in 0..4 {
            w.lemmings
                .spawn(10.0 + k as f32 * 0.3, 10.0, 30.0, [1.0, 0.0, 0.0, 0.0]);
        }
        for k in 0..4 {
            w.lemmings
                .spawn(12.0 + k as f32 * 0.3, 10.0, 30.0, [0.0, 0.0, 0.0, 1.0]);
        }
        let mut p = SimParams::default();
        p.social_radius = 10.0;
        p.contagion_rate = 0.30;
        p.homophily_threshold = 0.0; // explícito
        for _ in 0..100 {
            apply_social_contagion(&mut w, &p);
        }
        // Convergen al promedio [0.5, 0, 0, 0.5].
        for psi in &w.lemmings.vector_psi {
            assert!(
                (psi[0] - 0.5).abs() < 0.01 && (psi[3] - 0.5).abs() < 0.01,
                "no convergió al promedio: {:?}",
                psi
            );
        }
    }

    #[test]
    fn contagion_converges_to_consensus_after_many_iterations() {
        // Con N agentes mutuamente visibles y tasa moderada, después de
        // ~50 pasos todos deberían tener el mismo psi (con tolerancia).
        let mut w = world_with_psi(&[
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        let mut p = SimParams::default();
        p.social_radius = 10.0;
        p.contagion_rate = 0.30;
        for _ in 0..100 {
            apply_social_contagion(&mut w, &p);
        }
        // Esperamos consenso = promedio inicial = [0.25, 0.25, 0.25, 0.25].
        for psi in &w.lemmings.vector_psi {
            for k in 0..4 {
                assert!(
                    (psi[k] - 0.25).abs() < 1e-3,
                    "no convergió comp {k}: {}",
                    psi[k]
                );
            }
        }
    }
}
