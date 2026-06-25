//! Difusión y entropía de los campos de la grilla.
//!
//! Ecuación de fluidos discreta: cada celda intercambia una fracción de
//! su valor con sus 4 vecinas, y luego pierde una fracción al ambiente
//! (entropía). Difunden los 3 campos dinámicos — materia, psique,
//! poder. `oro` (materia sólida) y `degradacion` (cicatriz permanente)
//! no difunden.

use std::cell::RefCell;

use dominium_core::{Grid, SimParams};

thread_local! {
    /// Buffer de lectura reutilizado por la difusión. Antes cada capa hacía
    /// `layer.to_vec()` — alocaba la grilla entera 3 veces por tick, lo que en
    /// un mundo grande (512² = ~262k celdas) es ~3 MB de alloc/free por tick.
    /// Acá se reusa un único `Vec` por hilo: se **sobreescribe por completo**
    /// (`extend_from_slice`) en cada uso, así no cruza estado entre llamadas y
    /// el resultado es idéntico al de alocar fresco (bit-exacto). Por hilo, así
    /// un caller multi-thread no comparte el buffer.
    static DIFFUSE_SCRATCH: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// Computa una celda de borde con el chequeo de 4-vecindad completo (von
/// Neumann, bordes sin wrap). Es **el cálculo histórico exacto**: misma suma en
/// orden izq/der/arriba/abajo, mismo `count` `f32`, mismo `sum/count` — sólo lo
/// usan las celdas de los 4 bordes, donde el fast-path del interior no aplica.
#[inline(always)]
fn cell_border(
    layer: &mut [f32],
    old: &[f32],
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    rate: f32,
    keep: f32,
) {
    let c = y * width + x;
    let mut sum = 0.0f32;
    let mut count = 0.0f32;
    if x > 0 {
        sum += old[c - 1];
        count += 1.0;
    }
    if x + 1 < width {
        sum += old[c + 1];
        count += 1.0;
    }
    if y > 0 {
        sum += old[c - width];
        count += 1.0;
    }
    if y + 1 < height {
        sum += old[c + width];
        count += 1.0;
    }
    let neighbor_avg = if count > 0.0 { sum / count } else { old[c] };
    let diffused = old[c] + rate * (neighbor_avg - old[c]);
    layer[c] = diffused * keep;
}

/// Difunde una sola capa: `new[c] = c + rate·(media_vecinos − c)`, luego
/// entropía. Lee el estado viejo del `DIFFUSE_SCRATCH` reutilizado (no aloca).
///
/// **Bit-exacto al algoritmo histórico**: el interior (celdas con las 4 vecinas
/// presentes) toma el fast-path sin ramas — `count` siempre vale `4.0`, así que
/// `sum/4.0` es idéntico al `sum/count` general y la suma va en el mismo orden
/// (izq, der, arriba, abajo). Los 4 bordes caen a [`cell_border`], que es el
/// cálculo original. Quitar las 4 ramas por celda del 99% interior es el grueso
/// de la aceleración.
fn diffuse_layer(layer: &mut [f32], width: usize, height: usize, rate: f32, entropy: f32) {
    let keep = 1.0 - entropy;
    DIFFUSE_SCRATCH.with(|cell| {
        let mut scratch = cell.borrow_mut();
        scratch.clear();
        scratch.extend_from_slice(layer);
        let old = scratch.as_slice();

        // Grillas degeneradas (sin interior): todo por el camino chequeado.
        if width < 3 || height < 3 {
            for y in 0..height {
                for x in 0..width {
                    cell_border(layer, old, x, y, width, height, rate, keep);
                }
            }
            return;
        }

        // Filas 0 y height-1: borde completo.
        for x in 0..width {
            cell_border(layer, old, x, 0, width, height, rate, keep);
            cell_border(layer, old, x, height - 1, width, height, rate, keep);
        }
        // Filas interiores: borde en x=0 y x=width-1, fast-path en el medio.
        for y in 1..height - 1 {
            let row = y * width;
            cell_border(layer, old, 0, y, width, height, rate, keep);
            for x in 1..width - 1 {
                let c = row + x;
                // 4 vecinas presentes ⇒ count=4. Mismo orden y división que el
                // general (sum/4.0 == sum/count con count=4.0).
                let sum = old[c - 1] + old[c + 1] + old[c - width] + old[c + width];
                let neighbor_avg = sum / 4.0;
                let diffused = old[c] + rate * (neighbor_avg - old[c]);
                layer[c] = diffused * keep;
            }
            cell_border(layer, old, width - 1, y, width, height, rate, keep);
        }
    });
}

/// Aplica un paso de difusión + entropía a los 3 campos dinámicos con
/// tasas explícitas — pensado para el `tick` que ya tiene calculada la
/// modulación estacional. La versión `diffuse(grid, p)` queda como wrapper
/// estable para callers que no quieren saber del ciclo de estaciones.
///
/// Las 3 capas reusan el mismo `DIFFUSE_SCRATCH` (cero alocación por tick tras
/// el primer uso).
pub fn diffuse_with(grid: &mut Grid, rate: f32, entropy: f32) {
    let (w, h) = (grid.width, grid.height);
    diffuse_layer(&mut grid.materia, w, h, rate, entropy);
    diffuse_layer(&mut grid.psique, w, h, rate, entropy);
    diffuse_layer(&mut grid.poder, w, h, rate, entropy);
}

/// Regrowth logístico de `materia`: cada celda recibe una fracción del
/// espacio libre que le falta para llegar a `cap`. Sólo aplica a la capa
/// de biomasa — las otras capas (psique, poder) no se regeneran solas.
/// Es la fuente termodinámica que la simulación necesita para no
/// extinguirse: sin ella la entropía vence siempre.
///
/// Vive *dentro* de la fase de difusión (el motor lo llama después de
/// `diffuse_with`), así no agrega una fase nueva al §1.5 ni rompe el
/// contrato del tick determinista.
pub fn regrow_materia(grid: &mut Grid, rate: f32, cap: f32) {
    if rate <= 0.0 {
        return;
    }
    for m in grid.materia.iter_mut() {
        let gap = (cap - *m).max(0.0);
        *m += rate * gap;
    }
}

/// Saturación física de los campos: clampa cada celda de las 5 capas a
/// `cap`. Es el techo duro que evita que las inyecciones por Conceptos (que
/// suman cada tick sin límite) o la `degradacion` (que sólo sube por
/// extracción) lleven el valor de una celda a infinito — y con él la altura
/// de la columna del render.
///
/// `cap <= 0.0` deshabilita (no toca nada) → motor histórico bit-exacto.
pub fn saturate_fields(grid: &mut Grid, cap: f32) {
    if cap <= 0.0 {
        return;
    }
    for layer in [
        &mut grid.materia,
        &mut grid.psique,
        &mut grid.poder,
        &mut grid.oro,
        &mut grid.degradacion,
    ] {
        for v in layer.iter_mut() {
            if *v > cap {
                *v = cap;
            }
        }
    }
}

/// Aplica un paso de difusión + entropía a los 3 campos dinámicos usando
/// las tasas base de `SimParams` sin modulación estacional, e incluye el
/// regrowth de materia. Es el wrapper "todo en uno" — útil para tests y
/// herramientas. El motor (`tick`) llama a las dos sub-fases por separado
/// para poder inyectar el factor de estación.
pub fn diffuse(grid: &mut Grid, p: &SimParams) {
    diffuse_with(grid, p.diffusion_rate, p.entropy_rate);
    regrow_materia(grid, p.regrowth_rate, p.carrying_capacity);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Implementación de **referencia**: el algoritmo histórico tal cual era
    /// (alocando `old`, con las 4 ramas por celda, sin fast-path de interior).
    /// El test de abajo exige que `diffuse_with` lo iguale **bit a bit**.
    fn diffuse_layer_ref(layer: &mut [f32], width: usize, height: usize, rate: f32, entropy: f32) {
        let old = layer.to_vec();
        for y in 0..height {
            for x in 0..width {
                let c = y * width + x;
                let mut sum = 0.0f32;
                let mut count = 0.0f32;
                if x > 0 {
                    sum += old[c - 1];
                    count += 1.0;
                }
                if x + 1 < width {
                    sum += old[c + 1];
                    count += 1.0;
                }
                if y > 0 {
                    sum += old[c - width];
                    count += 1.0;
                }
                if y + 1 < height {
                    sum += old[c + width];
                    count += 1.0;
                }
                let neighbor_avg = if count > 0.0 { sum / count } else { old[c] };
                let diffused = old[c] + rate * (neighbor_avg - old[c]);
                layer[c] = diffused * (1.0 - entropy);
            }
        }
    }

    #[test]
    fn diffuse_es_bit_exacto_a_la_referencia() {
        // Grillas de varios tamaños (incluye degeneradas 1×N y 2×N donde no hay
        // interior) y varias tasas, sembradas con un patrón determinista que
        // ejercita bordes, esquinas e interior. El nuevo camino (scratch + fast
        // -path) debe dar EXACTAMENTE el mismo `f32` que la referencia histórica.
        let dims = [(1, 7), (2, 9), (3, 3), (5, 4), (40, 40), (97, 33)];
        let cfgs = [(0.10f32, 0.005f32), (0.0, 0.2), (0.33, 0.0), (0.5, 0.5)];
        for &(w, h) in &dims {
            for &(rate, entropy) in &cfgs {
                let mut a = Grid::new(w, h);
                // Patrón pseudoaleatorio determinista por celda.
                for i in 0..(w * h) {
                    let v = (((i as u64).wrapping_mul(2654435761) >> 11) & 0xFFFF) as f32 / 521.0;
                    a.materia[i] = v;
                    a.psique[i] = v * 0.5 + 1.0;
                    a.poder[i] = (w * h - i) as f32 * 0.01;
                }
                let mut ref_mat = a.materia.clone();
                let mut ref_psi = a.psique.clone();
                let mut ref_pod = a.poder.clone();
                diffuse_layer_ref(&mut ref_mat, w, h, rate, entropy);
                diffuse_layer_ref(&mut ref_psi, w, h, rate, entropy);
                diffuse_layer_ref(&mut ref_pod, w, h, rate, entropy);

                diffuse_with(&mut a, rate, entropy);

                assert_eq!(a.materia, ref_mat, "materia {w}x{h} rate={rate} ent={entropy}");
                assert_eq!(a.psique, ref_psi, "psique {w}x{h} rate={rate} ent={entropy}");
                assert_eq!(a.poder, ref_pod, "poder {w}x{h} rate={rate} ent={entropy}");
            }
        }
    }

    #[test]
    fn diffusion_spreads_a_spike_to_neighbors() {
        let mut g = Grid::new(5, 5);
        let center = g.idx(2, 2);
        g.materia[center] = 100.0;
        let p = SimParams::default();
        diffuse(&mut g, &p);
        // El pico bajó; las vecinas subieron desde 0.
        assert!(g.materia[center] < 100.0);
        assert!(g.materia[g.idx(1, 2)] > 0.0);
        assert!(g.materia[g.idx(3, 2)] > 0.0);
    }

    #[test]
    fn entropy_decays_a_uniform_field() {
        let mut g = Grid::new(4, 4);
        for v in g.psique.iter_mut() {
            *v = 10.0;
        }
        let p = SimParams::default();
        diffuse(&mut g, &p);
        // Campo uniforme: la difusión no cambia nada, pero la entropía sí.
        for &v in &g.psique {
            assert!(v < 10.0 && v > 9.0);
        }
    }

    #[test]
    fn diffusion_conserves_mass_minus_entropy() {
        let mut g = Grid::new(6, 6);
        let c = g.idx(3, 3);
        g.materia[c] = 60.0;
        let total_before: f32 = g.materia.iter().sum();
        let mut p = SimParams::default();
        p.entropy_rate = 0.0; // sin pérdida → masa conservada
        p.regrowth_rate = 0.0; // sin fuente externa → masa cerrada
        diffuse(&mut g, &p);
        let total_after: f32 = g.materia.iter().sum();
        assert!((total_before - total_after).abs() < 1e-2);
    }

    #[test]
    fn regrowth_pushes_empty_cells_toward_capacity() {
        let mut g = Grid::new(4, 4);
        // Toda la grilla en 0; con cap=40 y rate=0.5 → en un tick suben a 20.
        regrow_materia(&mut g, 0.5, 40.0);
        for &v in &g.materia {
            assert!((v - 20.0).abs() < 1e-4);
        }
        // Un segundo tick los lleva a 20 + 0.5·20 = 30.
        regrow_materia(&mut g, 0.5, 40.0);
        for &v in &g.materia {
            assert!((v - 30.0).abs() < 1e-4);
        }
    }

    #[test]
    fn regrowth_never_exceeds_capacity() {
        let mut g = Grid::new(3, 3);
        // Una celda ya por encima de cap; regrow no la baja, sólo no la sube.
        let c = g.idx(1, 1);
        g.materia[c] = 80.0;
        regrow_materia(&mut g, 0.9, 40.0);
        assert_eq!(g.materia[c], 80.0, "regrow no degrada lo que excede cap");
        // Las vecinas vacías van hacia 40.
        let other = g.idx(0, 0);
        assert!((g.materia[other] - 36.0).abs() < 1e-4);
    }

    #[test]
    fn saturate_clamps_all_layers_to_cap() {
        let mut g = Grid::new(3, 3);
        let c = g.idx(1, 1);
        g.materia[c] = 500.0;
        g.psique[c] = 1000.0;
        g.poder[c] = 200.0;
        g.oro[c] = 80.0;
        g.degradacion[c] = 999.0;
        saturate_fields(&mut g, 150.0);
        assert_eq!(g.materia[c], 150.0);
        assert_eq!(g.psique[c], 150.0);
        assert_eq!(g.poder[c], 150.0);
        assert_eq!(g.oro[c], 80.0, "lo que está debajo del cap no se toca");
        assert_eq!(g.degradacion[c], 150.0, "la degradacion (sólo sube) también se capa");
    }

    #[test]
    fn saturate_disabled_when_cap_zero() {
        let mut g = Grid::new(2, 2);
        let c = g.idx(0, 0);
        g.materia[c] = 1e6;
        saturate_fields(&mut g, 0.0);
        assert_eq!(g.materia[c], 1e6, "cap 0 → no toca nada (bit-exacto histórico)");
    }

    #[test]
    fn regrowth_disabled_when_rate_zero() {
        let mut g = Grid::new(3, 3);
        regrow_materia(&mut g, 0.0, 40.0);
        for &v in &g.materia {
            assert_eq!(v, 0.0);
        }
    }
}
