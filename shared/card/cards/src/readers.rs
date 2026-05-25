//! Readers V1: tres formatos JSON ya existentes en el monorepo.
//!
//! Cada reader implementa:
//! - `can_read`: heurística estructural para decidir si el JSON es
//!   suyo. No requiere flag explícito en el input — los inputs legacy
//!   no los tienen.
//! - `read`: deserializa el JSON al tipo del crate origen (sin tocarlo)
//!   y lo envuelve en [`Card`] derivando los campos del wrapper.
//!
//! Convenciones para derivar el wrapper:
//! - `id`: del campo `id` del input (cada formato lo expone). Si es
//!   ULID se serializa a string canónico.
//! - `label`: del campo `label`.
//! - `lineage`: del campo `lineage` cuando existe (Ente/Monad).
//! - `extensions`: campos JSON desconocidos respecto a la struct del
//!   crate origen. Hoy lo mantenemos vacío (los crates origen ya
//!   tienen sus propios `extensions` internos via `#[serde(flatten)]`)
//!   — no duplicamos. Si en el futuro queremos mover el "extras" del
//!   crate origen al wrapper, esta es la palanca.

use serde_json::Value;

use crate::{Card, CardBody, CardLoadError, CardReader, EnteCard, MonadManifest, UiModuleSpec, CARD_SCHEMA_VERSION};

// ============================================================================
// Ente (brahman-card)
// ============================================================================

/// Reader para el shape JSON de [`brahman_card::Card`].
///
/// Heurística de detección: el input tiene `payload` Y `supervision`
/// — son los campos requeridos del schema Ente que ningún otro
/// formato del monorepo tiene.
pub struct EnteJsonReader;

impl CardReader for EnteJsonReader {
    fn name(&self) -> &'static str {
        "ente-json"
    }

    fn can_read(&self, input: &Value) -> bool {
        let obj = match input.as_object() {
            Some(o) => o,
            None => return false,
        };
        obj.contains_key("payload") && obj.contains_key("supervision")
    }

    fn read(&self, input: Value) -> Result<Card, CardLoadError> {
        let id = pull_string(&input, "id").unwrap_or_default();
        let label = pull_string(&input, "label").unwrap_or_default();
        let lineage = pull_string(&input, "lineage");

        let ente: EnteCard =
            serde_json::from_value(input).map_err(|e| CardLoadError::ReaderFailed {
                reader: "ente-json",
                message: e.to_string(),
            })?;

        Ok(Card {
            id,
            schema_version: CARD_SCHEMA_VERSION,
            lineage,
            label,
            extensions: Default::default(),
            body: CardBody::Ente(ente),
        })
    }
}

// ============================================================================
// Monad (chasqui-card)
// ============================================================================

/// Reader para el shape JSON de [`chasqui_card::MonadManifest`].
///
/// Heurística: tiene `members` (BTreeSet<FileId>) Y `cardinality`
/// (u32). La combinación es exclusiva del MonadManifest.
pub struct MonadJsonReader;

impl CardReader for MonadJsonReader {
    fn name(&self) -> &'static str {
        "monad-json"
    }

    fn can_read(&self, input: &Value) -> bool {
        let obj = match input.as_object() {
            Some(o) => o,
            None => return false,
        };
        obj.contains_key("members") && obj.contains_key("cardinality")
    }

    fn read(&self, input: Value) -> Result<Card, CardLoadError> {
        let id = pull_string(&input, "id").unwrap_or_default();
        let label = pull_string(&input, "label").unwrap_or_default();
        let lineage = pull_string(&input, "lineage");

        let monad: MonadManifest =
            serde_json::from_value(input).map_err(|e| CardLoadError::ReaderFailed {
                reader: "monad-json",
                message: e.to_string(),
            })?;

        Ok(Card {
            id,
            schema_version: CARD_SCHEMA_VERSION,
            lineage,
            label,
            extensions: Default::default(),
            body: CardBody::Monad(monad),
        })
    }
}

// ============================================================================
// UiModule (nahual-meta-schema)
// ============================================================================

/// Reader para el shape JSON de los `module.json` de la metainterfaz
/// ([`nahual_meta_schema::Module`]).
///
/// Heurística: tiene `entities` Y `views` Y `menu`. Es el shape más
/// específico del repo, así que va primero en el orden default — si
/// matchea, ningún otro reader debería intentar.
pub struct UiModuleJsonReader;

impl CardReader for UiModuleJsonReader {
    fn name(&self) -> &'static str {
        "ui-module-json"
    }

    fn can_read(&self, input: &Value) -> bool {
        let obj = match input.as_object() {
            Some(o) => o,
            None => return false,
        };
        obj.contains_key("entities") && obj.contains_key("views") && obj.contains_key("menu")
    }

    fn read(&self, input: Value) -> Result<Card, CardLoadError> {
        let id = pull_string(&input, "id").unwrap_or_default();
        let label = pull_string(&input, "label").unwrap_or_default();
        // UiModule no tiene lineage en su schema, queda None.
        let module: UiModuleSpec =
            serde_json::from_value(input).map_err(|e| CardLoadError::ReaderFailed {
                reader: "ui-module-json",
                message: e.to_string(),
            })?;

        Ok(Card {
            id,
            schema_version: CARD_SCHEMA_VERSION,
            lineage: None,
            label,
            extensions: Default::default(),
            body: CardBody::UiModule(module),
        })
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn pull_string(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(|s| s.to_string())
}
