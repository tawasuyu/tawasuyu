//! `foreign-docx` — importa archivos `.docx` (Office Open XML) como
//! cuerpos madre del multilienzo de pluma.
//!
//! Un `.docx` es un zip que contiene `word/document.xml` con el cuerpo
//! del documento serializado en XML. Este crate hace lo mínimo
//! necesario: descomprime, abre `document.xml`, recorre los párrafos
//! `<w:p>`, junta los runs `<w:t>` de cada uno, y produce un
//! `NarrativeAtom` por párrafo.
//!
//! NO interpreta formato (negrita, cursiva, color), estilos, headers,
//! footers, tablas, comments, ni elementos avanzados. La meta es
//! ingestar el contenido legible — quien quiera fidelidad de formato
//! debería trabajar el doc en su editor nativo, no traerlo a pluma.
//!
//! ## Ejemplo
//!
//! ```no_run
//! use foreign_docx::parse_docx;
//! let bytes = std::fs::read("informe.docx").unwrap();
//! let imp = parse_docx(&bytes, "es", "informe.docx", 0).unwrap();
//! // imp.cuerpo: Intencion::Original
//! // imp.atoms: un NarrativeAtom por párrafo del .docx
//! ```

#![forbid(unsafe_code)]

use std::io::{Cursor, Read};

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use thiserror::Error;

use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};

/// Resultado del import: el cuerpo madre + sus átomos. Misma shape que
/// `pluma_md::DocumentoImportado` para que los demos puedan tratar
/// ambos igual.
#[derive(Debug)]
pub struct DocumentoImportado {
    pub cuerpo: Cuerpo,
    pub atoms: Vec<NarrativeAtom>,
}

#[derive(Debug, Error)]
pub enum DocxError {
    #[error("no es un zip válido (.docx): {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("falta `word/document.xml` en el archivo")]
    DocumentoFaltante,
    #[error("lectura del archivo interno falló: {0}")]
    Io(#[from] std::io::Error),
    #[error("XML malformado: {0}")]
    Xml(#[from] quick_xml::Error),
}

/// Importa un `.docx` (bytes crudos del archivo, p. ej. devuelto por
/// `std::fs::read`) como cuerpo Original. `branch_id`, `nombre` y
/// `ahora` se anotan en `MetaCuerpo`.
pub fn parse_docx(
    bytes: &[u8],
    branch_id: impl Into<String>,
    nombre: impl Into<String>,
    ahora: u64,
) -> Result<DocumentoImportado, DocxError> {
    let cursor = Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor)?;
    let mut xml = String::new();
    {
        let mut entry = zip
            .by_name("word/document.xml")
            .map_err(|_| DocxError::DocumentoFaltante)?;
        entry.read_to_string(&mut xml)?;
    }
    let parrafos = extraer_parrafos(&xml)?;

    let branch = branch_id.into();
    let mut cuerpo = Cuerpo::nuevo(branch.clone(), nombre, Intencion::Original, ahora);
    let mut atoms = Vec::with_capacity(parrafos.len());
    for texto in parrafos {
        let texto = texto.trim();
        if texto.is_empty() {
            continue;
        }
        let atom = NarrativeAtom::new(texto.to_string(), &branch);
        cuerpo.agregar(atom.id, ahora);
        atoms.push(atom);
    }
    Ok(DocumentoImportado { cuerpo, atoms })
}

/// Parser SAX mínimo: cada `<w:p>` abre un buffer; cada `<w:t>` añade
/// su texto; `</w:p>` cierra y empuja al output.
///
/// Reconocemos tanto `w:t` como `t` (algunos generadores omiten el
/// prefijo; el namespace queda igual al ser el default del documento).
/// Saltos `<w:br/>` se ignoran — quedan como espacio implícito entre
/// los runs adyacentes.
fn extraer_parrafos(xml: &str) -> Result<Vec<String>, quick_xml::Error> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(false);

