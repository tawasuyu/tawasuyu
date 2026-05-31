//! `llimphi-widget-timeline` — barra de progreso/scrub clickeable.
//!
//! Pattern análogo a `llimphi-widget-slider`/`-progress`: el widget **no
//! mantiene estado**. El caller guarda la posición actual en su `Model`,
//! le pasa la **fracción de avance** (`0.0..=1.0` = posición/duración) y un
//! handler `Fn(f32) -> Option<Msg>` que recibe la fracción **donde el
//! usuario clickeó** (scrub absoluto, estilo VLC). El widget no sabe de
//! tiempo ni de duración: sólo pinta el avance y reporta dónde se clickeó
//! como fracción del ancho de la barra (`on_click_at`). Quien mapea esa
//! fracción a un seek concreto es la app.
//!
//! ```text
//!   [ ██████████▏░░░░░░░░░░░░ ]
//!        recorrido  playhead   resto
//! ```
//!
//! Uso típico (reproductor):
//!
//! ```ignore
//! let frac = pos.as_secs_f64() / dur.as_secs_f64();
//! timeline_view(frac as f32, &TimelinePalette::default(), |f| {
//!     Some(Msg::Command(MediaCommand::SeekTo { fraction: f }))
//! })
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::View;

/// Paleta + dimensiones del timeline. Las medidas viajan acá (igual que
/// `SliderPalette`) porque definen cómo se ve la barra — el caller no
/// toca el `Style` directamente.
#[derive(Debug, Clone, Copy)]
pub struct TimelinePalette {
    /// Color de la pista de fondo (el track entero).
    pub track: Color,
    /// Color del tramo recorrido (de 0 al playhead).
    pub fill: Color,
    /// Color del playhead (la barrita vertical en la posición actual).
    pub knob: Color,
    /// Alto total del widget en pixels.
    pub height: f32,
    /// Radio de las esquinas del track.
    pub radius: f64,
}

impl Default for TimelinePalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TimelinePalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            track: t.bg_button,
            fill: t.accent,
            knob: t.fg_text,
            height: 14.0,
            radius: 7.0,
        }
    }
}

/// Compone una barra de progreso clickeable.
///
/// `progress` es la fracción recorrida (`0.0..=1.0`); se clampea. El
/// handler `on_seek` recibe la fracción `0.0..=1.0` donde el usuario
/// clickeó (`local_x / ancho`) y devuelve el `Msg` a despachar (o `None`
/// para ignorar el click). El widget es stateless: redibujá pasando un
/// `progress` nuevo en cada frame y el playhead avanza solo.
pub fn timeline_view<Msg, F>(progress: f32, palette: &TimelinePalette, on_seek: F) -> View<Msg>
where
    Msg: 'static,
    F: Fn(f32) -> Option<Msg> + Send + Sync + 'static,
{
    let p = progress.clamp(0.0, 1.0);
    let fill_color = palette.fill;
    let knob_color = palette.knob;
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(palette.height),
        },
        ..Default::default()
    })
    .fill(palette.track)
    .radius(palette.radius)
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 2.0 || rect.h <= 2.0 {
            return;
        }
        let pad: f32 = 2.0;
        let x0 = rect.x + pad;
        let y0 = rect.y + pad;
        let w = (rect.w - 2.0 * pad).max(1.0);
        let h = (rect.h - 2.0 * pad).max(1.0);
        // Tramo recorrido.
        let fw = (w * p).max(0.0);
        if fw > 0.5 {
            let fill = Rect::new(x0 as f64, y0 as f64, (x0 + fw) as f64, (y0 + h) as f64);
            scene.fill(Fill::NonZero, Affine::IDENTITY, fill_color, None, &fill);
        }
        // Playhead — fina barra vertical en la posición actual.
        let kx = x0 + fw;
        let kw: f32 = 3.0;
        let knob = Rect::new(
            (kx - kw * 0.5) as f64,
            y0 as f64,
            (kx + kw * 0.5) as f64,
            (y0 + h) as f64,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, knob_color, None, &knob);
    })
    .on_click_at(move |lx, _ly, w, _h| {
        if w <= 0.0 {
            return None;
        }
        on_seek((lx / w).clamp(0.0, 1.0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Msg de prueba: el handler reporta la fracción clickeada.
    #[derive(Debug, PartialEq)]
    struct Seek(f32);

    #[test]
    fn from_theme_usa_colores_semanticos() {
        let t = llimphi_theme::Theme::dark();
        let p = TimelinePalette::from_theme(&t);
        assert_eq!(p.track, t.bg_button);
        assert_eq!(p.fill, t.accent);
        assert_eq!(p.knob, t.fg_text);
    }

    #[test]
    fn construye_sin_panic_en_extremos() {
        // El widget se arma para fracciones fuera de rango (se clampea
        // internamente al pintar) sin reventar.
        let pal = TimelinePalette::default();
        let _ = timeline_view(-0.5, &pal, |f| Some(Seek(f)));
        let _ = timeline_view(0.0, &pal, |f| Some(Seek(f)));
        let _ = timeline_view(1.0, &pal, |f| Some(Seek(f)));
        let _ = timeline_view(2.0, &pal, |f| Some(Seek(f)));
    }
}
