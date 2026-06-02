//! Contrato de sesiones interactivas (PTY) del plano de control.
//!
//! Extiende [`Engine`] con la capacidad de encarnar una Card **interactiva**
//! (atada a un PTY). El attach NO viaja por este trait: el front se conecta
//! **out-of-band** al socket canónico de la sesión ([`InteractiveEngine::
//! session_socket_path`], `<run_dir>/<card_id>.sock`), sin asumir quién
//! atiende detrás — hoy el engine in-process, mañana un holder por sesión.

use crate::{Engine, EngineError, ExecHandle, Intent};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ulid::Ulid;

/// Tamaño del PTY en celdas. Tipo del contrato (viaja por el wire del
/// `DaemonEngine` en `RunInteractive`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

/// Un `Engine` capaz de sesiones interactivas. Lo cumplen `LocalEngine`
/// (in-process) y `DaemonEngine` (reenvía por el wire). El daemon de sandokan
/// sirve engines de este tipo: es el plano interactivo de la suite.
#[async_trait]
pub trait InteractiveEngine: Engine {
    /// Encarna una Card interactiva (aislada como `run`, pero con stdio en un
    /// PTH que el engine retiene). Aparece en `list`/`status`/`stop` como
    /// cualquier entidad. El cliente attacha luego por [`Self::session_socket_path`].
    async fn run_interactive(
        &self,
        intent: Intent,
        size: PtySize,
    ) -> Result<ExecHandle, EngineError>;

    /// Path del socket canónico de la sesión: el front se conecta **siempre**
    /// acá por `card_id` para recibir scrollback + stream vivo y mandar input.
    fn session_socket_path(&self, card_id: Ulid) -> PathBuf;
}
