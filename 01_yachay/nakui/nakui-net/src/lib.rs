//! `nakui-net` — transporte de red soberano para nakui sobre `card-net`
//! (libp2p: TCP+Noise+yamux, Kademlia, mDNS, relay/DCUtR/AutoNAT).
//!
//! Es la fase 2 de "nakui en red": el seam abierto en [`nakui_sync`] (el
//! trait [`Transport`](nakui_sync::Transport)) se enchufa ahora a la red.
//! Dos piezas:
//!
//! - [`serve`] — levanta el **servidor**: hospeda un escritor autoritativo
//!   (vía un [`LocalTransport`](nakui_sync::LocalTransport)) y lo sirve a
//!   clientes remotos. Es un puente card-net ↔ LocalTransport: una
//!   intención remota se valida/ordena/materializa igual que una local, y
//!   cada commit se difunde a todos los conectados.
//! - [`CardNetTransport`] — el **cliente**: implementa el mismo trait
//!   `Transport` que `LocalTransport`, hablando con un servidor por la red.
//!   La UI no sabe si está co-locada o remota.
//!
//! Soberano y sin segundo protocolo: en LAN, card-net conecta directo
//! (mDNS, sin relay); en WAN, NAT-traversal o relay — mismo código, mismo
//! `CardNetTransport`. La identidad es la PeerId Ed25519 del nodo.
//!
//! El puente sync↔async calca el patrón canónico de `ayni-minga`: un hilo
//! dedicado corre un runtime tokio; el API sync manda comandos por canal.

mod client;
mod server;
mod wire;

pub use client::CardNetTransport;
pub use server::{serve, ServerHandle};
pub use wire::{ClientMsg, ServerMsg};

use card_net::{Stream as LpStream, StreamProtocol};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::compat::Compat;

/// Protocolo libp2p del transporte de Nakui. Multiplexado sobre el mismo
/// nodo que cualquier otro protocolo de la suite.
pub(crate) const PROTO: StreamProtocol = StreamProtocol::new("/nakui/sync/1.0.0");

/// Techo de un frame serializado: 64 MiB. Un commit puede acarrear un
/// snapshot grande tras un seed masivo, pero no más que eso.
pub(crate) const MAX_FRAME: usize = 64 * 1024 * 1024;

pub(crate) type CompatStream = Compat<LpStream>;

/// Falla de construcción o uso del transporte de red.
#[derive(Debug, thiserror::Error)]
pub enum ErrorNet {
    #[error("nakui-net :: fallo al arrancar el nodo libp2p: {0}")]
    Arranque(String),
    #[error("nakui-net :: no se pudo conectar al servidor: {0}")]
    Conexion(String),
    #[error("nakui-net :: el runtime de red está cerrado")]
    Cerrado,
}

/// Lee un frame `[u32 LE len][payload]`. `Err` en EOF, frame corrupto o
/// tamaño fuera de rango.
pub(crate) async fn leer_frame<R: AsyncReadExt + Unpin>(rd: &mut R) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    rd.read_exact(&mut len).await?;
    let n = u32::from_le_bytes(len) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame fuera de rango",
        ));
    }
    let mut buf = vec![0u8; n];
    rd.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Escribe un frame `[u32 LE len][payload]` y hace flush.
pub(crate) async fn escribir_frame<W: AsyncWriteExt + Unpin>(
    wr: &mut W,
    bytes: &[u8],
) -> std::io::Result<()> {
    wr.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    wr.write_all(bytes).await?;
    wr.flush().await?;
    Ok(())
}
