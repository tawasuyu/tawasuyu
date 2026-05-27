//! Métricas psicológicas sobre la población — lectura pura, no muta nada.
//!
//! Complemento de `metrics::WorldStats`: aquellos eran agregados macro
//! (Gini de energía, conteo por acción, varianza global de psi). Estos son
//! métricas *psicológicas* en sentido estricto:
//!
//! 1. **Polarización Esteban-Ray** sobre cada componente del `vector_psi`.
//!    Detecta distribuciones bimodales/multimodales — la población se está
//!    rompiendo en tribus psicológicas. Cero cuando todos son iguales o la
//!    distribución es unimodal centrada; sube cuando se forman polos.
//!
//! 2. **Correlación punto-biserial `psi[k] ↔ accion == a`**: una matriz
//!    `4×6` que mide cuánto predice cada componente del psi cada acción.
//!    Con `ActionPolicy::Fixed` y `psi_effect_modulation == 0` (motor
//!    histórico), los valores fluctúan cerca de 0 porque la acción no
//!    depende del psi. Con `PsiArgmax` se concentran en celdas donde
//!    `action_weights[a][k]` es alto — exactamente el efecto que Fase A
//!    instaló y que necesitamos *medir*.
//!
//! Determinismo bit-exacto: iteración lineal, sumas en `f64`, `libm::sqrt`/
//! `powf` para constantes precomputadas en orden fijo. No hay paralelismo,
//! ni hashing, ni ordenamiento sensible a empates.

use crate::lemmings::Lemmings;
use crate::world::World;

/// Cantidad de bins usados por la polarización Esteban-Ray. Pocos bins
/// son robustos a poblaciones chicas; con K=8 podemos detectar hasta 4
/// modos sin que el ruido domine.
pub const POLARIZATION_BINS: usize = 8;
/// Exponente de Esteban-Ray (`α`). `α=1` es el valor canónico que enfatiza
/// la concentración de masa en pocos polos sin desplomar el aporte de
/// distancia. `α=0` colapsaría a Gini; `α=1.6` (otro canónico) penaliza
/// más los polos chicos.
pub const POLARIZATION_ALPHA: f32 = 1.0;

/// Snapshot psicológico instantáneo. Foto, no historia.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PsiMetrics {
    /// Polarización Esteban-Ray por componente del `vector_psi`
    /// (`[ORDEN, MIEDO, CURIOSIDAD, CORRUPTIBILIDAD]`). Cero cuando todos
    /// los agentes tienen el mismo valor del componente o la varianza es
    /// despreciable.
    pub polarization: [f32; 4],
    /// Correlación punto-biserial `r[k][a]` entre el componente `k` del
    /// `vector_psi` (continuo) y el indicador `1[accion == a]` (binario).
    /// Rango teórico `[-1, 1]`. Cero por convención cuando no hay agentes
    /// con la acción `a` (o todos la tienen) o cuando `var(psi[k]) ≈ 0`.
    pub psi_action_corr: [[f32; 6]; 4],
}

impl PsiMetrics {
    /// Computa todas las métricas en pasadas lineales sobre los agentes.
    /// Vacío o N<2 → ceros (no hay señal).
    pub fn from_world(w: &World) -> Self {
        let l = &w.lemmings;
        let n = l.len();
        if n < 2 {
            return Self::default();
        }
        let mut polarization = [0.0f32; 4];
        for k in 0..4 {
            // Construir el componente k como slice virtual.
            let mut buf = Vec::with_capacity(n);
            for i in 0..n {
                buf.push(l.vector_psi[i][k]);
            }
            polarization[k] = polarization_esteban_ray(&buf);
        }
        let psi_action_corr = psi_action_corr_all(l);
        Self { polarization, psi_action_corr }
    }
}

