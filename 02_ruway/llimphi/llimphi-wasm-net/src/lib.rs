//! llimphi-wasm-net — transporte P2P para distribución de apps WASM Tier 3.
//!
//! Un protocolo de stream mínimo sobre `BrahmanNet` (card-net, la malla libp2p
//! TCP+Noise+Yamux+Kad compartida):
//!
//! - [`serve_blobs`] levanta un responder que, por cada stream entrante en
//!   `/llimphi-wasm/blob/1.0.0`, atiende un `Get{hash}` sirviendo el bytecode
//!   de un [`DiskStore`] (o `NotFound`).
//! - [`fetch_blob`] abre un stream a un provider y pide un blob por hash.
//! - [`fetch_blob_dht`] descubre providers por la DHT y prueba con cada uno.
//! - [`RemoteSource`] envuelve lo anterior como un [`BlobSource`] síncrono, para
//!   que la cadena `resolve→verificar` de `llimphi-wasm-dist` corra sobre la red
//!   sin enterarse del transporte.
//!
//! El framing copia el de minga (`[u32 LE len][postcard]`). La integridad NO se
//! confía a la red: `llimphi-wasm-dist::resolve` rehashea el blob recibido
//! contra el hash pedido, así un provider malicioso no puede colar otro wasm.

use std::sync::Arc;
use std::time::{Duration, Instant};

use card_net::{BrahmanNet, PeerId, Stream, StreamProtocol};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use serde::{Deserialize, Serialize};

pub use llimphi_wasm_core::{BlobSource, DiskStore, Hash};

/// El protocolo de stream para pedir bytecodes por hash.
pub const BLOB_PROTOCOL: StreamProtocol = StreamProtocol::new("/llimphi-wasm/blob/1.0.0");

/// Tope de un blob en la red (64 MiB). Un wasm de app real está muy por debajo.
const MAX_BLOB: u32 = 64 * 1024 * 1024;

/// Mensajes del protocolo. Un request, una respuesta, stream cerrado.
#[derive(Debug, Serialize, Deserialize)]
enum BlobMsg {
    Get { hash: Hash },
    Data { hash: Hash, data: Vec<u8> },
    NotFound { hash: Hash },
}

/// Errores del transporte.
#[derive(Debug)]
pub enum NetError {
    /// No se pudo abrir el stream al provider.
    OpenStream(String),
    /// El provider no tiene el blob.
    NotFound,
    /// Respuesta fuera de protocolo.
    Protocol,
    /// Ningún provider entregó el blob.
    SinProviders,
    /// Error de I/O sobre el stream.
    Io(std::io::Error),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::OpenStream(e) => write!(f, "abrir stream: {e}"),
            NetError::NotFound => write!(f, "el provider no tiene el blob"),
            NetError::Protocol => write!(f, "respuesta fuera de protocolo"),
            NetError::SinProviders => write!(f, "ningún provider entregó el blob"),
            NetError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for NetError {}

// =====================================================================
// Framing — [u32 LE len][postcard], igual que minga
// =====================================================================

async fn send_msg<S: futures::AsyncWrite + Unpin>(s: &mut S, m: &BlobMsg) -> std::io::Result<()> {
    let bytes = postcard::to_allocvec(m).expect("postcard encode");
    let len = bytes.len() as u32;
    s.write_all(&len.to_le_bytes()).await?;
    s.write_all(&bytes).await?;
    s.flush().await?;
    Ok(())
}

async fn recv_msg<S: futures::AsyncRead + Unpin>(s: &mut S) -> std::io::Result<BlobMsg> {
    let mut len_buf = [0u8; 4];
    s.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_BLOB {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "blob excede el tope",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    s.read_exact(&mut buf).await?;
    postcard::from_bytes(&buf)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "postcard inválido"))
}

// =====================================================================
// Responder
// =====================================================================

