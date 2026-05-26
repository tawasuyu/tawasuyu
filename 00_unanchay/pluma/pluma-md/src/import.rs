//! Importador: markdown → `(Cuerpo, Vec<NarrativeAtom>)`.
//!
//! Convierte un documento markdown en un cuerpo madre apto para el
//! multilienzo, con un `NarrativeAtom` por *bloque*. Bloque = párrafo,
//! ítem de lista, encabezado, code block, blockquote, tabla. Cada uno
//! aporta un átomo independiente que puede traducirse o transformarse
//! con `pluma-transform-llm` y alinearse con `pluma-align-embeddings`.
//!
//! El texto de cada bloque se aplana: el átomo lleva el plain-text del
//! bloque (sin sintaxis markdown). Eso permite que un párrafo `**hola
//! mundo**` se traduzca como "hola mundo" sin que el modelo lidie con
//! asteriscos. Si el caller necesita el markdown original, lo guarda
//! aparte; la idea de "cuerpo" no es preservar formato, es preservar
//! contenido legible alineable.
//!
//! Encabezados (`#`, `##`, …) se inyectan como átomos con prefijo
//! `"# "` / `"## "` / etc. al texto del título. Ese pequeño marcador
//! le dice al modelo (o al lector) que es un heading sin necesidad de
//! un campo extra en `NarrativeAtom`.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};

use crate::default_options;

/// Resultado del import: el cuerpo madre + sus átomos.
pub struct DocumentoImportado {
    pub cuerpo: Cuerpo,
    pub atoms: Vec<NarrativeAtom>,
}

/// Importa markdown como cuerpo Original. `branch_id` y `nombre` se
/// anotan en los metadatos del cuerpo; `ahora` es el timestamp de
/// creación. Devuelve el cuerpo + los `NarrativeAtom`s, listos para
/// `graph.insert(atom)` en el caller.
pub fn parse_md(
    md: &str,
    branch_id: impl Into<String>,
    nombre: impl Into<String>,
    ahora: u64,
) -> DocumentoImportado {
    let branch = branch_id.into();
    let mut cuerpo = Cuerpo::nuevo(branch.clone(), nombre, Intencion::Original, ahora);
    let bloques = bloques_planos(md, default_options());
    let mut atoms = Vec::with_capacity(bloques.len());
    for texto in bloques {
        let texto = texto.trim();
        if texto.is_empty() {
            continue;
        }
        let atom = NarrativeAtom::new(texto.to_string(), &branch);
        cuerpo.agregar(atom.id, ahora);
        atoms.push(atom);
    }
    DocumentoImportado { cuerpo, atoms }
}

