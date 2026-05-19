//! `subscriber` — cliente brahman que loguea cada `MatchEvent` recibido.
//!
//! Declara una Card con un input `in` de tipo `json`. Cada vez que el
//! broker matchea (o desmatch) ese input contra un productor, imprime
//! una línea. Útil para visualizar la dinámica del broker en vivo.
//!
//! Uso:
//! ```sh
//! cargo run -p brahman-handshake --example subscriber [label]
//! ```

use std::collections::BTreeSet;
use std::time::Duration;

use brahman_card::{
    ulid::Ulid, Card, Flow, Flows, Lifecycle, Payload, Priority, Supervision, TypeRef,
    CARD_SCHEMA_VERSION,
};
use brahman_handshake::{client::Client, transport};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let label = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "subscriber".into());

    let card = Card {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        label: label.clone(),
        provides: BTreeSet::new(),
        requires: BTreeSet::new(),
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        lifecycle: Lifecycle::Daemon,
        priority: Priority::Normal,
        flow: Flows {
            input: vec![Flow {
                name: "in".into(),
                ty: TypeRef::Primitive {
                    name: "json".into(),
                },
                pin_to: None,
            }],
            output: vec![],
        },
        ..Default::default()
    };

    let path = transport::default_socket_path();
    eprintln!("[{label}] connecting to {}", path.display());
    let mut client = Client::connect(&path, card).await?;
    eprintln!(
        "[{label}] attached: session={} init={}",
        client.session(),
        client.server_info().init_attached
    );

    // Loop: espera hasta 25s por un MatchEvent. Si timeout, ping para
    // mantener la conexión viva.
    loop {
        match client.await_event(Duration::from_secs(25)).await? {
            Some(ev) => {
                eprintln!(
                    "[{label}] {:?}  {}  ←  {}.{}  via={:?}{}",
                    ev.kind,
                    ev.consumer_flow,
                    if ev.producer_label.is_empty() {
                        "<none>"
                    } else {
                        &ev.producer_label
                    },
                    ev.producer_flow,
                    ev.via,
                    if ev.pinned { "  📌" } else { "" }
                );
            }
            None => {
                let _ts = client.ping().await?;
            }
        }
    }
}
