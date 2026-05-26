//! Estadísticas agregadas del mundo — **lectura pura, no muta nada**.
//!
//! Pensado para alimentar HUDs, CSV del CLI y eventuales tests de invariantes
//! macro (¿la energía total decae? ¿el Gini se dispara con ciertos packs?).
//!
//! Determinista bit-exacto: itera en orden lineal, suma `f32` en el mismo
//! orden en cualquier plataforma, sin paralelismo ni hashing.

use crate::world::World;

/// Foto del estado agregado del mundo en un instante.
///
/// Convención del `vector_psi`: las 4 componentes en orden
/// `[ORDEN, MIEDO, CURIOSIDAD, CORRUPTIBILIDAD]`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WorldStats {
    /// Cantidad de Lemmings vivos.
    pub n: usize,
    /// Coeficiente de Gini sobre `energia` ∈ [0, 1]. 0 = perfecta igualdad,
    /// 1 = un único agente concentra todo. `0.0` si `n < 2`.
    pub gini_energia: f32,
    /// Varianza poblacional de cada componente del `vector_psi` ∈ ℝ⁺. `0.0`
    /// para componentes con `n == 0`.
    pub var_psi: [f32; 4],
    /// Conteo de cuántos Lemmings ejecutan cada `Action` (0..=5).
    pub action_counts: [u32; 6],
    /// Suma de las 5 capas del Sustrato — útil para detectar drift de masa.
    pub total_materia: f32,
    pub total_psique: f32,
    pub total_poder: f32,
    pub total_oro: f32,
    pub total_degradacion: f32,
    /// Media de `edad` (0 si `n == 0`).
    pub mean_edad: f32,
    /// Suma de `energia` (0 si `n == 0`).
    pub total_energia: f32,
}

impl WorldStats {
    /// Calcula todas las métricas en una sola pasada por agente + cinco
    /// sumas lineales por las capas. Asignación: un `Vec<f32>` temporal del
    /// largo de la población para el Gini (ordenamiento necesario).
    pub fn from_world(w: &World) -> Self {
        let n = w.lemmings.len();
        let mut action_counts = [0u32; 6];
        let mut sum_psi = [0.0f64; 4];
        let mut sum_psi2 = [0.0f64; 4];
        let mut sum_edad: u64 = 0;
        let mut sum_energia: f64 = 0.0;

        for i in 0..n {
            let a = w.lemmings.accion[i];
            if (a as usize) < action_counts.len() {
                action_counts[a as usize] += 1;
            }
            let psi = w.lemmings.vector_psi[i];
            for k in 0..4 {
                let v = psi[k] as f64;
                sum_psi[k] += v;
                sum_psi2[k] += v * v;
            }
            sum_edad += w.lemmings.edad[i] as u64;
            sum_energia += w.lemmings.energia[i] as f64;
        }

        // Var(X) = E[X²] − E[X]²; en f64 internamente, downcast al final.
        let mut var_psi = [0.0f32; 4];
        if n > 0 {
            let nf = n as f64;
            for k in 0..4 {
                let mean = sum_psi[k] / nf;
                let v = (sum_psi2[k] / nf) - mean * mean;
                var_psi[k] = v.max(0.0) as f32;
            }
        }

        let mean_edad = if n > 0 { (sum_edad as f64 / n as f64) as f32 } else { 0.0 };

        let gini_energia = gini_of(&w.lemmings.energia);

        let g = &w.grid;
        Self {
            n,
            gini_energia,
            var_psi,
            action_counts,
            total_materia: sum_layer(&g.materia),
            total_psique: sum_layer(&g.psique),
            total_poder: sum_layer(&g.poder),
            total_oro: sum_layer(&g.oro),
            total_degradacion: sum_layer(&g.degradacion),
            mean_edad,
            total_energia: sum_energia as f32,
        }
    }
}

/// Suma de `f32` acumulada en `f64` para no perder precisión en grillas
/// grandes — la salida es `f32` pero el orden de la suma queda fijado por
/// el orden lineal del slice, así que sigue siendo bit-exacto.
fn sum_layer(layer: &[f32]) -> f32 {
    let mut acc: f64 = 0.0;
    for &v in layer {
        acc += v as f64;
    }
    acc as f32
}

