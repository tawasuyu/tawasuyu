//! probe — herramienta de diagnóstico del handshake.
//!
//! Conecta a un Init brahman vivo, hace handshake, un ping, y se va.
//! Ruta del socket: `$BRAHMAN_INIT_SOCKET` o el default
//! ([`brahman_handshake::transport::default_socket_path`]).
//!
//! Uso:
//! ```sh
//! cargo run -p brahman-handshake --example probe
//! ```

use std::collections::BTreeSet;

use brahman_card::{Card, Payload, Supervision, CARD_SCHEMA_VERSION};
use brahman_handshake::{client::Client, transport};
use ulid::Ulid;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let card = Card {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        label: "brahman-probe".into(),
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        provides: BTreeSet::new(),
        requires: BTreeSet::new(),
        ..Default::default()
    };

    let path = transport::default_socket_path();
    println!("connecting to {}", path.display());

    let mut client = Client::connect(&path, card).await?;
    let info = client.server_info();
    println!(
        "  HelloAck: session={} server={} protocol={} init_attached={}",
        client.session(),
        info.server_version,
        info.protocol_version,
        info.init_attached
    );

    let ts = client.ping().await?;
    println!("  Pong: ts={}ms", ts);

    client.farewell().await?;
    println!("  Farewell OK");

    Ok(())
}
