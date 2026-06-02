//! `nahual-tree-viewer-llimphi` — visor de estructuras JSON/TOML.
//!
//! Sexto visor del shell meta-app. `shuma-discern` marca JSON y TOML con
//! lens `tree`, pero hasta ahora caían al text viewer — que muestra un
//! JSON **minificado** como una sola línea inservible. Este visor parsea
//! el documento a un árbol (`serde_json::Value`, unificando JSON y TOML)
//! y lo pinta **indentado**, con el tipo y el tamaño de cada nodo: se
//! escanea aunque el archivo venga en una línea.
//!
//! Patrón fino de los otros viewers: carga sync en [`load_tree`], render
//! en [`tree_viewer_view`]. No conoce el AppBus: el caller pasa el path.
//!
//! MVP feo-primero: el árbol es un bloque de texto indentado, estático
//! (sin colapsar nodos con click todavía). Capa primero la utilidad —
//! ver la forma del dato — sobre la interacción.

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
pub use nahual_viewer_core::tree::*;

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct TreeViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
}

impl Default for TreeViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TreeViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

/// Pinta header (nombre del archivo) + body con el árbol.
pub fn tree_viewer_view<Msg>(
    state: &TreePreview,
    path: Option<&Path>,
    palette: &TreeViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let header_text = match path {
        Some(p) => format!(
            "tree · {}",
            p.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string())
        ),
        None => "(seleccioná un JSON/TOML)".to_string(),
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
        TreePreview::Empty => ("—".to_string(), palette.fg_muted),
        TreePreview::Tree(s) => (s.clone(), palette.fg_text),
        TreePreview::TooBig(n) => (
            format!("(árbol muy grande: {n} bytes — sin preview)"),
            palette.fg_muted,
        ),
        TreePreview::Error(e) => (format!("(no parsea: {e})"), palette.fg_error),
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
