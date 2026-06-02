//! `nahual-hex-viewer-llimphi` — volcado hex/ASCII de binarios.
//!
//! Séptimo visor del shell meta-app. Los binarios que `shuma-discern`
//! reconoce por magic-bytes (ELF, wasm, gzip, zip…) hasta ahora caían al
//! text viewer, que sólo dice "(binario — sin preview)". Este visor los
//! vuelca como un clásico dump `offset  hex  |ascii|`: alcanza para
//! inspeccionar una cabecera, confirmar un magic number o ver la forma
//! de un blob, sin salir del shell.
//!
//! Patrón fino de los otros viewers: carga sync en [`load_hex`], render
//! en [`hex_viewer_view`]. Lee sólo los primeros KB (un dump más largo
//! no se escanea a ojo). El cuerpo se pide en fuente **monoespaciada**
//! (`font_family = "monospace"`) para que las columnas cuadren.

#![forbid(unsafe_code)]

use std::path::Path;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

// El dominio (parseo + tipos) vive en `nahual-viewer-core`; lo
// re-exportamos para no romper a los consumidores.
pub use nahual_viewer_core::hex::*;

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct HexViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
}

impl Default for HexViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl HexViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

/// Pinta header (nombre · tamaño · "primeros N B" si truncó) + body con
/// el dump en monoespaciada.
pub fn hex_viewer_view<Msg>(
    state: &HexPreview,
    path: Option<&Path>,
    palette: &HexViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let name = match path {
        Some(p) => p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| p.display().to_string()),
        None => "(seleccioná un binario)".to_string(),
    };
    let header_text = match state {
        HexPreview::Dump { total, shown, .. } => {
            if (*shown as u64) < *total {
                format!("hex · {name} · {total} B (primeros {shown})")
            } else {
                format!("hex · {name} · {total} B")
            }
        }
        _ => format!("hex · {name}"),
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
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let (body_text, body_color) = match state {
        HexPreview::Empty => ("—".to_string(), palette.fg_muted),
        HexPreview::Dump { text, .. } => (text.clone(), palette.fg_text),
        HexPreview::Error(e) => (format!("(error: {e})"), palette.fg_error),
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
    .text_aligned_full(
        body_text,
        12.0,
        body_color,
        Alignment::Start,
        false,
        Some("monospace".to_string()),
    );

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
