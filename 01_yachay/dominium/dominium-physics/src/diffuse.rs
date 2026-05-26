//! Difusión y entropía de los campos de la grilla.
//!
//! Ecuación de fluidos discreta: cada celda intercambia una fracción de
//! su valor con sus 4 vecinas, y luego pierde una fracción al ambiente
//! (entropía). Difunden los 3 campos dinámicos — materia, psique,
//! poder. `oro` (materia sólida) y `degradacion` (cicatriz permanente)
//! no difunden.

use dominium_core::{Grid, SimParams};

/// Difunde una sola capa: `new[c] = c + rate·(media_vecinos − c)`, y luego
/// aplica la entropía. Usa un buffer de lectura separado (la difusión
/// debe leer el estado viejo).
fn diffuse_layer(layer: &mut [f32], width: usize, height: usize, rate: f32, entropy: f32) {
    let old = layer.to_vec();
    for y in 0..height {
        for x in 0..width {
            let c = y * width + x;
            let mut sum = 0.0f32;
            let mut count = 0.0f32;
            // 4-vecindad (von Neumann), bordes sin wrap.
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

/// Aplica un paso de difusión + entropía a los 3 campos dinámicos con
/// tasas explícitas — pensado para el `tick` que ya tiene calculada la
/// modulación estacional. La versión `diffuse(grid, p)` queda como wrapper
/// estable para callers que no quieren saber del ciclo de estaciones.
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
    fn regrowth_disabled_when_rate_zero() {
        let mut g = Grid::new(3, 3);
        regrow_materia(&mut g, 0.0, 40.0);
        for &v in &g.materia {
            assert_eq!(v, 0.0);
        }
    }
}
