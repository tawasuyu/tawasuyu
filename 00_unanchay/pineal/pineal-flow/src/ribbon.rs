//! Tesela y dibuja las bandas (ribbons) de un Sankey.
//!
//! Cada banda es una franja con curva S: `x` avanza lineal entre nodos y
//! `y` interpola con `smoothstep`, lo que da tangentes horizontales en
//! ambos extremos (el look clásico de Sankey). Se emite como un triangle
//! strip `[top0,bot0,top1,bot1,…]`, un draw call por ribbon.

use crate::layout::{LinkBand, SankeyLayout};
use pineal_render::{Canvas, Color};

/// Segmentos por ribbon — controla la suavidad de la curva.
const RIBBON_SEGMENTS: usize = 24;

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Tesela una banda en coords interleaved `[x,y,…]` de un triangle strip.
pub fn ribbon_strip(band: &LinkBand) -> Vec<f32> {
    let mut coords = Vec::with_capacity((RIBBON_SEGMENTS + 1) * 4);
    for i in 0..=RIBBON_SEGMENTS {
        let t = i as f32 / RIBBON_SEGMENTS as f32;
        let e = smoothstep(t);
        let x_top = band.src_top.x + (band.dst_top.x - band.src_top.x) * t;
        let y_top = band.src_top.y + (band.dst_top.y - band.src_top.y) * e;
        let x_bot = band.src_bot.x + (band.dst_bot.x - band.src_bot.x) * t;
        let y_bot = band.src_bot.y + (band.dst_bot.y - band.src_bot.y) * e;
        coords.push(x_top);
        coords.push(y_top);
        coords.push(x_bot);
        coords.push(y_bot);
    }
    coords
}

/// Dibuja una sola banda con el color dado.
pub fn paint_ribbon(band: &LinkBand, color: Color, canvas: &mut dyn Canvas) {
    let coords = ribbon_strip(band);
    let colors = vec![color; coords.len() / 2];
    canvas.fill_triangle_strip(&coords, &colors);
}

/// Dibuja un Sankey completo: ribbons primero (al fondo), nodos encima.
pub fn paint_sankey(
    layout: &SankeyLayout,
    node_color: Color,
    link_color: Color,
    canvas: &mut dyn Canvas,
) {
    for band in &layout.links {
        paint_ribbon(band, link_color, canvas);
    }
    for nb in &layout.nodes {
        if nb.rect.w > 0.0 && nb.rect.h > 0.0 {
            canvas.fill_rect(nb.rect, node_color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{compute_layout, SankeyLink, SankeyNode};
    use pineal_render::{PlanRecorder, Rect, RenderCmd};

    #[test]
    fn ribbon_strip_has_expected_vertex_count() {
        let band = LinkBand {
            link: 0,
            src_top: pineal_render::Point::new(0.0, 0.0),
            src_bot: pineal_render::Point::new(0.0, 10.0),
            dst_top: pineal_render::Point::new(100.0, 50.0),
            dst_bot: pineal_render::Point::new(100.0, 60.0),
        };
        let coords = ribbon_strip(&band);
        assert_eq!(coords.len(), (RIBBON_SEGMENTS + 1) * 4);
    }

    #[test]
    fn paint_sankey_emits_nodes_and_ribbons() {
        let nodes = vec![
            SankeyNode::new("a"),
            SankeyNode::new("b"),
            SankeyNode::new("c"),
        ];
        let links = [
            SankeyLink { source: 0, target: 1, value: 5.0 },
            SankeyLink { source: 1, target: 2, value: 3.0 },
        ];
        let layout = compute_layout(
            &nodes,
            &links,
            Rect::new(0.0, 0.0, 300.0, 150.0),
            18.0,
            6.0,
        );
        let mut rec = PlanRecorder::new();
        paint_sankey(&layout, Color::from_hex(0x335577), Color::from_hex(0x88aacc), &mut rec);
        let cmds = rec.into_plan().cmds;
        let rects = cmds.iter().filter(|c| matches!(c, RenderCmd::FillRect { .. })).count();
        let strips = cmds.iter().filter(|c| matches!(c, RenderCmd::FillTriangleStrip { .. })).count();
        assert_eq!(rects, 3, "un fill_rect por nodo");
        assert_eq!(strips, 2, "un triangle strip por link");
    }
}
