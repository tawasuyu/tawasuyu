//! `foreign-xlsx` — puente Excel (`.xlsx`) ↔ hoja nativa `nakui_sheet::Sheet`.
//!
//! Un `.xlsx` es un ZIP de XML (SpreadsheetML). Importar = abrir el zip, leer la
//! tabla de strings compartidos y la primera hoja, y reconstruir un `Sheet`
//! cuyas fórmulas parsea/evalúa `yupay`. Exportar = serializar un `Sheet` al
//! conjunto mínimo de partes que Excel necesita para abrir el archivo.
//!
//! **Las fórmulas de Excel viajan en inglés canónico** (`SUM`, `VLOOKUP`…) con
//! referencias A1 — el mismo lenguaje que entiende `yupay`, así que mapean
//! directo. (Excel guarda siempre el inglés, sin importar el idioma de la UI.)
//!
//! MVP (regla #4: el formato ajeno no entra al núcleo, vive aquí):
//! - Primera hoja (`xl/worksheets/sheet1.xml`).
//! - Tipos de celda: número, texto (shared string e inline), bool, error,
//!   fórmula (con su valor cacheado).
//! - Round-trip verificado: `Sheet → export → import → Sheet` preserva
//!   valores y fórmulas.
//!
//! Post-MVP: estilos ricos, múltiples hojas, formatos numéricos, gráficos,
//! tablas dinámicas, y canonicalización es→en al exportar fórmulas en español
//! (hoy el raw se escribe tal cual: el inglés round-trips, el español saldría
//! como `#NAME?` en Excel hasta traducir el AST).

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use nakui_sheet::{CellRef, Sheet, SheetValue};
use yupay_core::formula::FormulaExpr;
use yupay_core::value::SheetError;

#[derive(Debug, thiserror::Error)]
pub enum XlsxError {
    #[error("zip inválido: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("xml malformado: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("no se encontró ninguna hoja (`xl/worksheets/sheet1.xml`)")]
    NoSheet,
    #[error("referencia de celda inválida `{0}`")]
    BadRef(String),
}

// ───────────────────────────── Importar ─────────────────────────────

/// Lee un `.xlsx` en memoria y devuelve la primera hoja como `Sheet` nativa.
pub fn import_xlsx(bytes: &[u8]) -> Result<Sheet, XlsxError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;

    // Strings compartidos (opcional: una hoja sin texto no tiene esta parte).
    let shared = match read_zip_text(&mut zip, "xl/sharedStrings.xml") {
        Some(xml) => parse_shared_strings(&xml)?,
        None => Vec::new(),
    };

    let sheet_xml = read_zip_text(&mut zip, "xl/worksheets/sheet1.xml").ok_or(XlsxError::NoSheet)?;
    parse_worksheet(&sheet_xml, &shared)
}

