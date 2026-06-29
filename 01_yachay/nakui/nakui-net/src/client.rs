//! El cliente: [`CardNetTransport`], un impl de
//! [`Transport`](nakui_sync::Transport) que habla con un servidor remoto
//! por card-net.
//!
//! Mismo contrato que `LocalTransport`, así que la UI (vía `NakuiBackend` o
//! cualquier cliente) no distingue local de remoto. El puente sync↔async
//! corre un runtime tokio en un hilo dedicado: `submit` manda la intención
//! por un canal y bloquea esperando la respuesta; `subscribe` devuelve un
//! receiver que la tarea lectora alimenta con cada `Broadcast`.

use std::collections::HashMap;
use std::sync::mpsc::{channel as std_channel, Receiver as StdReceiver, Sender as StdSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use card_net::{BrahmanNet, Multiaddr, PeerId as LpPeerId, Protocol};
use tokio::io::ReadHalf;
use tokio::sync::{mpsc as tmpsc, Mutex as TMutex};
use tokio_util::compat::FuturesAsyncReadCompatExt;

use nakui_sync::{Commit, Intent, Transport};

use crate::wire::{ClientMsg, ServerMsg};
use crate::{escribir_frame, leer_frame, CompatStream, ErrorNet, PROTO};

/// Cuánto espera `submit` la respuesta del servidor antes de rendirse.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Respuesta a un submit pendiente, ruteada por `req_id`.
type ReplyTx = StdSender<Result<Commit, String>>;

/// Comando del API sync hacia el runtime tokio interno.
enum Cmd {
    Submit { intent: Intent, reply: ReplyTx },
}

/// Cliente de red: implementa [`Transport`] hablando con un servidor
/// [`crate::serve`] por card-net. Construir con [`CardNetTransport::connect`].
pub struct CardNetTransport {
    cmd_tx: tmpsc::UnboundedSender<Cmd>,
    /// Subscribers a los commits difundidos por el servidor. La tarea
    /// lectora empuja cada `Broadcast` acá; `subscribe` agrega un sender.
    subscribers: Arc<Mutex<Vec<StdSender<Commit>>>>,
}

impl CardNetTransport {
    /// Conecta a un servidor dada su multiaddr COMPLETA (con `/p2p/<peerid>`),
    /// tal como la expone [`crate::ServerHandle::dial_addr`]. Bloquea hasta
    /// que el stream al servidor quedó abierto (o falla el intento).
    pub fn connect(server_addr: &str) -> Result<CardNetTransport, ErrorNet> {
        let (cmd_tx, cmd_rx) = tmpsc::unbounded_channel::<Cmd>();
        let (listo_tx, listo_rx) = std_channel::<Result<(), String>>();
        let subscribers: Arc<Mutex<Vec<StdSender<Commit>>>> = Arc::new(Mutex::new(Vec::new()));
        let subscribers_net = subscribers.clone();
        let server_addr = server_addr.to_string();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = listo_tx.send(Err(e.to_string()));
                    return;
                }
            };
            rt.block_on(async move {
                conducir_cliente(server_addr, cmd_rx, subscribers_net, listo_tx).await;
            });
        });

        match listo_rx.recv() {
            Ok(Ok(())) => Ok(CardNetTransport { cmd_tx, subscribers }),
            Ok(Err(e)) => Err(ErrorNet::Conexion(e)),
            Err(_) => Err(ErrorNet::Conexion("el hilo de runtime murió".into())),
        }
    }
}

impl Transport for CardNetTransport {
    fn submit(&self, intent: Intent) -> Result<Commit, String> {
        let (reply_tx, reply_rx) = std_channel::<Result<Commit, String>>();
        self.cmd_tx
            .send(Cmd::Submit { intent, reply: reply_tx })
            .map_err(|_| "nakui-net :: runtime de red cerrado".to_string())?;
        match reply_rx.recv_timeout(SUBMIT_TIMEOUT) {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err("nakui-net :: timeout esperando respuesta del servidor".into())
            }
            Err(_) => Err("nakui-net :: conexión perdida".into()),
        }
    }

    fn subscribe(&self) -> StdReceiver<Commit> {
        let (tx, rx) = std_channel();
        if let Ok(mut subs) = self.subscribers.lock() {
            subs.push(tx);
        }
        rx
    }
}

