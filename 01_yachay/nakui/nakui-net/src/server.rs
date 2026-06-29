//! El servidor: hospeda el escritor autoritativo y lo sirve a clientes
//! remotos por card-net.
//!
//! El `Writer` contiene executors Rhai, que son `!Send`: no pueden cruzar
//! threads ni vivir dentro de un `tokio::spawn`. Por eso el escritor corre
//! como **actor en su propio thread fijo** ([`WriterHandle`]): construido
//! ahí, nunca se mueve. Las tareas async de red sólo hablan con él por
//! canales (que sí son `Send`), aislando el Rhai del executor async.
//!
//! El actor es el punto de serialización (un solo thread procesa los commits
//! en orden) y el origen de la difusión. Cada cliente tiene una tarea propia
//! que multiplexa, sobre su único stream, sus `Submit`s y los `Broadcast`s
//! que le tocan. Al engancharse, el actor le entrega —en un solo turno,
//! atómico respecto a los commits— un snapshot del estado + su suscripción,
//! así no hay hueco ni duplicado entre el catch-up y los broadcasts.

use std::sync::mpsc::{channel as std_channel, Sender as StdSender};

use card_net::{BrahmanNet, Multiaddr, PeerId as LpPeerId};
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::{mpsc as tmpsc, oneshot};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use uuid::Uuid;

use nakui_core::store::Store;
use nakui_sync::{Commit, Intent, Writer};

use crate::wire::{ClientMsg, ServerMsg};
use crate::{escribir_frame, leer_frame, ErrorNet, PROTO};

/// El estado autoritativo capturado para un catch-up: cada record + el
/// cursor (`last_seq`) hasta donde llega.
type SnapshotData = (Vec<(String, Uuid, Value)>, Option<u64>);

// ---- actor del escritor -----------------------------------------------------

/// Comando hacia el thread del escritor.
enum ActorCmd {
    Commit {
        intent: Intent,
        reply: oneshot::Sender<Result<Commit, String>>,
    },
    /// Engancha un cliente: registra su canal de difusión y devuelve, en el
    /// mismo turno, el snapshot del estado actual. Atómico respecto a los
    /// commits — ningún commit se cuela entre la captura y el alta.
    Attach {
        sub: tmpsc::UnboundedSender<Commit>,
        reply: oneshot::Sender<SnapshotData>,
    },
}

/// Handle `Send + Clone` al escritor autoritativo, que vive en un thread
/// propio (porque los executors Rhai son `!Send`). Esto es lo que las
/// tareas async usan en lugar del `Writer` directo.
#[derive(Clone)]
struct WriterHandle {
    cmd_tx: StdSender<ActorCmd>,
}

impl WriterHandle {
    /// Entrega una intención al escritor y espera el commit autoritativo.
    async fn commit(&self, intent: Intent) -> Result<Commit, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ActorCmd::Commit { intent, reply: tx })
            .map_err(|_| "nakui-net :: el escritor está caído".to_string())?;
        rx.await
            .map_err(|_| "nakui-net :: el escritor no respondió".to_string())?
    }

    /// Engancha un cliente: devuelve el snapshot inicial; el `sub` queda
    /// registrado para recibir cada commit posterior.
    async fn attach(&self, sub: tmpsc::UnboundedSender<Commit>) -> Result<SnapshotData, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ActorCmd::Attach { sub, reply: tx })
            .map_err(|_| "nakui-net :: el escritor está caído".to_string())?;
        rx.await
            .map_err(|_| "nakui-net :: el escritor no respondió".to_string())
    }
}

/// Arranca el thread del escritor. `build` construye el `Writer` *en ese
/// thread* (los executors `!Send` nacen y mueren ahí). El thread es el
/// punto de serialización: procesa commits de a uno, en orden, y difunde
/// cada éxito a los subscribers.
fn spawn_writer_actor<F>(build: F) -> WriterHandle
where
    F: FnOnce() -> Writer + Send + 'static,
{
    let (cmd_tx, cmd_rx) = std_channel::<ActorCmd>();
    std::thread::spawn(move || {
        let mut writer = build();
        let mut subs: Vec<tmpsc::UnboundedSender<Commit>> = Vec::new();
        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                ActorCmd::Commit { intent, reply } => {
                    let result = writer.commit(intent);
                    if let Ok(commit) = &result {
                        // Difunde sólo commits con efecto; purga subs caídos.
                        if !commit.entries.is_empty() {
                            subs.retain(|s| s.send(commit.clone()).is_ok());
                        }
                    }
                    let _ = reply.send(result);
                }
                ActorCmd::Attach { sub, reply } => {
                    subs.push(sub);
                    let snap = capturar_snapshot(&writer);
                    let _ = reply.send(snap);
                }
            }
        }
    });
    WriterHandle { cmd_tx }
}

/// Captura el estado autoritativo completo para el catch-up de un cliente.
fn capturar_snapshot(writer: &Writer) -> SnapshotData {
    let store = writer.store_handle();
    let g = match store.lock() {
        Ok(g) => g,
        Err(_) => return (Vec::new(), None),
    };
    let records: Vec<(String, Uuid, Value)> = g.iter().map(|it| it.collect()).unwrap_or_default();
    let last_seq = g.last_applied_seq().ok().flatten();
    (records, last_seq)
}

