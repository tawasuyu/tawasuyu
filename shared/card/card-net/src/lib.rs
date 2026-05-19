//! `brahman-net` — capa P2P compartida de la red Brahman.
//!
//! Provee un nodo libp2p genérico que cualquier protocolo de la
//! familia (handshake brahman remoto, sync minga, futuros) puede
//! reusar. La idea: una sola malla, múltiples sub-protocolos
//! multiplexados por `StreamProtocol`.
//!
//! ## Stack
//!
//! - **TCP + Noise + Yamux**: transporte autenticado y multiplexado.
//! - **`stream::Behaviour`**: streams bidireccionales por
//!   `StreamProtocol`. Cada protocolo (`/brahman/handshake/1.0.0`,
//!   `/minga/sync/1.0.0`, …) se registra independientemente vía el
//!   `stream::Control` que `BrahmanNet` expone.
//! - **`kad::Behaviour<MemoryStore>`**: Kademlia DHT en modo Server
//!   para discovery (peers cercanos + content providers).
//! - **`identify::Behaviour`**: cada peer anuncia sus listen-addrs
//!   reales; las inyectamos automáticamente al routing table de Kad.
//!
//! ## Modelo
//!
//! El swarm corre en una task tokio dedicada. La interfaz pública son:
//! 1. **Comandos** (canal mpsc): `dial`, `listen`, `add_dht_peer`,
//!    `find_closest_peers`, `start_providing`, `find_providers`.
//! 2. **`stream::Control`** (acceso directo): para abrir/aceptar
//!    streams de un protocolo concreto. Cada protocolo se ocupa de
//!    su propia lógica sobre el stream resultante.
//!
//! La separación entre comandos y control permite que la lógica de
//! red (DHT, dial, listen) y la lógica de protocolos (handshake/sync)
//! evolucionen independientes — el protocolo no necesita conocer al
//! swarm, sólo pide streams.
//!
//! ## Identidad
//!
//! Por defecto se genera una keypair Ed25519 efímera. Para identidad
//! persistente (la misma `peer_id` across reboots), pasar la keypair
//! con [`BrahmanNet::with_keypair`]. Esa misma keypair puede ser la
//! base para firmas de Cards (cuando se implemente trust remoto).

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use libp2p::{
    identify, identity, kad, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Swarm, SwarmBuilder,
};
use libp2p_allow_block_list::{self as allow_block_list, BlockedPeers};
use libp2p_stream as stream;
use tokio::sync::{mpsc, oneshot, Mutex};

pub use libp2p::{
    identity::{Keypair, PublicKey},
    multiaddr::Protocol,
    Multiaddr, PeerId, Stream, StreamProtocol,
};
pub use libp2p_stream::OpenStreamError;

const IDENTIFY_PROTOCOL: &str = "/brahman-net/0.1.0";
const IDLE_CONNECTION_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(NetworkBehaviour)]
struct BrahmanBehaviour {
    /// Block-list a nivel de swarm: peers en este behaviour son
    /// rechazados ANTES del handshake Noise. Más eficiente que
    /// rechazar al nivel del handshake brahman (ahorra round-trip
    /// TCP+Noise por intento denegado). Sincronizado con la
    /// `PeerPolicy.deny` vía `block_peer`/`unblock_peer` exposed
    /// en `BrahmanNet`.
    block_list: allow_block_list::Behaviour<BlockedPeers>,
    stream: stream::Behaviour,
    kad: kad::Behaviour<kad::store::MemoryStore>,
    identify: identify::Behaviour,
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("transport build failed: {0}")]
    Build(String),
}

#[derive(Debug)]
enum Command {
    Dial(Multiaddr),
    Listen(Multiaddr),
    AddDhtPeer(PeerId, Multiaddr),
    FindClosestPeers(PeerId, oneshot::Sender<Vec<DiscoveredPeer>>),
    StartProviding(Vec<u8>),
    StopProviding(Vec<u8>),
    GetProviders(Vec<u8>, oneshot::Sender<Vec<PeerId>>),
    BlockPeer(PeerId),
    UnblockPeer(PeerId),
}

/// Peer descubierto vía DHT: identidad + direcciones conocidas.
#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub peer_id: PeerId,
    pub addrs: Vec<Multiaddr>,
}

