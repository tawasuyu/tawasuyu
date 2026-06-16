//! Metadatos de fuentes TTF/OTF — núcleo agnóstico de render.
//!
//! Extrae los datos escalares de una fuente (familia, estilo, nº de glifos,
//! unidades/em, ascender/descender) con `ttf-parser`. Los **contornos** de los
//! glifos (que sí son render: van a un `BezPath`) los arma el frontend
//! `nahual-font-viewer-llimphi`; acá sólo vive lo que cualquier consumidor
//! (UI, CLI, indexador) reusaría sin pintar nada (Regla 2).

/// Metadatos escalares de una fuente abierta. Sin contornos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontMeta {
    pub family: String,
    pub subfamily: String,
    pub num_glyphs: u16,
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
}

/// Extrae los metadatos de una `Face` ya parseada (el frontend ya la tiene para
/// los contornos; así no se re-parsea).
pub fn meta_from_face(face: &ttf_parser::Face<'_>) -> FontMeta {
    FontMeta {
        family: pick_name(face, 1).unwrap_or_else(|| "(sin nombre)".to_string()),
        subfamily: pick_name(face, 2).unwrap_or_else(|| "Regular".to_string()),
        num_glyphs: face.number_of_glyphs(),
        units_per_em: face.units_per_em(),
        ascender: face.ascender(),
        descender: face.descender(),
    }
}

/// Parsea los metadatos directamente desde los bytes del archivo.
pub fn parse_meta(bytes: &[u8]) -> Result<FontMeta, String> {
    let face =
        ttf_parser::Face::parse(bytes, 0).map_err(|e| format!("no parsea como fuente: {e}"))?;
    Ok(meta_from_face(&face))
}

/// Toma el primer `name` legible con el `name_id` pedido (1=familia,
/// 2=subfamilia). `ttf-parser` sólo devuelve string para encodings
/// Unicode/Mac, así que algunos nombres salen `None`.
fn pick_name(face: &ttf_parser::Face<'_>, want_id: u16) -> Option<String> {
    face.names()
        .into_iter()
        .filter(|n| n.name_id == want_id)
        .find_map(|n| n.to_string())
        .filter(|s| !s.is_empty())
}