/// Levanta el responder de blobs sobre `net`: por cada stream entrante en
/// [`BLOB_PROTOCOL`], sirve el bytecode pedido desde `store`. Devuelve el
/// `JoinHandle` del loop de aceptación (vive mientras se sirva).
///
/// Llamar UNA vez por `net` (una sola `accept` por protocolo).
pub fn serve_blobs(net: &BrahmanNet, store: Arc<DiskStore>) -> tokio::task::JoinHandle<()> {
    let mut control = net.control.clone();
    tokio::spawn(async move {
        let mut incoming = match control.accept(BLOB_PROTOCOL) {
            Ok(i) => i,
            Err(_) => return, // ya hay otro handle aceptando este protocolo
        };
        while let Some((_peer, stream)) = incoming.next().await {
            let store = Arc::clone(&store);
            tokio::spawn(serve_one(stream, store));
        }
    })
}

async fn serve_one(mut stream: Stream, store: Arc<DiskStore>) {
    if let Ok(BlobMsg::Get { hash }) = recv_msg(&mut stream).await {
        let reply = match store.get(&hash) {
            Some(data) => BlobMsg::Data { hash, data },
            None => BlobMsg::NotFound { hash },
        };
        let _ = send_msg(&mut stream, &reply).await;
        let _ = stream.close().await;
    }
}

/// Anuncia en la DHT que este nodo provee un bytecode (para [`fetch_blob_dht`]).
pub fn announce(net: &BrahmanNet, hash: &Hash) {
    net.start_providing(hash);
}

// =====================================================================
// Cliente
// =====================================================================

/// Pide un blob por hash a un provider concreto. Reintenta abrir el stream
/// hasta 5 s (la conexión puede no estar lista al instante tras un `dial`).
pub async fn fetch_blob(net: &BrahmanNet, peer: PeerId, hash: &Hash) -> Result<Vec<u8>, NetError> {
    let mut control = net.control.clone();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match control.open_stream(peer, BLOB_PROTOCOL).await {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(NetError::OpenStream(e.to_string())),
        }
    };
    send_msg(&mut stream, &BlobMsg::Get { hash: *hash })
        .await
        .map_err(NetError::Io)?;
    match recv_msg(&mut stream).await.map_err(NetError::Io)? {
        BlobMsg::Data { data, .. } => Ok(data),
        BlobMsg::NotFound { .. } => Err(NetError::NotFound),
        _ => Err(NetError::Protocol),
    }
}

/// Descubre providers del `hash` por la DHT y pide el blob a cada uno hasta que
/// alguno lo entregue.
pub async fn fetch_blob_dht(net: &BrahmanNet, hash: &Hash) -> Result<Vec<u8>, NetError> {
    let providers = net.find_providers(hash).await;
    for peer in providers {
        if let Ok(data) = fetch_blob(net, peer, hash).await {
            return Ok(data);
        }
    }
    Err(NetError::SinProviders)
}

// =====================================================================
// Puente síncrono: BlobSource sobre la red
// =====================================================================

/// Adapta el fetch P2P (async) al trait síncrono [`BlobSource`], para componerlo
/// con `llimphi_wasm_dist::{resolve, LayeredSource}`. Usa la DHT para descubrir
/// providers.
///
/// `fetch` hace `block_on` sobre el handle del runtime, así que **no debe
/// llamarse desde dentro de ese runtime** (sí desde el loop síncrono del runner
/// Llimphi, que no es async). Componer con un [`DiskStore`] vía `LayeredSource`
/// para servir desde caché local primero y caer a la red sólo si falta.
pub struct RemoteSource {
    net: Arc<BrahmanNet>,
    rt: tokio::runtime::Handle,
}

impl RemoteSource {
    pub fn new(net: Arc<BrahmanNet>, rt: tokio::runtime::Handle) -> Self {
        Self { net, rt }
    }
}

impl BlobSource for RemoteSource {
    fn fetch(&self, hash: &Hash) -> Option<Vec<u8>> {
        self.rt.block_on(fetch_blob_dht(&self.net, hash)).ok()
    }
}
