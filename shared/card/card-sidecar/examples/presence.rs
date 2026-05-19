//! `presence` — módulo brahman dummy para pruebas y demos.
//!
//! Declara una Card mínima con label tomado del primer argumento (default
//! `presence-default`) y mantiene la sesión viva hasta SIGTERM/SIGINT.
//! Útil para poblar el broker con sesiones de prueba.
//!
//! Uso:
//! ```sh
//! cargo run -p brahman-sidecar --example presence -- mi-modulo
//! ```

use std::collections::BTreeSet;
use std::time::Duration;

use brahman_card::{
    ulid::Ulid, Card, Flow, Flows, Lifecycle, Payload, Priority, Supervision, TypeRef,
    CARD_SCHEMA_VERSION,
};
use brahman_sidecar::{spawn_with_handle, SidecarConfig};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let label = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "presence-default".into());

    let card = Card {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        label: label.clone(),
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        lifecycle: Lifecycle::Daemon,
        priority: Priority::Normal,
        provides: BTreeSet::new(),
        requires: BTreeSet::new(),
        flow: Flows {
            input: vec![Flow {
                name: "in".into(),
                ty: TypeRef::Primitive {
                    name: "json".into(),
                },
                pin_to: None,
            }],
            output: vec![Flow {
                name: "out".into(),
                ty: TypeRef::Primitive {
                    name: "json".into(),
                },
                pin_to: None,
            }],
        },
        ..Default::default()
    };

    let _handle = spawn_with_handle(SidecarConfig {
        card,
        wit: None,
        ping_interval: Duration::from_secs(5),
    });

    eprintln!("presence({label}): sidecar lanzado, durmiendo (Ctrl-C para salir)");
    std::thread::park();
}
