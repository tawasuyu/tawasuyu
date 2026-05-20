//! `chasqui-card` — manifiesto de Mónada.
//!
//! Una **Mónada** es una agrupación semántica de archivos: el archivo
//! físico no se mueve, pero su pertenencia se modela por un objeto
//! ([`MonadManifest`]) con identidad propia, métricas y un "lente" de
//! visualización. La idea hereda el espíritu de la Tarjeta de
//! Presentación de Brahman (`brahman-card::Card`): un manifiesto
//! tipado, validado y serializable que define qué es la entidad y
//! cómo el sistema debe interactuar con ella.
//!
//! Diferencia con `brahman-card::Card`:
//!
//! | brahman::Card                       | chasqui::MonadManifest         |
//! |-------------------------------------|-------------------------------|
//! | Describe una **entidad runtime**    | Describe una **agrupación**   |
//! | Tiene `payload`/`soma`/`supervision`| No tiene proceso detrás       |
//! | Vive durante una sesión             | Vive en una DB persistente    |
//! | Fluye por handshake/postcard        | Fluye por queries del backend |
//!
//! Este crate sólo define los tipos. La lógica de scan, cluster,
//! attraction vive en `chasqui-core`.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ulid::Ulid;

// Re-export para consumidores
pub use ::ulid;

pub mod query;

/// Versión del esquema del manifiesto. Bump al cambiar el schema.
pub const MONAD_SCHEMA_VERSION: u16 = 1;

/// Identificador opaco de un archivo registrado en la DB.
pub type FileId = Ulid;

/// Identificador opaco de una Mónada.
pub type MonadId = Ulid;

// =====================================================================
// FileEntry — el archivo como dato indexado
// =====================================================================

/// Registro físico de un archivo en la DB. Es la unidad atómica que
/// pertenece a (potencialmente varias) Mónadas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: FileId,
    pub path: PathBuf,
    /// Hash de contenido (blake3) — sólo se computa si el archivo es
    /// chico o el usuario lo pidió. `None` por default en Phase 0.
    #[serde(default)]
    pub content_hash: Option<[u8; 32]>,
    /// Tamaño en bytes.
    pub size: u64,
    /// `mtime` como ms desde UNIX_EPOCH.
    pub mtime_ms: u64,
    /// Extensión normalizada en lowercase, sin punto. `None` si no tiene.
    #[serde(default)]
    pub extension: Option<String>,
}

// =====================================================================
// Lens — la "vista" preferida de una Mónada
// =====================================================================

/// Lente de visualización dominante. La UI (nahual) elige cómo renderizar
/// los miembros de una Mónada según este hint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lens {
    /// Grid genérico: thumbnail + nombre + meta.
    #[default]
    Grid,
    /// Editor de código con highlighting (rs, py, ts, ...).
    Code,
    /// Galería de imágenes (png, jpg, svg, ...).
    Gallery,
    /// Vista tabular (csv, sqlite, ...).
    Database,
    /// Texto renderizado (md, rst, txt).
    Markdown,
    /// Árbol jerárquico (cuando la Mónada es estructural).
    Tree,
}

// =====================================================================
// MonadManifest — la Tarjeta de Presentación de la Mónada
// =====================================================================

