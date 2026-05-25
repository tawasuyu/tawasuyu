//! Painter agnóstico del treemap: tiles → `fill_rect` contra un `Canvas`.

use crate::squarify::squarify;
use pineal_render::{Canvas, Color, Rect};

/// Una tile del treemap: su peso (área relativa) y su color.
#[derive(Debug, Clone, Copy)]
pub struct Tile {
    pub weight: f64,
    pub color: Color,
}

impl Tile {
    pub fn new(weight: f64, color: Color) -> Self {
        Self { weight, color }
    }
}

/// Dibuja un treemap de `tiles` dentro de `area`. `gap` es el margen
/// (en px) que se recorta de cada lado de cada tile, para separarlas
/// visualmente. Tiles cuya área no alcanza para el gap se omiten.
pub fn paint_treemap(tiles: &[Tile], area: Rect, gap: f32, canvas: &mut dyn Canvas) {
    let weights: Vec<f64> = tiles.iter().map(|t| t.weight).collect();
    let rects = squarify(&weights, area);
    for (tile, r) in tiles.iter().zip(&rects) {
        let inset = Rect::new(
            r.x + gap,
            r.y + gap,
            r.w - 2.0 * gap,
            r.h - 2.0 * gap,
        );
        if inset.w > 0.0 && inset.h > 0.0 {
            canvas.fill_rect(inset, tile.color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pineal_render::{PlanRecorder, RenderCmd};

    #[test]
    fn one_fill_rect_per_visible_tile() {
        let tiles = [
            Tile::new(3.0, Color::WHITE),
            Tile::new(2.0, Color::BLACK),
            Tile::new(1.0, Color::from_hex(0x00ff00)),
        ];
        let mut rec = PlanRecorder::new();
        paint_treemap(&tiles, Rect::new(0.0, 0.0, 300.0, 200.0), 1.0, &mut rec);
        let n = rec
            .into_plan()
            .cmds
            .iter()
            .filter(|c| matches!(c, RenderCmd::FillRect { .. }))
            .count();
        assert_eq!(n, 3);
    }

    #[test]
    fn zero_weight_tile_is_skipped() {
        let tiles = [Tile::new(1.0, Color::WHITE), Tile::new(0.0, Color::BLACK)];
        let mut rec = PlanRecorder::new();
        paint_treemap(&tiles, Rect::new(0.0, 0.0, 100.0, 100.0), 0.0, &mut rec);
        let n = rec
            .into_plan()
            .cmds
            .iter()
            .filter(|c| matches!(c, RenderCmd::FillRect { .. }))
            .count();
        assert_eq!(n, 1);
    }
}
