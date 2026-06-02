//! `nahual-archive-viewer-llimphi` — visor de archivos comprimidos.
//!
//! Décimo visor del shell meta-app. Un `.zip`/`.tar`/`.tar.gz` lo detecta
//! `shuma-discern` por su magic, pero hasta ahora caían al **hex viewer**
//! (o al texto) — bytes ilegibles. Un archivo comprimido es un
//! *contenedor*: lo útil es ver qué hay **dentro**, no su entropía. Este
//! visor lista cada entrada con su tamaño (y, para ZIP, su ratio).
//!
//! Soporta tres formatos, decidido por el **contenido** (no la extensión):
//! - **ZIP** (`PK`): lee el directorio central con `by_index_raw`, sin
//!   descomprimir. Cubre la familia entera — `.jar`/`.apk`/`.epub` y los
//!   ofimáticos OOXML (`.docx`/`.xlsx`/`.pptx`) son ZIPs.
//! - **tar** (`ustar` en off 257): recorre los headers en streaming.
//! - **tar.gz** (`1f 8b`): descomprime en streaming con `flate2` y recorre
//!   el tar interno; salta los datos de cada entrada (sólo lee headers),
//!   así no carga el archivo entero en memoria.
//!
//! Patrón fino de los otros viewers: carga sync en [`load_archive`],
//! render en [`archive_viewer_view`]. No conoce el AppBus: el caller
//! pasa el path. MVP feo-primero: la lista es un bloque de texto
//! monoespaciado, estático (sin extraer entradas con click todavía).

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
pub use nahual_viewer_core::archive::*;

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
}

impl Default for ArchiveViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl ArchiveViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

/// Pinta header (nombre del archivo) + body con el listado monoespaciado.
pub fn archive_viewer_view<Msg>(
    state: &ArchivePreview,
    path: Option<&Path>,
    palette: &ArchiveViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let header_text = match path {
        Some(p) => format!(
            "archive · {}",
            p.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string())
        ),
        None => "(seleccioná un ZIP/tar/tar.gz)".to_string(),
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
        ArchivePreview::Empty => ("—".to_string(), palette.fg_muted),
        ArchivePreview::Listing(l) => (render_listing(l), palette.fg_text),
        ArchivePreview::Error(e) => (format!("(no se pudo abrir: {e})"), palette.fg_error),
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