/// Devuelve el contenido de una entrada del zip como String, o `None` si no
/// existe. Otros errores de lectura se silencian a `None` (parte ausente).
fn read_zip_text(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Tabla de strings compartidos: cada `<si>` aporta un string (concatenando
/// sus `<t>`, lo que cubre el caso de runs de texto enriquecido `<r><t>…`).
fn parse_shared_strings(xml: &str) -> Result<Vec<String>, XlsxError> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);

    let mut out = Vec::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut cur = String::new();

    loop {
        match reader.read_event()? {
            Event::Start(e) if e.local_name().as_ref() == b"si" => {
                in_si = true;
                cur.clear();
            }
            Event::End(e) if e.local_name().as_ref() == b"si" => {
                in_si = false;
                out.push(std::mem::take(&mut cur));
            }
            Event::Start(e) if e.local_name().as_ref() == b"t" => in_t = true,
            Event::End(e) if e.local_name().as_ref() == b"t" => in_t = false,
            Event::Text(t) if in_si && in_t => cur.push_str(&t.unescape()?),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Estado de captura del parser de hoja: qué texto estamos acumulando.
#[derive(PartialEq)]
enum Cap {
    None,
    Formula,
    Value,
    Inline,
}

fn parse_worksheet(xml: &str, shared: &[String]) -> Result<Sheet, XlsxError> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);

    let mut sheet = Sheet::new();

    let mut cur_ref: Option<CellRef> = None;
    let mut cur_type = String::new(); // "" | s | str | b | e | inlineStr
    let mut cur_f: Option<String> = None;
    let mut cur_v: Option<String> = None;
    let mut cap = Cap::None;

    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"c" => {
                // Nueva celda: leer atributos r (ref) y t (tipo).
                cur_ref = None;
                cur_type.clear();
                cur_f = None;
                cur_v = None;
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"r" => {
                            let v = attr.unescape_value()?;
                            cur_ref = Some(parse_ref(&v)?);
                        }
                        b"t" => cur_type = attr.unescape_value()?.into_owned(),
                        _ => {}
                    }
                }
                // `<c .../>` vacío: nada que materializar.
            }
            Event::Start(e) if e.local_name().as_ref() == b"f" => cap = Cap::Formula,
            Event::Start(e) if e.local_name().as_ref() == b"v" => cap = Cap::Value,
            Event::Start(e) if e.local_name().as_ref() == b"t" && cur_type == "inlineStr" => {
                cap = Cap::Inline
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"f" | b"v" | b"t" => cap = Cap::None,
                b"c" => {
                    if let Some(cr) = cur_ref.take() {
                        materialize(&mut sheet, cr, &cur_type, &cur_f, &cur_v, shared)?;
                    }
                }
                _ => {}
            },
            Event::Text(t) => {
                let s = t.unescape()?;
                match cap {
                    Cap::Formula => cur_f.get_or_insert_with(String::new).push_str(&s),
                    Cap::Value | Cap::Inline => cur_v.get_or_insert_with(String::new).push_str(&s),
                    Cap::None => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(sheet)
}

/// Inserta la celda leída en el `Sheet`. Las fórmulas se setean con su fuente
/// (`=…`) para que yupay las re-evalúe; los literales con el tipo correcto
/// (forzando texto vía `set_cell_expr` para que una shared string como `"42"`
/// no se interprete como número).
fn materialize(
    sheet: &mut Sheet,
    cr: CellRef,
    ty: &str,
    f: &Option<String>,
    v: &Option<String>,
    shared: &[String],
) -> Result<(), XlsxError> {
    // Fórmula: prima sobre el valor cacheado.
    if let Some(src) = f {
        let raw = format!("={src}");
        let _ = sheet.set_cell(cr, &raw);
        return Ok(());
    }
    let Some(v) = v else { return Ok(()) };

    match ty {
        "s" => {
            // Índice a la tabla de strings compartidos.
            let idx: usize = v.trim().parse().unwrap_or(usize::MAX);
            let text = shared.get(idx).cloned().unwrap_or_default();
            let _ = sheet.set_cell_expr(cr, FormulaExpr::Text(text.clone()), text);
        }
        "inlineStr" | "str" => {
            let _ = sheet.set_cell_expr(cr, FormulaExpr::Text(v.clone()), v.clone());
        }
        "b" => {
            let raw = if v.trim() == "1" { "TRUE" } else { "FALSE" };
            let _ = sheet.set_cell(cr, raw);
        }
        "e" => {
            let err = error_from_token(v.trim());
            let _ = sheet.set_cell_expr(cr, FormulaExpr::ErrorLiteral(err), v.clone());
        }
        _ => {
            // Número (tipo por defecto, atributo `t` ausente).
            let _ = sheet.set_cell(cr, v.trim());
        }
    }
    Ok(())
}

fn error_from_token(tok: &str) -> SheetError {
    match tok {
        "#DIV/0!" => SheetError::DivZero,
        "#REF!" => SheetError::Ref,
        "#NAME?" => SheetError::Name,
        "#N/A" => SheetError::NotApplicable,
        "#NUM!" => SheetError::Num,
        "#CYCLE!" => SheetError::Cycle,
        _ => SheetError::Value,
    }
}

fn parse_ref(s: &str) -> Result<CellRef, XlsxError> {
    s.parse::<CellRef>()
        .map_err(|_| XlsxError::BadRef(s.to_string()))
}

// ───────────────────────────── Exportar ─────────────────────────────

/// Serializa un `Sheet` nativo a un `.xlsx` en memoria (zip deflate).
pub fn export_xlsx(sheet: &Sheet) -> Result<Vec<u8>, XlsxError> {
    // Agrupar celdas pobladas por fila → columna, en orden.
    let mut rows: BTreeMap<u32, BTreeMap<u32, CellRef>> = BTreeMap::new();
    for (cr, _) in sheet.iter_values() {
        rows.entry(cr.row).or_default().insert(cr.col, cr);
    }

    // Tabla de strings compartidos (texto literal y resultados de fórmula tipo
    // texto). Index estable por orden de inserción.
    let mut shared: Vec<String> = Vec::new();
    let mut shared_idx: BTreeMap<String, usize> = BTreeMap::new();
    let mut intern = |s: &str| -> usize {
        if let Some(&i) = shared_idx.get(s) {
            return i;
        }
        let i = shared.len();
        shared.push(s.to_string());
        shared_idx.insert(s.to_string(), i);
        i
    };

    // Cuerpo de la hoja.
    let mut sheet_data = String::new();
    for (row, cols) in &rows {
        sheet_data.push_str(&format!("<row r=\"{}\">", row + 1));
        for cr in cols.values() {
            sheet_data.push_str(&cell_xml(sheet, *cr, &mut intern));
        }
        sheet_data.push_str("</row>");
    }

    let worksheet = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
<sheetData>{sheet_data}</sheetData></worksheet>"
    );

    let mut shared_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" count=\"{n}\" uniqueCount=\"{n}\">",
        n = shared.len()
    );
    for s in &shared {
        shared_xml.push_str(&format!("<si><t xml:space=\"preserve\">{}</t></si>", esc(s)));
    }
    shared_xml.push_str("</sst>");

    // Empaquetar el zip con las partes mínimas.
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        let mut put = |name: &str, body: &str| -> Result<(), XlsxError> {
            zip.start_file(name, opts)?;
            zip.write_all(body.as_bytes())?;
            Ok(())
        };

        put("[Content_Types].xml", CONTENT_TYPES)?;
        put("_rels/.rels", ROOT_RELS)?;
        put("xl/workbook.xml", WORKBOOK)?;
        put("xl/_rels/workbook.xml.rels", WORKBOOK_RELS)?;
        put("xl/worksheets/sheet1.xml", &worksheet)?;
        if !shared.is_empty() {
            put("xl/sharedStrings.xml", &shared_xml)?;
        }
        zip.finish()?;
    }
    Ok(buf)
}

