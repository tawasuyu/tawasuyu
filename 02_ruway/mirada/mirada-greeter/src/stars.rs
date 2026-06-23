//! «starfield» — campo de estrellas en *warp* para el fondo del greeter.
//!
//! Mismo principio que [`crate::rain`]: render **puro y determinista**. Cada
//! estrella deriva su rumbo y fase del hash de su índice; sólo avanza un `f32`
//! de reloj. Las estrellas salen del centro hacia los bordes, acelerando y
//! aclarándose — el clásico «viaje a la velocidad de la luz».
//!
//! Firma idéntica a `rain::paint` para que el despachador del fondo
//! (`crate::bg::paint`) las trate por igual; `ts` no se usa (no hay glifos).

use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

/// Semilla del campo (estable entre arranques).
const SEED: u64 = 0x5354_4152_0001;

/// splitmix64 — hash entero rápido.
fn hash(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash → `f32` en `[0, 1)`.
fn hf(x: u64) -> f32 {
    (hash(x) >> 40) as f32 / (1u64 << 24) as f32
}

/// Pinta un frame del campo de estrellas sobre `rect`. `t` en segundos;
/// `bright` el color base (RGB del tema o de la paleta elegida).
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
    let cx = (rect.x + rect.w / 2.0) as f64;
    let cy = (rect.y + rect.h / 2.0) as f64;
    // La estrella «cae» del centro al borde; el radio máximo cubre la esquina.
    let max_r = ((rect.w * rect.w + rect.h * rect.h).sqrt() / 2.0).max(1.0);
    // Densidad proporcional al área, con techo para no ahogar la GPU.
    let count = ((rect.w * rect.h) / 6500.0).clamp(60.0, 420.0) as u64;

    for i in 0..count {
        let base = hash(SEED ^ i.wrapping_mul(0x1_0000_01B3));
        let ang = hf(base ^ 1) * std::f32::consts::TAU;
        let speed = 0.18 + hf(base ^ 2) * 0.55; // fracción de max_r por segundo
        let phase = hf(base ^ 3);
        // Progreso 0→1 del centro al borde, con wrap continuo.
        let p = (phase + t * speed).fract();
        // Aceleración cuadrática: arranca lento, sale disparada.
        let rr = p * p;
        let r = rr * max_r;
        let (sa, ca) = ang.sin_cos();
        let px = cx + (ca * r) as f64;
        let py = cy + (sa * r) as f64;
        // Brillo y tamaño crecen con la distancia (efecto de acercarse).
        let a = (40.0 + 215.0 * rr).clamp(0.0, 255.0) as u8;
        let size = (0.4 + rr * 2.1) as f64;
        let col = Color::from_rgba8(bright.0, bright.1, bright.2, a);
        scene.fill(Fill::NonZero, Affine::IDENTITY, col, None, &Circle::new((px, py), size));
    }
}
