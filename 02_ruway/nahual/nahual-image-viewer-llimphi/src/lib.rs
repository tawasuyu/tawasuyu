//! `nahual-image-viewer-llimphi` — visor de imágenes sobre Llimphi.
//!
//! Reemplazo Llimphi del `nahual-image-viewer` GPUI. Crate fino: la
//! lógica de carga vive en [`load_image`] (size cap + decode → Rgba8),
//! el render en [`image_viewer_view`].
//!
//! La carga es sync: para imágenes >2 MB conviene envolver
//! `load_image` en `Handle::spawn` y reentrar con un Msg al terminar.
//!
//! Formatos soportados: PNG y JPEG (features `image/png` + `image/jpeg`).
//! Para WebP/AVIF/etc., habilitar la feature correspondiente del crate
//! `image` desde la app consumidora.

#![forbid(unsafe_code)]

use std::fs;
use std::path::Path;

use image::ImageReader;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Tope por defecto de bytes a leer (8 MB). Las imágenes RGBA8
/// decodificadas pueden ocupar mucho más en memoria (un PNG 4K son
/// ~64 MB descomprimidos), pero el cap aplica al archivo en disco.
pub const DEFAULT_IMAGE_BYTES_MAX: u64 = 8 * 1024 * 1024;

/// Estado del preview. `Image` lleva el `peniko::Image` ya armado +
/// las dimensiones originales para mostrar en el header.
#[derive(Clone)]
pub enum ImagePreviewState {
    Empty,
    Image {
        image: Image,
        width: u32,
        height: u32,
    },
    TooBig(u64),
    Unsupported(String),
    Error(String),
}

impl Default for ImagePreviewState {
    fn default() -> Self {
        ImagePreviewState::Empty
    }
}

/// Lee, decodifica y arma el `peniko::Image`. Sync.
pub fn load_image(path: &Path, max_bytes: u64) -> ImagePreviewState {
    match fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => return ImagePreviewState::TooBig(meta.len()),
        Err(e) => return ImagePreviewState::Error(e.to_string()),
        _ => {}
    }
    let reader = match ImageReader::open(path) {
        Ok(r) => r,
        Err(e) => return ImagePreviewState::Error(e.to_string()),
    };
    let reader = match reader.with_guessed_format() {
        Ok(r) => r,
        Err(e) => return ImagePreviewState::Error(e.to_string()),
    };
    // `format()` es `None` si el formato detectado no está habilitado
    // por feature. Reportamos diferenciado de error de IO.
    if reader.format().is_none() {
        return ImagePreviewState::Unsupported(
            "formato no soportado (sólo PNG/JPEG en esta build)".to_string(),
        );
    }
    let img = match reader.decode() {
        Ok(i) => i,
        Err(e) => return ImagePreviewState::Error(e.to_string()),
    };
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let blob = Blob::from(rgba.into_raw());
    let peniko_image = Image::new(blob, ImageFormat::Rgba8, w, h);
    ImagePreviewState::Image {
        image: peniko_image,
        width: w,
        height: h,
    }
}

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct ImageViewerPalette {
    pub bg: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
}

impl Default for ImageViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl ImageViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

/// Pinta header (nombre + dimensiones si las hay) + body con la
/// imagen aspect-fit o un placeholder de estado.
pub fn image_viewer_view<Msg>(
    state: &ImagePreviewState,
    path: Option<&Path>,
    palette: &ImageViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let name = path
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "(seleccioná una imagen)".to_string());
    let header_text = match state {
        ImagePreviewState::Image { width, height, .. } => {
            format!("{name} · {width}×{height}")
        }
        _ => name,
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

    let body = match state {
        ImagePreviewState::Empty => placeholder_body("—", palette.fg_muted),
        ImagePreviewState::Image { image, .. } => image_body(image.clone()),
        ImagePreviewState::TooBig(n) => placeholder_body(
            &format!("(archivo muy grande: {n} bytes — sin preview)"),
            palette.fg_muted,
        ),
        ImagePreviewState::Unsupported(s) => placeholder_body(s, palette.fg_muted),
        ImagePreviewState::Error(e) => {
            placeholder_body(&format!("(error: {e})"), palette.fg_error)
        }
    };

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

fn placeholder_body<Msg>(text: &str, color: Color) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
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
    .text_aligned(text.to_string(), 12.0, color, Alignment::Center)
}

fn image_body<Msg>(image: Image) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(6.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .image(image)
}