/// Polarización Esteban-Ray con K=`POLARIZATION_BINS` bins igualmente
/// espaciados entre `[min, max]` del slice. `α=POLARIZATION_ALPHA`.
///
/// ```text
/// P_α(p, x) = Σᵢ Σⱼ pᵢ^(1+α) · pⱼ · |xᵢ − xⱼ|
/// ```
///
/// `min == max` → 0.0 (todos iguales, no hay nada que polarizar).
/// `n < 2` → 0.0.
fn polarization_esteban_ray(values: &[f32]) -> f32 {
    let n = values.len();
    if n < 2 {
        return 0.0;
    }
    let mut min = values[0];
    let mut max = values[0];
    for &v in &values[1..] {
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
    }
    let span = max - min;
    if span < 1e-9 {
        return 0.0;
    }
    let bins = POLARIZATION_BINS;
    let mut counts = vec![0u32; bins];
    for &v in values {
        // Bin = floor((v - min) / span * bins), clampeado a [0, bins-1].
        let raw = ((v - min) / span) * bins as f32;
        let mut bi = raw as i64;
        if bi >= bins as i64 {
            bi = bins as i64 - 1;
        }
        if bi < 0 {
            bi = 0;
        }
        counts[bi as usize] += 1;
    }
    let nf = n as f64;
    let bin_width = span as f64 / bins as f64;
    let mut probs = [0.0f64; POLARIZATION_BINS];
    for i in 0..bins {
        probs[i] = counts[i] as f64 / nf;
    }
    // Centros de bin: min + (i + 0.5) · bin_width.
    let mut centers = [0.0f64; POLARIZATION_BINS];
    for i in 0..bins {
        centers[i] = min as f64 + (i as f64 + 0.5) * bin_width;
    }
    // `α + 1` precomputado en f32 — libm::powf garantiza el mismo bit a
    // bit en x86 y ARM.
    let exp = (POLARIZATION_ALPHA + 1.0) as f64;
    let mut acc: f64 = 0.0;
    for i in 0..bins {
        if probs[i] <= 0.0 {
            continue;
        }
        let pi_alpha = libm::pow(probs[i], exp);
        for j in 0..bins {
            if probs[j] <= 0.0 {
                continue;
            }
            let diff = (centers[i] - centers[j]).abs();
            acc += pi_alpha * probs[j] * diff;
        }
    }
    acc as f32
}