/// Manifiesto de una Mónada. Equivalente conceptual a la Tarjeta de
/// Presentación de Brahman, pero para una agrupación de datos.
///
/// Se serializa a JSON/TOML para persistencia y debugging; es el
/// "ADN" que la UI lee para saber cómo presentar la Mónada sin tocar
/// el disco.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonadManifest {
    /// Versión del esquema. Bump = romper compatibilidad de DB.
    pub schema_version: u16,

    /// Identificador opaco. ULID — orderable por tiempo de creación.
    pub id: MonadId,

    /// Mónada de la que ésta fue derivada (split, merge), si aplica.
    #[serde(default)]
    pub lineage: Option<MonadId>,

    /// Nombre humano corto (1-4 palabras, generado por reglas o por Nous).
    pub label: String,

    /// Resumen de propósito (1-2 oraciones). Generado por Nous cuando
    /// la masa de la Mónada justifica la consulta.
    #[serde(default)]
    pub summary: String,

    /// Centroide vectorial (embedding promedio de los miembros). Vacío
    /// en Phase 0 (sin embeddings); se llena cuando entran las
    /// pseudo-embeddings o el modelo real.
    #[serde(default)]
    pub centroid: Vec<f32>,

    /// Identificador del modelo que produjo `centroid`. Si está set, los
    /// consumidores deben verificar coincidencia antes de comparar vía
    /// cosine similarity con embeddings recientes; al cambiar de modelo
    /// (mock-pseudo-32d → real-fastembed-384d, etc.) los centroides
    /// previos quedan inválidos por dimensión y semántica.
    /// `None` = legacy (centroides sin tag, pre-versioning).
    #[serde(default)]
    pub centroid_model: Option<String>,

    /// Identidad estable derivada del origen de los miembros. Para
    /// Mónadas creadas por `cluster::by_directory`, es el path
    /// canónico del directorio padre. Permite que la hidratación
    /// reuse el mismo ULID across re-scans (mismo path_hint = misma
    /// identidad, aunque cambien los miembros internamente).
    /// `None` para Mónadas creadas por estrategias que no se anclan a
    /// un origen físico.
    #[serde(default)]
    pub path_hint: Option<String>,

    /// Tokens dominantes: extensiones, palabras clave, etc.
    /// 5-10 elementos típicamente.
    #[serde(default)]
    pub keywords: Vec<String>,

    /// Cantidad de miembros (== `members.len()`). Cacheado para evitar
    /// el cost de leer la lista cada vez.
    pub cardinality: u32,

    /// Métrica de dispersión interna [0.0, 1.0]:
    /// - 0.0: todos los miembros son muy similares (Mónada coherente).
    /// - 1.0: miembros muy heterogéneos (sugerencia: bifurcar).
    ///
    /// Calculada como entropía de Shannon normalizada sobre las
    /// extensiones de los miembros.
    #[serde(default)]
    pub entropy: f32,

    /// Lente preferido para visualización en la UI.
    #[serde(default)]
    pub dominant_lens: Lens,

    /// Archivos anclados manualmente: NO se mueven en re-clustering
    /// automático. El usuario "fija" estos miembros.
    #[serde(default)]
    pub pins: BTreeSet<FileId>,

    /// IDs de archivos miembros (incluye pins).
    pub members: BTreeSet<FileId>,

    /// Unix ms de creación de la Mónada.
    pub created_at_ms: u64,

    /// Unix ms de la última actualización (re-cluster, re-name, ...).
    pub updated_at_ms: u64,

    /// Forward-compat: campos JSON desconocidos preservados.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

// =====================================================================
// Errores y validación
// =====================================================================

#[derive(Debug, Error)]
pub enum MonadError {
    #[error("schema mismatch: got {got}, expected {expected}")]
    SchemaMismatch { got: u16, expected: u16 },
    #[error("label vacío")]
    EmptyLabel,
    #[error("label demasiado largo: {0} bytes (max 256)")]
    LabelTooLong(usize),
    #[error("entropía fuera de [0,1]: {0}")]
    InvalidEntropy(f32),
    #[error("Monad sin miembros y sin pins")]
    Empty,
    #[error("cardinalidad declarada {declared} ≠ members.len() {actual}")]
    CardinalityMismatch { declared: u32, actual: u32 },
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
}

