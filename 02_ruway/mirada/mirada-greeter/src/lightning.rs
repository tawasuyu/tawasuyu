//! «lightning» — rayos ramificados sobre fondo oscuro para el greeter.
//!
//! Render **puro y determinista**: el tiempo se parte en ventanas de período
//! fijo; en cada ventana cae un rayo cuyo trazo (y sus ramas) se derivan por
//! hashing del índice de ventana. Mientras el rayo está «vivo» se pinta un
//! destello tenue de pantalla completa que decae. Sin estado entre frames.
//! Comparte firma con [`crate::rain::paint`]; `ts` no se usa.

use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Point, Rect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

/// Período entre rayos (s) y duración del fogonazo (s).
const PERIOD: f32 = 2.0;
const FLASH: f32 = 0.28;
/// Segmentos verticales del trazo principal.
const SEGMENTS: i32 = 14;

fn hash(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
fn hf(x: u64) -> f32 {
    (hash(x) >> 40) as f32 / (1u64 << 24) as f32
}

/// Construye el trazo zigzagueante de un rayo desde `(x0, y0)` bajando `h`,
/// con `seed` para el jitter horizontal. `amp` escala el zigzag.
fn bolt_path(x0: f32, y0: f32, h: f32, seed: u64, amp: f32, segs: i32) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(Point::new(x0 as f64, y0 as f64));
    let mut x = x0;
    for s in 1..=segs {
        let y = y0 + h * (s as f32 / segs as f32);
        let jitter = (hf(seed ^ s as u64) - 0.5) * 2.0 * amp;
        x += jitter;
        p.line_to(Point::new(x as f64, y as f64));
    }
    p
}

pub fn paint(
    scene: &mut vello::Scene,
    _ts: &mut Typesetter,
    rect: PaintRect,
    t: f32,
    bright: (u8, u8, u8),
) {
    if rect.w < 8.0 || rect.h < 8.0 {
        return;
    }
    let idx = (t / PERIOD).floor() as u64;
    let local = t - idx as f32 * PERIOD;
    if local > FLASH {
        return; // entre rayos: fondo oscuro (la raíz ya pinta el bg)
    }
    // Curva de vida 1→0 durante el fogonazo (decae rápido).
    let life = (1.0 - local / FLASH).powf(1.4);

    // Fogonazo de pantalla completa, muy tenue.
    let flash_a = (life * 36.0) as u8;
    if flash_a > 0 {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(bright.0, bright.1, bright.2, flash_a),
            None,
            &Rect::new(
                rect.x as f64,
                rect.y as f64,
                (rect.x + rect.w) as f64,
                (rect.y + rect.h) as f64,
            ),
        );
    }

    // Trazo principal, desde arriba.
    let x0 = rect.x + hf(idx ^ 0xBEEF) * rect.w;
    let amp = (rect.w * 0.03).clamp(8.0, 60.0);
    let main = bolt_path(x0, rect.y, rect.h, idx, amp, SEGMENTS);

    let glow = Color::from_rgba8(bright.0, bright.1, bright.2, (life * 120.0) as u8);
    let core = Color::from_rgba8(
        (bright.0 as u16 + 255) as u8 / 2 + 64,
        (bright.1 as u16 + 255) as u8 / 2 + 64,
        (bright.2 as u16 + 255) as u8 / 2 + 64,
        (life * 255.0) as u8,
    );
    // Resplandor ancho + núcleo brillante (doble pasada).
    scene.stroke(&Stroke::new(6.0), Affine::IDENTITY, glow, None, &main);
    scene.stroke(&Stroke::new(1.6), Affine::IDENTITY, core, None, &main);

    // Dos ramas que salen de puntos intermedios del trazo.
    for b in 0..2u64 {
        let frac = 0.35 + 0.3 * hf(idx ^ (0xA1 + b));
        let bx = x0 + (hf(idx ^ (0xB2 + b)) - 0.5) * amp * 2.0;
        let by = rect.y + rect.h * frac;
        let bh = rect.h * (0.18 + 0.18 * hf(idx ^ (0xC3 + b)));
        let branch = bolt_path(bx, by, bh, idx ^ (0xD4 + b), amp * 0.7, SEGMENTS / 2);
        scene.stroke(&Stroke::new(3.0), Affine::IDENTITY, glow, None, &branch);
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, core, None, &branch);
    }
}
