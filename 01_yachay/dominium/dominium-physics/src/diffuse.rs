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

/// Aplica un paso de difusión + entropía a los 3 campos dinámicos.
pub fn diffuse(grid: &mut Grid, p: &SimParams) {
    let (w, h) = (grid.width, grid.height);
    let (rate, ent) = (p.diffusion_rate, p.entropy_rate);
    diffuse_layer(&mut grid.materia, w, h, rate, ent);
    diffuse_layer(&mut grid.psique, w, h, rate, ent);
    diffuse_layer(&mut grid.poder, w, h, rate, ent);
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
        diffuse(&mut g, &p);
        let total_after: f32 = g.materia.iter().sum();
        assert!((total_before - total_after).abs() < 1e-2);
    }
}
