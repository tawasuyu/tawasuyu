//! La celda — la unidad de un notebook.

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

/// Identificador de una celda dentro de su notebook.
pub type CellId = u64;

/// Coordenadas de una celda en el canvas infinito. Si una celda no tiene
/// posición, vive sólo en el orden lineal de presentación; con posición,
/// se le agrega una capa espacial. Las posiciones **no** entran al
/// `content_hash` ni al digest: son presentación, no contenido.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

impl Position {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Qué clase de contenido lleva una celda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellKind {
    /// Prosa en markdown.
    Markdown,
    /// Código ejecutable en un lenguaje.
    Code { language: String },
    /// Una visualización de un módulo brahman (`"dominium"`, `"pineal"`,
    /// `"takiy"`) — pluma_notebook_app integra el ecosistema.
    Embed { module: String },
}

impl CellKind {
    /// Etiqueta estable que distingue las clases en el hash de contenido.
    fn tag(&self) -> &'static str {
        match self {
            CellKind::Markdown => "md",
            CellKind::Code { .. } => "code",
            CellKind::Embed { .. } => "embed",
        }
    }
}

/// Carga tipada del resultado de una celda. Le dice al renderer qué
/// "puerto visual" exponer en el borde (texto, tabla, imagen, geometría…).
/// Un kernel que no sepa producir un tipo rico devuelve `Text(stdout)` y
/// listo — backwards compat con stdout puro.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputPayload {
    /// Sin valor productivo (ej. celda que sólo imprime stdout).
    None,
    /// Texto plano (repr del último valor, log, etc.).
    Text(String),
    /// Un escalar numérico.
    Scalar(f64),
    /// Tabla rectangular columna-encabezada — fila puro string.
    Table { columns: Vec<String>, rows: Vec<Vec<String>> },
    /// Imagen rasterizada con su MIME y bytes crudos (PNG/JPEG/etc.).
    Image { width: u32, height: u32, mime: String, bytes: Vec<u8> },
    /// Nube de puntos / malla / vectores en 3D.
    Geometry { kind: String, points: Vec<[f32; 3]> },
}

impl OutputPayload {
    /// Etiqueta corta del tipo de puerto que la celda expone — sirve para
    /// que la UI elija el ícono/visor sin pattern-matchear el enum entero.
    pub fn port_kind(&self) -> &'static str {
        match self {
            OutputPayload::None => "none",
            OutputPayload::Text(_) => "text",
            OutputPayload::Scalar(_) => "scalar",
            OutputPayload::Table { .. } => "table",
            OutputPayload::Image { .. } => "image",
            OutputPayload::Geometry { .. } => "geometry",
        }
    }
}

/// Última salida persistida de una celda. Espejo de lo que devuelve un
/// kernel al ejecutar; el `exec` la guarda en `Cell::last_output` para que
/// el visor pueda mostrarla y para que un `notebook_digest()` futuro
/// pueda mezclarla (cuando se cierre el loop de outputs addressable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellOutput {
    pub stdout: String,
    pub value: Option<String>,
    pub payload: OutputPayload,
}

impl CellOutput {
    pub fn text(stdout: impl Into<String>) -> Self {
        let s = stdout.into();
        Self { stdout: s.clone(), value: None, payload: OutputPayload::Text(s) }
    }
    pub fn empty() -> Self {
        Self { stdout: String::new(), value: None, payload: OutputPayload::None }
    }
}

/// Estado de frescura de una celda respecto de sus dependencias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellState {
    /// Al día: su resultado corresponde a las fuentes actuales.
    Fresh,
    /// Obsoleta: ella o una dependencia cambió y falta re-ejecutar.
    Stale,
    /// Su última ejecución falló.
    Failed,
}

/// Una celda del notebook: contenido + sus dependencias lógicas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub id: CellId,
    pub kind: CellKind,
    /// El texto fuente — markdown, código o el spec del embed.
    pub source: String,
    /// Celdas prerrequisito (deben ejecutarse antes).
    pub depends_on: Vec<CellId>,
    pub state: CellState,
    /// Posición opcional en el canvas espacial. `None` = sólo orden lineal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    /// Última salida de ejecución, si la hay. No entra al `content_hash`
    /// (es resultado, no fuente); persistirla acá deja que el visor la
    /// muestre y que un guardado/recarga la conserve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_output: Option<CellOutput>,
}

impl Cell {
    /// Hash BLAKE3 del contenido propio de la celda — clase + fuente.
    /// No incluye dependencias; eso es el [`crate::Notebook::digest`].
    pub fn content_hash(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(self.kind.tag().as_bytes());
        h.update(b"\0");
        match &self.kind {
            CellKind::Code { language } => {
                h.update(language.as_bytes());
            }
            CellKind::Embed { module } => {
                h.update(module.as_bytes());
            }
            CellKind::Markdown => {}
        }
        h.update(b"\0");
        h.update(self.source.as_bytes());
        *h.finalize().as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn cell(kind: CellKind, source: &str) -> Cell {
        Cell {
            id: 1,
            kind,
            source: source.into(),
            depends_on: vec![],
            state: CellState::Stale,
            position: None,
            last_output: None,
        }
    }

    #[test]
    fn output_payload_port_kind() {
        assert_eq!(OutputPayload::None.port_kind(), "none");
        assert_eq!(OutputPayload::Text("x".into()).port_kind(), "text");
        assert_eq!(OutputPayload::Scalar(1.0).port_kind(), "scalar");
    }

    #[test]
    fn last_output_does_not_change_the_hash() {
        let mut a = cell(CellKind::Code { language: "rust".into() }, "x");
        let mut b = cell(CellKind::Code { language: "rust".into() }, "x");
        a.last_output = Some(CellOutput::text("hola"));
        b.last_output = Some(CellOutput { stdout: "otra cosa".into(), value: Some("99".into()), payload: OutputPayload::Scalar(99.0) });
        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn same_content_hashes_equal() {
        let a = cell(CellKind::Markdown, "hola");
        let b = cell(CellKind::Markdown, "hola");
        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn kind_changes_the_hash() {
        let md = cell(CellKind::Markdown, "x");
        let code = cell(CellKind::Code { language: "rust".into() }, "x");
        assert_ne!(md.content_hash(), code.content_hash());
    }

    #[test]
    fn language_changes_the_hash() {
        let rust = cell(CellKind::Code { language: "rust".into() }, "1+1");
        let python = cell(CellKind::Code { language: "python".into() }, "1+1");
        assert_ne!(rust.content_hash(), python.content_hash());
    }

    #[test]
    fn position_does_not_change_the_hash() {
        let mut a = cell(CellKind::Markdown, "x");
        let mut b = cell(CellKind::Markdown, "x");
        a.position = Some(Position::new(0.0, 0.0));
        b.position = Some(Position::new(420.0, -77.5));
        assert_eq!(a.content_hash(), b.content_hash());
    }
}
