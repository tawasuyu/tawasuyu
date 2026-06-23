//! «fire» — fuego ascendente para el fondo del greeter.
//!
//! Render **puro**: una grilla gruesa de celdas; cada columna tiene una altura
//! de llama que late con el tiempo (suma de senos + hash por columna), y cada
//! celda se tiñe por su intensidad de calor (negro → color base → blanco). Sin
//! estado entre frames. Comparte firma con [`crate::rain::paint`]; `ts` no se
//! usa.

use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

const CELL: f32 = 16.0;

/// splitmix64.
fn hash(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
fn hf(x: u64) -> f32 {
    (hash(x) >> 40) as f32 / (1u64 << 24) as f32
}

/// Color del fuego por intensidad `i` en `[0, 1]`: brasa tenue → color base →
/// blanco en la punta. `bright` da el tono dominante.
fn fire_color(i: f32, bright: (u8, u8, u8)) -> Color {
    let i = i.clamp(0.0, 1.0);
    let (br, bg, bb) = (bright.0 as f32, bright.1 as f32, bright.2 as f32);
    // Brasa: una fracción oscura/rojiza del color base.
    let (r, g, b) = if i < 0.5 {
        let f = i / 0.5; // 0 → brasa, 1 → color base
        (br * (0.25 + 0.75 * f), bg * (0.05 + 0.45 * f), bb * (0.05 + 0.25 * f))
    } else {
        let f = (i - 0.5) / 0.5; // color base → blanco
        (
            br + (255.0 - br) * f,
            bg + (255.0 - bg) * f,
            bb + (255.0 - bb) * f,
        )
    };
    let a = (40.0 + 215.0 * i.powf(0.6)).clamp(0.0, 255.0) as u8;
    Color::from_rgba8(r as u8, g as u8, b as u8, a)
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
    for gx in 0..cols {
        // Altura de llama de la columna en [0.35, 1.0] de la pantalla, con
        // parpadeo continuo (dos senos desfasados + hash de columna).
        let base = hash(gx as u64 ^ 0x1234_5678);
        let phase = hf(base) * 6.28;
        let flick = 0.5
            + 0.25 * (t * 3.1 + phase).sin()
            + 0.25 * (t * 7.3 + phase * 1.7).sin();
        let heat_col = (0.45 + 0.55 * flick).clamp(0.2, 1.0);
        for gy in 0..rows {
            // 0 abajo → 1 arriba.
            let y_frac = 1.0 - (gy as f32 / rows as f32);
            if y_frac > heat_col {
                continue; // por encima de la llama: nada
            }
            // Intensidad: 1 en la base, cae hacia la punta de la llama.
            let i = (heat_col - y_frac) / heat_col.max(0.001);
            // Ondulación lateral leve del calor.
            let wob = 0.85 + 0.15 * (gy as f32 * 0.5 + t * 4.0 + phase).sin();
            let col = fire_color(i * wob, bright);
            let x0 = (rect.x + gx as f32 * CELL) as f64;
            let y0 = (rect.y + gy as f32 * CELL) as f64;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                col,
                None,
                &Rect::new(x0, y0, x0 + CELL as f64, y0 + CELL as f64),
            );
        }
    }
}