// ---- API pública ------------------------------------------------------------

/// Handle vivo de un servidor. El punto de acceso a la dirección dialable
/// que un cliente pasa a [`crate::CardNetTransport::connect`].
pub struct ServerHandle {
    dial_addr: String,
}

impl ServerHandle {
    /// La dirección dialable completa de este servidor (con `/p2p/<peerid>`).
    pub fn dial_addr(&self) -> &str {
        &self.dial_addr
    }
}

/// Levanta el servidor: arranca un nodo libp2p, escucha en `bind` (una
/// multiaddr, p. ej. `"/ip4/0.0.0.0/tcp/0"`) y sirve el escritor a la red.
///
/// `build_writer` construye el `Writer` dentro del thread del actor — es la
/// forma de cruzar la frontera `!Send`: la clausura sólo captura datos
/// `Send` (rutas, configs) y materializa los executors Rhai ahí adentro.
/// Bloquea hasta que el nodo resolvió su dirección de escucha.
pub fn serve<F>(build_writer: F, bind: &str) -> Result<ServerHandle, ErrorNet>
where
    F: FnOnce() -> Writer + Send + 'static,
{
    let (listo_tx, listo_rx) = std_channel::<Result<String, String>>();
    let bind = bind.to_string();

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = listo_tx.send(Err(e.to_string()));
                return;
            }
        };
        rt.block_on(async move {
            let handle = spawn_writer_actor(build_writer);
            let node = match BrahmanNet::new() {
                Ok(n) => n,
                Err(e) => {
                    let _ = listo_tx.send(Err(format!("{e:?}")));
                    return;
                }
            };
            let addr: Multiaddr = match bind.parse() {
                Ok(a) => a,
                Err(e) => {
                    let _ = listo_tx.send(Err(format!("multiaddr inválida: {e}")));
                    return;
                }
            };
            let listen_addr = node.listen(addr).await;
            let dial = format!("{}/p2p/{}", listen_addr, node.peer_id);

            // Registrar el handler del protocolo ANTES de anunciar readiness:
            // si un cliente abre el stream antes de que `accept(PROTO)` exista,
            // libp2p resetea el substream y el cliente ve un EOF inmediato.
            let mut control = node.control.clone();
            let incoming = match control.accept(PROTO) {
                Ok(i) => i,
                Err(e) => {
                    let _ = listo_tx.send(Err(format!("accept({PROTO}): {e}")));
                    return;
                }
            };
            let _ = listo_tx.send(Ok(dial));

            let mut incoming = Box::pin(incoming);
            while let Some((peer, stream)) = incoming.next().await {
                tokio::spawn(atender_cliente(peer, stream, handle.clone()));
            }
        });
    });

    match listo_rx.recv() {
        Ok(Ok(dial_addr)) => Ok(ServerHandle { dial_addr }),
        Ok(Err(e)) => Err(ErrorNet::Arranque(e)),
        Err(_) => Err(ErrorNet::Arranque("el hilo de runtime murió".into())),
    }
}

/// La tarea de un cliente: engancha (snapshot + suscripción), envía el
/// catch-up, y luego multiplexa sobre su único stream los `Submit`s que
/// recibe y los `Broadcast`s que le difunden. Un solo dueño del escritor
/// del stream ⇒ sin contención ni mutex sobre el writer.
async fn atender_cliente(peer: LpPeerId, stream: card_net::Stream, handle: WriterHandle) {
    let _ = peer; // identidad del peer; reservado para autorización (fase 3+).
    let compat = stream.compat();
    let (mut rd, mut wr) = tokio::io::split(compat);

    // Enganche atómico: snapshot del estado + alta de la suscripción.
    let (sub_tx, mut sub_rx) = tmpsc::unbounded_channel::<Commit>();
    let (records, last_seq) = match handle.attach(sub_tx).await {
        Ok(s) => s,
        Err(_) => return,
    };

    // Catch-up: el estado actual, antes de cualquier broadcast.
    let frame = match serde_json::to_vec(&ServerMsg::Snapshot { records, last_seq }) {
        Ok(b) => b,
        Err(_) => return,
    };
    if escribir_frame(&mut wr, &frame).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            entrante = leer_frame(&mut rd) => {
                let bytes = match entrante {
                    Ok(b) => b,
                    Err(_) => break,
                };
                let ClientMsg::Submit { req_id, intent } = match serde_json::from_slice(&bytes) {
                    Ok(m) => m,
                    Err(_) => break,
                };
                // El actor valida/ordena/materializa y difunde a todos (este
                // peer incluido, vía sub_rx, que dedup-ea por seq). Acá sólo
                // respondemos el req específico.
                let result = handle.commit(intent).await;
                let frame = match serde_json::to_vec(&ServerMsg::CommitResult { req_id, result }) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                if escribir_frame(&mut wr, &frame).await.is_err() {
                    break;
                }
            }
            difundido = sub_rx.recv() => {
                let commit = match difundido {
                    Some(c) => c,
                    None => break,
                };
                let frame = match serde_json::to_vec(&ServerMsg::Broadcast { commit }) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                if escribir_frame(&mut wr, &frame).await.is_err() {
                    break;
                }
            }
        }
    }
}
