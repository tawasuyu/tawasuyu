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
/// Radio de vecindad (en unidades de celda) usado por el `from_world`
/// default para Moran's I. Pares de agentes con distancia ≤ este radio
/// son considerados vecinos espaciales con peso 1; el resto, peso 0
/// (vecindad binaria). Valor calibrado para grids 30–80: detecta
/// autocorrelación local sin colapsar al promedio global.
pub const MORANS_RADIUS_DEFAULT: f32 = 6.0;
/// Cantidad de clusters fija para `kmeans_psi`. Tres es el mínimo que
/// detecta "centro + dos polos" — el patrón típico cuando emerge
/// polarización en la población.
pub const KMEANS_K: usize = 3;
/// Iteraciones máximas del k-means. 20 alcanza para 4 dimensiones y
/// poblaciones <10k; con convergencia temprana cuando `Δinertia < EPS`.
pub const KMEANS_MAX_ITER: u32 = 20;
/// Tolerancia de convergencia para `kmeans_psi` — cuando la inercia entre
/// iteraciones consecutivas cambia menos que esto, asume convergencia.
pub const KMEANS_EPS: f32 = 1e-4;

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
    /// Índice de Moran I por componente del `vector_psi`. Mide
    /// autocorrelación espacial: cuán parecido es el psi de un agente al
    /// de sus vecinos en radio `MORANS_RADIUS_DEFAULT`. Rango teórico
    /// aprox. `[-1, +1]`:
    /// - `+1`: vecinos muy parecidos → segregación residencial (Schelling).
    /// - `0`: psi distribuido al azar espacialmente.
    /// - `-1`: vecinos opuestos (patrón "tablero de ajedrez").
    /// Cero por convención cuando `n < 2`, `var(psi[k]) ≈ 0`, o ningún
    /// par está dentro del radio.
    pub moran_i: [f32; 4],
    /// Polarización Esteban-Ray de la 5ª dimensión `psi5` (Big Five
    /// Extraversion). `0.0` cuando el motor corre en Big Four o cuando la
    /// 5ª dimensión es uniforme.
    pub polarization_ext: f32,
    /// Índice de Moran I de la 5ª dimensión `psi5`. `0.0` en Big Four o
    /// distribución uniforme/azarosa.
    pub moran_i_ext: f32,
}

impl PsiMetrics {
    /// Computa todas las métricas con el radio de Moran default
    /// (`MORANS_RADIUS_DEFAULT`). Vacío o N<2 → ceros (no hay señal).
    pub fn from_world(w: &World) -> Self {
        Self::from_world_with_moran_radius(w, MORANS_RADIUS_DEFAULT)
    }

    /// Como `from_world`, pero el caller decide el radio de vecindad
    /// espacial usado por Moran's I. Útil cuando el grid es muy chico o
    /// muy grande y el default no aplica.
    pub fn from_world_with_moran_radius(w: &World, moran_radius: f32) -> Self {
        let l = &w.lemmings;
        let n = l.len();
        if n < 2 {
            return Self::default();
        }
        let mut polarization = [0.0f32; 4];
        let mut moran_i = [0.0f32; 4];
        for k in 0..4 {
            let mut buf = Vec::with_capacity(n);
            for i in 0..n {
                buf.push(l.vector_psi[i][k]);
            }
            polarization[k] = polarization_esteban_ray(&buf);
            moran_i[k] = morans_i_for(&buf, &l.pos_x, &l.pos_y, moran_radius);
        }
        // Big Five: si la columna psi5 está poblada (len == n), computa
        // polarización y Moran sobre ella. En motor Big Four la columna está
        // vacía o uniforme y los valores quedan en cero por convención.
        let (polarization_ext, moran_i_ext) = if l.psi5.len() == n {
            let buf: &[f32] = &l.psi5;
            (
                polarization_esteban_ray(buf),
                morans_i_for(buf, &l.pos_x, &l.pos_y, moran_radius),
            )
        } else {
            (0.0, 0.0)
        };
        let psi_action_corr = psi_action_corr_all(l);
        Self {
            polarization,
            psi_action_corr,
            moran_i,
            polarization_ext,
            moran_i_ext,
        }
    }
}

