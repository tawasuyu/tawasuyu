//! `llimphi-widget-switch` — toggle binario (track + thumb).
//!
//! Render-only: la app guarda el `bool` en su modelo y dispatcha el
//! Msg de toggle al click. Visualmente:
//! - Track horizontal (40×22 default) con color del estado activo.
//! - Thumb circular (18px) que se posiciona a la izquierda (off) o
//!   derecha (on) del track.
//!
//! Para animar la transición, la app puede guardar un `Tween<f32>` con
//! el progreso 0→1 y leerlo desde `view` para interpolar la posición
//! del thumb. Sin tween la transición es instantánea — funcional pero
//! menos elegante.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, Position, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_theme::Theme;

/// Paleta del switch.
#[derive(Debug, Clone, Copy)]
pub struct SwitchPalette {
    pub track_off: Color,
    pub track_on: Color,
    pub thumb: Color,
}

impl SwitchPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            track_off: t.bg_button,
            track_on: t.accent,
            thumb: t.fg_text,
        }
    }
}

const TRACK_W: f32 = 40.0;
const TRACK_H: f32 = 22.0;
const THUMB_R: f32 = 9.0; // radio en px → diámetro 18
const PAD: f32 = 2.0;

/// Construye un switch. `progress` en `[0.0, 1.0]` indica la
/// posición animada del thumb (0 = off, 1 = on). Para la transición
/// instantánea usar `if state { 1.0 } else { 0.0 }`.
///
/// `on_toggle` se dispatcha al click; la app actualiza su `bool` y
/// (opcionalmente) lanza un `Tween` que actualiza `progress` por frame.
pub fn switch_view<Msg: Clone + 'static>(
    progress: f32,
    on_toggle: Msg,
    palette: &SwitchPalette,
) -> View<Msg> {
    let p = progress.clamp(0.0, 1.0);

    // Track color interpola entre off y on según progress.
    let track_color = lerp_color(palette.track_off, palette.track_on, p);

    // Thumb absolute dentro del track. Range del centro: PAD+THUMB_R a TRACK_W-PAD-THUMB_R.
    let min_x = PAD;
    let max_x = TRACK_W - PAD - THUMB_R * 2.0;
    let thumb_x = min_x + (max_x - min_x) * p;
    let thumb_y = (TRACK_H - THUMB_R * 2.0) * 0.5;

    let thumb = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(thumb_x),
            top: length(thumb_y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(THUMB_R * 2.0),
            height: length(THUMB_R * 2.0),
        },
        ..Default::default()
    })
    .fill(palette.thumb)
    .radius(THUMB_R as f64)
    .paint_with(move |scene, _ts, rect| {
        // Highlight radial pequeño en cuadrante superior — el thumb se
        // lee como esfera, no como círculo plano. Mismo patrón que el
        // dot del badge (P6).
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.32) as f64;
        let r = (rect.w as f64 * 0.18).max(1.0);
        let highlight = Color::from_rgba8(255, 255, 255, 70);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            highlight,
            None,
            &Circle::new((cx, cy), r),
        );
    });

    let track_radius = (TRACK_H * 0.5) as f64;
    View::new(Style {
        size: Size {
            width: length(TRACK_W),
            height: length(TRACK_H),
        },
        ..Default::default()
    })
    .fill(track_color)
    .radius(track_radius)
    .paint_with(move |scene, _ts, rect| {
        // Gloss superior en el track — pill con luz cayendo desde arriba.
        // El track interpola color (off/on) en el fill, el gloss queda
        // estable encima en ambos estados.
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, RoundedRect};
        use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let x0 = rect.x as f64;
        let y0 = rect.y as f64;
        let x1 = (rect.x + rect.w) as f64;
        let y1 = (rect.y + rect.h) as f64;
        let y_mid = y0 + (y1 - y0) * 0.5;
        let rr = RoundedRect::new(x0, y0, x1, y1, track_radius);
        let top = Color::from_rgba8(255, 255, 255, 28);
        let bot = Color::from_rgba8(255, 255, 255, 0);
        let g = Gradient::new_linear(Point::new(x0, y0), Point::new(x0, y_mid))
            .with_stops([top, bot].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
    })
    // Semántica: track switch (Checkbox con `pressed` para que el lector
    // diga "interruptor on/off" en vez de "casilla marcada"). Sin un rol
    // dedicado en accesskit; Button + pressed cubre la intención.
    .role(llimphi_ui::Role::Button)
    .aria_pressed(p >= 0.5)
    .on_click(on_toggle)
    .children(vec![thumb])
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let [r0, g0, b0, a0] = a.components;
    let [r1, g1, b1, a1] = b.components;
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    AlphaColor::new([
        r0 + (r1 - r0) * t,
        g0 + (g1 - g0) * t,
        b0 + (b1 - b0) * t,
        a0 + (a1 - a0) * t,
    ])
}
