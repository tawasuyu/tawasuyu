//! Listener del bus interno. Vive en PID 1, acepta conexiones de Entes hijos,
//! extrae credenciales del kernel vía SO_PEERCRED, y enruta cada request al
//! grafo. Conexión bidireccional: el grafo puede *empujar* requests hacia
//! una conexión registrada (forwarding de Invoke al proveedor).
//!
//! ## Por qué bidireccional
//!
//! Un Ente que provee `Capability::Endpoint` debe poder *recibir* invokes
//! sin abrir más sockets. Después de Announce, el grafo guarda el lado de
//! escritura de su conexión y lo usa para forwardear.

use crate::events::GraphEvent;
use arje_bus::{read_frame, write_frame, BusMessage, BusPayload, BusResponse, PeerCreds};
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, trace, warn};
use ulid::Ulid;

pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var(arje_bus::ENV_BUS_SOCK) {
        return p.into();
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()));
    let user = std::env::var("USER").unwrap_or_else(|_| "ente".into());
    format!("{runtime}/ente-bus-{user}.sock").into()
}

/// ¿Hay un peer respondiendo en ese socket? Es la diferencia entre un socket
/// *stale* (archivo huérfano de un arje-zero anterior que crasheó sin Drop) y
/// uno *vivo* (otro PID 1 corriendo HOY). `connect()` síncrono es la prueba
/// más limpia bajo Unix: ECONNREFUSED indica que el archivo está pero nadie
/// hace `accept()`, ENOENT que ni existe, y Ok que alguien está atendiendo.
///
/// Devolver `true` sólo cuando hay un listener; cualquier otro fallo se
/// trata como "limpiar y seguir", para que el operador no quede locked-out
/// por un archivo con permisos raros tras un crash.
fn socket_in_use(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    StdUnixStream::connect(path).is_ok()
}

pub fn spawn_bus(path: PathBuf, graph_tx: mpsc::Sender<GraphEvent>) -> anyhow::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Defensa contra dos PID 1 simultáneos: si el socket está atendido,
    // abortamos antes de pisarlo. El operador ve un error claro en vez de
    // un escenario silencioso donde el viejo daemon queda huérfano y las
    // conexiones nuevas van al nuevo. Si está stale (post-crash), seguimos
    // — el remove_file de abajo libera el path.
    if socket_in_use(&path) {
        anyhow::bail!(
            "el socket {} ya está atendido por otro arje-zero — abortando para no pisarlo",
            path.display()
        );
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    info!(path = %path.display(), "bus interno escuchando");

    let path_returned = path.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let tx = graph_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_conn(stream, tx).await {
                            warn!(?e, "bus connection ended");
                        }
                    });
                }
                Err(e) => {
                    error!(?e, "bus accept failed, listener cerrando");
                    return;
                }
            }
        }
    });

    Ok(path_returned)
}

