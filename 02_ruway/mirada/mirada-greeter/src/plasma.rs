//! «plasma» — plasma clásico de interferencia, más rico que `waves`.
//!
//! Render **puro**: una grilla fina donde cada celda suma cuatro fuentes seno
//! (dos lineales, una diagonal y una radial pulsante). El valor mapea a una
//! rampa de 3 tramos (oscuro → color base → blanco), dando los lóbulos típicos
//! del plasma. Comparte firma con [`crate::rain::paint`]; `ts` no se usa.

use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

const CELL: f32 = 16.0;

/// Rampa de color del plasma por `n` en `[0, 1]`.
fn plasma_color(n: f32, bright: (u8, u8, u8)) -> Color {
    let (br, bg, bb) = (bright.0 as f32, bright.1 as f32, bright.2 as f32);
    let n = n.clamp(0.0, 1.0);
    let (r, g, b) = if n < 0.5 {
        let f = n / 0.5; // oscuro → color base
        (br * (0.10 + 0.90 * f), bg * (0.10 + 0.90 * f), bb * (0.10 + 0.90 * f))
    } else {
        let f = (n - 0.5) / 0.5; // color base → blanco
        (br + (255.0 - br) * f, bg + (255.0 - bg) * f, bb + (255.0 - bb) * f)
    };
    Color::from_rgba8(r as u8, g as u8, b as u8, 235)
}

pub fn paint(
    scene: &mut vello::Scene,
    _ts: &mut Typesetter,
    rect: PaintRect,
    t: f32,
    bright: (u8, u8, u8),
) {
    if rect.w < CELL || rect.h < CELL {
        return;
    }
    let cols = (rect.w / CELL).ceil() as i32;
    let rows = (rect.h / CELL).ceil() as i32;
    let cx = cols as f32 / 2.0;
    let cy = rows as f32 / 2.0;
    for gy in 0..rows {
        for gx in 0..cols {
            let fx = gx as f32 / 8.0;
            let fy = gy as f32 / 8.0;
            // Centro pulsante para la fuente radial.
            let mcx = cx + 6.0 * (t * 0.5).sin();
            let mcy = cy + 4.0 * (t * 0.43).cos();
            let dx = (gx as f32 - mcx) / 7.0;
            let dy = (gy as f32 - mcy) / 7.0;
            let rad = (dx * dx + dy * dy).sqrt();
            let v = (fx + t * 0.9).sin()
                + (fy + t * 0.7).cos()
                + ((fx + fy) * 0.5 + t).sin()
                + (rad * 2.0 - t * 1.3).sin();
            let n = (v / 4.0 + 1.0) * 0.5; // a [0, 1]
            let x0 = (rect.x + gx as f32 * CELL) as f64;
            let y0 = (rect.y + gy as f32 * CELL) as f64;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                plasma_color(n, bright),
                None,
                &Rect::new(x0, y0, x0 + CELL as f64, y0 + CELL as f64),
            );
        }
    }
}
