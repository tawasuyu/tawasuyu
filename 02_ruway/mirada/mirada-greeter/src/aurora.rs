//! «aurora» — cortinas de aurora boreal para el fondo del greeter.
//!
//! Render **puro**: varias cortinas verticales cuyo centro ondula con `x` y el
//! tiempo; cada celda se ilumina según una caída gaussiana respecto al centro
//! de la cortina más cercana, más fuerte en la mitad superior. Sin estado entre
//! frames. Comparte firma con [`crate::rain::paint`]; `ts` no se usa.

use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

const CELL_W: f32 = 12.0;
const CELL_H: f32 = 14.0;

/// Número de cortinas superpuestas.
const CURTAINS: usize = 3;

pub fn paint(
    scene: &mut vello::Scene,
    _ts: &mut Typesetter,
    rect: PaintRect,
    t: f32,
    bright: (u8, u8, u8),
) {
    if rect.w < CELL_W || rect.h < CELL_H {
        return;
    }
    let cols = (rect.w / CELL_W).ceil() as i32;
    let rows = (rect.h / CELL_H).ceil() as i32;
    let rows_f = rows as f32;
    for gx in 0..cols {
        let xf = gx as f32;
        for gy in 0..rows {
            let yf = gy as f32;
            // Intensidad = máxima cercanía a alguna cortina.
            let mut inten = 0.0_f32;
            for k in 0..CURTAINS {
                let kf = k as f32;
                // Centro de la cortina k (en filas): cuelga del tercio superior y
                // ondula con x y el tiempo; cada cortina con su fase.
                let center = rows_f * (0.18 + 0.12 * kf)
                    + rows_f * 0.16 * (xf * 0.06 + t * (0.5 + 0.2 * kf) + kf * 2.1).sin();
                let sigma = rows_f * (0.10 + 0.03 * kf);
                let d = (yf - center) / sigma.max(0.001);
                let g = (-0.5 * d * d).exp();
                // Brillo serpenteante a lo largo de la cortina.
                let shimmer = 0.6 + 0.4 * (xf * 0.3 + t * 1.7 + kf).sin();
                inten = inten.max(g * shimmer);
            }
            // Se desvanece hacia abajo (la aurora vive arriba).
            let fade = (1.0 - yf / rows_f).clamp(0.0, 1.0).powf(0.6);
            let a = (inten * fade * 210.0).clamp(0.0, 230.0) as u8;
            if a < 6 {
                continue; // celda casi invisible: ahorra fills
            }
            // Mezcla hacia blanco en los núcleos más brillantes.
            let lift = (inten - 0.6).max(0.0) / 0.4;
            let mix = |c: u8| (c as f32 + (255.0 - c as f32) * 0.5 * lift) as u8;
            let col = Color::from_rgba8(mix(bright.0), mix(bright.1), mix(bright.2), a);
            let x0 = (rect.x + xf * CELL_W) as f64;
            let y0 = (rect.y + yf * CELL_H) as f64;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                col,
                None,
                &Rect::new(x0, y0, x0 + CELL_W as f64, y0 + CELL_H as f64),
            );
        }
    }
}
