//! sandokan — el orquestador del ecosistema brahman (umbrella).
//!
//! `sandokan` es una **library horizontal embebible**, no un daemon
//! supremo. Cualquier binario lo embebe y elige cómo correrlo:
//!
//! - [`LocalEngine`] — orquesta in-process (encarna Cards localmente).
//! - [`DaemonEngine`] — delega a otro proceso vía Unix socket.
//! - `RemoteEngine` — delega a otro host vía SSH (crate `sandokan-remote`).
//!
//! [`auto`] implementa el patrón "el primero que arranca gana": prueba
//! si hay un daemon escuchando y, si lo hay, se le suma como
//! `DaemonEngine`; si no, corre su propio `LocalEngine`.

pub use sandokan_core::{
    Engine, EngineError, ExecContext, ExecHandle, Intent, IsolationLevel,
    LifecycleEvent, TelemetryFrame,
};
pub use sandokan_daemon::{serve, DaemonEngine, DaemonRequest, DaemonResponse};
pub use sandokan_local::LocalEngine;

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

/// Elige el engine según el entorno: si hay un daemon escuchando en
/// `socket_path`, devuelve un [`DaemonEngine`] (delega); si no, un
/// [`LocalEngine`] (orquesta in-process).
pub async fn auto(socket_path: &Path) -> Box<dyn Engine> {
    let daemon = DaemonEngine::new(socket_path);
    if daemon.is_reachable().await {
        Box::new(daemon)
    } else {
        Box::new(LocalEngine::new())
    }
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
