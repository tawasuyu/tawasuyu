//! `nahual-text-viewer-llimphi` — visor de texto plano sobre Llimphi.
//!
//! Reemplazo Llimphi del `nahual-text-viewer` GPUI. Crate fino: la
//! lógica de carga vive en [`load_preview`] (size cap + null-byte
//! guard + UTF-8 check + truncate por líneas/chars), el render en
//! [`text_viewer_view`].
//!
//! La carga es sync: Llimphi-ui no tiene `cx.spawn` async como GPUI,
//! pero el límite de tamaño (`max_bytes`, default 256 KB) hace que un
//! `fs::read` típico complete dentro del budget de un frame. Para
//! archivos más grandes, el caller debería envolver `load_preview` en
//! un `Handle::spawn` y reentrar con un Msg al terminar.
//!
//! No incluye AppBus: el caller pasa `path: Option<&Path>` directo a
//! `text_viewer_view`. Eso lo hace consumible por `nahual-shell-llimphi`
//! sin depender de un bus aún no portado.

#![forbid(unsafe_code)]

use std::fs;
use std::path::Path;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

use llimphi_icons::Icon;
use llimphi_theme::{alpha, motion};
use llimphi_widget_empty::{empty_view, EmptyPalette};

/// Tope por defecto de bytes a leer (256 KB). El caller puede pasar
/// otro a [`load_preview`] si el dominio lo justifica.
pub const DEFAULT_PREVIEW_BYTES_MAX: u64 = 256 * 1024;

/// Estado del preview de un archivo. Mismo shape que el viejo GPUI
/// pero sin las variantes async (`Loading`/`Unsupported`) — el caller
/// puede modelarlas afuera si necesita carga diferida.
#[derive(Debug, Clone)]
pub enum PreviewState {
    /// Sin archivo seleccionado.
    Empty,
    /// Texto válido (posiblemente truncado a [`MAX_LINES`]/[`MAX_CHARS`]).
    Text(String),
    /// Bytes con null o no UTF-8 — etiquetado sin contenido.
    Binary,
    /// Excede `max_bytes` — etiquetado con el tamaño real.
    TooBig(u64),
    /// `fs::metadata` o `fs::read` falló — mensaje en el body.
    Error(String),
}

impl Default for PreviewState {
    fn default() -> Self {
        PreviewState::Empty
    }
}

const MAX_LINES: usize = 200;
const MAX_CHARS: usize = 8_000;

/// Lee el archivo y devuelve el estado correspondiente. Sync — ver el
/// note del crate sobre archivos grandes. `max_bytes` corta antes de
/// leer (vía `fs::metadata`), así no leemos un blob enorme aunque la
/// detección de binario nos saque después.
pub fn load_preview(path: &Path, max_bytes: u64) -> PreviewState {
    match fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => return PreviewState::TooBig(meta.len()),
        Err(e) => return PreviewState::Error(e.to_string()),
        _ => {}
    }
    match fs::read(path) {
        Ok(bytes) => {
            if bytes.contains(&0) {
                PreviewState::Binary
            } else {
                match String::from_utf8(bytes) {
                    Ok(s) => PreviewState::Text(truncate_preview(&s)),
                    Err(_) => PreviewState::Binary,
                }
            }
        }
        Err(e) => PreviewState::Error(e.to_string()),
    }
}

/// Recorta `s` a `MAX_LINES` líneas o `MAX_CHARS` chars (lo que se
/// alcance primero) y agrega "\n…" al final. Mantiene el render
/// instantáneo aunque el archivo esté en el límite de `max_bytes`
/// (parley tarda en wrappear textos muy largos).
fn truncate_preview(s: &str) -> String {
    let mut out = String::new();
    for (i, line) in s.lines().enumerate() {
        if i >= MAX_LINES || out.len() + line.len() + 1 > MAX_CHARS {
            out.push_str("\n…");
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Paleta del viewer. Slots semánticos para que el caller pueda
/// reusar el tema de la app — la default usa `Theme::dark()`.
#[derive(Debug, Clone, Copy)]
pub struct TextViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
}

impl Default for TextViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TextViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

/// Pinta el viewer: header con el nombre del archivo (o un placeholder
/// si no hay path) + body con el contenido según `state`. Usa `clip`
/// para que el texto no sangre al vecino.
pub fn text_viewer_view<Msg>(
    state: &PreviewState,
    path: Option<&Path>,
    palette: &TextViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let header_text = match path {
        Some(p) => p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| p.display().to_string()),
        None => "(seleccioná un archivo)".to_string(),
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
        // Empty-state con orientación (en vez de un guión solo) cuando
        // todavía no hay archivo seleccionado.
        PreviewState::Empty => empty_body(palette),
        // Pop-in suave al cargar: la `key` (hash del path) es estable
        // mientras se mire el mismo archivo, así el fade corre una sola vez.
        PreviewState::Text(s) => {
            text_body(s.clone(), palette.fg_text).animated_enter(key_of(path), motion::NORMAL)
        }
        PreviewState::Binary => text_body(
            "(archivo binario — sin preview)".to_string(),
            palette.fg_muted,
        ),
        PreviewState::TooBig(n) => text_body(
            format!("(archivo muy grande: {} bytes — sin preview)", n),
            palette.fg_muted,
        ),
        PreviewState::Error(e) => text_body(format!("(error: {e})"), palette.fg_error),
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

/// Body de texto: un pane con padding que pinta `text` alineado al inicio.
fn text_body<Msg>(text: String, color: Color) -> View<Msg>
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
    .text_aligned(text, 12.0, color, Alignment::Start)
}

/// Empty-state con orientación cuando todavía no hay archivo seleccionado.
fn empty_body<Msg>(palette: &TextViewerPalette) -> View<Msg>
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
        Icon::FileText,
        "Sin archivo",
        Some("Seleccioná un archivo de texto para previsualizarlo."),
        &empty_palette(palette),
    )])
}

/// Deriva una [`EmptyPalette`] desde la [`TextViewerPalette`] (que no acarrea
/// un `Theme`). Mismo criterio que en `nahual-image-viewer-llimphi`: ícono y
/// descripción apagados sobre `fg_muted`.
fn empty_palette(p: &TextViewerPalette) -> EmptyPalette {
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

/// Hash estable del path → `key` para el pop-in implícito de Llimphi. El mismo
/// archivo produce siempre la misma key entre repintados, así el fade-in corre
/// sólo al cambiar de archivo.
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
