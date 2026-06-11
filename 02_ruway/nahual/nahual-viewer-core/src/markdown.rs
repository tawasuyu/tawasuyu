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

/// Formato inline acumulado sobre un rango de bytes del texto de un bloque.
/// Cada flag es un override que el render traduce a un `TextSpan`
/// (negrita → weight 700, código → monospace, link → color de acento…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InlineFlags {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strikethrough: bool,
    /// El tramo es (o está dentro de) un enlace — el render lo colorea con
    /// el acento. El destino (`href`) no se conserva: el visor no navega.
    pub link: bool,
}

impl InlineFlags {
    /// `true` si no hay ningún override — el tramo hereda el estilo base del
    /// bloque y no necesita un span propio.
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }
}

/// Texto de un bloque con sus tramos de formato inline. `text` es el
/// contenido aplanado (sin marcadores `**`/`` ` ``); `spans` marca rangos
/// de bytes `[start, end)` con su formato. Los rangos son contiguos y no se
/// superponen (cada empuje de texto muestrea el formato activo en ese
/// momento), así el render los pasa directo a `View::text_spans`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Inline {
    pub text: String,
    pub spans: Vec<(usize, usize, InlineFlags)>,
}

impl Inline {
    /// Texto sin formato (todo hereda del bloque). Útil en tests y para los
    /// callers que sólo tienen un `String`.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            spans: Vec::new(),
        }
    }
}

impl From<&str> for Inline {
    fn from(s: &str) -> Self {
        Inline::plain(s)
    }
}

impl From<String> for Inline {
    fn from(s: String) -> Self {
        Inline::plain(s)
    }
}

/// Acumula texto + spans de un bloque mientras se recorren los eventos
/// inline de pulldown-cmark. Mantiene contadores por estilo (anidan) y
/// muestrea el formato activo en cada empuje de texto.
#[derive(Default)]
struct InlineBuilder {
    text: String,
    spans: Vec<(usize, usize, InlineFlags)>,
    bold: u32,
    italic: u32,
    strike: u32,
    link: u32,
}

impl InlineBuilder {
    fn flags(&self) -> InlineFlags {
        InlineFlags {
            bold: self.bold > 0,
            italic: self.italic > 0,
            code: false,
            strikethrough: self.strike > 0,
            link: self.link > 0,
        }
    }

    /// Empuja texto corrido con el formato activo.
    fn push_text(&mut self, t: &str) {
        self.push_with(t, self.flags());
    }

    /// Empuja un tramo de código inline (monospace), respetando además el
    /// formato activo (un `**`code`**` raro queda bold+code).
    fn push_code(&mut self, t: &str) {
        let mut f = self.flags();
        f.code = true;
        self.push_with(t, f);
    }

    fn push_with(&mut self, t: &str, f: InlineFlags) {
        if t.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(t);
        let end = self.text.len();
        if !f.is_plain() {
            self.spans.push((start, end, f));
        }
    }

    fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Consume el builder en un `Inline` con el texto trimeado y los spans
    /// reajustados al recorte (el trim de bordes corre los offsets).
    fn take(&mut self) -> Inline {
        let raw = std::mem::take(&mut self.text);
        let spans = std::mem::take(&mut self.spans);
        let lead = raw.len() - raw.trim_start().len();
        let trimmed = raw.trim().to_string();
        let tlen = trimmed.len();
        let spans = spans
            .into_iter()
            .filter_map(|(s, e, f)| {
                let ns = s.saturating_sub(lead).min(tlen);
                let ne = e.saturating_sub(lead).min(tlen);
                (ne > ns).then_some((ns, ne, f))
            })
            .collect();
        Inline {
            text: trimmed,
            spans,
        }
    }
}

