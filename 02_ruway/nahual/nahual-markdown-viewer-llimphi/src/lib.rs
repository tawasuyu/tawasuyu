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

/// Tope de bytes a leer (1 MiB). Un Markdown más grande que eso no es un
/// documento a ojo; el caller puede subirlo si hace falta.
pub const DEFAULT_MARKDOWN_BYTES_MAX: u64 = 1024 * 1024;

/// Bloques máximos a renderizar. Corta documentos enormes para que el
/// panel siga instantáneo.
const MAX_BLOCKS: usize = 500;
/// Indentación máxima de listas anidadas (en niveles).
const MAX_LIST_DEPTH: u8 = 8;

/// Un bloque del documento con su estilo semántico. El render mapea cada
/// variante a un tamaño/fuente/color.
#[derive(Debug, Clone, PartialEq)]
pub enum MdBlock {
    /// Encabezado `#`..`######` (nivel 1–6) con su texto aplanado.
    Heading { level: u8, text: String },
    /// Párrafo de texto corrido.
    Paragraph(String),
    /// Bloque de código (fenced o indentado), en monoespaciada.
    Code(String),
    /// Ítem de lista; `depth` 0 = nivel raíz.
    ListItem { depth: u8, text: String },
    /// Cita (`>`), en itálica.
    Quote(String),
    /// Regla horizontal (`---`).
    Rule,
}

/// Estado del visor. Replica la forma de los otros para que el shell lo
/// trate igual.
#[derive(Debug, Clone, Default)]
pub enum MarkdownPreview {
    /// Sin archivo seleccionado.
    #[default]
    Empty,
    /// Documento parseado a bloques (posiblemente truncado).
    Doc { blocks: Vec<MdBlock>, truncated: bool },
    /// Excede el tope de tamaño.
    TooBig(u64),
    /// E/S falló.
    Error(String),
}

/// Lee el archivo y lo parsea a bloques. La detección de tipo ya la hizo
/// el shell (lens `markdown`); acá sólo leemos UTF-8 y parseamos.
pub fn load_markdown(path: &Path, max_bytes: u64) -> MarkdownPreview {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => return MarkdownPreview::TooBig(meta.len()),
        Err(e) => return MarkdownPreview::Error(e.to_string()),
        _ => {}
    }
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return MarkdownPreview::Error(e.to_string()),
    };
    let (blocks, truncated) = parse_blocks(&src);
    MarkdownPreview::Doc { blocks, truncated }
}