async fn handle_conn(stream: UnixStream, graph_tx: mpsc::Sender<GraphEvent>) -> anyhow::Result<()> {
    // SO_PEERCRED: el kernel adjunta pid/uid/gid al socket en connect/accept.
    // No-falsificable desde el cliente.
    let creds = getsockopt(&stream, PeerCredentials)
        .map_err(|e| anyhow::anyhow!("getsockopt PEERCRED: {e}"))?;
    let peer = PeerCreds {
        pid: creds.pid(),
        uid: creds.uid(),
        gid: creds.gid(),
    };
    trace!(?peer, "bus conn aceptada");

    let (mut reader, mut writer) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::channel::<BusMessage>(64);

    // Writer task: única vía de escritura al socket. Multiplexa entre
    // respuestas a peticiones del cliente y forwards iniciados por el grafo.
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if let Err(e) = write_frame(&mut writer, &msg).await {
                warn!(?e, "bus writer falló, terminando");
                return;
            }
        }
    });

    let mut announced_id: Option<Ulid> = None;
    let result: anyhow::Result<()> = (async {
        loop {
            let msg = match read_frame(&mut reader).await {
                Ok(m) => m,
                Err(e) => {
                    trace!(?e, "bus conn read terminó");
                    return Ok(());
                }
            };
            match msg.payload {
                BusPayload::Request(req) => {
                    let is_announce = matches!(req, arje_bus::BusRequest::Announce { .. });
                    let (reply_tx, reply_rx) = oneshot::channel();
                    if graph_tx.send(GraphEvent::BusRequest {
                        peer,
                        from: msg.from,
                        request: req,
                        outbound: out_tx.clone(),
                        reply: reply_tx,
                    }).await.is_err() {
                        warn!("graph cerrado, terminando bus connection");
                        return Ok(());
                    }
                    let response = reply_rx.await.unwrap_or_else(|_| {
                        BusResponse::Error("graph dropped reply channel".into())
                    });
                    if is_announce && matches!(response, BusResponse::Ok) {
                        // Auth del Announce ya fue verificada por el grafo;
                        // memorizamos para cleanup en cierre.
                        announced_id = msg.from;
                    }
                    let out = BusMessage {
                        from: None,
                        seq: msg.seq,
                        payload: BusPayload::Response(response),
                    };
                    if out_tx.send(out).await.is_err() { return Ok(()); }
                }
                BusPayload::Response(resp) => {
                    // Respuesta a un Invoke que el grafo forwardeó a este peer.
                    let _ = graph_tx.send(GraphEvent::BusResponse {
                        seq: msg.seq,
                        response: resp,
                    }).await;
                }
                BusPayload::Event(ev) => {
                    // Los eventos son sólo server→cliente. Un cliente que los
                    // envía está fuera de protocolo; lo ignoramos con warn.
                    warn!(?ev, "cliente envió un BusEvent — ignorado (eventos son server→cliente)");
                }
            }
        }
    }).await;

    if let Some(id) = announced_id {
        let _ = graph_tx.send(GraphEvent::BusConnClosed { ente_id: Some(id) }).await;
    }
    writer_task.abort();
    result
}

#[cfg(test)]
mod bus_lifecycle_tests {
    //! Verifica el contrato de `socket_in_use` y la defensa de `spawn_bus`
    //! contra dos PID 1 simultáneos. El loop de aceptación se descarta:
    //! sólo nos interesa el bind, no la lógica de handle_conn.
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn socket_in_use_es_false_si_el_archivo_no_existe() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("no-existe.sock");
        assert!(!socket_in_use(&path));
    }

    #[test]
    fn socket_in_use_es_false_si_el_archivo_es_un_regular_huerfano() {
        // Caso post-crash típico: el archivo del socket quedó en disco pero
        // ningún proceso lo escucha. Debe leerse como STALE, no como vivo.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("stale.sock");
        std::fs::write(&path, b"").unwrap();
        assert!(!socket_in_use(&path), "un archivo sin listener no está vivo");
    }

    #[tokio::test]
    async fn socket_in_use_es_true_cuando_hay_listener_atendiendo() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vivo.sock");
        let _listener = UnixListener::bind(&path).expect("primer bind");
        assert!(
            socket_in_use(&path),
            "con listener activo connect() debe responder Ok"
        );
    }

    #[tokio::test]
    async fn spawn_bus_aborta_si_otro_daemon_ya_atiende_el_path() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("conflicto.sock");

        // Simulamos otro arje-zero corriendo: bind manual del listener.
        let _otro = UnixListener::bind(&path).expect("listener simulado del 'otro' daemon");

        let (tx, _rx) = mpsc::channel::<GraphEvent>(8);
        let err = spawn_bus(path.clone(), tx)
            .expect_err("spawn_bus no debe pisar a otro daemon vivo");
        let msg = err.to_string();
        assert!(
            msg.contains("ya está atendido"),
            "mensaje esperaba 'ya está atendido', fue: {msg}"
        );
    }

    #[tokio::test]
    async fn spawn_bus_limpia_socket_stale_y_arranca() {
        // El daemon previo crasheó y dejó el archivo. spawn_bus debe poder
        // levantarse encima.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("post-crash.sock");
        std::fs::write(&path, b"residuo").unwrap();

        let (tx, _rx) = mpsc::channel::<GraphEvent>(8);
        let returned = spawn_bus(path.clone(), tx)
            .expect("spawn_bus debe limpiar el archivo stale y arrancar");
        assert_eq!(returned, path);
        assert!(
            socket_in_use(&path),
            "tras spawn_bus, un connect debe contestar"
        );
    }
}
