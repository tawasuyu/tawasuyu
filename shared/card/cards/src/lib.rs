//! `brahman-cards` — brazo unificado de Cards.
//!
//! Brahman maneja varios formatos legítimos de "Card" (la unidad
//! declarativa que describe identidad, datos, módulos, widgets, ...).
//! Cada formato vive en su propio crate de origen y conserva su shape
//! público; lo que este crate aporta es **un único punto de entrada**
//! que sabe interpretar cada uno de ellos y proyectarlos a una sola
//! estructura interna canónica [`Card`].
//!
//! Diseño:
//!
//! ```text
//! ┌─────────────┐  ┌──────────────┐  ┌─────────────┐
//! │ Ente JSON   │  │ Monad JSON   │  │ UiModule    │ … futuro
//! │ (brahman-   │  │ (akasha-     │  │ (nakui-ui-  │
//! │  card)      │  │  card)       │  │  schema)    │
//! └─────┬───────┘  └──────┬───────┘  └──────┬──────┘
//!       │                 │                 │
//!       └────────┬────────┴────────┬────────┘
//!                │  brahman-cards  │
//!                │   (este crate)  │
//!                └────────┬────────┘
//!                         │
//!                  ┌──────▼──────┐
//!                  │   `Card`    │ ← único tipo canónico
//!                  │   wrapper   │   que consumen UI runtime,
//!                  │   común +   │   storage, DHT, wire.
//!                  │   variant   │
//!                  │   body      │
//!                  └─────────────┘
//! ```
//!
//! Los formatos NO se disuelven. Si en el futuro hay que soportar un
//! formato simplificado nuevo, se agrega un reader acá y nadie aguas
//! abajo se entera — siguen recibiendo `Card`.
//!
//! V1 (este commit) sólo soporta inputs JSON. La extensión a Nickel
//! (con templates de defaults vía merge nativo de Nickel) llega en un
//! commit separado para aislar la dependencia `nickel-lang-core`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub use brahman_card::Card as EnteCard;
pub use akasha_card::MonadManifest;
pub use nahual_meta_schema::Module as UiModuleSpec;

/// Estructura canónica única que consumen los downstream del sistema
/// (UI runtime, storage, DHT, wire). Cada formato input se proyecta
/// a ésta vía un reader del brazo.
///
/// El wrapper común agrupa lo que TODOS los formatos comparten
/// (identidad legible + extensiones forward-compat); el body preserva
/// el typing rico de cada dominio sin colapsarlos.
// PartialEq se omite porque algunos body variants vienen de crates
// que no lo implementan (MonadManifest, nahual_meta_schema::Module).
// Si downstream necesita igualdad, comparar via JSON round-trip o
// agregar PartialEq en los crates origen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    /// Identificador opaco. String en el wrapper para no obligar a
    /// los formatos a un mismo tipo concreto (Ente/Monad usan ULID,
    /// UiModule usa slug human-friendly como `"sales_engine"`).
    /// Cada reader documenta qué formato exige.
    pub id: String,

    /// Versión del schema canónico de este wrapper. Bump = romper
    /// compat de los consumers downstream. Distinto de los
    /// `schema_version` internos de cada body variant, que siguen
    /// su propio versioning.
    pub schema_version: u16,

    /// Ancestro del que esta Card desciende (si aplica). Significado
    /// específico al body variant (Ente: lineage del proceso; Monad:
    /// split/merge de Mónada padre; UiModule: típicamente None).
    #[serde(default)]
    pub lineage: Option<String>,

    /// Etiqueta humana legible. Cada reader la deriva del campo
    /// equivalente del input (label/title/etc.).
    pub label: String,

    /// Campos no reconocidos del input se preservan acá. Permite
    /// forward-compat: leer un input con campos nuevos no rompe la
    /// carga, y volver a serializar conserva el extra.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,

    /// Cuerpo tipado por dominio. La elección del variant es
    /// responsabilidad del reader (basada en el input shape).
    pub body: CardBody,
}

/// Versión actual del schema canónico de [`Card`]. Bump cuando cambie
/// la shape del wrapper o las invariantes que comparten todos los
/// variants.
pub const CARD_SCHEMA_VERSION: u16 = 1;

