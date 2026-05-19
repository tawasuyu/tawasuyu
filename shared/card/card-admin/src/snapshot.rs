//! Tipos del snapshot que el admin server emite.

use brahman_broker::{BrokeredCard, Match};
use serde::{Deserialize, Serialize};

/// Snapshot completo del estado del Init en un instante.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusSnapshot {
    /// Versión del crate del Init que respondió.
    pub server_version: String,
    /// Versión del protocolo brahman.
    pub protocol_version: String,
    /// `true` si el Init está atado al servidor.
    pub init_attached: bool,
    /// Contexto operativo activo del broker (p. ej. `"test"`, `"prod"`).
    /// `None` si no hay contexto configurado — los biases per-contexto
    /// declarados en las Cards quedan inactivos.
    #[serde(default)]
    pub current_context: Option<String>,
    /// Cards actualmente registradas (sesiones vivas).
    pub sessions: Vec<BrokeredCard>,
    /// Matches consumer↔producer derivados del set actual.
    pub matches: Vec<Match>,
}