/// Un bloque del documento con su estilo semántico. El render mapea cada
/// variante a un tamaño/fuente/color; el formato inline viaja en el
/// [`Inline`] de cada variante de texto.
#[derive(Debug, Clone, PartialEq)]
pub enum MdBlock {
    /// Encabezado `#`..`######` (nivel 1–6).
    Heading { level: u8, text: Inline },
    /// Párrafo de texto corrido.
    Paragraph(Inline),
    /// Bloque de código (fenced o indentado), en monoespaciada.
    Code(String),
    /// Ítem de lista; `depth` 0 = nivel raíz.
    ListItem { depth: u8, text: Inline },
    /// Cita (`>`), en itálica.
    Quote(Inline),
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
/// `true` si se cortó en [`MAX_BLOCKS`]. El formato inline (negrita,
/// itálica, código, tachado, enlaces) se conserva como spans dentro del
/// [`Inline`] de cada bloque de texto; sólo el `href` de los enlaces se
/// descarta (el visor no navega).
pub fn parse_blocks(src: &str) -> (Vec<MdBlock>, bool) {
    use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

    let mut blocks: Vec<MdBlock> = Vec::new();
    // Acumulador del texto + spans del bloque inline en curso.
    let mut inl = InlineBuilder::default();
    // Buffer separado para bloques de código (texto plano, sin spans).
    let mut code_buf = String::new();
    let mut in_code_block = false;
    // Profundidad de listas anidadas (cantidad de `List` abiertas).
    let mut list_depth: u8 = 0;
    let mut in_item = false;
    let mut quote_depth: u8 = 0;

    let push = |blocks: &mut Vec<MdBlock>, b: MdBlock| {
        if blocks.len() < MAX_BLOCKS {
            blocks.push(b);
        }
    };

    for ev in Parser::new_ext(src, pulldown_cmark::Options::ENABLE_STRIKETHROUGH) {
        if blocks.len() >= MAX_BLOCKS {
            return (blocks, true);
        }
        match ev {
            Event::Start(Tag::Heading { .. }) => {
                inl = InlineBuilder::default();
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
                        text: inl.take(),
                    },
                );
            }
            Event::Start(Tag::CodeBlock(_)) => {
                code_buf.clear();
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                let code = std::mem::take(&mut code_buf);
                push(
                    &mut blocks,
                    MdBlock::Code(code.trim_end_matches('\n').to_string()),
                );
            }
            Event::Start(Tag::List(_)) => {
                // Una lista anidada arranca dentro de un ítem; el texto de
                // cabecera (el del ítem padre) está en `inl` y se perdería
                // al reiniciarlo en el `Start(Item)` hijo. Lo emitimos ahora,
                // a la profundidad del ítem padre.
                if in_item && !inl.is_empty() {
                    let depth = list_depth.saturating_sub(1).min(MAX_LIST_DEPTH);
                    push(
                        &mut blocks,
                        MdBlock::ListItem {
                            depth,
                            text: inl.take(),
                        },
                    );
                }
                list_depth = list_depth.saturating_add(1);
            }
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
            }
            Event::Start(Tag::Item) => {
                in_item = true;
                inl = InlineBuilder::default();
            }
            Event::End(TagEnd::Item) => {
                in_item = false;
                if !inl.is_empty() {
                    let depth = list_depth.saturating_sub(1).min(MAX_LIST_DEPTH);
                    push(
                        &mut blocks,
                        MdBlock::ListItem {
                            depth,
                            text: inl.take(),
                        },
                    );
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
                if in_item || inl.is_empty() {
                    continue;
                }
                let text = inl.take();
                if quote_depth > 0 {
                    push(&mut blocks, MdBlock::Quote(text));
                } else {
                    push(&mut blocks, MdBlock::Paragraph(text));
                }
            }
            Event::Start(Tag::Strong) => inl.bold += 1,
            Event::End(TagEnd::Strong) => inl.bold = inl.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => inl.italic += 1,
            Event::End(TagEnd::Emphasis) => inl.italic = inl.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => inl.strike += 1,
            Event::End(TagEnd::Strikethrough) => inl.strike = inl.strike.saturating_sub(1),
            Event::Start(Tag::Link { .. }) => inl.link += 1,
            Event::End(TagEnd::Link) => inl.link = inl.link.saturating_sub(1),
            Event::Text(t) => {
                if in_code_block {
                    code_buf.push_str(&t);
                } else {
                    inl.push_text(&t);
                }
            }
            Event::Code(t) => inl.push_code(&t),
            Event::SoftBreak => inl.push_text(" "),
            Event::HardBreak => inl.push_text("\n"),
            Event::Rule => {
                inl = InlineBuilder::default();
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
    fn parrafo_conserva_inline_como_spans() {
        let (b, _) = parse_blocks("hola **mundo** y `code` final\n");
        // El texto se aplana SIN marcadores; el formato viaja en spans.
        let MdBlock::Paragraph(inl) = &b[0] else {
            panic!("esperaba párrafo, fue {:?}", b[0]);
        };
        assert_eq!(inl.text, "hola mundo y code final");
        // "mundo" en negrita.
        let bold = inl
            .spans
            .iter()
            .find(|(_, _, f)| f.bold)
            .expect("span negrita");
        assert_eq!(&inl.text[bold.0..bold.1], "mundo");
        // "code" en monospace.
        let code = inl
            .spans
            .iter()
            .find(|(_, _, f)| f.code)
            .expect("span code");
        assert_eq!(&inl.text[code.0..code.1], "code");
    }

    #[test]
    fn enlace_marca_link() {
        let (b, _) = parse_blocks("ver [docs](http://x) acá\n");
        let MdBlock::Paragraph(inl) = &b[0] else {
            panic!("esperaba párrafo");
        };
        assert_eq!(inl.text, "ver docs acá");
        let link = inl.spans.iter().find(|(_, _, f)| f.link).expect("link");
        assert_eq!(&inl.text[link.0..link.1], "docs");
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
