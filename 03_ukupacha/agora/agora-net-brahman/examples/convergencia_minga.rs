//! Demo de convergencia minga + ágora sobre **un solo** `BrahmanNet`.
//!
//! Muestra que el mismo nodo libp2p (mismo PeerId, misma Kademlia)
//! puede servir simultáneamente `/minga/sync/1.0.0` y
//! `/agora/gossip/1.0.0`. Esa es la convergencia que el README de ágora
//! anuncia: *"agora corre sobre la red de pares de minga cuando ambos
//! están activos"*.
//!
//! Corre con:
//!
//! ```sh
//! cargo run -p agora-net-brahman --example convergencia_minga
//! ```
//!
//! La demo no necesita conectividad — usa loopback. La intención es
//! puramente ilustrativa: arma el setup compartido, registra ambos
//! protocolos, y reporta los PeerIds resultantes.

use std::sync::Arc;

use agora_core::{IdentityKind, Keypair as AgoraKeypair};
use agora_graph::TrustGraph;
use agora_net_brahman::{AgoraNet, GOSSIP_PROTOCOL};
use card_net::BrahmanNet;
use minga_core::Keypair as MingaKeypair;
use minga_p2p::{network::SYNC_PROTOCOL, MingaPeer};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Un solo nodo libp2p compartido.
    let net = Arc::new(BrahmanNet::new()?);
    let peer_id = net.peer_id;
    println!("Nodo libp2p compartido:");
    println!("  PeerID: {peer_id}");

    // 2. MingaPeer adopta el mismo nodo.
    let minga_repo = tempfile::tempdir()?;
    let minga = MingaPeer::open_with_node(
        MingaKeypair::generate(),
        minga_repo.path(),
        Arc::clone(&net),
    )?;
    let _minga_accept = minga.run_passive_accept();
    println!("\n· minga registró {SYNC_PROTOCOL}");
    println!("  DID: {}", minga.peer_id());

    // 3. AgoraNet adopta el MISMO nodo, sin abrir otro socket.
    let yumaira = AgoraKeypair::from_seed([20; 32]);
    let mut graph = TrustGraph::new();
    graph.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
    let agora = AgoraNet::sharing(Arc::clone(&net), graph);
    let _agora_accept = agora.run_passive_accept();
    println!("\n· ágora registró {GOSSIP_PROTOCOL}");
    println!("  PeerID (mismo): {}", agora.peer_id());

    // 4. Listen sobre loopback. La dirección sirve para ambos
    //    protocolos — los demultiplexa libp2p stream behaviour.
    let addr = minga
        .listen("/ip4/127.0.0.1/tcp/0".parse()?)
        .await;
    println!("\nEscuchando en {addr}");
    println!("  /p2p/{peer_id}/<protocolo>");
    println!("  donde <protocolo> ∈ {{ {SYNC_PROTOCOL}, {GOSSIP_PROTOCOL} }}");

    println!("\nUn solo nodo, dos protocolos. Convergencia: ✓");

    Ok(())
}
