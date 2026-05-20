//! Errores del orquestador.

use ulid::Ulid;

/// Falla de una operación del `Engine`. Las impls concretas mapean sus
/// errores internos (encarnación, IPC, SSH) a estas variantes.
#[derive(Debug, thiserror::Error)]
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
}
