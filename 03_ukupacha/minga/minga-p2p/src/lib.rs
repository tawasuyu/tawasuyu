//! minga-p2p: protocolo de sincronización entre repositorios Minga.
//!
//! Este crate define el **protocolo** y la **máquina de estados** de la
//! sincronización P2P, sin acoplarse a un transporte concreto. Un peer
//! manipula una `SyncSession` (puramente lógica) que consume mensajes
//! entrantes y produce mensajes salientes; el transporte real —libp2p,
//! HTTP, in-memory, lo que sea— se reduce a serializar/deserializar y
//! mover bytes.
//!
//! Este orden refleja el principio bottom-up del proyecto: validamos la
//! convergencia del protocolo con un `harness` in-memory determinístico
//! antes de invertir en async runtime + libp2p.

pub mod async_driver;
pub mod harness;
pub mod message;
pub mod network;
pub mod peer;
pub mod session;

pub use async_driver::{run_sync_async, AsyncSyncError};
pub use harness::{run_sync, SyncStats};
pub use message::Message;
pub use network::{DiscoveredPeer, LibP2pNode, NodeError, SYNC_PROTOCOL};
pub use peer::{MingaPeer, PeerOpenError, PeerSyncError};
pub use session::SyncSession;
