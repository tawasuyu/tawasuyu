//! Integración libp2p con behaviour compuesto: streams Minga +
//! Kademlia DHT.
//!
//! - **TCP + Noise + Yamux**: transporte autenticado y multiplexado.
//! - **`stream::Behaviour`**: streams bidireccionales para el
//!   protocolo `/minga/sync/1.0.0`.
//! - **`kad::Behaviour<MemoryStore>`**: tabla de routing distribuida
//!   para descubrimiento. Cada nodo arranca en modo `Server` y
//!   responde a queries del DHT.
//!
//! El swarm corre en una task tokio dedicada que procesa comandos
//! externos (Dial, Listen, AddDhtPeer, FindClosestPeers) y eventos
//! del swarm (NewListenAddr para señalar address resuelto, eventos
//! Kad para completar queries). Los métodos públicos solo envían
//! comandos por canal.

use std::collections::HashMap;
use std::time::Duration;

use futures::StreamExt;
use libp2p::{
    identify, identity, kad, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use libp2p_stream as stream;
use tokio::sync::{mpsc, oneshot, Mutex};

pub const SYNC_PROTOCOL: StreamProtocol = StreamProtocol::new("/minga/sync/1.0.0");
const IDENTIFY_PROTOCOL: &str = "/minga/0.1.0";

#[derive(NetworkBehaviour)]
struct MingaBehaviour {
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
    GetProviders(Vec<u8>, oneshot::Sender<Vec<PeerId>>),
}

/// Peer descubierto vía DHT: identidad + direcciones conocidas.
#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub peer_id: PeerId,
    pub addrs: Vec<Multiaddr>,
}

pub struct LibP2pNode {
    pub peer_id: PeerId,
    cmd_tx: mpsc::UnboundedSender<Command>,
    listen_rx: Mutex<mpsc::UnboundedReceiver<Multiaddr>>,
    /// Control para abrir/aceptar streams.
    pub control: stream::Control,
}

impl LibP2pNode {
    pub fn new() -> Result<Self, NodeError> {
        let id = identity::Keypair::generate_ed25519();
        let peer_id = id.public().to_peer_id();

        let mut swarm: Swarm<MingaBehaviour> = SwarmBuilder::with_existing_identity(id)
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
                // Modo Server: respondemos a queries del DHT. Por
                // defecto kad arranca en Auto, que requiere detectar
                // reachability. Para tests en localhost forzamos Server.
                kad.set_mode(Some(kad::Mode::Server));
                let identify = identify::Behaviour::new(
                    identify::Config::new(IDENTIFY_PROTOCOL.to_string(), key.public())
                        .with_agent_version(format!("minga/{}", env!("CARGO_PKG_VERSION"))),
                );
                MingaBehaviour {
                    stream: stream::Behaviour::new(),
                    kad,
                    identify,
                }
            })
            .map_err(|e| NodeError::Build(format!("{e}")))?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
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
                                // y se servirá vía get_providers de quien
                                // tenga conexión con nosotros.
                                let _ = swarm.behaviour_mut().kad.start_providing(key.into());
                            }
                            Command::GetProviders(key, tx) => {
                                let qid = swarm.behaviour_mut().kad.get_providers(key.into());
                                pending_providers.insert(qid, (Vec::new(), tx));
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
                            SwarmEvent::Behaviour(MingaBehaviourEvent::Identify(
                                identify::Event::Received { peer_id, info, .. }
                            )) => {
                                for addr in info.listen_addrs {
                                    swarm.behaviour_mut().kad.add_address(&peer_id, addr);
                                }
                            }
                            SwarmEvent::Behaviour(MingaBehaviourEvent::Kad(
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
            cmd_tx,
            listen_rx: Mutex::new(listen_rx),
            control,
        })
    }

    pub async fn listen(&self, addr: Multiaddr) -> Multiaddr {
        self.cmd_tx
            .send(Command::Listen(addr))
            .expect("swarm task alive");
        let mut rx = self.listen_rx.lock().await;
        rx.recv().await.expect("listen address arrives")
    }

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
    /// `find_providers(key)`. Best-effort: si la replicación falla
    /// inicialmente, el record vive en el store local.
    pub fn start_providing(&self, key: &[u8]) {
        let _ = self.cmd_tx.send(Command::StartProviding(key.to_vec()));
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