/// Variantes tipadas del body de [`Card`]. Una por dominio.
///
/// **Convención de extensión**: agregar un variant nuevo aquí + un
/// reader que produzca ese variant. Los consumers que sólo manejen
/// algunos variants pueden hacer `match { Ente(..) => ..., _ => skip }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CardBody {
    /// Entidad runtime con proceso/payload/supervision (lo que era
    /// `brahman_card::Card` directo).
    Ente(EnteCard),

    /// Agrupación semántica de archivos (Mónada de Nouser). No tiene
    /// proceso; describe membership + signals semánticas (centroid,
    /// keywords, lens).
    Monad(MonadManifest),

    /// Descriptor de módulo de UI: entities + views + menu + actions.
    /// Lo que hoy lee la metainterface de Nakui desde
    /// `examples/nakui-modules/<id>/module.json`.
    UiModule(UiModuleSpec),
}

impl CardBody {
    /// Etiqueta corta del variant — útil para mensajes de error y
    /// dispatch en la UI sin necesitar match exhaustivo.
    pub fn kind_name(&self) -> &'static str {
        match self {
            CardBody::Ente(_) => "ente",
            CardBody::Monad(_) => "monad",
            CardBody::UiModule(_) => "ui_module",
        }
    }
}

/// Errores de carga del brazo.
#[derive(Debug, Error)]
pub enum CardLoadError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse JSON: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("ningún reader registrado matcheó el input (shape no reconocido)")]
    NoMatchingReader,

    #[error("reader '{reader}' falló: {message}")]
    ReaderFailed { reader: &'static str, message: String },

    #[error("formato no soportado: extensión '{ext}'. Soportadas: {supported:?}")]
    UnsupportedExtension {
        ext: String,
        supported: Vec<&'static str>,
    },

    #[error("evaluación Nickel: {0}")]
    Nickel(#[from] NickelEvalError),
}

/// Trait de reader. Cada formato implementa una instancia.
///
/// El dispatcher del brazo (`load_card`) prueba los readers en el
/// orden registrado y se queda con el primero cuyo `can_read`
/// devuelve `true`. Por eso el orden importa: poner los más
/// específicos antes que los más laxos.
pub trait CardReader: Send + Sync {
    /// Nombre del reader, para mensajes de error.
    fn name(&self) -> &'static str;

    /// Dado un JSON Value (el input ya parseado a serde Value),
    /// decide si este reader puede manejarlo. Heurística estructural
    /// — el shape del input identifica el formato, no flags
    /// explícitos (los inputs legacy no los tienen).
    fn can_read(&self, input: &Value) -> bool;

    /// Produce el [`Card`] canónico. Sólo se llama si `can_read`
    /// devolvió `true`.
    fn read(&self, input: Value) -> Result<Card, CardLoadError>;
}

mod nickel_eval;
mod readers;
pub use nickel_eval::{eval_nickel_file, NickelEvalError, BRAHMAN_CARDS_TEMPLATES_ENV};
pub use readers::{EnteJsonReader, MonadJsonReader, UiModuleJsonReader};

/// Path al directorio de templates Nickel canónicos shipped con el
/// crate (`crates/core/brahman-cards/templates/` en el repo).
///
/// Este directorio contiene los `*_basic.ncl` para cada body kind:
/// - `ente_basic.ncl`
/// - `monad_basic.ncl`
/// - `ui_module_basic.ncl`
///
/// Usar como path para [`BRAHMAN_CARDS_TEMPLATES_ENV`] o pasarlo
/// directo a Nickel via env. Resuelto via `CARGO_MANIFEST_DIR` —
/// funciona en `cargo test`/`cargo run` desde el workspace. Para
/// distribución del binary standalone (cuando emerja el caso de
/// uso), incluir los templates como recursos via `include_dir!` o
/// instalar el directorio junto al ejecutable.
pub fn canonical_templates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates")
}

/// Construye el set default de readers para inputs JSON. El orden
/// es deliberado: el más específico (UiModule, que tiene `entities`
/// y `views` simultáneamente) antes que el más laxo. Si dos readers
/// matchean, gana el primero.
pub fn default_readers() -> Vec<Box<dyn CardReader>> {
    vec![
        Box::new(UiModuleJsonReader),
        Box::new(MonadJsonReader),
        Box::new(EnteJsonReader),
    ]
}

