//! `markdown` — núcleo agnóstico del visor de markdown de nahual (parseo + tipos de preview). El render vive en `nahual-markdown-viewer-llimphi`.

use std::path::Path;

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
    Doc {
        blocks: Vec<MdBlock>,
        truncated: bool,
    },
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
                    MdBlock::Heading {
                        level: lvl,
                        text: std::mem::take(&mut buf).trim().to_string(),
                    },
                );
            }
            Event::Start(Tag::CodeBlock(_)) => {
                buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                let code = std::mem::take(&mut buf);
                push(
                    &mut blocks,
                    MdBlock::Code(code.trim_end_matches('\n').to_string()),
                );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encabezados_por_nivel() {
        let (b, _) = parse_blocks("# uno\n\n## dos\n\n### tres\n");
        assert_eq!(
            b[0],
            MdBlock::Heading {
                level: 1,
                text: "uno".into()
            }
        );
        assert_eq!(
            b[1],
            MdBlock::Heading {
                level: 2,
                text: "dos".into()
            }
        );
        assert_eq!(
            b[2],
            MdBlock::Heading {
                level: 3,
                text: "tres".into()
            }
        );
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
        assert_eq!(
            b[0],
            MdBlock::ListItem {
                depth: 0,
                text: "a".into()
            }
        );
        assert_eq!(
            b[1],
            MdBlock::ListItem {
                depth: 0,
                text: "b".into()
            }
        );
        assert_eq!(
            b[2],
            MdBlock::ListItem {
                depth: 1,
                text: "c".into()
            }
        );
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
