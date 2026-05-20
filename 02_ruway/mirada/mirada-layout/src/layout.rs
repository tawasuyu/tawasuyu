//! Modos de teselado — cómo se reparte la pantalla entre ventanas.

use serde::{Deserialize, Serialize};

use crate::geometry::{split, Rect};

/// Estrategia de teselado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutMode {
    /// Una ventana maestra a la izquierda; el resto apiladas a la derecha.
    MasterStack,
    /// Todas a pantalla completa, superpuestas — sólo se ve la enfocada.
    Monocle,
    /// Rejilla uniforme.
    Grid,
    /// Columnas verticales de igual ancho.
    Columns,
}

/// Parámetros del teselado.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LayoutParams {
    pub mode: LayoutMode,
    /// Fracción del ancho para la ventana maestra en `MasterStack`
    /// (se acota a `0.05..=0.95`).
    pub master_ratio: f32,
    /// Margen en píxeles alrededor de cada ventana.
    pub gap: i32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self { mode: LayoutMode::MasterStack, master_ratio: 0.6, gap: 8 }
    }
}

/// Calcula el rectángulo de cada una de las `count` ventanas dentro de
/// `screen`. El vector resultante tiene exactamente `count` elementos,
/// en el mismo orden que las ventanas.
pub fn tile(screen: Rect, count: usize, params: &LayoutParams) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let cells = match params.mode {
        LayoutMode::Monocle => vec![screen; count],
        LayoutMode::Columns => columns(screen, count),
        LayoutMode::Grid => grid(screen, count),
        LayoutMode::MasterStack => master_stack(screen, count, params.master_ratio),
    };
    // El margen se aplica al final, uniforme para todos los modos.
    cells.into_iter().map(|c| c.inset(params.gap)).collect()
}

/// Columnas verticales de igual ancho.
fn columns(screen: Rect, count: usize) -> Vec<Rect> {
    split(screen.w, count)
        .into_iter()
        .map(|(off, w)| Rect::new(screen.x + off, screen.y, w, screen.h))
        .collect()
}

/// Rejilla `cols × rows` lo más cuadrada posible.
fn grid(screen: Rect, count: usize) -> Vec<Rect> {
    let cols = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(cols);
    let col_parts = split(screen.w, cols);
    let row_parts = split(screen.h, rows);
    (0..count)
        .map(|i| {
            let (cx, cw) = col_parts[i % cols];
            let (ry, rh) = row_parts[i / cols];
            Rect::new(screen.x + cx, screen.y + ry, cw, rh)
        })
        .collect()
}

/// Ventana maestra a la izquierda + pila a la derecha.
fn master_stack(screen: Rect, count: usize, ratio: f32) -> Vec<Rect> {
    if count == 1 {
        return vec![screen];
    }
    let ratio = ratio.clamp(0.05, 0.95);
    let master_w = (screen.w as f32 * ratio).round() as i32;
    let master = Rect::new(screen.x, screen.y, master_w, screen.h);

    let stack_x = screen.x + master_w;
    let stack_w = screen.w - master_w;
    let mut out = vec![master];
    for (off, h) in split(screen.h, count - 1) {
        out.push(Rect::new(stack_x, screen.y + off, stack_w, h));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };

    fn params(mode: LayoutMode) -> LayoutParams {
        LayoutParams { mode, master_ratio: 0.6, gap: 0 }
    }

    #[test]
    fn empty_count_yields_no_rects() {
        assert!(tile(SCREEN, 0, &params(LayoutMode::Grid)).is_empty());
    }

    #[test]
    fn tile_count_matches_window_count() {
        for mode in [
            LayoutMode::MasterStack,
            LayoutMode::Monocle,
            LayoutMode::Grid,
            LayoutMode::Columns,
        ] {
            for n in 1..=9 {
                assert_eq!(tile(SCREEN, n, &params(mode)).len(), n);
            }
        }
    }

    #[test]
    fn monocle_gives_every_window_the_full_screen() {
        for r in tile(SCREEN, 4, &params(LayoutMode::Monocle)) {
            assert_eq!(r, SCREEN);
        }
    }

    #[test]
    fn columns_partition_the_width_exactly() {
        let rects = tile(SCREEN, 3, &params(LayoutMode::Columns));
        assert_eq!(rects.iter().map(|r| r.w).sum::<i32>(), 1920);
        // Todas ocupan el alto completo.
        assert!(rects.iter().all(|r| r.h == 1080));
    }

    #[test]
    fn master_stack_master_takes_its_ratio() {
        let rects = tile(SCREEN, 3, &params(LayoutMode::MasterStack));
        // 60% de 1920 = 1152.
        assert_eq!(rects[0].w, 1152);
        // Las dos de la pila comparten el resto del ancho y el alto.
        assert_eq!(rects[1].w, 1920 - 1152);
        assert_eq!(rects[1].h + rects[2].h, 1080);
    }

    #[test]
    fn master_stack_single_window_fills_screen() {
        let rects = tile(SCREEN, 1, &params(LayoutMode::MasterStack));
        assert_eq!(rects[0], SCREEN);
    }

    #[test]
    fn grid_tiles_cover_the_screen_without_overlap() {
        // 4 ventanas → rejilla 2×2, cada una un cuarto.
        let rects = tile(SCREEN, 4, &params(LayoutMode::Grid));
        let total: i64 = rects.iter().map(|r| r.area()).sum();
        assert_eq!(total, SCREEN.area());
    }

    #[test]
    fn gap_shrinks_every_window() {
        let p = LayoutParams { mode: LayoutMode::Columns, master_ratio: 0.6, gap: 10 };
        for r in tile(SCREEN, 2, &p) {
            // Cada celda de 960 de ancho se encoge 20 (10 por lado).
            assert_eq!(r.w, 960 - 20);
            assert_eq!(r.h, 1080 - 20);
        }
    }

    #[test]
    fn layout_is_deterministic() {
        let p = params(LayoutMode::Grid);
        assert_eq!(tile(SCREEN, 7, &p), tile(SCREEN, 7, &p));
    }
}
