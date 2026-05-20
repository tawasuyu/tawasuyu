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
use std::path::PathBuf;
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

pub fn spawn_bus(path: PathBuf, graph_tx: mpsc::Sender<GraphEvent>) -> anyhow::Result<PathBuf> {
    let _ = std::fs::remove_file(&path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
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
            }
        }
    }).await;

    if let Some(id) = announced_id {
        let _ = graph_tx.send(GraphEvent::BusConnClosed { ente_id: Some(id) }).await;
    }
    writer_task.abort();
    result
}
