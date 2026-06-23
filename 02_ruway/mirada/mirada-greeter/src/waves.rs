//! «waves» — plasma sinusoidal de baja resolución para el fondo del greeter.
//!
//! Render **puro**: una grilla gruesa de celdas, cada una teñida por la suma de
//! tres ondas seno de `(x, y, t)`. Barato (cientos de rects), suave, y comparte
//! firma con [`crate::rain::paint`] para enchufarse al despachador del fondo.
//! `ts` no se usa (no hay glifos).

use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

/// Lado de la celda de plasma, en px. Grueso a propósito: suaviza y abarata.
const CELL: f32 = 22.0;

/// Pinta un frame de plasma sobre `rect`. `t` en segundos; `bright` el color
/// base (RGB del tema o de la paleta elegida).
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
    for gy in 0..rows {
        for gx in 0..cols {
            let fx = gx as f32 * CELL / 90.0;
            let fy = gy as f32 * CELL / 90.0;
            // Tres ondas desfasadas → interferencia tipo plasma en [0, 1].
            let v = (fx + t * 0.7).sin()
                + (fy + t * 0.5).sin()
                + ((fx + fy) * 0.6 + t * 0.9).sin();
            let n = (v / 3.0 + 1.0) * 0.5; // normaliza a [0, 1]
            // Brillo modulado por el plasma; el fondo no se va a negro puro.
            let f = 0.12 + 0.88 * n;
            let col = Color::from_rgba8(
                (bright.0 as f32 * f) as u8,
                (bright.1 as f32 * f) as u8,
                (bright.2 as f32 * f) as u8,
                235,
            );
            let x0 = (rect.x + gx as f32 * CELL) as f64;
            let y0 = (rect.y + gy as f32 * CELL) as f64;
            let cell = Rect::new(x0, y0, x0 + CELL as f64, y0 + CELL as f64);
            scene.fill(Fill::NonZero, Affine::IDENTITY, col, None, &cell);
        }
    }
}
