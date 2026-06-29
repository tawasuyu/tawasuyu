//! `nahual-card-viewer-llimphi` — visor estructurado de Cards.
//!
//! Cuarto visor del shell meta-app (tras texto/imagen/video). Una Card
//! (`shared/card`) es JSON, así que el text viewer la abriría como tal;
//! pero el `lens` `card` que `shuma-discern` produce sobre su contenido
//! merece un visor que la **presente** — no el blob crudo. Este crate
//! lee la Card, extrae los campos salientes (identidad, naturaleza,
//! payload, supervisión, capacidades, permisos, referencias) y los pinta
//! como filas legibles.
//!
//! Sigue el patrón fino de los otros viewers: la carga vive en
//! [`load_card`] (sync — una Card es chica), el render en
//! [`card_viewer_view`]. No conoce el AppBus: el caller pasa el path.
//!
//! MVP feo-primero: el cuerpo es un bloque de texto `clave  valor` por
//! línea, no una tabla con layout. Es legible y autocontenido; cuando un
//! widget de propiedades reusable exista en el elegance kit, se migra.

#![forbid(unsafe_code)]

use std::path::Path;

use card_core::CardKind;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

// El dominio (parseo + tipos) vive en `nahual-viewer-core`; lo
// re-exportamos para no romper a los consumidores.
pub use nahual_viewer_core::card::*;

/// Paleta del visor. Reusa los slots semánticos del tema.
#[derive(Debug, Clone, Copy)]
pub struct CardViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
    pub accent: Color,
}

impl Default for CardViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl CardViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
            accent: t.accent,
        }
    }
}

/// Pinta header (label · naturaleza) + body con las filas de la Card.
pub fn card_viewer_view<Msg>(
    state: &CardPreview,
    path: Option<&Path>,
    palette: &CardViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let header_text = match state {
        CardPreview::Card(c) => {
            let kind = match c.kind {
                CardKind::Ente => "ente",
                CardKind::Data => "data",
            };
            format!("card · {} · {kind}", c.label)
        }
        _ => match path {
            Some(p) => format!(
                "card · {}",
                p.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.display().to_string())
            ),
            None => rimay_localize::t("nahual-card-select"),
        },
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, palette.accent, Alignment::Start);

    let (body_text, body_color) = match state {
        CardPreview::Empty => ("—".to_string(), palette.fg_muted),
        CardPreview::Card(c) => (summarize(c), palette.fg_text),
        CardPreview::Error(e) => (rimay_localize::t_args("nahual-card-invalid", &[("err", e.to_string().into())]), palette.fg_error),
    };

    let body = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(body_text, 12.0, body_color, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![header, body])
}
