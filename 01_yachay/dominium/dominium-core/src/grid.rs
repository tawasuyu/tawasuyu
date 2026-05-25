//! El Sustrato Plano — grilla SoA de 5 capas de `f32`.

use serde::{Deserialize, Serialize};

/// Grilla de campos: 5 capas paralelas, cada una `width × height` `f32`,
/// indexadas `y * width + x`. Toda la física opera sobre estos arrays
/// contiguos (cache-friendly).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grid {
    pub width: usize,
    pub height: usize,
    /// Biomasa / energía / alimento disponible.
    pub materia: Vec<f32>,
    /// Densidad de información / frecuencia dogmática.
    pub psique: Vec<f32>,
    /// Tensión de control / deuda / atractores del Estado Profundo.
    pub poder: Vec<f32>,
    /// Materia prima densa intercambiable.
    pub oro: Vec<f32>,
    /// Contaminación / cicatrices industriales del suelo.
    pub degradacion: Vec<f32>,
}

impl Grid {
    /// Grilla de `width × height` con todas las capas en cero.
    pub fn new(width: usize, height: usize) -> Self {
        let n = width * height;
        Self {
            width,
            height,
            materia: vec![0.0; n],
            psique: vec![0.0; n],
            poder: vec![0.0; n],
            oro: vec![0.0; n],
            degradacion: vec![0.0; n],
        }
    }

    /// Cantidad de celdas (`width * height`).
    pub fn cells(&self) -> usize {
        self.width * self.height
    }

    /// Índice plano de `(x, y)`. El caller garantiza bounds válidos.
    pub fn idx(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }

    /// `true` si `(x, y)` cae dentro de la grilla.
    pub fn in_bounds(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height
    }

    /// Clampa una coordenada continua a una celda válida.
    pub fn clamp_cell(&self, x: f32, y: f32) -> (usize, usize) {
        let cx = (x.floor() as i64).clamp(0, self.width as i64 - 1) as usize;
        let cy = (y.floor() as i64).clamp(0, self.height as i64 - 1) as usize;
        (cx, cy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_grid_is_zeroed() {
        let g = Grid::new(8, 4);
        assert_eq!(g.cells(), 32);
        assert!(g.materia.iter().all(|&v| v == 0.0));
        assert_eq!(g.materia.len(), 32);
    }

    #[test]
    fn idx_and_bounds() {
        let g = Grid::new(10, 5);
        assert_eq!(g.idx(3, 2), 23);
        assert!(g.in_bounds(9, 4));
        assert!(!g.in_bounds(10, 4));
        assert!(!g.in_bounds(-1, 0));
    }

    #[test]
    fn clamp_cell_keeps_in_range() {
        let g = Grid::new(10, 10);
        assert_eq!(g.clamp_cell(-5.0, 3.7), (0, 3));
        assert_eq!(g.clamp_cell(99.0, 99.0), (9, 9));
    }
}
