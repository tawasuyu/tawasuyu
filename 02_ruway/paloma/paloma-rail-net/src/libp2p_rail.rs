//! `Libp2pRail` — el transporte del rail sobre **libp2p** (card-net): discovery
//! por **DHT Kademlia** + NAT traversal (relay/dcutr/autonat), sin conocer la
//! dirección del peer de antemano.
//!
//! Modelo *push*, espejo de `khipu-brahman` pero por **identidad**: cada nodo
//! **se anuncia** en la DHT bajo su propia `RailId` (clave pública). Para
//! mandarle a `to`, el emisor hace `find_providers(to)` → obtiene el `PeerId` →
//! abre un stream `/paloma/rail/1.0.0` → escribe el sobre. La recepción acepta
//! streams y empuja los sobres por un canal que el anfitrión drena (abre/
//! verifica/despacha), igual que [`crate::TcpRail`].
//!
//! La identidad libp2p se deriva de la **misma seed** que la identidad agora,
//! así el `PeerId` es estable entre arranques y atado a la persona.
//!
//! Reusa `card-net` tal cual (no lo modifica); el sobre va firmado, así que el
//! transporte no necesita ser confiable.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use card_net::{BrahmanNet, Keypair, Multiaddr, PeerId, StreamProtocol};
use futures::StreamExt;
use paloma_rail::{RailEnvelope, RailError, RailId, RailTransport};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Runtime;
use tokio_util::compat::FuturesAsyncReadCompatExt;

/// Protocolo de stream para un sobre del rail.
pub const RAIL_PROTOCOL: StreamProtocol = StreamProtocol::new("/paloma/rail/1.0.0");

/// Tope de un sobre entrante (16 MiB).
const MAX_FRAME: u32 = 16 * 1024 * 1024;

fn err<E: std::fmt::Display>(e: E) -> RailError {
    RailError::Transport(e.to_string())
}

/// Transporte libp2p del rail. Posee su propio runtime tokio (el swarm corre en
/// una task) + el `BrahmanNet` compartido de la suite.
pub struct Libp2pRail {
    rt: Runtime,
    net: Arc<BrahmanNet>,
    me: RailId,
}

impl Libp2pRail {
    /// Crea el nodo libp2p con identidad derivada de `seed` y arranca el bucle
    /// de aceptación de streams. Devuelve el transporte y el receptor de sobres
    /// entrantes (que el anfitrión drena). No se anuncia todavía: llamá
    /// [`Self::announce`] tras unirte a la malla (dial/listen).
    pub fn new(seed: [u8; 32], me: RailId) -> Result<(Self, Receiver<RailEnvelope>), RailError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(err)?;

