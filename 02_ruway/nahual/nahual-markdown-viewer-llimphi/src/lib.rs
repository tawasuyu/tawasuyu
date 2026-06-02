//! `nahual-markdown-viewer-llimphi` — visor de Markdown renderizado.
//!
//! Noveno visor del shell meta-app. `shuma-discern` marca los `.md` con
//! lens `markdown`, pero hasta ahora caían al text viewer — que muestra
//! la sintaxis cruda (`# título`, `**negrita**`, ```` ``` ````). Este
//! visor parsea el documento con `pulldown-cmark` a una lista de bloques
//! con estilo y los pinta: encabezados con tamaño creciente según nivel,
//! bloques de código en monoespaciada sobre panel, listas con viñeta
//! indentada, citas en itálica. Se *lee* en vez de leerse el código.
//!
//! Patrón fino de los otros viewers: carga sync en [`load_markdown`],
//! render en [`markdown_viewer_view`]. No conoce el AppBus: el caller
//! pasa el path.
//!
//! MVP feo-primero: el formato inline (negrita/itálica/enlaces) se aplana
//! a texto — sólo la **estructura de bloques** se respeta visualmente. El
//! código inline se conserva con backticks. Sin scroll (clip, como los
//! demás visores estáticos); capamos por bloques y bytes para que parley
//! no se atragante.

#![forbid(unsafe_code)]

use std::path::Path;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

// El dominio (parseo + tipos) vive en `nahual-viewer-core`; lo
// re-exportamos para no romper a los consumidores.
pub use nahual_viewer_core::markdown::*;

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct MarkdownViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_heading: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
    pub code_bg: Color,
    pub code_fg: Color,
}

impl Default for MarkdownViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl MarkdownViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_heading: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
            code_bg: t.bg_panel,
            code_fg: t.fg_text,
        }
    }
}

/// Tamaño de fuente por nivel de encabezado.
fn heading_size(level: u8) -> f32 {
    match level {
        1 => 24.0,
        2 => 20.0,
        3 => 17.0,
        4 => 15.0,
        _ => 13.5,
    }
}

/// Pinta header (nombre del archivo) + body con los bloques apilados.
pub fn markdown_viewer_view<Msg>(
    state: &MarkdownPreview,
    path: Option<&Path>,
    palette: &MarkdownViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let header_text = match path {
        Some(p) => format!(
            "markdown · {}",
            p.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string())
        ),
        None => "(seleccioná un .md)".to_string(),
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: pad(12.0, 0.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let body = match state {
        MarkdownPreview::Empty => simple_body("—", palette.fg_muted, palette),
        MarkdownPreview::TooBig(n) => simple_body(
            &format!("(documento muy grande: {n} bytes — sin preview)"),
            palette.fg_muted,
            palette,
        ),
        MarkdownPreview::Error(e) => simple_body(
            &format!("(no se pudo leer: {e})"),
            palette.fg_error,
            palette,
        ),
        MarkdownPreview::Doc { blocks, truncated } => {
            let mut children: Vec<View<Msg>> = blocks
                .iter()
                .map(|b| block_view::<Msg>(b, palette))
                .collect();
            if *truncated {
                children.push(View::new(block_style(6.0, 2.0)).text_aligned(
                    "… (documento truncado)".to_string(),
                    11.0,
                    palette.fg_muted,
                    Alignment::Start,
                ));
            }
            View::new(Style {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                padding: pad(14.0, 8.0),
                ..Default::default()
            })
            .children(children)
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

/// Renderiza un bloque a su View con el estilo correspondiente.
fn block_view<Msg>(block: &MdBlock, palette: &MarkdownViewerPalette) -> View<Msg>
where
    Msg: Clone + 'static,
{
    match block {
        MdBlock::Heading { level, text } => View::new(block_style(8.0, 3.0)).text_aligned(
            text.clone(),
            heading_size(*level),
            palette.fg_heading,
            Alignment::Start,
        ),
        MdBlock::Paragraph(text) => View::new(block_style(4.0, 3.0)).text_aligned(
            text.clone(),
            13.0,
            palette.fg_text,
            Alignment::Start,
        ),
        MdBlock::ListItem { depth, text } => {
            let indent = "    ".repeat(*depth as usize);
            View::new(block_style(2.0, 1.0)).text_aligned(
                format!("{indent}•  {text}"),
                13.0,
                palette.fg_text,
                Alignment::Start,
            )
        }
        MdBlock::Quote(text) => View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            padding: Rect {
                left: length(12.0_f32),
                right: length(0.0_f32),
                top: length(3.0_f32),
                bottom: length(3.0_f32),
            },
            ..Default::default()
        })
        .text_aligned_italic(
            format!("▌  {text}"),
            13.0,
            palette.fg_muted,
            Alignment::Start,
            true,
        ),
        MdBlock::Code(code) => View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            padding: Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(6.0_f32),
                bottom: length(6.0_f32),
            },
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(4.0_f32),
                bottom: length(4.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.code_bg)
        .text_aligned_full(
            code.clone(),
            12.0,
            palette.code_fg,
            Alignment::Start,
            false,
            Some("monospace".to_string()),
        ),
        MdBlock::Rule => View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(1.0_f32),
            },
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.fg_muted),
    }
}

/// Body de una sola línea (estados Empty/TooBig/Error).
fn simple_body<Msg>(text: &str, color: Color, _palette: &MarkdownViewerPalette) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: pad(14.0, 8.0),
        ..Default::default()
    })
    .text_aligned(text.to_string(), 12.0, color, Alignment::Start)
}

/// Estilo de bloque: ancho completo, padding vertical configurable.
fn block_style(top: f32, bottom: f32) -> Style {
    Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(top),
            bottom: length(bottom),
        },
        ..Default::default()
    }
}

/// Padding horizontal `h` + vertical `v`.
fn pad(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}