/// Índice de Moran I clásico con vecindad binaria por radio:
///
/// ```text
///   I = (n / S₀) · Σᵢ Σⱼ wᵢⱼ · (xᵢ − μ) · (xⱼ − μ) / Σᵢ (xᵢ − μ)²
/// ```
///
/// `wᵢⱼ = 1` si `|posᵢ − posⱼ| ≤ radius` y `i ≠ j`, sino `0`.
/// `S₀ = Σᵢⱼ wᵢⱼ` (el número total de pares vecinos).
///
/// Devuelve `0.0` para casos patológicos (n<2, varianza ~0, S₀==0).
/// Acumulador en `f64` para estabilidad numérica en grids grandes.
pub fn morans_i_for(values: &[f32], xs: &[f32], ys: &[f32], radius: f32) -> f32 {
    let n = values.len();
    if n < 2 {
        return 0.0;
    }
    if radius <= 0.0 {
        return 0.0;
    }
    let r2 = radius * radius;
    let nf = n as f64;
    let mut mean: f64 = 0.0;
    for &v in values {
        mean += v as f64;
    }
    mean /= nf;
    let mut variance: f64 = 0.0;
    for &v in values {
        let d = v as f64 - mean;
        variance += d * d;
    }
    if variance < 1e-12 {
        return 0.0;
    }
    let mut numerator: f64 = 0.0;
    let mut s0: f64 = 0.0;
    for i in 0..n {
        let xi = xs[i];
        let yi = ys[i];
        let di = values[i] as f64 - mean;
        for j in 0..n {
            if i == j {
                continue;
            }
            let dx = xs[j] - xi;
            let dy = ys[j] - yi;
            if dx * dx + dy * dy > r2 {
                continue;
            }
            let dj = values[j] as f64 - mean;
            numerator += di * dj;
            s0 += 1.0;
        }
    }
    if s0 < 1e-12 {
        return 0.0;
    }
    ((nf / s0) * (numerator / variance)) as f32
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

/// Resultado del k-means determinista sobre `vector_psi`.
#[derive(Debug, Clone, PartialEq)]
pub struct KMeansResult {
    /// Centroides finales en el espacio psi 4D. `[cluster][componente]`.
    pub centroids: [[f32; 4]; KMEANS_K],
    /// Cantidad de agentes asignados a cada cluster.
    pub sizes: [u32; KMEANS_K],
    /// Asignación por agente: byte `0..KMEANS_K`. Largo = `world.lemmings.len()`.
    pub assignments: Vec<u8>,
    /// Suma de distancias cuadradas de cada agente a su centroide. Métrica
    /// agregada de "compactness" de los clusters. Cero = clusters perfectos
    /// (todos los agentes están en su centroide); valores grandes = clusters
    /// difusos.
    pub inertia: f32,
    /// Iteraciones efectivamente corridas hasta la convergencia.
    pub iterations: u32,
}

/// k-means determinista sobre `vector_psi` con `k = KMEANS_K = 3`. Cero
/// RNG: inicialización por buckets `i % k`. Convergencia cuando la inercia
/// entre iteraciones consecutivas cambia menos que `KMEANS_EPS`. Asignación
/// tie-break por menor índice de cluster.
///
/// Devuelve `None` cuando hay menos de `KMEANS_K` agentes.
pub fn kmeans_psi(world: &World) -> Option<KMeansResult> {
    let l = &world.lemmings;
    let n = l.len();
    if n < KMEANS_K {
        return None;
    }
    // Inicialización determinista: buckets por índice módulo K.
    let mut centroids: [[f32; 4]; KMEANS_K] = [[0.0; 4]; KMEANS_K];
    {
        let mut sums = [[0.0f64; 4]; KMEANS_K];
        let mut counts = [0u32; KMEANS_K];
        for i in 0..n {
            let c = i % KMEANS_K;
            for d in 0..4 {
                sums[c][d] += l.vector_psi[i][d] as f64;
            }
            counts[c] += 1;
        }
        for c in 0..KMEANS_K {
            if counts[c] == 0 {
                continue;
            }
            for d in 0..4 {
                centroids[c][d] = (sums[c][d] / counts[c] as f64) as f32;
            }
        }
    }
    let mut assignments = vec![0u8; n];
    let mut prev_inertia: f64 = f64::INFINITY;
    let mut iterations: u32 = 0;
    let mut last_inertia: f64 = 0.0;
    for it in 0..KMEANS_MAX_ITER {
        iterations = it + 1;
        // Step 1: asignar cada agente al centroide más cercano.
        let mut inertia: f64 = 0.0;
        for i in 0..n {
            let mut best_c: u8 = 0;
            let mut best_d2: f32 = f32::MAX;
            for c in 0..KMEANS_K {
                let mut d2: f32 = 0.0;
                for d in 0..4 {
                    let diff = l.vector_psi[i][d] - centroids[c][d];
                    d2 += diff * diff;
                }
                if d2 < best_d2 {
                    best_d2 = d2;
                    best_c = c as u8;
                }
            }
            assignments[i] = best_c;
            inertia += best_d2 as f64;
        }
        last_inertia = inertia;
        // Convergencia: si la inercia no se mueve, paramos.
        if (prev_inertia - inertia).abs() < KMEANS_EPS as f64 {
            break;
        }
        prev_inertia = inertia;
        // Step 2: recomputar centroides como medias del cluster. Clusters
        // vacíos preservan su centroide del paso anterior (no se actualizan).
        let mut new_sums = [[0.0f64; 4]; KMEANS_K];
        let mut new_counts = [0u32; KMEANS_K];
        for i in 0..n {
            let c = assignments[i] as usize;
            for d in 0..4 {
                new_sums[c][d] += l.vector_psi[i][d] as f64;
            }
            new_counts[c] += 1;
        }
        for c in 0..KMEANS_K {
            if new_counts[c] == 0 {
                continue;
            }
            for d in 0..4 {
                centroids[c][d] = (new_sums[c][d] / new_counts[c] as f64) as f32;
            }
        }
    }
    let mut sizes = [0u32; KMEANS_K];
    for &a in &assignments {
        sizes[a as usize] += 1;
    }
    Some(KMeansResult {
        centroids,
        sizes,
        assignments,
        inertia: last_inertia as f32,
        iterations,
    })
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
    fn morans_i_is_high_when_neighbors_are_alike() {
        // Dos clusters físicos+psi distintos: izquierda con psi[0]=1,
        // derecha con psi[0]=0. Como cada agente está rodeado de iguales,
        // Moran's I debe ser cercano a +1.
        let mut w = World::new(40, 40);
        for k in 0..6 {
            w.lemmings
                .spawn(5.0 + (k % 3) as f32, 5.0 + (k / 3) as f32, 30.0, [1.0, 0.0, 0.0, 0.0]);
        }
        for k in 0..6 {
            w.lemmings
                .spawn(30.0 + (k % 3) as f32, 30.0 + (k / 3) as f32, 30.0, [0.0, 0.0, 0.0, 0.0]);
        }
        let m = PsiMetrics::from_world(&w);
        // En psi[ORDEN], la segregación física espeja la variación → Moran alto.
        assert!(
            m.moran_i[0] > 0.5,
            "Moran's I bajo aunque hay clustering espacial claro: {}",
            m.moran_i[0]
        );
    }

    #[test]
    fn morans_i_is_zero_when_psi_is_spatially_random() {
        // Mismas posiciones que el test anterior pero alternando psi:
        // patrón A B A B A B en ambas zonas → autocorrelación ≈ 0.
        let mut w = World::new(40, 40);
        for k in 0..12 {
            let psi_val = if k % 2 == 0 { 1.0 } else { 0.0 };
            let x = 5.0 + (k % 4) as f32 * 2.0;
            let y = 5.0 + (k / 4) as f32 * 2.0;
            w.lemmings.spawn(x, y, 30.0, [psi_val, 0.0, 0.0, 0.0]);
        }
        let m = PsiMetrics::from_world(&w);
        // Patrón tipo ajedrez: Moran's I tiende a ser negativo (vecinos
        // distintos). Aceptamos un rango amplio: muy lejos de +1.
        assert!(
            m.moran_i[0] < 0.5,
            "Moran's I alto en distribución alternante: {}",
            m.moran_i[0]
        );
    }

    #[test]
    fn morans_i_zero_when_uniform_population() {
        let mut w = World::new(40, 40);
        for _ in 0..20 {
            w.lemmings.spawn(10.0, 10.0, 30.0, [0.5; 4]);
        }
        let m = PsiMetrics::from_world(&w);
        for k in 0..4 {
            assert!(
                m.moran_i[k].abs() < 1e-5,
                "Moran[{k}] no es cero en pop uniforme: {}",
                m.moran_i[k]
            );
        }
    }

    #[test]
    fn kmeans_returns_none_when_too_few_agents() {
        let w = World::new(8, 8);
        assert!(kmeans_psi(&w).is_none());
        let mut w = World::new(8, 8);
        w.lemmings.spawn(1.0, 1.0, 10.0, [0.5; 4]);
        w.lemmings.spawn(2.0, 2.0, 10.0, [0.5; 4]);
        assert!(kmeans_psi(&w).is_none()); // sólo 2 agentes, K=3
    }

    #[test]
    fn kmeans_finds_three_distinct_clusters() {
        // Tres grupos en zonas opuestas del espacio psi: [1,0,0,0],
        // [0,1,0,0], [0,0,1,0]. 10 agentes por grupo.
        let mut w = World::new(8, 8);
        for _ in 0..10 {
            w.lemmings.spawn(1.0, 1.0, 10.0, [1.0, 0.0, 0.0, 0.0]);
        }
        for _ in 0..10 {
            w.lemmings.spawn(1.0, 1.0, 10.0, [0.0, 1.0, 0.0, 0.0]);
        }
        for _ in 0..10 {
            w.lemmings.spawn(1.0, 1.0, 10.0, [0.0, 0.0, 1.0, 0.0]);
        }
        let r = kmeans_psi(&w).expect("k-means corre");
        // Los 3 clusters deben quedar de tamaño ~10 cada uno.
        let mut sizes = r.sizes.to_vec();
        sizes.sort();
        assert_eq!(sizes, vec![10, 10, 10]);
        // Inertia muy chica porque los clusters son compactos.
        assert!(r.inertia < 0.1, "inertia alta: {}", r.inertia);
    }

    #[test]
    fn kmeans_is_deterministic_under_same_input() {
        // Dos mundos idénticos deben producir k-means idéntico.
        let build = || {
            let mut w = World::new(8, 8);
            for k in 0..18 {
                let val = (k as f32 * 0.37).fract();
                w.lemmings.spawn(1.0, 1.0, 10.0, [val, 1.0 - val, val * val, 0.5]);
            }
            w
        };
        let a = kmeans_psi(&build()).expect("a");
        let b = kmeans_psi(&build()).expect("b");
        assert_eq!(a.centroids, b.centroids);
        assert_eq!(a.sizes, b.sizes);
        assert_eq!(a.assignments, b.assignments);
        assert_eq!(a.inertia, b.inertia);
        assert_eq!(a.iterations, b.iterations);
    }

    #[test]
    fn psi_metrics_calcula_ext_cuando_psi5_esta_poblado() {
        // Población bimodal sólo en la 5ª dimensión: mitad psi5=0,
        // mitad psi5=1. Las 4 primeras componentes uniformes.
        let mut w = World::new(8, 8);
        for k in 0..40 {
            let v5 = if k < 20 { 0.0 } else { 1.0 };
            w.lemmings.spawn_big5(1.0, 1.0, 10.0, [0.5; 4], v5);
        }
        let m = PsiMetrics::from_world(&w);
        // polarization_ext debe ser alta porque la distribución es bimodal.
        assert!(m.polarization_ext > 0.1, "polar_ext bimodal: {}", m.polarization_ext);
        // Las 4 primeras componentes uniformes → polarization ~0.
        for k in 0..4 {
            assert!(m.polarization[k].abs() < 1e-4, "comp {k}: {}", m.polarization[k]);
        }
    }

    #[test]
    fn psi_metrics_ext_es_cero_sin_columna_psi5() {
        // Build manual de un Lemmings con psi5 vacío (motor Big Four
        // serializado antes del cambio).
        use crate::lemmings::Lemmings;
        let mut w = World::new(8, 8);
        // Llenamos los vectores básicos a mano para simular un deserialize
        // viejo que no traía psi5.
        w.lemmings = Lemmings {
            pos_x: vec![1.0, 2.0],
            pos_y: vec![1.0, 2.0],
            edad: vec![0; 2],
            energia: vec![10.0; 2],
            vector_psi: vec![[0.5; 4]; 2],
            accion: vec![0; 2],
            hack_lock: vec![0; 2],
            psi5: Vec::new(),
        };
        let m = PsiMetrics::from_world(&w);
        assert_eq!(m.polarization_ext, 0.0);
        assert_eq!(m.moran_i_ext, 0.0);
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
