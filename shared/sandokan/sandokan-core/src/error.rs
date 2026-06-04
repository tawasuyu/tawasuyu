//! Errores del orquestador.

use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Falla de una operación del `Engine`. Las impls concretas mapean sus
/// errores internos (encarnación, IPC, SSH) a estas variantes.
///
/// Es `Serialize`/`Deserialize` porque viaja por el wire del
/// `DaemonEngine` (postcard sobre Unix socket).
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
pub enum EngineError {
    /// No existe ninguna entidad activa con ese `card_id`.
    #[error("card `{0}` no encontrada")]
    NotFound(Ulid),

    /// La encarnación falló (clone/namespaces/exec).
    #[error("encarnación falló: {0}")]
    Incarnate(String),

    /// Falla de transporte (Unix socket del daemon, túnel SSH).
    #[error("transporte: {0}")]
    Transport(String),

    /// La intención es inconsistente (Card inválida, contexto imposible).
    #[error("intención inválida: {0}")]
    InvalidIntent(String),

    /// La operación excedió su deadline.
    #[error("timeout")]
    Timeout,

    /// El `Engine` no sabe encarnar este tipo de `Card::Payload`. Hoy
    /// `LocalEngine` cubre `Native`, `Legacy` y `Virtual`; `Wasm` se
    /// delega a un futuro `ente-wasm` y por ahora rebota como esta
    /// variante para que el caller distinga "no soportado" de "fallé".
    #[error("payload no soportado por este Engine: {kind}")]
    UnsupportedPayload { kind: String },
}
