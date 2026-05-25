//! Re-export del nodo de la red Brahman especializado para Minga.
//!
//! Antes este módulo contenía el swarm libp2p completo. Ahora vive en
//! `brahman-net` (capa P2P compartida con el resto de la familia
//! brahman: `/brahman/handshake/1.0.0`, futuros sub-protocolos). Este
//! módulo se reduce a:
//!
//! - Re-exportar `BrahmanNet` bajo el alias histórico `LibP2pNode`
//!   para zero churn en `MingaPeer`.
//! - Declarar la const `SYNC_PROTOCOL` específica de Minga
//!   (`/minga/sync/1.0.0`).
//!
//! Cualquier consumer que necesite armar un nodo P2P puede importar
//! `brahman_net::BrahmanNet` directo y registrar sus propios protocolos
//! sin pasar por minga.

pub use brahman_net::{BrahmanNet as LibP2pNode, DiscoveredPeer, NodeError};

use libp2p::StreamProtocol;

/// Sub-protocolo de sync Minga sobre la malla brahman-net.
pub const SYNC_PROTOCOL: StreamProtocol = StreamProtocol::new("/minga/sync/1.0.0");