/// Carga un Card desde una ruta. Detecta formato por extensión, y
/// dentro de JSON detecta el shape probando los readers default en
/// orden.
///
/// Para custom reader sets, usar [`load_card_with`].
pub fn load_card(path: impl AsRef<Path>) -> Result<Card, CardLoadError> {
    load_card_with(path, &default_readers())
}

/// Variante de [`load_card`] con readers custom. Útil para tests o
/// para apps que quieren restringir formatos soportados.
pub fn load_card_with(
    path: impl AsRef<Path>,
    readers: &[Box<dyn CardReader>],
) -> Result<Card, CardLoadError> {
    let path = path.as_ref();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "json" => {
            let bytes = std::fs::read(path)?;
            let value: Value = serde_json::from_slice(&bytes)?;
            dispatch_to_reader(value, readers)
        }
        "ncl" => {
            // Nickel pipeline: leer archivo → evaluar deeply → exportar
            // a JSON → parsear como Value → dispatch a los readers JSON
            // estándar. Templates funcionan via los `import` nativos de
            // Nickel; el evaluator resuelve relativo al input y al
            // `BRAHMAN_CARDS_TEMPLATES_DIR` env (si está set).
            let value = eval_nickel_file(path)?;
            dispatch_to_reader(value, readers)
        }
        other => Err(CardLoadError::UnsupportedExtension {
            ext: other.to_string(),
            supported: vec!["json", "ncl"],
        }),
    }
}

/// Recorre los readers en orden, se queda con el primero que matchea
/// y delega la conversión.
fn dispatch_to_reader(
    input: Value,
    readers: &[Box<dyn CardReader>],
) -> Result<Card, CardLoadError> {
    for r in readers {
        if r.can_read(&input) {
            return r.read(input);
        }
    }
    Err(CardLoadError::NoMatchingReader)
}

/// Filenames convencionales que [`load_cards_from_dir`] busca dentro
/// de cada subdir, en orden de preferencia. Si `card.ncl` existe se
/// usa ese; sino `card.json`; sino los aliases legacy `module.*`. Los
/// últimos dos son por compat con el layout actual de
/// `examples/nakui-modules/<id>/module.json`.
pub const DEFAULT_CARD_FILENAMES: &[&str] =
    &["card.ncl", "card.json", "module.ncl", "module.json"];

/// Carga todas las Cards encontradas como subdirs inmediatos de
/// `dir`. Por cada subdir, busca los filenames convencionales (ver
/// [`DEFAULT_CARD_FILENAMES`]) y carga el primero que existe. Subdirs
/// sin ningún filename matching se skipean silenciosamente — permite
/// que un dir contenga subdirs auxiliares (assets, fixtures, etc.).
///
/// Devuelve las Cards en orden lexicográfico por subdir name (estable
/// across runs). NO ordena por `Card.id` — el caller decide si quiere
/// re-ordenar y/o dedupar.
///
/// Errores: cualquier I/O al leer el `dir` mismo, o cualquier
/// `CardLoadError` de un archivo encontrado (NO continúa tras el
/// primer fallo — fallo loud > corrupción silenciosa).
pub fn load_cards_from_dir(dir: impl AsRef<Path>) -> Result<Vec<Card>, CardLoadError> {
    load_cards_from_dir_with(dir, DEFAULT_CARD_FILENAMES, &default_readers())
}

/// Variante de [`load_cards_from_dir`] con filenames y readers
/// custom. Útil para apps que quieren restringir formatos o usar un
/// nombre canónico distinto.
pub fn load_cards_from_dir_with(
    dir: impl AsRef<Path>,
    filenames: &[&str],
    readers: &[Box<dyn CardReader>],
) -> Result<Vec<Card>, CardLoadError> {
    let dir = dir.as_ref();
    let mut subdir_paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.is_dir() { Some(p) } else { None }
        })
        .collect();
    // Orden estable por subdir name — el output del brazo no debería
    // depender del orden de read_dir (que varía por filesystem).
    subdir_paths.sort();

    let mut out: Vec<Card> = Vec::new();
    for sub in subdir_paths {
        for fname in filenames {
            let candidate = sub.join(fname);
            if candidate.exists() {
                out.push(load_card_with(&candidate, readers)?);
                break;
            }
        }
    }
    Ok(out)
}
