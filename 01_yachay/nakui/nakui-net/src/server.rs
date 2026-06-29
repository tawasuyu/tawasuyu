//! El servidor: hospeda el escritor autoritativo y lo sirve a clientes
//! remotos por card-net.
//!
//! El `Writer` contiene executors Rhai, que son `!Send`: no pueden cruzar
//! threads ni vivir dentro de un `tokio::spawn`. Por eso el escritor corre
//! como **actor en su propio thread fijo** ([`WriterHandle`]): construido
//! ahí, nunca se mueve. Las tareas async de red sólo hablan con él por
//! canales (que sí son `Send`), aislando el Rhai del executor async.
//!
//! El actor es además el punto de serialización (un solo thread procesa los
//! commits en orden) y el origen de la difusión: tras cada commit exitoso,
//! lo empuja a los subscribers; el forwarder lo manda a todos los streams.

use std::collections::HashMap;
use std::sync::mpsc::{channel as std_channel, Sender as StdSender};
use std::sync::Arc;

use card_net::{BrahmanNet, Multiaddr, PeerId as LpPeerId};
use futures::StreamExt;
use tokio::io::WriteHalf;
use tokio::sync::{mpsc as tmpsc, oneshot, Mutex as TMutex};
use tokio_util::compat::FuturesAsyncReadCompatExt;

use nakui_sync::{Commit, Intent, Writer};

use crate::wire::{ClientMsg, ServerMsg};
use crate::{escribir_frame, leer_frame, CompatStream, ErrorNet, PROTO};

type Escritor = WriteHalf<CompatStream>;
type MapaEscritores = Arc<TMutex<HashMap<LpPeerId, Escritor>>>;

// ---- actor del escritor -----------------------------------------------------

/// Comando hacia el thread del escritor.
enum ActorCmd {
    Commit {
        intent: Intent,
        reply: oneshot::Sender<Result<Commit, String>>,
    },
    Subscribe {
        sub: tmpsc::UnboundedSender<Commit>,
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

    /// Registra un canal para recibir cada commit autoritativo difundido.
    fn subscribe(&self, sub: tmpsc::UnboundedSender<Commit>) {
        let _ = self.cmd_tx.send(ActorCmd::Subscribe { sub });
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
                ActorCmd::Subscribe { sub } => subs.push(sub),
            }
        }
    });
    WriterHandle { cmd_tx }
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
            conducir_servidor(handle, incoming).await;
        });
    });

    match listo_rx.recv() {
        Ok(Ok(dial_addr)) => Ok(ServerHandle { dial_addr }),
        Ok(Err(e)) => Err(ErrorNet::Arranque(e)),
        Err(_) => Err(ErrorNet::Arranque("el hilo de runtime murió".into())),
    }
}

/// El bucle del servidor: forwarder de difusión + aceptación de clientes.
async fn conducir_servidor(
    handle: WriterHandle,
    incoming: impl futures::Stream<Item = (LpPeerId, card_net::Stream)>,
) {
    let escritores: MapaEscritores = Arc::new(TMutex::new(HashMap::new()));

    // Forwarder: cada commit autoritativo se difunde a todos los streams.
    {
        let (sub_tx, mut brx) = tmpsc::unbounded_channel::<Commit>();
        handle.subscribe(sub_tx);
        let escritores = escritores.clone();
        tokio::spawn(async move {
            while let Some(commit) = brx.recv().await {
                let frame = match serde_json::to_vec(&ServerMsg::Broadcast { commit }) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let mut g = escritores.lock().await;
                let peers: Vec<LpPeerId> = g.keys().cloned().collect();
                let mut muertos = Vec::new();
                for p in peers {
                    if let Some(wr) = g.get_mut(&p) {
                        if escribir_frame(wr, &frame).await.is_err() {
                            muertos.push(p);
                        }
                    }
                }
                for p in muertos {
                    g.remove(&p);
                }
            }
        });
    }

    // Aceptación de clientes.
    let mut entrantes = Box::pin(incoming);
    while let Some((peer, stream)) = entrantes.next().await {
        registrar_cliente(peer, stream, escritores.clone(), handle.clone()).await;
    }
}

/// Registra un stream de cliente: guarda su escritor y lanza la tarea que
/// lee `Submit`s, los manda al actor escritor y responde el `CommitResult`.
async fn registrar_cliente(
    peer: LpPeerId,
    stream: card_net::Stream,
    escritores: MapaEscritores,
    handle: WriterHandle,
) {
    let compat = stream.compat();
    let (mut rd, wr) = tokio::io::split(compat);
    escritores.lock().await.insert(peer, wr);

    let escritores_r = escritores.clone();
    tokio::spawn(async move {
        loop {
            let bytes = match leer_frame(&mut rd).await {
                Ok(b) => b,
                Err(_) => break,
            };
            let ClientMsg::Submit { req_id, intent } = match serde_json::from_slice(&bytes) {
                Ok(m) => m,
                Err(_) => break,
            };
            // El actor valida/ordena/materializa y difunde a todos (incluido
            // este peer, que dedup-ea por seq). Acá sólo respondemos al req.
            let result = handle.commit(intent).await;
            let frame = match serde_json::to_vec(&ServerMsg::CommitResult { req_id, result }) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let mut g = escritores_r.lock().await;
            if let Some(wr) = g.get_mut(&peer) {
                if escribir_frame(wr, &frame).await.is_err() {
                    break;
                }
            }
        }
        escritores_r.lock().await.remove(&peer);
    });
}
