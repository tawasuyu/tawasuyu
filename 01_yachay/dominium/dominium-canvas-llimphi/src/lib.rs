//! `dominium-canvas-llimphi` — el único crate de dominium que importa
//! `llimphi-ui`.
//!
//! Toda la cadena `dominium-core → physics → iso → render-plan` es
//! agnóstica de backend. Este crate cierra el circuito: una función
//! [`canvas_view`] que recibe un [`RenderPlan`] ya resuelto y devuelve
//! un `View<Msg>` con `paint_with` que pinta los quads vía vello,
//! centrando la maqueta en los bounds asignados por taffy.
//!
//! Reemplazo Llimphi del `dominium-canvas-gpui`. Igual contrato:
//! el `Element` (acá `View`) no guarda estado entre frames — el host
//! reconstruye el View con el `RenderPlan` del frame actual.

#![forbid(unsafe_code)]

use dominium_render_plan::{Color as PlanColor, RenderPlan};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, TextBlock};
use llimphi_ui::{PaintRect, View};

/// Convierte el RGBA lineal del plan (`[f32;4]` en [0,1]) al `Color`
/// de peniko. Mantiene la convención sin gamma del backend GPUI.
fn plan_color(c: PlanColor) -> Color {
    let to_byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to_byte(c[0]), to_byte(c[1]), to_byte(c[2]), to_byte(c[3]))
}

/// Construye un View que pinta `plan` en su rect. Si `background` está
/// presente, se pinta como fondo sólido antes de los quads (el `fill`
/// del View ya lo cubriría — pero esta API mantiene el shape del
/// `DominiumCanvas::background` del backend GPUI).
pub fn canvas_view<Msg>(plan: RenderPlan, background: Option<Color>) -> View<Msg>
where
    Msg: Clone + 'static,
{
    // El plan es Send + Sync (Vec<Quad> con Copy). Lo movemos a la
    // closure de paint; el runtime la invoca por frame.
    let view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    });
    let view = if let Some(bg) = background {
        view.fill(bg)
    } else {
        view
    };
    view.paint_with(move |scene, ts, rect: PaintRect| {
        if plan.quads.is_empty() && plan.glyphs.is_empty() {
            return;
        }
        // Centra la maqueta: el centro de la caja envolvente del plan
        // se alinea con el centro del rect del nodo.
        let plan_cx = (plan.min_x + plan.max_x) * 0.5;
        let plan_cy = (plan.min_y + plan.max_y) * 0.5;
        let off_x = (rect.x + rect.w * 0.5 - plan_cx) as f64;
        let off_y = (rect.y + rect.h * 0.5 - plan_cy) as f64;

        for q in &plan.quads {
            let x0 = q.x as f64 + off_x;
            let y0 = q.y as f64 + off_y;
            let x1 = x0 + q.w as f64;
            let y1 = y0 + q.h as f64;
            let r = KurboRect::new(x0, y0, x1, y1);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                plan_color(q.color),
                None,
                &r,
            );
        }
        // Glifos por encima de los quads, sin re-shaping cacheado (cada
        // glyph hace un layout breve; son pocos: ~decenas).
        for gl in &plan.glyphs {
            let s = gl.ch.to_string();
            let block = TextBlock::simple(
                &s,
                gl.size_px,
                plan_color(gl.color),
                (gl.x as f64 + off_x, gl.y as f64 + off_y),
            );
            draw_block(scene, ts, &block);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_red_round_trips() {
        let c = plan_color([1.0, 0.0, 0.0, 1.0]).to_rgba8();
        assert_eq!((c.r, c.g, c.b, c.a), (255, 0, 0, 255));
    }

    #[test]
    fn alpha_passes_through() {
        let c = plan_color([0.0, 0.0, 1.0, 0.25]).to_rgba8();
        assert_eq!(c.b, 255);
        assert_eq!(c.a, 64); // 0.25 * 255 = 63.75 ~> 64
    }

    #[test]
    fn out_of_range_clamps() {
        let c = plan_color([1.5, -0.2, 0.5, 1.0]).to_rgba8();
        assert_eq!((c.r, c.g, c.b), (255, 0, 128));
    }
}