impl MonadManifest {
    /// Constructor con defaults razonables. `id` y timestamps se
    /// generan; resto vacío.
    pub fn new(label: impl Into<String>) -> Self {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            schema_version: MONAD_SCHEMA_VERSION,
            id: Ulid::new(),
            lineage: None,
            label: label.into(),
            summary: String::new(),
            centroid: Vec::new(),
            centroid_model: None,
            path_hint: None,
            keywords: Vec::new(),
            cardinality: 0,
            entropy: 0.0,
            dominant_lens: Lens::default(),
            pins: BTreeSet::new(),
            members: BTreeSet::new(),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            extensions: BTreeMap::new(),
        }
    }

    /// Validación semántica.
    pub fn validate(&self) -> Result<(), MonadError> {
        if self.schema_version != MONAD_SCHEMA_VERSION {
            return Err(MonadError::SchemaMismatch {
                got: self.schema_version,
                expected: MONAD_SCHEMA_VERSION,
            });
        }
        if self.label.trim().is_empty() {
            return Err(MonadError::EmptyLabel);
        }
        if self.label.len() > 256 {
            return Err(MonadError::LabelTooLong(self.label.len()));
        }
        if !(0.0..=1.0).contains(&self.entropy) {
            return Err(MonadError::InvalidEntropy(self.entropy));
        }
        if self.members.is_empty() && self.pins.is_empty() {
            return Err(MonadError::Empty);
        }
        let actual = self.members.len() as u32;
        if self.cardinality != actual {
            return Err(MonadError::CardinalityMismatch {
                declared: self.cardinality,
                actual,
            });
        }
        Ok(())
    }

    /// Serializa a JSON pretty.
    pub fn to_json_pretty(&self) -> Result<String, MonadError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Deserializa desde JSON y valida.
    pub fn from_json(src: &str) -> Result<Self, MonadError> {
        let m: Self = serde_json::from_str(src)?;
        m.validate()?;
        Ok(m)
    }

    /// Recalcula `cardinality` y `updated_at_ms` desde `members`.
    /// Usar tras mutaciones del set de miembros.
    pub fn touch(&mut self) {
        self.cardinality = self.members.len() as u32;
        self.updated_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
    }

    /// Proyecta el `MonadManifest` a la `brahman_card::Card` que viaja
    /// por el protocolo. La Card resultante:
    ///
    /// - hereda `id` y `label` del manifiesto (ULID estable).
    /// - `kind = CardKind::Data` (se distingue de un Ente).
    /// - `payload = Virtual`, `supervision = Delegate`,
    ///   `lifecycle = Daemon` — placeholder semántico: la Mónada no se
    ///   "ejecuta", el daemon dueño la mantiene viva.
    /// - `data = Some(DataFacet { ... })` con summary, keywords,
    ///   centroide, member_count, dispersión y un hint de presentación
    ///   derivado del `dominant_lens`.
    /// - Los miembros completos NO viajan en la Card — se consultan al
    ///   daemon dueño bajo demanda. Lo que viaja es metadata liviana
    ///   apta para el wire postcard.
    pub fn to_brahman_card(&self) -> brahman_card::Card {
        use brahman_card::{
            Card, CardKind, DataFacet, Lifecycle, Payload, Priority, Supervision,
        };

        let presentation_hint = match self.dominant_lens {
            Lens::Grid => "grid",
            Lens::Code => "code",
            Lens::Gallery => "gallery",
            Lens::Database => "database",
            Lens::Markdown => "markdown",
            Lens::Tree => "tree",
        }
        .to_string();

        Card {
            schema_version: brahman_card::CARD_SCHEMA_VERSION,
            id: self.id,
            label: self.label.clone(),
            payload: Payload::Virtual,
            supervision: Supervision::Delegate,
            lifecycle: Lifecycle::Daemon,
            priority: Priority::Normal,
            kind: CardKind::Data,
            data: Some(DataFacet {
                summary: self.summary.clone(),
                keywords: self.keywords.clone(),
                centroid: self.centroid.clone(),
                member_count: self.cardinality,
                dispersion: self.entropy,
                presentation_hint,
            }),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_minimal() {
        let mut m = MonadManifest::new("test");
        m.members.insert(Ulid::new());
        m.touch();
        m.validate().expect("debe validar");
    }

    #[test]
    fn empty_label_rejected() {
        let mut m = MonadManifest::new("x");
        m.label = String::new();
        m.members.insert(Ulid::new());
        m.touch();
        assert!(matches!(m.validate(), Err(MonadError::EmptyLabel)));
    }

    #[test]
    fn entropy_out_of_range_rejected() {
        let mut m = MonadManifest::new("x");
        m.members.insert(Ulid::new());
        m.entropy = 1.5;
        m.touch();
        assert!(matches!(m.validate(), Err(MonadError::InvalidEntropy(_))));
    }

    #[test]
    fn empty_members_rejected() {
        let m = MonadManifest::new("x");
        assert!(matches!(m.validate(), Err(MonadError::Empty)));
    }

    #[test]
    fn cardinality_mismatch_caught() {
        let mut m = MonadManifest::new("x");
        m.members.insert(Ulid::new());
        // No llamamos touch — cardinality queda en 0 con 1 miembro.
        assert!(matches!(
            m.validate(),
            Err(MonadError::CardinalityMismatch { .. })
        ));
    }

    #[test]
    fn projects_to_brahman_card() {
        let mut m = MonadManifest::new("test-monad");
        m.summary = "monad de prueba".into();
        m.keywords = vec!["rs".into(), "toml".into()];
        m.dominant_lens = Lens::Code;
        m.entropy = 0.42;
        m.members.insert(Ulid::new());
        m.members.insert(Ulid::new());
        m.members.insert(Ulid::new());
        m.touch();

        let bc = m.to_brahman_card();
        assert_eq!(bc.id, m.id);
        assert_eq!(bc.label, "test-monad");
        assert_eq!(bc.kind, brahman_card::CardKind::Data);
        let data = bc.data.expect("data facet presente");
        assert_eq!(data.summary, "monad de prueba");
        assert_eq!(data.keywords, vec!["rs".to_string(), "toml".to_string()]);
        assert_eq!(data.member_count, 3);
        assert!((data.dispersion - 0.42).abs() < 1e-6);
        assert_eq!(data.presentation_hint, "code");
    }

    #[test]
    fn json_roundtrip() {
        let mut m = MonadManifest::new("test-monad");
        m.members.insert(Ulid::new());
        m.members.insert(Ulid::new());
        m.keywords = vec!["rs".into(), "toml".into()];
        m.summary = "test summary".into();
        m.dominant_lens = Lens::Code;
        m.touch();
        let s = m.to_json_pretty().unwrap();
        let m2 = MonadManifest::from_json(&s).unwrap();
        assert_eq!(m2.label, m.label);
        assert_eq!(m2.cardinality, 2);
        assert_eq!(m2.dominant_lens, Lens::Code);
        assert_eq!(m2.keywords, m.keywords);
    }
}
