//! `llimphi-widget-slider` — slider horizontal con label + track + valor.
//!
//! Pattern análogo a `llimphi-widget-splitter`: el widget no mantiene
//! estado. El caller guarda el valor actual en su `Model` y le pasa un
//! handler `Fn(DragPhase, f32) -> Option<Msg>` que recibe **el delta de
//! valor** (no el delta de pixels) entre eventos consecutivos. El widget
//! traduce internamente `dx_pixels` a `dv` usando `track_width`.
//!
//! Visualmente es un *fillbar*: el track entero es draggable y se rellena
//! una fracción proporcional a `(value - min) / (max - min)`. No hay
//! pulgar separado — el límite entre relleno y vacío es el indicador.
//!
//! Layout fila:
//!
//! ```text
//!   [ label_width ]  [ ████░░░░░░ ]  [ value_width ]
//!        "psique"      0.4 / 1.0           " 0.40"
//! ```
//!
//! Uso típico (sliders sobre `LayerMods` de un Concepto):
//!
//! ```ignore
//! slider_view(
//!     "psique",
//!     model.selected.mods.psique,
//!     -1.0, 1.0,
//!     &palette,
//!     |phase, dv| match phase {
//!         DragPhase::Move => Some(Msg::EditMod(Layer::Psique, dv)),
//!         DragPhase::End => None,
//!     },
//! )
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};

/// Paleta del slider. Las dimensiones también viajan acá porque definen
/// el layout fila — el caller no toca el `Style` del slider directamente.
#[derive(Debug, Clone, Copy)]
pub struct SliderPalette {
    pub track: Color,
    pub track_filled: Color,
    pub track_hover: Color,
    pub fg_label: Color,
    pub fg_value: Color,
    pub radius: f64,
    /// Alto total del widget en pixels.
    pub row_height: f32,
    /// Ancho fijo del bloque del label (a la izquierda).
    pub label_width: f32,
    /// Ancho fijo del bloque del valor numérico (a la derecha).
    pub value_width: f32,
    /// Ancho fijo del track draggable (al medio). Único valor que el
    /// widget usa para convertir dx_pixels → dv_value.
    pub track_width: f32,
    /// Grosor (alto) del track en pixels.
    pub track_thickness: f32,
}

impl Default for SliderPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl SliderPalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            track: t.bg_button,
            track_filled: t.accent,
            track_hover: t.bg_button_hover,
            fg_label: t.fg_muted,
            fg_value: t.fg_text,
            radius: 3.0,
            row_height: 22.0,
            label_width: 80.0,
            value_width: 56.0,
            track_width: 120.0,
            track_thickness: 6.0,
        }
    }
}

/// Compone un slider horizontal: label + track-fillbar draggable + valor.
///
/// `value`, `min`, `max` son sólo para presentación visual y conversión
/// `dx → dv`; el caller mantiene el estado y aplica el delta en su
/// `update`. El handler recibe `(DragPhase, delta_value)`; devolver
/// `None` deja el drag activo sin emitir Msg.
pub fn slider_view<Msg, F>(
    label: impl Into<String>,
    value: f32,
    min: f32,
    max: f32,
    palette: &SliderPalette,
    on_change: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(DragPhase, f32) -> Option<Msg> + Send + Sync + 'static,
{
    let range = (max - min).max(f32::EPSILON);
    let ratio = ((value - min) / range).clamp(0.0, 1.0);
    let track_width = palette.track_width.max(1.0);

    // Drag: dx_pixels → dv_value. Escala FIJA (no depende del valor actual).
    let span = max - min;
    let handler = move |phase: DragPhase, dx: f32, _dy: f32| -> Option<Msg> {
        let dv = dx * span / track_width;
        on_change(phase, dv)
    };

    // Bloque del label.
    let label_view = View::new(Style {
        size: Size {
            width: length(palette.label_width),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label.into(), 12.0, palette.fg_label, Alignment::Start);

    // Track draggable: fill = track bg, hijo = porción rellena (accent).
    let filled_radius = palette.radius;
    let filled = View::new(Style {
        size: Size {
            width: percent(ratio),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.track_filled)
    .radius(filled_radius)
    .paint_with(move |scene, _ts, rect| {
        // Gloss superior sobre la stripe accent — la barra se lee como
        // luz que avanza, no como rect plano. Mismo patrón button/progress
        // (P6/P7). Alpha bajo (40) porque el track es muy delgado (6px
        // default) y un sheen fuerte le mete glitter.
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
        let rr = RoundedRect::new(x0, y0, x1, y1, filled_radius);
        let top = Color::from_rgba8(255, 255, 255, 40);
        let bot = Color::from_rgba8(255, 255, 255, 0);
        let g = Gradient::new_linear(Point::new(x0, y0), Point::new(x0, y_mid))
            .with_stops([top, bot].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
    });

    let track = View::new(Style {
        size: Size {
            width: length(track_width),
            height: length(palette.track_thickness),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.track)
    .hover_fill(palette.track_hover)
    .radius(palette.radius)
    .draggable(handler)
    .children(vec![filled]);

    // Wrapper del track para centrarlo verticalmente sobre la fila.
    let track_cell = View::new(Style {
        size: Size {
            width: length(track_width),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![track]);

    // Bloque del valor.
    let value_text = format_value(value);
    let value_view = View::new(Style {
        size: Size {
            width: length(palette.value_width),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(value_text, 12.0, palette.fg_value, Alignment::End);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(palette.row_height),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![label_view, track_cell, value_view])
}

/// Formato uniforme para los valores: 2 decimales con signo explícito si
/// la magnitud es chica, 1 decimal si es grande. Cabe en `value_width: 56`.
fn format_value(v: f32) -> String {
    let abs = v.abs();
    if abs >= 1000.0 {
        format!("{v:.0}")
    } else if abs >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:+.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_value_pretty_for_three_regimes() {
        assert_eq!(format_value(0.34), "+0.34");
        assert_eq!(format_value(-0.10), "-0.10");
        assert_eq!(format_value(42.5), "42.5");
        assert_eq!(format_value(1234.0), "1234");
    }
}