type Pendientes = Arc<TMutex<HashMap<u64, ReplyTx>>>;

/// El bucle de red del cliente: abre el stream al servidor, lanza el lector,
/// señala readiness, y traduce comandos `Submit` a frames.
async fn conducir_cliente(
    server_addr: String,
    mut cmd_rx: tmpsc::UnboundedReceiver<Cmd>,
    subscribers: Arc<Mutex<Vec<StdSender<Commit>>>>,
    listo_tx: StdSender<Result<(), String>>,
) {
    let node = match BrahmanNet::new() {
        Ok(n) => n,
        Err(e) => {
            let _ = listo_tx.send(Err(format!("{e:?}")));
            return;
        }
    };
    let addr: Multiaddr = match server_addr.parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = listo_tx.send(Err(format!("multiaddr inválida: {e}")));
            return;
        }
    };
    let Some(peer) = peer_de(&addr) else {
        let _ = listo_tx.send(Err("la multiaddr no incluye /p2p/<peerid>".into()));
        return;
    };

    node.dial(addr);
    // Reintenta abrir el stream hasta que la conexión se establezca.
    let mut control = node.control.clone();
    let limite = Instant::now() + Duration::from_secs(8);
    let stream = loop {
        match control.open_stream(peer, PROTO).await {
            Ok(s) => break s,
            Err(_) if Instant::now() < limite => {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            Err(e) => {
                let _ = listo_tx.send(Err(format!("open_stream: {e}")));
                return;
            }
        }
    };

    let compat = stream.compat();
    let (rd, mut wr) = tokio::io::split(compat);
    let pendientes: Pendientes = Arc::new(TMutex::new(HashMap::new()));

    // Tarea lectora: rutea CommitResult a su pendiente, Broadcast a los subs.
    {
        let pendientes = pendientes.clone();
        tokio::spawn(leer_servidor(rd, pendientes, subscribers));
    }

    let _ = listo_tx.send(Ok(()));

    // Bucle de comandos.
    let mut req_id: u64 = 0;
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            Cmd::Submit { intent, reply } => {
                req_id += 1;
                pendientes.lock().await.insert(req_id, reply);
                let frame = match serde_json::to_vec(&ClientMsg::Submit { req_id, intent }) {
                    Ok(b) => b,
                    Err(e) => {
                        if let Some(tx) = pendientes.lock().await.remove(&req_id) {
                            let _ = tx.send(Err(format!("serialización: {e}")));
                        }
                        continue;
                    }
                };
                if escribir_frame(&mut wr, &frame).await.is_err() {
                    if let Some(tx) = pendientes.lock().await.remove(&req_id) {
                        let _ = tx.send(Err("nakui-net :: escritura al servidor falló".into()));
                    }
                }
            }
        }
    }
}

/// Lee frames del servidor: resuelve respuestas pendientes y empuja
/// difusiones a los subscribers. Al cerrarse el stream, falla todo lo que
/// quedó pendiente.
async fn leer_servidor(
    mut rd: ReadHalf<CompatStream>,
    pendientes: Pendientes,
    subscribers: Arc<Mutex<Vec<StdSender<Commit>>>>,
) {
    loop {
        let bytes = match leer_frame(&mut rd).await {
            Ok(b) => b,
            Err(_) => break,
        };
        match serde_json::from_slice::<ServerMsg>(&bytes) {
            Ok(ServerMsg::CommitResult { req_id, result }) => {
                if let Some(tx) = pendientes.lock().await.remove(&req_id) {
                    let _ = tx.send(result);
                }
            }
            Ok(ServerMsg::Broadcast { commit }) => {
                if let Ok(mut subs) = subscribers.lock() {
                    subs.retain(|s| s.send(commit.clone()).is_ok());
                }
            }
            Err(_) => break,
        }
    }
    // Conexión cerrada: nadie va a responder los pendientes.
    let mut p = pendientes.lock().await;
    for (_, tx) in p.drain() {
        let _ = tx.send(Err("nakui-net :: el servidor cerró la conexión".into()));
    }
}

/// Extrae el `PeerId` del componente `/p2p/...` de una multiaddr.
fn peer_de(addr: &Multiaddr) -> Option<LpPeerId> {
    addr.iter().find_map(|p| match p {
        Protocol::P2p(pid) => Some(pid),
        _ => None,
    })
}