        let (net, rx) = {
            let _guard = rt.enter(); // BrahmanNet::new spawnea el swarm: necesita runtime
            let mut s = seed;
            let kp = Keypair::ed25519_from_bytes(&mut s).map_err(err)?;
            let net = Arc::new(BrahmanNet::with_keypair(kp).map_err(err)?);

            // Bucle de aceptación: cada sobre entrante va al canal.
            let (tx, rx): (Sender<RailEnvelope>, Receiver<RailEnvelope>) = channel();
            let mut control = net.control.clone();
            rt.spawn(async move {
                let mut incoming = match control.accept(RAIL_PROTOCOL) {
                    Ok(i) => i,
                    Err(_) => return, // ya hay un accept para este protocolo
                };
                while let Some((_peer, stream)) = incoming.next().await {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let mut compat = stream.compat();
                        if let Ok(bytes) = read_frame(&mut compat).await {
                            if let Ok(env) = RailEnvelope::from_bytes(&bytes) {
                                let _ = tx.send(env);
                            }
                        }
                    });
                }
            });
            (net, rx)
        };

        Ok((Self { rt, net, me }, rx))
    }

    /// Identidad de red (libp2p) de este nodo.
    pub fn peer_id(&self) -> PeerId {
        self.net.peer_id
    }

    /// Empieza a escuchar en `addr` (multiaddr, p. ej. `/ip4/0.0.0.0/tcp/7720`).
    /// Devuelve la **dirección para compartir** (con `/p2p/<peer-id>`).
    pub fn listen(&self, addr: &str) -> Result<String, RailError> {
        let m: Multiaddr = addr.parse().map_err(err)?;
        let bound = self.rt.block_on(self.net.listen(m));
        let s = bound.to_string();
        Ok(if s.contains("/p2p/") {
            s
        } else {
            format!("{s}/p2p/{}", self.net.peer_id)
        })
    }

    /// Marca a un peer (multiaddr con `/p2p/<id>`) para unirse a la malla /
    /// bootstrap de la DHT.
    pub fn dial(&self, addr: &str) -> Result<(), RailError> {
        let m: Multiaddr = addr.parse().map_err(err)?;
        self.net.dial(m);
        Ok(())
    }

    /// Anuncia esta identidad en la DHT (otros la encuentran con
    /// `find_providers(mi RailId)`). Llamar tras unirse a la malla.
    pub fn announce(&self) {
        self.net.start_providing(&self.me);
    }

    /// Resuelve una identidad a sus `PeerId`s vía la DHT (para diagnóstico/tests).
    pub fn resolve(&self, to: &RailId) -> Vec<PeerId> {
        self.rt.block_on(self.net.find_providers(to))
    }
}

impl RailTransport for Libp2pRail {
    fn send(&self, to: RailId, envelope: &RailEnvelope) -> Result<(), RailError> {
        let net = self.net.clone();
        let bytes = envelope.to_bytes()?;
        // Async, sin bloquear la UI: resolver por DHT + abrir stream + escribir.
        self.rt.spawn(async move {
            let providers = net.find_providers(&to).await;
            if providers.is_empty() {
                eprintln!("paloma · rail libp2p: identidad destino no encontrada en la DHT");
                return;
            }
            // `find_providers` sólo da PeerIds; para poder abrir el stream el
            // swarm necesita la dirección del peer. La resolvemos por la DHT
            // (`find_closest_peers`) y la sembramos.
            // Resolver direcciones del/los proveedor(es) por la DHT y sembrarlas
            // (en mallas con relay/identify esto las puebla; si no, el anfitrión
            // las aporta vía dial al rendezvous/relay).
            for peer in &providers {
                for dp in net.find_closest_peers(*peer).await {
                    for addr in dp.addrs {
                        net.add_dht_peer(dp.peer_id, addr);
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            for peer in providers {
                let mut control = net.control.clone();
                if let Ok(stream) = control.open_stream(peer, RAIL_PROTOCOL).await {
                    let mut compat = stream.compat();
                    if write_frame(&mut compat, &bytes).await.is_ok() {
                        return; // entregado
                    }
                }
            }
            eprintln!("paloma · rail libp2p: no se pudo abrir stream a ningún proveedor");
        });
        Ok(())
    }
}

async fn write_frame<S>(stream: &mut S, payload: &[u8]) -> Result<(), RailError>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    if payload.len() as u64 > MAX_FRAME as u64 {
        return Err(RailError::Transport("sobre demasiado grande".into()));
    }
    stream.write_all(&(payload.len() as u32).to_be_bytes()).await.map_err(err)?;
    stream.write_all(payload).await.map_err(err)?;
    stream.flush().await.map_err(err)?;
    Ok(())
}

async fn read_frame<S>(stream: &mut S) -> Result<Vec<u8>, RailError>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await.map_err(err)?;
    let n = u32::from_be_bytes(len);
    if n == 0 || n > MAX_FRAME {
        return Err(RailError::Transport("marco inválido".into()));
    }
    let mut buf = vec![0u8; n as usize];
    stream.read_exact(&mut buf).await.map_err(err)?;
    Ok(buf)
}