/// Serializa una celda a su `<c>`. Decide el tipo por el `raw` (fórmula si
/// empieza con `=`) y por el `SheetValue` cacheado.
fn cell_xml(sheet: &Sheet, cr: CellRef, intern: &mut impl FnMut(&str) -> usize) -> String {
    let r = cr.to_string();
    let raw = sheet.raw(cr).unwrap_or("");
    let value = sheet.value(cr);

    if let Some(src) = raw.strip_prefix('=') {
        // Fórmula + valor cacheado. `t="str"` si el resultado es texto.
        let f = esc(src);
        return match &value {
            SheetValue::Text(t) => {
                format!("<c r=\"{r}\" t=\"str\"><f>{f}</f><v>{}</v></c>", esc(t))
            }
            SheetValue::Bool(b) => format!(
                "<c r=\"{r}\" t=\"b\"><f>{f}</f><v>{}</v></c>",
                if *b { 1 } else { 0 }
            ),
            SheetValue::Error(e) => {
                format!("<c r=\"{r}\" t=\"e\"><f>{f}</f><v>{}</v></c>", esc(e.token()))
            }
            SheetValue::Number(n) => format!("<c r=\"{r}\"><f>{f}</f><v>{}</v></c>", n.normalize()),
            SheetValue::Empty => format!("<c r=\"{r}\"><f>{f}</f></c>"),
        };
    }

    // Literal.
    match &value {
        SheetValue::Number(n) => format!("<c r=\"{r}\"><v>{}</v></c>", n.normalize()),
        SheetValue::Bool(b) => {
            format!("<c r=\"{r}\" t=\"b\"><v>{}</v></c>", if *b { 1 } else { 0 })
        }
        SheetValue::Error(e) => format!("<c r=\"{r}\" t=\"e\"><v>{}</v></c>", esc(e.token())),
        SheetValue::Text(t) => {
            let idx = intern(t);
            format!("<c r=\"{r}\" t=\"s\"><v>{idx}</v></c>")
        }
        SheetValue::Empty => String::new(),
    }
}

