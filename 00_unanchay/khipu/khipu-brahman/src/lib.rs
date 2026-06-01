//! `khipu-brahman` — transporte de sobres khipu sobre libp2p, vía la capa
//! P2P compartida [`card_net::BrahmanNet`] (TCP + Noise + Yamux + Kademlia).
//!
//! Es el hermano WAN del transporte LAN de `khipu-share::net`: en vez de un
//! `TcpStream` directo, abre un stream libp2p sobre el protocolo
//! [`SOBRE_PROTOCOL`] y manda el mismo sobre serializado. Como el sobre va
//! firmado y direccionado por contenido, **el transporte no necesita ser
//! confiable**: quien recibe verifica con [`khipu_share::open`] antes de
//! ingerir. El cifrado Noise sólo agrega confidencialidad en tránsito.
//!
//! El descubrimiento es por DHT Kademlia: [`KhipuNode::anunciar`] publica
//! bajo una clave fija y [`KhipuNode::descubrir`] lista a quién la provee —
//! rendezvous sin saber la `Multiaddr` de antemano (hace falta al menos un
//! peer bootstrap en la tabla, vía [`KhipuNode::add_peer`]). La travesía de
//! NAT (relay + dcutr + autonat) **sí** está cableada en `BrahmanNet`
//! (`shared/card/card-net`); ver el test `jalar_a_traves_de_un_relay` en
//! `tests/p2p_roundtrip.rs`, que jala un sobre a través de un circuito relay.
//! Detrás de NAT simétrico todavía conviene un relay público alcanzable.
//!
//! Marco de cable: `u32` big-endian con el largo del sobre + el sobre
//! (postcard). Un sobre por stream — espejo del marco de `khipu-share::net`.

use std::sync::Arc;
use std::time::Duration;

use card_net::{BrahmanNet, Multiaddr, PeerId, Protocol};
use futures::StreamExt;
use khipu_share::SignedBundle;
use libp2p::StreamProtocol;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::compat::FuturesAsyncReadCompatExt;

/// Protocolo de stream libp2p para transferir un sobre khipu.
pub const SOBRE_PROTOCOL: StreamProtocol = StreamProtocol::new("/khipu/sobre/1.0.0");

/// Clave DHT bajo la que los cuadernos khipu se anuncian y se descubren.
const KHIPU_DHT_KEY: &[u8] = b"khipu/sobre/1.0.0";

/// Tope defensivo de un sobre entrante (64 MiB), igual que el transporte LAN.
const MAX_SOBRE: u32 = 64 * 1024 * 1024;

/// Falla del transporte libp2p de sobres.
#[derive(Debug, thiserror::Error)]
pub enum BrahmanError {
    #[error("nodo libp2p: {0}")]
    Nodo(String),
    #[error("abrir stream: {0}")]
    Stream(String),
    #[error("marco inválido: {0}")]
    Marco(String),
    #[error("io de red: {0}")]
    Io(String),
    #[error("sobre ilegible: {0}")]
    Sobre(String),
}

/// Un cuaderno khipu en la red P2P. Envoltura fina sobre [`BrahmanNet`]
/// que habla el protocolo de sobres.
pub struct KhipuNode {
    net: Arc<BrahmanNet>,
}

impl KhipuNode {
    /// Crea un nodo con su propio [`BrahmanNet`] (keypair efímera). Debe
    /// llamarse dentro de un runtime de tokio — el swarm corre en una task.
    pub fn standalone() -> Result<Self, BrahmanError> {
        let net = BrahmanNet::new().map_err(|e| BrahmanError::Nodo(e.to_string()))?;
        Ok(Self { net: Arc::new(net) })
    }

    /// Comparte un [`BrahmanNet`] ya existente (p. ej. el mismo nodo que
    /// usan agora/minga), registrando el protocolo de sobres encima.
    pub fn sharing(net: Arc<BrahmanNet>) -> Self {
        Self { net }
    }

    /// Identidad de red de este nodo.
    pub fn peer_id(&self) -> PeerId {
        self.net.peer_id
    }

    /// Empieza a escuchar en `addr`; devuelve la dirección efectiva.
    pub async fn listen(&self, addr: Multiaddr) -> Multiaddr {
        self.net.listen(addr).await
    }

    /// Escucha en una multiaddr dada como texto y devuelve la **dirección
    /// para compartir** ya con `/p2p/<peer-id>` — lista para pegarle a un
    /// par. Pensada para callers que no quieren tocar tipos de libp2p.
    pub async fn listen_str(&self, addr: &str) -> Result<String, BrahmanError> {
        let m: Multiaddr = addr
            .parse()
            .map_err(|e| BrahmanError::Nodo(format!("multiaddr inválida: {e}")))?;
        let bound = self.net.listen(m).await;
        let s = bound.to_string();
        // Una reserva de circuito ya viene con `…/p2p-circuit/p2p/<self>`:
        // no le agregamos otro `/p2p/`. Un tcp pelado no trae peer-id, así
        // que sí se lo anexamos para que sea una dirección de marcado.
        if s.contains("/p2p/") {
            Ok(s)
        } else {
            Ok(format!("{s}/p2p/{}", self.net.peer_id))
        }
    }

    /// Marca a un par para conectarse.
    pub fn dial(&self, addr: Multiaddr) {
        self.net.dial(addr);
    }

    /// Marca a un par dado como texto (multiaddr).
    pub fn dial_str(&self, addr: &str) -> Result<(), BrahmanError> {
        let m: Multiaddr = addr
            .parse()
            .map_err(|e| BrahmanError::Nodo(format!("multiaddr inválida: {e}")))?;
        self.net.dial(m);
        Ok(())
    }