/// Recorre el stream de eventos pulldown y produce un string por
/// bloque. Mantiene un cursor de texto + un prefijo de heading que se
/// resetea al cerrar el bloque.
fn bloques_planos(md: &str, opts: Options) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut en_code_block = false;
    let mut prefijo_pendiente: Option<String> = None;

    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    let hashes = "#".repeat(level as usize);
                    prefijo_pendiente = Some(format!("{hashes} "));
                }
                Tag::CodeBlock(_) => {
                    en_code_block = true;
                }
                Tag::List(_) | Tag::Item | Tag::BlockQuote(_) | Tag::Paragraph => {
                    // Estos tags abren un bloque nuevo — el "end" lo cierra.
                }
                Tag::Table(_) | Tag::TableHead | Tag::TableRow | Tag::TableCell => {
                    // Tablas: las aplanamos por celda separada por `\t` por fila.
                }
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::TableRow
                | TagEnd::TableHead => {
                    let mut bloque = buf.trim().to_string();
                    if let Some(p) = prefijo_pendiente.take() {
                        bloque = format!("{p}{bloque}");
                    }
                    if !bloque.is_empty() {
                        out.push(bloque);
                    }
                    buf.clear();
                    en_code_block = false;
                }
                _ => {}
            },
            Event::Text(t) => {
                buf.push_str(&t);
            }
            Event::Code(t) => {
                // Código inline: lo metemos al buffer del bloque actual.
                buf.push_str(&t);
            }
            Event::SoftBreak | Event::HardBreak => {
                // Un salto blando dentro de un párrafo: lo aplanamos a
                // espacio. Los saltos duros pierden contexto, pero como
                // el átomo es un bloque, una sola línea visible alcanza.
                if en_code_block {
                    buf.push('\n');
                } else {
                    buf.push(' ');
                }
            }
            Event::TaskListMarker(_) | Event::Html(_) | Event::InlineHtml(_) => {
                // Ignorados — el texto que los rodea ya se captura.
            }
            Event::Rule => {
                // Separador horizontal: cerrar bloque actual.
                let bloque = buf.trim().to_string();
                if !bloque.is_empty() {
                    out.push(bloque);
                }
                buf.clear();
            }
            _ => {}
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn textos(d: &DocumentoImportado) -> Vec<String> {
        d.atoms.iter().map(|a| a.content.to_string()).collect()
    }

    #[test]
    fn parrafos_basicos_se_separan() {
        let md = "Primer párrafo.\n\nSegundo párrafo.\n\nTercer párrafo.";
        let d = parse_md(md, "es", "doc.md", 1);
        assert_eq!(textos(&d), vec![
            "Primer párrafo.".to_string(),
            "Segundo párrafo.".to_string(),
            "Tercer párrafo.".to_string(),
        ]);
        assert_eq!(d.cuerpo.orden.len(), 3);
        assert_eq!(d.cuerpo.metadatos.intencion, Intencion::Original);
        assert_eq!(d.cuerpo.branch_id, "es");
    }

    #[test]
    fn encabezado_lleva_prefijo_hash_segun_nivel() {
        let d = parse_md("# Título\n\n## Subtítulo\n\nTexto.", "es", "x", 0);
        let ts = textos(&d);
        assert_eq!(ts[0], "# Título");
        assert_eq!(ts[1], "## Subtítulo");
        assert_eq!(ts[2], "Texto.");
    }

    #[test]
    fn lista_genera_un_atom_por_item() {
        let d = parse_md("- uno\n- dos\n- tres", "es", "x", 0);
        let ts = textos(&d);
        assert_eq!(ts, vec!["uno", "dos", "tres"]);
    }

    #[test]
    fn formato_inline_se_aplana_a_texto() {
        let d = parse_md("Un texto con **negrita** y *cursiva* y `código`.", "es", "x", 0);
        let ts = textos(&d);
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0], "Un texto con negrita y cursiva y código.");
    }

    #[test]
    fn code_block_se_preserva_como_bloque() {
        let md = "Texto.\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\nFin.";
        let d = parse_md(md, "es", "x", 0);
        let ts = textos(&d);
        assert_eq!(ts.len(), 3);
        assert_eq!(ts[0], "Texto.");
        assert!(ts[1].contains("fn main"));
        assert!(ts[1].contains("println!"));
        assert_eq!(ts[2], "Fin.");
    }

    #[test]
    fn lineas_vacias_no_producen_atoms_huerfanos() {
        let d = parse_md("\n\n\nPárrafo.\n\n\n", "es", "x", 0);
        assert_eq!(d.atoms.len(), 1);
    }

    #[test]
    fn separador_horizontal_corta_el_bloque() {
        let md = "Uno.\n\n---\n\nDos.";
        let d = parse_md(md, "es", "x", 0);
        let ts = textos(&d);
        assert!(ts.iter().any(|t| t == "Uno."));
        assert!(ts.iter().any(|t| t == "Dos."));
    }

    #[test]
    fn blockquote_se_emite_como_bloque() {
        let d = parse_md("> Citado.\n\nDespués.", "es", "x", 0);
        let ts = textos(&d);
        assert!(ts.iter().any(|t| t == "Citado."));
        assert!(ts.iter().any(|t| t == "Después."));
    }

    #[test]
    fn cuerpo_y_atoms_referencian_los_mismos_uuids() {
        let d = parse_md("a\n\nb\n\nc", "es", "x", 0);
        for (atom, uuid_en_orden) in d.atoms.iter().zip(d.cuerpo.orden.iter()) {
            assert_eq!(&atom.id, uuid_en_orden);
        }
    }
}