    let mut parrafos = Vec::new();
    let mut buf = String::new();
    let mut en_parrafo = false;
    let mut en_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let local = nombre_local(name.as_ref());
                if local == b"p" {
                    en_parrafo = true;
                    buf.clear();
                } else if en_parrafo && local == b"t" {
                    en_text = true;
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let local = nombre_local(name.as_ref());
                if local == b"p" {
                    parrafos.push(buf.clone());
                    buf.clear();
                    en_parrafo = false;
                } else if local == b"t" {
                    en_text = false;
                }
            }
            Ok(Event::Text(e)) => {
                if en_parrafo && en_text {
                    let s = e.unescape()?;
                    buf.push_str(s.as_ref());
                }
            }
            // Word usa `<w:br/>` para saltos de línea suaves dentro de
            // un párrafo. Lo tratamos como espacio para no concatenar.
            Ok(Event::Empty(e)) => {
                let name = e.name();
                let local = nombre_local(name.as_ref());
                if en_parrafo && local == b"br" {
                    if !buf.ends_with(' ') && !buf.is_empty() {
                        buf.push(' ');
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(e),
            _ => {}
        }
    }
    Ok(parrafos)
}

/// Quita el prefijo de namespace (`w:p` → `p`). Acepta tanto QName con
/// prefijo (`w:p`) como sin (`p`). El default-ns del documento Word es
/// el namespace `w`, pero hay docs ajenos sin prefijo — soportamos los
/// dos.
fn nombre_local(qname: &[u8]) -> &[u8] {
    match qname.iter().position(|b| *b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::io::Write;

    /// Crea un `.docx` mínimo en memoria con los párrafos dados.
    /// Genera solo el `[Content_Types].xml` mínimo, `_rels/.rels`,
    /// `word/_rels/document.xml.rels` y `word/document.xml` con los
    /// párrafos. Suficiente para que ZipArchive lo abra y nuestro
    /// parser extraiga el contenido.
    fn docx_de(parrafos: &[&str]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::SimpleFileOptions = Default::default();
            // Content_Types mínimo. Word lo requiere para abrir el doc,
            // pero nuestro parser no lo consulta — basta con que el zip
            // tenga la entrada `word/document.xml`.
            w.start_file("[Content_Types].xml", opts).unwrap();
            w.write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
            )
            .unwrap();
            // El documento.
            let mut xml = String::from(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>"#,
            );
            for p in parrafos {
                xml.push_str(&format!(
                    "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                    p
                ));
            }
            xml.push_str("</w:body></w:document>");
            w.start_file("word/document.xml", opts).unwrap();
            w.write_all(xml.as_bytes()).unwrap();
            w.finish().unwrap();
        }
        buf
    }

    #[test]
    fn parrafos_se_separan_en_atoms() {
        let docx = docx_de(&["Primero.", "Segundo.", "Tercero."]);
        let imp = parse_docx(&docx, "es", "test.docx", 1).unwrap();
        assert_eq!(imp.atoms.len(), 3);
        assert_eq!(imp.atoms[0].content.as_str(), "Primero.");
        assert_eq!(imp.atoms[1].content.as_str(), "Segundo.");
        assert_eq!(imp.atoms[2].content.as_str(), "Tercero.");
        assert_eq!(imp.cuerpo.metadatos.intencion, Intencion::Original);
        assert_eq!(imp.cuerpo.branch_id, "es");
        assert_eq!(imp.cuerpo.orden.len(), 3);
    }

    #[test]
    fn parrafos_vacios_se_omiten() {
        let docx = docx_de(&["", "Solo este.", "", "   "]);
        let imp = parse_docx(&docx, "es", "x", 0).unwrap();
        assert_eq!(imp.atoms.len(), 1);
        assert_eq!(imp.atoms[0].content.as_str(), "Solo este.");
    }

    #[test]
    fn archivo_sin_document_xml_devuelve_error() {
        // Zip vacío.
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            w.start_file("hola.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(b"no soy docx").unwrap();
            w.finish().unwrap();
        }
        match parse_docx(&buf, "es", "x", 0) {
            Err(DocxError::DocumentoFaltante) => {}
            otro => panic!("esperaba DocumentoFaltante, fue {otro:?}"),
        }
    }

    #[test]
    fn bytes_no_zip_devuelve_zip_error() {
        let basura = b"esto no es un zip ni en broma";
        assert!(matches!(
            parse_docx(basura, "es", "x", 0),
            Err(DocxError::Zip(_))
        ));
    }

    #[test]
    fn cuerpo_orden_y_atoms_referencian_los_mismos_uuids() {
        let docx = docx_de(&["a", "b", "c"]);
        let imp = parse_docx(&docx, "es", "x", 0).unwrap();
        for (atom, uuid) in imp.atoms.iter().zip(imp.cuerpo.orden.iter()) {
            assert_eq!(&atom.id, uuid);
        }
    }
}