    /// Siembra la tabla DHT con un par conocido (bootstrap para descubrir).
    pub fn add_peer(&self, peer: PeerId, addr: Multiaddr) {
        self.net.add_dht_peer(peer, addr);
    }

    /// Anuncia que este nodo sirve un cuaderno khipu (clave DHT fija).
    pub fn anunciar(&self) {
        self.net.start_providing(KHIPU_DHT_KEY);
    }

    /// Descubre nodos khipu por DHT (sin incluirse a sí mismo).
    pub async fn descubrir(&self) -> Vec<PeerId> {
        let mut peers = self.net.find_providers(KHIPU_DHT_KEY).await;
        let me = self.net.peer_id;
        peers.retain(|p| *p != me);
        peers
    }

    /// Sirve sobres: por cada stream entrante llama a `supply` y manda lo
    /// que devuelva (típicamente leer `compartido.khipu`). `None` ⇒ no hay
    /// sobre, cierra el stream. Bloqueante: corre en su propia task hasta
    /// el shutdown del nodo.
    pub fn run_serve<F>(&self, supply: F) -> tokio::task::JoinHandle<()>
    where
        F: Fn() -> Option<Vec<u8>> + Send + Sync + 'static,
    {
        let mut control = self.net.control.clone();
        tokio::spawn(async move {
            let mut incoming = match control.accept(SOBRE_PROTOCOL) {
                Ok(i) => i,
                Err(_) => return, // ya hay un accept para este protocolo
            };
            let supply = Arc::new(supply);
            while let Some((_peer, stream)) = incoming.next().await {
                let supply = Arc::clone(&supply);
                tokio::spawn(async move {
                    if let Some(bytes) = supply() {
                        let mut compat = stream.compat();
                        let _ = write_frame(&mut compat, &bytes).await;
                    }
                });
            }
        })
    }

    /// Jala el sobre de un par dado por su **dirección de marcado** como
    /// texto (`/ip4/.../tcp/.../p2p/<peer-id>`): extrae el peer-id, dial-ea
    /// y reintenta el fetch unos segundos mientras se establece la
    /// conexión. La forma que usa la app.
    pub async fn fetch_addr_str(&self, addr: &str) -> Result<SignedBundle, BrahmanError> {
        let m: Multiaddr = addr
            .parse()
            .map_err(|e| BrahmanError::Nodo(format!("multiaddr inválida: {e}")))?;
        let peer = peer_from_multiaddr(&m)
            .ok_or_else(|| BrahmanError::Nodo("la multiaddr no trae /p2p/<peer-id>".into()))?;
        self.net.dial(m);
        let mut intentos = 0u32;
        loop {
            match self.fetch(peer).await {
                Ok(s) => return Ok(s),
                Err(_) if intentos < 60 => {
                    intentos += 1;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Jala el sobre de un par dado por su **peer-id** como texto (la forma
    /// que devuelve [`descubrir`](Self::descubrir)). Reintenta mientras el
    /// swarm establece la conexión usando las direcciones que aprendió por
    /// la DHT/identify. La app la usa para jalar de un par descubierto.
    pub async fn fetch_peer_str(&self, peer: &str) -> Result<SignedBundle, BrahmanError> {
        let pid: PeerId = peer
            .parse()
            .map_err(|e| BrahmanError::Nodo(format!("peer-id inválido: {e}")))?;
        let mut intentos = 0u32;
        loop {
            match self.fetch(pid).await {
                Ok(s) => return Ok(s),
                Err(_) if intentos < 60 => {
                    intentos += 1;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Abre un stream a `peer` y jala su sobre. **No lo verifica** — el
    /// caller debe pasarlo por [`khipu_share::open`] antes de confiar.
    pub async fn fetch(&self, peer: PeerId) -> Result<SignedBundle, BrahmanError> {
        let mut control = self.net.control.clone();
        let stream = control
            .open_stream(peer, SOBRE_PROTOCOL)
            .await
            .map_err(|e| BrahmanError::Stream(e.to_string()))?;
        let mut compat = stream.compat();
        let bytes = read_frame(&mut compat).await?;
        SignedBundle::from_bytes(&bytes).map_err(|e| BrahmanError::Sobre(format!("{e:?}")))
    }
}

/// Extrae el `PeerId` **destino** de una multiaddr: el ÚLTIMO componente
/// `/p2p/<id>`. En una directa (`…/tcp/P/p2p/<peer>`) es el único; en un
/// circuito (`…/p2p/<relay>/p2p-circuit/p2p/<destino>`) es el de después
/// del relay, no el relay.
fn peer_from_multiaddr(addr: &Multiaddr) -> Option<PeerId> {
    addr.iter()
        .filter_map(|p| match p {
            Protocol::P2p(id) => Some(id),
            _ => None,
        })
        .last()
}

async fn write_frame<S>(stream: &mut S, payload: &[u8]) -> Result<(), BrahmanError>
where
    S: AsyncWrite + Unpin,
{
    if payload.len() as u64 > MAX_SOBRE as u64 {
        return Err(BrahmanError::Marco("sobre demasiado grande".into()));
    }
    stream
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .map_err(|e| BrahmanError::Io(e.to_string()))?;
    stream
        .write_all(payload)
        .await
        .map_err(|e| BrahmanError::Io(e.to_string()))?;
    stream.flush().await.map_err(|e| BrahmanError::Io(e.to_string()))?;
    Ok(())
}

async fn read_frame<S>(stream: &mut S) -> Result<Vec<u8>, BrahmanError>
where
    S: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| BrahmanError::Io(e.to_string()))?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_SOBRE {
        return Err(BrahmanError::Marco(format!("largo {len} excede el tope")));
    }
    let mut buf = vec![0u8; len as usize];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| BrahmanError::Io(e.to_string()))?;
    Ok(buf)
}