/// Gini sobre energía no-negativa. Implementación clásica vía orden ascendente:
///
/// ```text
///   G = ( 2·Σ(i · x_i) − (n+1)·Σx_i ) / ( n · Σx_i )
/// ```
///
/// Robusto a entradas vacías (→ 0.0), a `Σx_i == 0` (→ 0.0) y a valores
/// negativos (los considera 0 — la energía nunca debería ser negativa pero
/// `act_degradar` puede dejarla en rojo un tick antes de la cosecha).
fn gini_of(values: &[f32]) -> f32 {
    let n = values.len();
    if n < 2 {
        return 0.0;
    }
    let mut v: Vec<f32> = values.iter().map(|x| x.max(0.0)).collect();
    // `sort_by` con comparación total — `f32` no implementa `Ord`. Las NaN
    // son imposibles aquí (sólo aritmética cerrada sobre f32 finitos), pero
    // por las dudas las tratamos como iguales para no panickear.
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut sum: f64 = 0.0;
    let mut weighted: f64 = 0.0;
    for (i, &x) in v.iter().enumerate() {
        sum += x as f64;
        weighted += (i + 1) as f64 * x as f64;
    }
    if sum <= 0.0 {
        return 0.0;
    }
    let nf = n as f64;
    let g = (2.0 * weighted - (nf + 1.0) * sum) / (nf * sum);
    g.clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimParams;

    #[test]
    fn empty_world_yields_zeros() {
        let w = World::new(4, 4);
        let s = WorldStats::from_world(&w);
        assert_eq!(s.n, 0);
        assert_eq!(s.gini_energia, 0.0);
        assert_eq!(s.action_counts, [0; 6]);
        assert_eq!(s.var_psi, [0.0; 4]);
        assert_eq!(s.mean_edad, 0.0);
    }

    #[test]
    fn gini_zero_when_all_equal() {
        assert_eq!(gini_of(&[10.0, 10.0, 10.0, 10.0]), 0.0);
    }

    #[test]
    fn gini_one_when_only_one_has_value() {
        // 0,0,0,…,100 → cerca de 1 (no exactamente; la cota teórica es (n-1)/n)
        let n = 100;
        let mut v = vec![0.0f32; n];
        v[n - 1] = 100.0;
        let g = gini_of(&v);
        let expected_upper = (n as f32 - 1.0) / n as f32;
        assert!((g - expected_upper).abs() < 1e-3, "gini={g}");
    }

    #[test]
    fn gini_rises_with_inequality() {
        let flat = gini_of(&[5.0, 5.0, 5.0, 5.0]);
        let mid = gini_of(&[2.0, 4.0, 6.0, 8.0]);
        let sharp = gini_of(&[0.5, 0.5, 0.5, 18.5]);
        assert!(flat < mid);
        assert!(mid < sharp);
    }

    #[test]
    fn action_counts_match_population_distribution() {
        let mut w = World::new(8, 8);
        // 3 con accion=2, 2 con accion=0, 1 con accion=5.
        for _ in 0..3 {
            let i = w.lemmings.spawn(1.0, 1.0, 10.0, [0.0; 4]);
            w.lemmings.accion[i] = 2;
        }
        for _ in 0..2 {
            let i = w.lemmings.spawn(1.0, 1.0, 10.0, [0.0; 4]);
            w.lemmings.accion[i] = 0;
        }
        let i = w.lemmings.spawn(1.0, 1.0, 10.0, [0.0; 4]);
        w.lemmings.accion[i] = 5;

        let s = WorldStats::from_world(&w);
        assert_eq!(s.action_counts[0], 2);
        assert_eq!(s.action_counts[2], 3);
        assert_eq!(s.action_counts[5], 1);
        assert_eq!(s.n, 6);
    }

    #[test]
    fn var_psi_zero_when_population_is_uniform() {
        let mut w = World::new(4, 4);
        for _ in 0..10 {
            w.lemmings.spawn(0.0, 0.0, 1.0, [0.5, 0.5, 0.5, 0.5]);
        }
        let s = WorldStats::from_world(&w);
        for k in 0..4 {
            assert!(s.var_psi[k] < 1e-6, "var[{k}] no es cero: {}", s.var_psi[k]);
        }
    }

    #[test]
    fn layer_totals_track_grid_state() {
        let mut w = World::new(4, 4);
        let idx = w.grid.idx(1, 1);
        w.grid.materia[idx] = 5.0;
        w.grid.oro[idx] = 3.0;
        let s = WorldStats::from_world(&w);
        assert!((s.total_materia - 5.0).abs() < 1e-5);
        assert!((s.total_oro - 3.0).abs() < 1e-5);
    }

    #[test]
    fn stats_silent_passthrough_with_simparams() {
        // Sanity: SimParams sigue siendo construible sin el módulo nuevo.
        let _ = SimParams::default();
        let w = World::new(2, 2);
        let _ = WorldStats::from_world(&w);
    }
}