/// Nodo Brahman en la malla P2P. Maneja el swarm libp2p y expone
/// API uniforme para listen/dial/DHT/streams.
pub struct BrahmanNet {
    /// Identidad libp2p de este nodo. Estable mientras viva la
    /// keypair (efímera por default; persistente si pasaste una
    /// vía [`with_keypair`]).
    pub peer_id: PeerId,
    /// Keypair compartida (Arc para compartir con consumers que
    /// necesitan firmar mensajes con la misma identidad — p. ej.
    /// `brahman_handshake::network::connect_libp2p` que firma el
    /// Hello). NO se expone públicamente; usar [`Self::keypair`].
    keypair: Arc<Keypair>,
    cmd_tx: mpsc::UnboundedSender<Command>,
    listen_rx: Mutex<mpsc::UnboundedReceiver<Multiaddr>>,
    /// Control para abrir y aceptar streams. Cada protocolo
    /// (handshake brahman, sync minga, etc.) llama
    /// `control.accept(StreamProtocol::new("/foo/1.0.0"))` para
    /// recibir streams entrantes, o `control.open_stream(peer, proto)`
    /// para abrirlos. Multiplexado y demultiplexado lo hace libp2p.
    pub control: stream::Control,
}

impl BrahmanNet {
    /// Crea un nodo con keypair Ed25519 generada al vuelo (peer_id
    /// efímero — cambia en cada arranque).
    pub fn new() -> Result<Self, NodeError> {
        Self::with_keypair(identity::Keypair::generate_ed25519())
    }

