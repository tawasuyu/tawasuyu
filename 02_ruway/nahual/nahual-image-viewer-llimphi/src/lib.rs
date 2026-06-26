//! `nahual-image-viewer-llimphi` — visor de imágenes sobre Llimphi.
//!
//! Reemplazo Llimphi del `nahual-image-viewer` GPUI. Crate fino: la
//! lógica de carga vive en [`load_image`] (size cap + decode → Rgba8 vía
//! `llimphi-image`), el render en [`image_viewer_view`].
//!
//! La carga es sync: para imágenes >2 MB conviene envolver
//! `load_image` en `Handle::spawn` y reentrar con un Msg al terminar.
//!
//! Formatos soportados: PNG, JPEG y WEBP (los que active el workspace
//! del crate `image` upstream — ver `llimphi-image`).

#![forbid(unsafe_code)]

use std::path::Path;

use llimphi_image::{load_path, DecodeError, Image};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect};
use llimphi_ui::llimphi_raster::peniko::{BlendMode, Color, Fill};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, GesturePhase, View};

use llimphi_icons::Icon;
use llimphi_theme::{alpha, motion};
use llimphi_widget_empty::{empty_view, EmptyPalette};

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

/// Lee, decodifica y arma el `peniko::Image`. Sync. Delega en
/// [`llimphi_image::load_path`] — el cap de tamaño aplica al archivo en
/// disco (no a la imagen decodificada en RGBA8, que puede ser mucho
/// mayor: un PNG 4K son ~64 MB descomprimidos).
pub fn load_image(path: &Path, max_bytes: u64) -> ImagePreviewState {
    match load_path(path, max_bytes) {
        Ok(image) => {
            let (width, height) = (image.image.width, image.image.height);
            ImagePreviewState::Image { image, width, height }
        }
        Err(DecodeError::TooBig { size_bytes, .. }) => ImagePreviewState::TooBig(size_bytes),
        Err(DecodeError::UnsupportedFormat) => {
            ImagePreviewState::Unsupported(rimay_localize::t("nahual-image-unsupported"))
        }
        Err(DecodeError::Io(e)) => ImagePreviewState::Error(e.to_string()),
        Err(DecodeError::Decode(s)) => ImagePreviewState::Error(s),
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

/// Estado de zoom/pan del viewer, **propiedad del caller** (Regla 2: el
/// widget es stateless; el foco vive en el modelo de la app).
///
/// - `zoom = 1.0` es el aspect-fit base centrado; valores mayores agrandan.
/// - `pan` es el desplazamiento en px de pantalla desde el centrado base.
///
/// v1: el zoom es **hacia el centro** del viewport (más el pan actual), no
/// hacia el cursor — el `update` de Elm no conoce el rect del nodo, así que el
/// punto focal del gesto se difiere. El [`on_scale`](View::on_scale) igualmente
/// entrega el focal por si una versión futura lo aprovecha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageViewport {
    pub zoom: f32,
    pub pan: (f32, f32),
}

impl Default for ImageViewport {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: (0.0, 0.0),
        }
    }
}

impl ImageViewport {
    /// No achicamos por debajo del aspect-fit (el fit ya entra entero).
    pub const MIN_ZOOM: f32 = 1.0;
    pub const MAX_ZOOM: f32 = 16.0;

    /// Vuelve al aspect-fit centrado.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Aplica el `factor` multiplicativo incremental de un gesto `on_scale`,
    /// clampeado al rango. Al volver al fit recentra (pan = 0).
    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        if self.zoom <= Self::MIN_ZOOM {
            self.pan = (0.0, 0.0);
        }
    }

    /// Desplaza el pan en px de pantalla (delta de un arrastre). No-op sin zoom.
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        if self.zoom <= Self::MIN_ZOOM {
            return;
        }
        self.pan.0 += dx;
        self.pan.1 += dy;
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
    let body = match state {
        ImagePreviewState::Empty => empty_body(palette),
        ImagePreviewState::Image { image, .. } => {
            // Pop-in suave al cargar: la `key` (hash del path) es estable
            // mientras se mire la misma imagen, así el fade corre una sola vez.
            image_body(image.clone()).animated_enter(key_of(path), motion::NORMAL)
        }
        ImagePreviewState::TooBig(n) => placeholder_body(
            &format!("(archivo muy grande: {n} bytes — sin preview)"),
            palette.fg_muted,
        ),
        ImagePreviewState::Unsupported(s) => placeholder_body(s, palette.fg_muted),
        ImagePreviewState::Error(e) => {
            placeholder_body(&format!("(error: {e})"), palette.fg_error)
        }
    };
    outer(header_view(state, path, palette), body, palette)
}

