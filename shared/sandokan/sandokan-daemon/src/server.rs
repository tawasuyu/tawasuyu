//! Loop servidor: envuelve cualquier `Engine` y lo expone por Unix socket.

use crate::protocol::{read_frame, write_frame, DaemonRequest, DaemonResponse};
use sandokan_core::Engine;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};

/// Sirve `engine` en `socket_path` hasta que el future se cancele.
///
/// Si el socket ya existe (daemon previo que no limpió), se borra antes
/// de bind. Cada conexión se atiende en su propia task; una conexión
/// puede mandar múltiples requests secuenciales.
pub async fn serve<E>(engine: Arc<E>, socket_path: &Path) -> std::io::Result<()>
where
    E: Engine + 'static,
{
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
    let listener = UnixListener::bind(socket_path)?;
    tracing::info!(socket = %socket_path.display(), "sandokan-daemon escuchando");

    loop {
        let (stream, _addr) = listener.accept().await?;
        let engine = Arc::clone(&engine);
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, engine).await {
                tracing::debug!(error = %e, "conexión terminada");
            }
        });
    }
}

/// Atiende una conexión: lee requests hasta EOF, responde cada uno.
async fn handle_conn<E>(mut stream: UnixStream, engine: Arc<E>) -> std::io::Result<()>
where
    E: Engine,
{
    loop {
        let req: DaemonRequest = match read_frame(&mut stream).await {
            Ok(r) => r,
            // EOF limpio = el cliente cerró; no es error.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let resp = dispatch(&*engine, req).await;
        write_frame(&mut stream, &resp).await?;
    }
}

/// Traduce un request a la llamada `Engine` correspondiente.
async fn dispatch<E: Engine>(engine: &E, req: DaemonRequest) -> DaemonResponse {
    match req {
        DaemonRequest::Run(intent) => match engine.run(intent).await {
            Ok(h) => DaemonResponse::Ran(h),
            Err(e) => DaemonResponse::Err(e),
        },
        DaemonRequest::Stop { card_id, grace_ms } => {
            match engine.stop(card_id, Duration::from_millis(grace_ms)).await {
                Ok(()) => DaemonResponse::Stopped,
                Err(e) => DaemonResponse::Err(e),
            }
        }
        DaemonRequest::List => match engine.list().await {
            Ok(v) => DaemonResponse::Listed(v),
            Err(e) => DaemonResponse::Err(e),
        },
        DaemonRequest::Status { card_id } => match engine.status(card_id).await {
            Ok(s) => DaemonResponse::Status(s),
            Err(e) => DaemonResponse::Err(e),
        },
        DaemonRequest::Telemetry { card_id } => match engine.telemetry(card_id).await {
            Ok(t) => DaemonResponse::Telemetry(t),
            Err(e) => DaemonResponse::Err(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandokan_core::{Engine, EngineError};
    use sandokan_local::LocalEngine;
    use ulid::Ulid;

    #[tokio::test]
    async fn roundtrip_list_and_notfound() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("sandokan.sock");

        let engine = Arc::new(LocalEngine::new());
        let sock_srv = sock.clone();
        let srv = tokio::spawn(async move {
            let _ = serve(engine, &sock_srv).await;
        });

        // Espera a que el socket esté listo.
        for _ in 0..50 {
            if sock.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let client = crate::DaemonEngine::new(sock.clone());
        assert!(client.is_reachable().await);

        // list() sobre engine vacío → vacío.
        let listed = client.list().await.expect("list");
        assert!(listed.is_empty());

        // status() de un id desconocido → NotFound propagado por el wire.
        let unknown = Ulid::new();
        match client.status(unknown).await {
            Err(EngineError::NotFound(id)) => assert_eq!(id, unknown),
            other => panic!("esperaba NotFound, fue {other:?}"),
        }

        srv.abort();
    }
}