/// Escape XML de los cinco caracteres reservados.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

// Partes fijas del paquete OOXML mínimo.
const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
<Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/>\
</Types>";

const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
</Relationships>";

const WORKBOOK: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
<sheets><sheet name=\"Hoja1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>";

const WORKBOOK_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings\" Target=\"sharedStrings.xml\"/>\
</Relationships>";

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn cr(a: &str) -> CellRef {
        a.parse().unwrap()
    }

    fn poblar() -> Sheet {
        let mut s = Sheet::new();
        s.set_cell(cr("A1"), "10").unwrap();
        s.set_cell(cr("A2"), "20").unwrap();
        s.set_cell(cr("A3"), "30").unwrap();
        s.set_cell(cr("B1"), "Hola mundo").unwrap();
        s.set_cell(cr("B2"), "=A1+A2*2").unwrap();
        s.set_cell(cr("B3"), "=SUM(A1:A3)").unwrap();
        s.set_cell(cr("C1"), "=A1>5").unwrap();
        s.set_cell(cr("C2"), "=1/0").unwrap();
        s
    }

    #[test]
    fn round_trip_preserva_valores_y_formulas() {
        let original = poblar();
        let bytes = export_xlsx(&original).unwrap();
        let vuelta = import_xlsx(&bytes).unwrap();

        // Valores numéricos y agregados.
        assert_eq!(vuelta.value(cr("A1")), SheetValue::Number(Decimal::from(10)));
        assert_eq!(vuelta.value(cr("B2")), SheetValue::Number(Decimal::from(50)));
        assert_eq!(vuelta.value(cr("B3")), SheetValue::Number(Decimal::from(60)));
        // Texto vía shared strings.
        assert_eq!(vuelta.value(cr("B1")), SheetValue::Text("Hola mundo".into()));
        // Bool y error.
        assert_eq!(vuelta.value(cr("C1")), SheetValue::Bool(true));
        assert_eq!(vuelta.value(cr("C2")), SheetValue::Error(SheetError::DivZero));

        // Las fórmulas siguen siendo fórmulas (raw con `=`), no valores planos.
        assert_eq!(vuelta.raw(cr("B3")), Some("=SUM(A1:A3)"));
        // Y siguen reactivas: cambiar A1 recomputa B3.
        let mut v2 = vuelta;
        v2.set_cell(cr("A1"), "100").unwrap();
        assert_eq!(v2.value(cr("B3")), SheetValue::Number(Decimal::from(150)));
    }

    #[test]
    fn produce_un_zip_valido() {
        let bytes = export_xlsx(&poblar()).unwrap();
        // Firma de zip local file header.
        assert_eq!(&bytes[..2], b"PK");
        let zip = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        // Las partes obligatorias están.
        let names: Vec<_> = zip.file_names().collect();
        assert!(names.contains(&"[Content_Types].xml"));
        assert!(names.contains(&"xl/worksheets/sheet1.xml"));
        assert!(names.contains(&"xl/sharedStrings.xml"));
    }

    #[test]
    fn texto_numerico_se_preserva_como_texto() {
        // Una shared string "42" no debe volver como número.
        let mut s = Sheet::new();
        s.set_cell_expr(cr("A1"), FormulaExpr::Text("42".into()), "42".into())
            .unwrap();
        let bytes = export_xlsx(&s).unwrap();
        let vuelta = import_xlsx(&bytes).unwrap();
        assert_eq!(vuelta.value(cr("A1")), SheetValue::Text("42".into()));
    }
}
