//! sandokan — el orquestador del ecosistema brahman (umbrella).
//!
//! `sandokan` es una **library horizontal embebible**, no un daemon
//! supremo. Cualquier binario lo embebe y elige cómo correrlo:
//!
//! - [`LocalEngine`] — orquesta in-process (encarna Cards localmente).
//! - [`DaemonEngine`] — delega a otro proceso vía Unix socket.
//! - `RemoteEngine` — delega a otro host vía SSH (crate `sandokan-remote`).
//! - [`ArjeEngine`] — habla con el init de sistema (arje-zero) por `arje-bus`.
//!
//! [`auto`] elige por **precedencia del SDD** (`shared/sandokan/SDD.md`):
//! primero el **Engine de sistema** (arje-zero sobre `arje-bus`, si hay init
//! escuchando), luego un **daemon** sandokan, y por último el **`LocalEngine`**
//! in-process. Un cliente embebe `auto()` y habla con el dueño del control de
//! ese entorno, siempre por el mismo contrato.

pub use sandokan_arje_engine::ArjeEngine;
pub use sandokan_core::{
    Engine, EngineError, ExecContext, ExecHandle, Intent, IsolationLevel,
    LifecycleEvent, TelemetryFrame,
};
pub use sandokan_daemon::{serve, DaemonEngine, DaemonRequest, DaemonResponse};
pub use sandokan_local::LocalEngine;
pub use sandokan_remote::RemoteEngine;

/// Re-export de las primitivas de lifecycle.
pub use sandokan_lifecycle as lifecycle;

use std::path::{Path, PathBuf};

/// Path por defecto del socket del daemon sandokan.
///
/// `$XDG_RUNTIME_DIR/sandokan.sock` si la variable está; si no,
/// `/run/brahman/sandokan.sock`.
pub fn default_socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(rt) => PathBuf::from(rt).join("sandokan.sock"),
        None => PathBuf::from("/run/brahman/sandokan.sock"),
    }
}

/// Elige el engine por precedencia del SDD:
/// 1. **Engine de sistema**: arje-zero sobre `arje-bus` (`$ENTE_BUS_SOCK`), si
///    hay un init escuchando. Es el dueño del control en un host arje.
/// 2. **Daemon** sandokan escuchando en `socket_path`.
/// 3. **`LocalEngine`** in-process (no hay nadie más; orquesto yo).
pub async fn auto(socket_path: &Path) -> Box<dyn Engine> {
    // 1. El init de sistema, si el bus está alcanzable.
    if let Ok(arje) = ArjeEngine::from_env() {
        if arje.is_reachable().await {
            return Box::new(arje);
        }
    }
    // 2. Un daemon sandokan.
    let daemon = DaemonEngine::new(socket_path);
    if daemon.is_reachable().await {
        return Box::new(daemon);
    }
    // 3. In-process.
    Box::new(LocalEngine::new())
}

/// [`auto`] con [`default_socket_path`].
pub async fn auto_default() -> Box<dyn Engine> {
    auto(&default_socket_path()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn auto_falls_back_to_local_without_daemon() {
        // Socket inexistente → debe caer a LocalEngine (list() vacío).
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("nope.sock");
        let engine = auto(&sock).await;
        assert!(engine.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn auto_picks_daemon_when_listening() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("sandokan.sock");

        let served = Arc::new(LocalEngine::new());
        let sock_srv = sock.clone();
        let srv = tokio::spawn(async move {
            let _ = serve(served, &sock_srv).await;
        });
        for _ in 0..50 {
            if sock.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Con el daemon escuchando, auto() debe conectar y operar vía wire.
        let engine = auto(&sock).await;
        assert!(engine.list().await.unwrap().is_empty());

        srv.abort();
    }

    #[test]
    fn default_socket_path_is_absolute() {
        assert!(default_socket_path().is_absolute());
    }
}
