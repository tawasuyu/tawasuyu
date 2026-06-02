//! Loop servidor: envuelve un `InteractiveEngine` y lo expone por Unix socket.
//!
//! El daemon de sandokan es el **plano interactivo** de la suite, así que sirve
//! engines interactivos (`LocalEngine` in-process, o reenviados). Las sesiones
//! interactivas se materializan por `RunInteractive`; el attach es out-of-band
//! contra `<run_dir>/<card_id>.sock` (la respuesta trae ese path).

use crate::protocol::{read_frame, write_frame, DaemonRequest, DaemonResponse};
use sandokan_core::InteractiveEngine;
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
    E: InteractiveEngine + 'static,
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
    E: InteractiveEngine,
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

/// Traduce un request a la llamada `Engine`/`InteractiveEngine` correspondiente.
async fn dispatch<E: InteractiveEngine>(engine: &E, req: DaemonRequest) -> DaemonResponse {
    match req {
        DaemonRequest::Run(intent) => match engine.run(intent).await {
            Ok(h) => DaemonResponse::Ran(h),
            Err(e) => DaemonResponse::Err(e),
        },
        DaemonRequest::RunInteractive { intent, size } => {
            match engine.run_interactive(intent, size).await {
                Ok(h) => {
                    let socket_path = engine.session_socket_path(h.card_id);
                    DaemonResponse::RanInteractive {
                        handle: h,
                        socket_path,
                    }
                }
                Err(e) => DaemonResponse::Err(e),
            }
        }
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

    /// Round-trip interactivo completo: el cliente pide `RunInteractive` por el
    /// wire, recibe el `socket_path` del daemon, y se conecta a ese
    /// `<card_id>.sock` para mandar un comando y leer su salida. Prueba el
    /// contrato del front: orquestar por el daemon, attachar por el socket.
    #[tokio::test]
    async fn interactive_roundtrip_over_daemon() {
        use card_core::{Card, NamespaceSet, Payload};
        use sandokan_core::{InteractiveEngine, Intent, PtySize};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixStream;

        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("sandokan.sock");
        // El run_dir del engine es el tempdir: ahí caen los <card_id>.sock.
        let engine = Arc::new(LocalEngine::with_run_dir(
            arje_incarnate::IncarnatorConfig::default(),
            dir.path().to_path_buf(),
        ));
        let sock_srv = sock.clone();
        let srv = tokio::spawn(async move {
            let _ = serve(engine, &sock_srv).await;
        });
        for _ in 0..50 {
            if sock.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let client = crate::DaemonEngine::new(sock.clone());
        let mut card = Card::new("term");
        card.payload = Payload::Native {
            exec: "/bin/sh".into(),
            argv: vec![],
            envp: vec![],
        };
        card.soma.namespaces = NamespaceSet {
            user: true,
            pid: true,
            mount: true,
            uts: true,
            ipc: true,
            net: false,
            cgroup: false,
        };
        let id = card.id;

        let handle = client
            .run_interactive(Intent::new(card), PtySize::default())
            .await
            .expect("run_interactive por el daemon");
        assert_eq!(handle.card_id, id);

        // El daemon reportó el socket de attach; el cliente lo cacheó.
        let path = client.session_socket_path(id);
        assert!(!path.as_os_str().is_empty(), "el daemon no reportó socket");
        for _ in 0..50 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(path.exists(), "no apareció el <card_id>.sock: {path:?}");

        // Attach out-of-band: comando + lectura por el socket de la sesión.
        let mut c = UnixStream::connect(&path).await.expect("connect attach");
        c.write_all(b"echo DAEMON_OK\n").await.unwrap();
        let mut acc = Vec::new();
        let mut buf = [0u8; 4096];
        let deadline = tokio::time::Instant::now() + Duration::from_millis(4000);
        let mut ok = false;
        loop {
            match tokio::time::timeout_at(deadline, c.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => {
                    acc.extend_from_slice(&buf[..n]);
                    if String::from_utf8_lossy(&acc).contains("DAEMON_OK") {
                        ok = true;
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(ok, "no llegó la salida por el socket de attach del daemon");

        client.stop(id, Duration::ZERO).await.ok();
        srv.abort();
    }
}
