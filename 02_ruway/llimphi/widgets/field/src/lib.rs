//! `llimphi-widget-field` — wrapper de formulario para inputs.
//!
//! Patrón estándar de campo:
//! ```text
//! Nombre del campo            (label — bold, fg_text)
//! [ input control aquí ]      (slot — viene como View<Msg>)
//! Descripción o error abajo   (helper — fg_muted o fg_destructive)
//! ```
//!
//! El widget no implementa el input — lo recibe como `View<Msg>` y lo
//! envuelve. Esto permite usarlo con `text-input`, `text-area`, `switch`,
//! `segmented` o cualquier otro control.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::Theme;

#[derive(Debug, Clone, Copy)]
pub struct FieldPalette {
    pub fg_label: Color,
    pub fg_helper: Color,
    pub fg_error: Color,
    pub fg_required: Color,
}

impl FieldPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            fg_label: t.fg_text,
            fg_helper: t.fg_muted,
            fg_error: t.fg_destructive,
            fg_required: t.fg_destructive,
        }
    }
}

/// Spec del field. `helper` y `error` son mutuamente excluyentes —
/// si hay error, se renderiza el error (rojo); si no, el helper.
pub struct FieldSpec<Msg: Clone + 'static> {
    pub label: String,
    /// El input/control concreto (text-input, switch, segmented, etc).
    pub control: View<Msg>,
    /// Marca el field como requerido — agrega un asterisco al label.
    pub required: bool,
    /// Texto explicativo debajo del control. `None` para omitirlo.
    pub helper: Option<String>,
    /// Mensaje de error — gana sobre `helper` cuando está presente.
    pub error: Option<String>,
    pub palette: FieldPalette,
}

const LABEL_H: f32 = 16.0;
const HELPER_H: f32 = 16.0;
const GAP_LABEL: f32 = 4.0;
const GAP_HELPER: f32 = 4.0;
const LABEL_FONT: f32 = 11.5;
const HELPER_FONT: f32 = 10.5;

pub fn field_view<Msg: Clone + 'static>(spec: FieldSpec<Msg>) -> View<Msg> {
    let FieldSpec {
        label,
        control,
        required,
        helper,
        error,
        palette,
    } = spec;

    // Label: nombre + (asterisco si required).
    let label_text = if required { format!("{label} *") } else { label };
    let label_view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(LABEL_H),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(label_text, LABEL_FONT, palette.fg_label, Alignment::Start);

    // Helper / error — error gana si presente.
    let helper_text = error.clone().or(helper.clone());
    let helper_color = if error.is_some() { palette.fg_error } else { palette.fg_helper };

    let mut children: Vec<View<Msg>> = vec![label_view, spacer(GAP_LABEL), control];
    if let Some(t) = helper_text {
        children.push(spacer(GAP_HELPER));
        children.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(HELPER_H),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .text_aligned(t, HELPER_FONT, helper_color, Alignment::Start),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
        },
        ..Default::default()
    })
    .children(children)
}

fn spacer<Msg: Clone + 'static>(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}