/// Como [`image_viewer_view`] pero **interactivo**: la imagen se pinta con el
/// `viewport` (zoom/pan) vía `paint_with` y declara los gestos de Llimphi —
/// `on_scale` (Ctrl+rueda en desktop / pinch en trackpad), arrastre para hacer
/// pan y doble-tap para resetear. El estado lo posee el caller ([`ImageViewport`]):
/// en el `update`, `on_zoom(factor, _, _)` → [`ImageViewport::zoom_by`] y
/// `on_pan(dx, dy)` → [`ImageViewport::pan_by`]; el doble-tap manda `on_reset`.
pub fn image_viewer_view_zoom<Msg, FZoom, FPan>(
    state: &ImagePreviewState,
    path: Option<&Path>,
    palette: &ImageViewerPalette,
    viewport: ImageViewport,
    on_zoom: FZoom,
    on_pan: FPan,
    on_reset: Msg,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FZoom: Fn(f32, f32, f32) -> Msg + Send + Sync + 'static,
    FPan: Fn(f32, f32) -> Msg + Send + Sync + 'static,
{
    let body = match state {
        ImagePreviewState::Image { image, .. } => {
            zoom_body(image.clone(), viewport, on_zoom, on_pan, on_reset)
                .animated_enter(key_of(path), motion::NORMAL)
        }
        ImagePreviewState::Empty => empty_body(palette),
        ImagePreviewState::TooBig(n) => placeholder_body(
            &format!("(archivo muy grande: {n} bytes — sin preview)"),
            palette.fg_muted,
        ),
        ImagePreviewState::Unsupported(s) => placeholder_body(s, palette.fg_muted),
        ImagePreviewState::Error(e) => {
            placeholder_body(&format!("(error: {e})"), palette.fg_error)
        }
    };
    outer(header_view(state, path, palette), body, palette)
}

/// Header común (nombre + dimensiones).
fn header_view<Msg>(
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
    View::new(Style {
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
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start)
}

/// Contenedor columna (header + body) con fondo y clip.
fn outer<Msg>(header: View<Msg>, body: View<Msg>, palette: &ImageViewerPalette) -> View<Msg>
where
    Msg: Clone + 'static,
{
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

/// Hash estable del path → `key` para el pop-in implícito de Llimphi. La
/// misma imagen produce siempre la misma key entre repintados (zoom/pan),
/// así el fade-in corre sólo al cambiar de imagen.
fn key_of(path: Option<&Path>) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    match path {
        Some(p) => p.to_string_lossy().hash(&mut h),
        None => 0u8.hash(&mut h),
    }
    h.finish()
}

/// Deriva una [`EmptyPalette`] desde la [`ImageViewerPalette`] (que no
/// acarrea un `Theme`). Mismo criterio que `EmptyPalette::from_theme`:
/// ícono y descripción apagados sobre `fg_muted`.
fn empty_palette(p: &ImageViewerPalette) -> EmptyPalette {
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    let dim = |a: u8| {
        let [r, g, b, _] = p.fg_muted.components;
        AlphaColor::new([r, g, b, a as f32 / 255.0])
    };
    EmptyPalette {
        fg_icon: dim(alpha::HINT),
        fg_title: p.fg_muted,
        fg_desc: dim(alpha::DISABLED),
    }
}

/// Empty-state con orientación (en vez de un guión solo) cuando todavía no
/// hay imagen seleccionada.
fn empty_body<Msg>(palette: &ImageViewerPalette) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![empty_view(
        Icon::Image,
        "Sin imagen",
        Some("Seleccioná una imagen para previsualizarla."),
        &empty_palette(palette),
    )])
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

/// Body interactivo: pinta la imagen con `viewport` (aspect-fit × zoom + pan,
/// recortado al rect) y cablea los gestos a los callbacks del caller.
fn zoom_body<Msg, FZoom, FPan>(
    image: Image,
    vp: ImageViewport,
    on_zoom: FZoom,
    on_pan: FPan,
    on_reset: Msg,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FZoom: Fn(f32, f32, f32) -> Msg + Send + Sync + 'static,
    FPan: Fn(f32, f32) -> Msg + Send + Sync + 'static,
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
    .clip(true)
    .paint_with(move |scene, _ts, rect| {
        if image.image.width == 0 || image.image.height == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let iw = image.image.width as f64;
        let ih = image.image.height as f64;
        // Escala base = aspect-fit; el zoom del usuario la multiplica.
        let s = (rect.w as f64 / iw).min(rect.h as f64 / ih) * vp.zoom as f64;
        let disp_w = iw * s;
        let disp_h = ih * s;
        // Centrado en el rect + pan del usuario.
        let ox = rect.x as f64 + (rect.w as f64 - disp_w) * 0.5 + vp.pan.0 as f64;
        let oy = rect.y as f64 + (rect.h as f64 - disp_h) * 0.5 + vp.pan.1 as f64;
        let clip = KurboRect::new(
            rect.x as f64,
            rect.y as f64,
            (rect.x + rect.w) as f64,
            (rect.y + rect.h) as f64,
        );
        scene.push_layer(Fill::NonZero, BlendMode::default(), 1.0, Affine::IDENTITY, &clip);
        scene.draw_image(&image, Affine::translate((ox, oy)) * Affine::scale(s));
        scene.pop_layer();
    })
    // Ctrl+rueda / pinch: sólo los Update con cambio real consumen el gesto.
    .on_scale(move |phase, factor, fx, fy| {
        if phase == GesturePhase::Update && (factor - 1.0).abs() > f32::EPSILON {
            Some(on_zoom(factor, fx, fy))
        } else {
            None
        }
    })
    // Arrastre = pan.
    .draggable(move |phase, dx, dy| match phase {
        DragPhase::Move => Some(on_pan(dx, dy)),
        _ => None,
    })
    // Doble-tap = volver al fit.
    .on_double_tap(on_reset)
}

#[cfg(test)]
mod tests {
    use super::ImageViewport;

    #[test]
    fn fit_por_defecto() {
        let vp = ImageViewport::default();
        assert_eq!(vp.zoom, 1.0);
        assert_eq!(vp.pan, (0.0, 0.0));
    }

    #[test]
    fn zoom_multiplica_y_clampa() {
        let mut vp = ImageViewport::default();
        vp.zoom_by(2.0);
        assert_eq!(vp.zoom, 2.0);
        vp.zoom_by(2.0);
        assert_eq!(vp.zoom, 4.0);
        // Tope superior.
        vp.zoom_by(100.0);
        assert_eq!(vp.zoom, ImageViewport::MAX_ZOOM);
        // No baja del fit.
        vp.zoom_by(0.0001);
        assert_eq!(vp.zoom, ImageViewport::MIN_ZOOM);
    }

    #[test]
    fn volver_al_fit_recentra() {
        let mut vp = ImageViewport::default();
        vp.zoom_by(4.0);
        vp.pan_by(50.0, -30.0);
        assert_eq!(vp.pan, (50.0, -30.0));
        // Al volver al fit, el pan se descarta.
        vp.zoom_by(0.01);
        assert_eq!(vp.zoom, ImageViewport::MIN_ZOOM);
        assert_eq!(vp.pan, (0.0, 0.0));
    }

    #[test]
    fn pan_es_noop_sin_zoom() {
        let mut vp = ImageViewport::default();
        vp.pan_by(10.0, 10.0);
        assert_eq!(vp.pan, (0.0, 0.0));
    }

    #[test]
    fn pan_acumula_con_zoom() {
        let mut vp = ImageViewport::default();
        vp.zoom_by(3.0);
        vp.pan_by(10.0, 5.0);
        vp.pan_by(-4.0, 2.0);
        assert_eq!(vp.pan, (6.0, 7.0));
    }

    #[test]
    fn reset_vuelve_al_default() {
        let mut vp = ImageViewport::default();
        vp.zoom_by(5.0);
        vp.pan_by(20.0, 20.0);
        vp.reset();
        assert_eq!(vp, ImageViewport::default());
    }
}