/// Correlación de Pearson punto-biserial entre cada componente del psi
/// (continuo) y el indicador `1[accion == a]` (binario), para cada
/// `k ∈ 0..4` y `a ∈ 0..6`. Fórmula clásica:
///
/// ```text
/// r_pb = ( μ_{X|Y=1} − μ_X ) · √( p / (1−p) ) / σ_X
/// ```
///
/// Devuelve ceros para entradas patológicas (varianza ~0, acción nunca
/// ejecutada o ejecutada por todos).
fn psi_action_corr_all(l: &Lemmings) -> [[f32; 6]; 4] {
    let n = l.len();
    if n < 2 {
        return [[0.0; 6]; 4];
    }
    let nf = n as f64;
    // Pasada 1: media de cada componente del psi.
    let mut mean_psi = [0.0f64; 4];
    for i in 0..n {
        for k in 0..4 {
            mean_psi[k] += l.vector_psi[i][k] as f64;
        }
    }
    for k in 0..4 {
        mean_psi[k] /= nf;
    }
    // Pasada 2: varianza de cada componente.
    let mut var_psi = [0.0f64; 4];
    for i in 0..n {
        for k in 0..4 {
            let d = l.vector_psi[i][k] as f64 - mean_psi[k];
            var_psi[k] += d * d;
        }
    }
    for k in 0..4 {
        var_psi[k] /= nf;
    }
    // Pasada 3: conteo por acción y suma del psi condicional.
    let mut count_a = [0u64; 6];
    let mut sum_psi_when_a = [[0.0f64; 6]; 4];
    for i in 0..n {
        let a = l.accion[i] as usize;
        if a < 6 {
            count_a[a] += 1;
            for k in 0..4 {
                sum_psi_when_a[k][a] += l.vector_psi[i][k] as f64;
            }
        }
    }
    let mut out = [[0.0f32; 6]; 4];
    for k in 0..4 {
        if var_psi[k] < 1e-12 {
            continue;
        }
        let sd = libm::sqrt(var_psi[k]);
        for a in 0..6 {
            let p = count_a[a] as f64 / nf;
            if p < 1e-9 || p > 1.0 - 1e-9 {
                continue;
            }
            let mean_when_a = sum_psi_when_a[k][a] / count_a[a] as f64;
            let r = (mean_when_a - mean_psi[k]) * libm::sqrt(p / (1.0 - p)) / sd;
            out[k][a] = r as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SimParams;
    use crate::world::World;

    #[test]
    fn empty_or_singleton_yields_zeros() {
        let w = World::new(4, 4);
        let m = PsiMetrics::from_world(&w);
        assert_eq!(m.polarization, [0.0; 4]);
        assert_eq!(m.psi_action_corr, [[0.0; 6]; 4]);

        let mut w = World::new(4, 4);
        w.lemmings.spawn(1.0, 1.0, 10.0, [0.5; 4]);
        let m = PsiMetrics::from_world(&w);
        assert_eq!(m.polarization, [0.0; 4]);
    }

    #[test]
    fn uniform_population_has_zero_polarization() {
        let mut w = World::new(4, 4);
        for _ in 0..50 {
            w.lemmings.spawn(1.0, 1.0, 10.0, [0.5; 4]);
        }
        let m = PsiMetrics::from_world(&w);
        for k in 0..4 {
            assert!(m.polarization[k].abs() < 1e-5, "comp {k}: {}", m.polarization[k]);
        }
    }

    #[test]
    fn bimodal_population_has_high_polarization() {
        let mut w = World::new(4, 4);
        // Mitad psi[0]=0, mitad psi[0]=1: distribución perfectamente bimodal.
        for k in 0..50 {
            let val = if k < 25 { 0.0 } else { 1.0 };
            w.lemmings.spawn(1.0, 1.0, 10.0, [val, 0.5, 0.5, 0.5]);
        }
        let m = PsiMetrics::from_world(&w);
        // El componente bimodal debe ser claramente más polarizado que los
        // unimodales centrados.
        assert!(
            m.polarization[0] > 0.1,
            "comp 0 bimodal debe polarizar: {}",
            m.polarization[0]
        );
        for k in 1..4 {
            assert!(
                m.polarization[k] < 1e-4,
                "comp {k} uniforme no debe polarizar: {}",
                m.polarization[k]
            );
        }
        // Y el bimodal debe ser mayor que el uniforme por un margen amplio.
        assert!(m.polarization[0] > m.polarization[1] * 100.0);
    }

    #[test]
    fn psi_action_correlation_emerges_when_psi_predicts_action() {
        // Construcción a mano: los lemmings con CORRUPTIBILIDAD alta están
        // todos en accion=Degradar (5). Los honestos están en accion=Mover (0).
        let mut w = World::new(4, 4);
        for _ in 0..30 {
            let i = w.lemmings.spawn(1.0, 1.0, 10.0, [0.0, 0.0, 0.0, 1.0]);
            w.lemmings.accion[i] = 5; // Degradar
        }
        for _ in 0..30 {
            let i = w.lemmings.spawn(1.0, 1.0, 10.0, [0.0, 0.0, 0.0, 0.0]);
            w.lemmings.accion[i] = 0; // Mover
        }
        let m = PsiMetrics::from_world(&w);
        // corr(CORRUPTIBILIDAD, Degradar) debe ser ~+1: alta CORR → Degradar.
        assert!(
            m.psi_action_corr[3][5] > 0.8,
            "corr CORR↔Degradar: {}",
            m.psi_action_corr[3][5]
        );
        // corr(CORRUPTIBILIDAD, Mover) debe ser ~-1: alta CORR → NO Mover.
        assert!(
            m.psi_action_corr[3][0] < -0.8,
            "corr CORR↔Mover: {}",
            m.psi_action_corr[3][0]
        );
        // Componentes irrelevantes (ORDEN, MIEDO, CURIOSIDAD) varianza 0
        // → correlación 0 por convención.
        for k in 0..3 {
            for a in 0..6 {
                assert!(
                    m.psi_action_corr[k][a].abs() < 1e-5,
                    "comp {k} action {a}: {}",
                    m.psi_action_corr[k][a]
                );
            }
        }
    }

    #[test]
    fn psi_action_correlation_zero_when_action_random_vs_psi() {
        // psi alternados con accion fija (todos hacen lo mismo). p(accion)=1
        // → fórmula devuelve 0 por convención (no se puede correlacionar
        // con un evento que siempre ocurre).
        let mut w = World::new(4, 4);
        for k in 0..40 {
            let val = if k % 2 == 0 { 0.0 } else { 1.0 };
            let i = w.lemmings.spawn(1.0, 1.0, 10.0, [val; 4]);
            w.lemmings.accion[i] = 1; // todos Extraer
        }
        let m = PsiMetrics::from_world(&w);
        // Todos hacen Extraer → p=1 → corr = 0 en todas las columnas.
        for k in 0..4 {
            for a in 0..6 {
                assert!(
                    m.psi_action_corr[k][a].abs() < 1e-5,
                    "comp {k} action {a} debe ser 0: {}",
                    m.psi_action_corr[k][a]
                );
            }
        }
    }

    #[test]
    fn metrics_run_on_typical_world_without_panicking() {
        let mut w = World::new(16, 16);
        for k in 0..40 {
            let x = (k % 8) as f32 + 2.0;
            let y = (k / 8) as f32 + 2.0;
            let psi = [
                (k as f32 * 0.13).fract(),
                (k as f32 * 0.27).fract(),
                (k as f32 * 0.41).fract(),
                (k as f32 * 0.59).fract(),
            ];
            let i = w.lemmings.spawn(x, y, 30.0, psi);
            w.lemmings.accion[i] = (k % 6) as u8;
        }
        let m = PsiMetrics::from_world(&w);
        // No deben aparecer NaN/inf.
        for k in 0..4 {
            assert!(m.polarization[k].is_finite());
            for a in 0..6 {
                assert!(m.psi_action_corr[k][a].is_finite());
            }
        }
        // Sanidad: con SimParams default el tipo se usa sin problemas.
        let _ = SimParams::default();
    }
}