    /// Crea un nodo con una keypair libp2p específica. Usá esto para
    /// `peer_id` estable (por ejemplo si tu identidad se persiste a
    /// disco, o si la derivás de la identidad criptográfica del
    /// módulo).
    ///
    /// Sólo Ed25519 se soporta — la `keypair` se duplica internamente
    /// vía clone del `ed25519::Keypair` para que tanto el swarm
    /// (Noise auth) como el caller (firma de Cards) compartan la
    /// misma identidad sin la fricción de que `identity::Keypair` no
    /// implemente `Clone`.
    pub fn with_keypair(keypair: identity::Keypair) -> Result<Self, NodeError> {
        let ed_kp = keypair
            .try_into_ed25519()
            .map_err(|_| NodeError::Build("brahman-net sólo soporta keypairs Ed25519".into()))?;
        let kp_for_swarm = identity::Keypair::from(ed_kp.clone());
        let kp_for_storage = Arc::new(identity::Keypair::from(ed_kp));
        let peer_id = kp_for_swarm.public().to_peer_id();

        let mut swarm: Swarm<BrahmanBehaviour> = SwarmBuilder::with_existing_identity(kp_for_swarm)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .map_err(|e| NodeError::Build(format!("{e}")))?
            .with_behaviour(|key| {
                let local = key.public().to_peer_id();
                let mut kad =
                    kad::Behaviour::new(local, kad::store::MemoryStore::new(local));
                // Modo Server: respondemos a queries del DHT. Auto
                // requiere detectar reachability; para entornos
                // controlados (localhost, redes privadas) Server es
                // lo correcto.
                kad.set_mode(Some(kad::Mode::Server));
                let identify = identify::Behaviour::new(
                    identify::Config::new(IDENTIFY_PROTOCOL.to_string(), key.public())
                        .with_agent_version(format!("brahman-net/{}", env!("CARGO_PKG_VERSION"))),
                );
                BrahmanBehaviour {
                    block_list: allow_block_list::Behaviour::default(),
                    stream: stream::Behaviour::new(),
                    kad,
                    identify,
                }
            })
            .map_err(|e| NodeError::Build(format!("{e}")))?
            .with_swarm_config(|c| c.with_idle_connection_timeout(IDLE_CONNECTION_TIMEOUT))
            .build();

        let control = swarm.behaviour().stream.new_control();

        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Command>();
        let (listen_tx, listen_rx) = mpsc::unbounded_channel::<Multiaddr>();

        tokio::spawn(async move {
            let mut pending_finds: HashMap<
                kad::QueryId,
                oneshot::Sender<Vec<DiscoveredPeer>>,
            > = HashMap::new();
            let mut pending_providers: HashMap<
                kad::QueryId,
                (Vec<PeerId>, oneshot::Sender<Vec<PeerId>>),
            > = HashMap::new();

            loop {
                tokio::select! {
                    Some(cmd) = cmd_rx.recv() => {
                        match cmd {
                            Command::Dial(addr) => {
                                let _ = swarm.dial(addr);
                            }
                            Command::Listen(addr) => {
                                let _ = swarm.listen_on(addr);
                            }
                            Command::AddDhtPeer(peer, addr) => {
                                swarm.behaviour_mut().kad.add_address(&peer, addr);
                            }
                            Command::FindClosestPeers(target, tx) => {
                                let qid = swarm.behaviour_mut().kad.get_closest_peers(target);
                                pending_finds.insert(qid, tx);
                            }
                            Command::StartProviding(key) => {
                                // Best-effort: si falla (sin peers cercanos para
                                // replicar), seguirá viviendo en el local store
                                // y se servirá vía get_providers de quien tenga
                                // conexión con nosotros.
                                let _ = swarm.behaviour_mut().kad.start_providing(key.into());
                            }
                            Command::StopProviding(key) => {
                                // Quitamos el record local del provider store.
                                // Los peers cercanos eventualmente expiran su
                                // copia replicada por TTL natural (~24h en
                                // libp2p kad default); para retiro inmediato
                                // habría que enviar un republish con sentinel,
                                // pero kad no expone esa primitiva. Aceptable
                                // para el caso "el provider local desapareció":
                                // queries que pasen por nosotros dejan de
                                // listarnos al instante.
                                swarm.behaviour_mut().kad.stop_providing(&key.into());
                            }
                            Command::GetProviders(key, tx) => {
                                let qid = swarm.behaviour_mut().kad.get_providers(key.into());
                                pending_providers.insert(qid, (Vec::new(), tx));
                            }
                            Command::BlockPeer(peer) => {
                                swarm.behaviour_mut().block_list.block_peer(peer);
                            }
                            Command::UnblockPeer(peer) => {
                                swarm.behaviour_mut().block_list.unblock_peer(peer);
                            }
                        }
                    }
                    event = swarm.select_next_some() => {
                        match event {
                            SwarmEvent::NewListenAddr { address, .. } => {
                                let _ = listen_tx.send(address);
                            }
                            // Identify nos dice las listen-addrs reales del
                            // peer. Las inyectamos a Kad para poblar el
                            // routing table sin necesidad de add_dht_peer
                            // manual — la propagación pasa a ser automática.
                            SwarmEvent::Behaviour(BrahmanBehaviourEvent::Identify(
                                identify::Event::Received { peer_id, info, .. }
                            )) => {
                                for addr in info.listen_addrs {
                                    swarm.behaviour_mut().kad.add_address(&peer_id, addr);
                                }
                            }
                            SwarmEvent::Behaviour(BrahmanBehaviourEvent::Kad(
                                kad::Event::OutboundQueryProgressed { id, result, step, .. }
                            )) => {
                                match result {
                                    kad::QueryResult::GetClosestPeers(Ok(ok)) if step.last => {
                                        if let Some(tx) = pending_finds.remove(&id) {
                                            let infos = ok.peers.into_iter()
                                                .map(|p| DiscoveredPeer {
                                                    peer_id: p.peer_id,
                                                    addrs: p.addrs,
                                                })
                                                .collect();
                                            let _ = tx.send(infos);
                                        }
                                    }
                                    kad::QueryResult::GetClosestPeers(Err(_)) if step.last => {
                                        if let Some(tx) = pending_finds.remove(&id) {
                                            let _ = tx.send(Vec::new());
                                        }
                                    }
                                    kad::QueryResult::GetProviders(Ok(ok)) => {
                                        if let Some((collected, _)) =
                                            pending_providers.get_mut(&id)
                                        {
                                            if let kad::GetProvidersOk::FoundProviders {
                                                providers, ..
                                            } = ok
                                            {
                                                for p in providers {
                                                    if !collected.contains(&p) {
                                                        collected.push(p);
                                                    }
                                                }
                                            }
                                        }
                                        if step.last {
                                            if let Some((providers, tx)) =
                                                pending_providers.remove(&id)
                                            {
                                                let _ = tx.send(providers);
                                            }
                                        }
                                    }
                                    kad::QueryResult::GetProviders(Err(_)) if step.last => {
                                        if let Some((providers, tx)) =
                                            pending_providers.remove(&id)
                                        {
                                            let _ = tx.send(providers);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        Ok(Self {
            peer_id,
            keypair: kp_for_storage,
            cmd_tx,
            listen_rx: Mutex::new(listen_rx),
            control,
        })
    }

    /// Acceso a la keypair de identidad del nodo. Usar para firmar
    /// payloads que viajan asociados al `peer_id` (handshake brahman
    /// firmado, futuros sub-protocolos con autenticación). El `Arc`
    /// permite compartir sin copia — la keypair libp2p no es `Clone`.
    pub fn keypair(&self) -> Arc<Keypair> {
        self.keypair.clone()
    }

    /// Bloquea conexiones desde/hacia `peer` a nivel del swarm.
    /// Conexiones existentes se cierran y nuevos intentos son
    /// rechazados ANTES del Noise handshake — más eficiente que
    /// rechazar al nivel del handshake brahman (ahorra round-trip
    /// TCP+Noise por intento). Idempotente.
    pub fn block_peer(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(Command::BlockPeer(peer));
    }

    /// Quita a `peer` de la block-list del swarm. Conexiones futuras
    /// son aceptadas con normalidad. Idempotente.
    pub fn unblock_peer(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(Command::UnblockPeer(peer));
    }

    /// Empieza a escuchar en `addr`. Bloquea hasta que el listener
    /// publique su dirección real (Multiaddr resuelta — útil cuando
    /// pediste `/ip4/0.0.0.0/tcp/0` y querés saber qué puerto te tocó).
    pub async fn listen(&self, addr: Multiaddr) -> Multiaddr {
        self.cmd_tx
            .send(Command::Listen(addr))
            .expect("swarm task alive");
        let mut rx = self.listen_rx.lock().await;
        rx.recv().await.expect("listen address arrives")
    }

    /// Inicia conexión con un peer en `addr`. No-op si ya hay
    /// conexión. Best-effort — fallos se loggean al swarm pero no se
    /// propagan al caller (consistente con libp2p).
    pub fn dial(&self, addr: Multiaddr) {
        let _ = self.cmd_tx.send(Command::Dial(addr));
    }

    /// Añade un peer al routing table de Kademlia. Punto de entrada
    /// para bootstrap: tras esto, el nodo puede dirigir queries DHT
    /// a través de este peer.
    pub fn add_dht_peer(&self, peer: PeerId, addr: Multiaddr) {
        let _ = self.cmd_tx.send(Command::AddDhtPeer(peer, addr));
    }

    /// Consulta el DHT por los peers más cercanos al `target` PeerId.
    /// Devuelve la lista resuelta (vacía si la query falla o si no
    /// hay peers conocidos). Bloquea hasta que la query completa.
    pub async fn find_closest_peers(&self, target: PeerId) -> Vec<DiscoveredPeer> {
        let (tx, rx) = oneshot::channel();
        let _ = self
            .cmd_tx
            .send(Command::FindClosestPeers(target, tx));
        rx.await.unwrap_or_default()
    }

    /// Anuncia en el DHT que este peer tiene el contenido identificado
    /// por `key`. Otros peers pueden luego descubrirlo vía
    /// [`find_providers`](Self::find_providers). Best-effort: si la
    /// replicación falla inicialmente, el record vive en el store
    /// local hasta que llegue conexión.
    pub fn start_providing(&self, key: &[u8]) {
        let _ = self.cmd_tx.send(Command::StartProviding(key.to_vec()));
    }

    /// Retira el anuncio previo de [`start_providing`] para `key`.
    /// El record local se borra al instante (queries que lleguen a
    /// nosotros dejan de listarnos). Los records replicados en peers
    /// remotos viven hasta su TTL — kad no expone primitiva para
    /// retracción inmediata cross-peer. Aceptable: simétrico al
    /// caso "el provider apareció" (también propagación eventual).
    pub fn stop_providing(&self, key: &[u8]) {
        let _ = self.cmd_tx.send(Command::StopProviding(key.to_vec()));
    }

    /// Consulta el DHT por peers que han anunciado proveer `key`.
    /// Devuelve la lista de `PeerId`s que se reportan como providers.
    /// Lista vacía si nadie anuncia.
    pub async fn find_providers(&self, key: &[u8]) -> Vec<PeerId> {
        let (tx, rx) = oneshot::channel();
        let _ = self
            .cmd_tx
            .send(Command::GetProviders(key.to_vec(), tx));
        rx.await.unwrap_or_default()
    }
}