/// Parsea Markdown a una lista plana de [`MdBlock`]. El segundo valor es
/// `true` si se cortó en [`MAX_BLOCKS`]. El formato inline se aplana a
/// texto; sólo la estructura de bloques sobrevive.
pub fn parse_blocks(src: &str) -> (Vec<MdBlock>, bool) {
    use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

    let mut blocks: Vec<MdBlock> = Vec::new();
    // Buffer del texto del bloque en curso.
    let mut buf = String::new();
    // Profundidad de listas anidadas (cantidad de `List` abiertas).
    let mut list_depth: u8 = 0;
    let mut in_item = false;
    let mut quote_depth: u8 = 0;

    let push = |blocks: &mut Vec<MdBlock>, b: MdBlock| {
        if blocks.len() < MAX_BLOCKS {
            blocks.push(b);
        }
    };

    for ev in Parser::new(src) {
        if blocks.len() >= MAX_BLOCKS {
            return (blocks, true);
        }
        match ev {
            Event::Start(Tag::Heading { .. }) => {
                buf.clear();
            }
            Event::End(TagEnd::Heading(level)) => {
                let lvl = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                push(
                    &mut blocks,
                    MdBlock::Heading { level: lvl, text: std::mem::take(&mut buf).trim().to_string() },
                );
            }
            Event::Start(Tag::CodeBlock(_)) => {
                buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                let code = std::mem::take(&mut buf);
                push(&mut blocks, MdBlock::Code(code.trim_end_matches('\n').to_string()));
            }
            Event::Start(Tag::List(_)) => {
                // Una lista anidada arranca dentro de un ítem; su texto de
                // cabecera (el del ítem padre) está en `buf` y se perdería
                // al limpiarlo en el `Start(Item)` hijo. Lo emitimos ahora,
                // a la profundidad del ítem padre.
                if in_item {
                    let text = std::mem::take(&mut buf).trim().to_string();
                    if !text.is_empty() {
                        let depth = list_depth.saturating_sub(1).min(MAX_LIST_DEPTH);
                        push(&mut blocks, MdBlock::ListItem { depth, text });
                    }
                }
                list_depth = list_depth.saturating_add(1);
            }
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
            }
            Event::Start(Tag::Item) => {
                in_item = true;
                buf.clear();
            }
            Event::End(TagEnd::Item) => {
                in_item = false;
                let text = std::mem::take(&mut buf).trim().to_string();
                if !text.is_empty() {
                    let depth = list_depth.saturating_sub(1).min(MAX_LIST_DEPTH);
                    push(&mut blocks, MdBlock::ListItem { depth, text });
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                quote_depth = quote_depth.saturating_add(1);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                quote_depth = quote_depth.saturating_sub(1);
            }
            Event::End(TagEnd::Paragraph) => {
                // El cierre de párrafo emite el bloque, salvo que el texto
                // pertenezca a un ítem de lista (lo emite End(Item)).
                if in_item {
                    continue;
                }
                let text = std::mem::take(&mut buf).trim().to_string();
                if text.is_empty() {
                    continue;
                }
                if quote_depth > 0 {
                    push(&mut blocks, MdBlock::Quote(text));
                } else {
                    push(&mut blocks, MdBlock::Paragraph(text));
                }
            }
            Event::Text(t) => buf.push_str(&t),
            Event::Code(t) => {
                // Código inline: conservamos los backticks como pista.
                buf.push('`');
                buf.push_str(&t);
                buf.push('`');
            }
            Event::SoftBreak => buf.push(' '),
            Event::HardBreak => buf.push('\n'),
            Event::Rule => {
                buf.clear();
                push(&mut blocks, MdBlock::Rule);
            }
            _ => {}
        }
    }

    (blocks, false)
}

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
        MarkdownPreview::Error(e) => {
            simple_body(&format!("(no se pudo leer: {e})"), palette.fg_error, palette)
        }
        MarkdownPreview::Doc { blocks, truncated } => {
            let mut children: Vec<View<Msg>> = blocks
                .iter()
                .map(|b| block_view::<Msg>(b, palette))
                .collect();
            if *truncated {
                children.push(
                    View::new(block_style(6.0, 2.0)).text_aligned(
                        "… (documento truncado)".to_string(),
                        11.0,
                        palette.fg_muted,
                        Alignment::Start,
                    ),
                );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encabezados_por_nivel() {
        let (b, _) = parse_blocks("# uno\n\n## dos\n\n### tres\n");
        assert_eq!(b[0], MdBlock::Heading { level: 1, text: "uno".into() });
        assert_eq!(b[1], MdBlock::Heading { level: 2, text: "dos".into() });
        assert_eq!(b[2], MdBlock::Heading { level: 3, text: "tres".into() });
    }

    #[test]
    fn parrafo_aplana_inline() {
        let (b, _) = parse_blocks("hola **mundo** y `code` final\n");
        // negrita se aplana a texto; inline code conserva backticks.
        assert_eq!(b[0], MdBlock::Paragraph("hola mundo y `code` final".into()));
    }

    #[test]
    fn lista_con_profundidad() {
        let (b, _) = parse_blocks("- a\n- b\n  - c\n");
        assert_eq!(b[0], MdBlock::ListItem { depth: 0, text: "a".into() });
        assert_eq!(b[1], MdBlock::ListItem { depth: 0, text: "b".into() });
        assert_eq!(b[2], MdBlock::ListItem { depth: 1, text: "c".into() });
    }

    #[test]
    fn bloque_de_codigo() {
        let (b, _) = parse_blocks("```rust\nfn main() {}\n```\n");
        assert_eq!(b[0], MdBlock::Code("fn main() {}".into()));
    }

    #[test]
    fn cita_y_regla() {
        let (b, _) = parse_blocks("> citado\n\n---\n");
        assert_eq!(b[0], MdBlock::Quote("citado".into()));
        assert_eq!(b[1], MdBlock::Rule);
    }

    #[test]
    fn documento_grande_se_trunca() {
        let src = "# h\n\n".repeat(MAX_BLOCKS + 50);
        let (b, truncated) = parse_blocks(&src);
        assert!(truncated);
        assert!(b.len() <= MAX_BLOCKS);
    }

    #[test]
    fn vacio_no_panica() {
        let (b, truncated) = parse_blocks("");
        assert!(b.is_empty());
        assert!(!truncated);
    }
}
