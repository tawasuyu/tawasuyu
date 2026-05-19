//! `presence-conscious` — módulo brahman que se presenta con su WIT.
//!
//! Variante de [`presence`] que toma un path a un archivo `.wit` (default
//! `shared_wit/protocol.wit` resuelto desde el cwd) y lo parsea con
//! `brahman-card-wit` antes de spawnear el sidecar. Demuestra el flujo
//! "módulo consciente": Hello incluye `WitInterface`, el server lo
//! registra como `ResolvedCard::from_conscious`, y aparece con marker
//! 🧠 en `brahman-status`.
//!
//! Uso:
//! ```sh
//! cargo run -p brahman-sidecar --example presence-conscious -- mi-modulo [path/al.wit]
//! ```

use std::collections::BTreeSet;
use std::path::PathBuf;
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

    let mut args = std::env::args().skip(1);
    let label = args.next().unwrap_or_else(|| "conscious-default".into());
    let wit_path: PathBuf = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("shared_wit/protocol.wit"));

    let wit = match brahman_card_wit::parse_wit_file(&wit_path) {
        Ok(worlds) => match worlds.into_iter().next() {
            Some(w) => {
                eprintln!(
                    "[{label}] cargado wit: {} / {}",
                    w.package, w.world
                );
                Some(w)
            }
            None => {
                eprintln!("[{label}] {} no declara worlds", wit_path.display());
                None
            }
        },
        Err(e) => {
            eprintln!("[{label}] falló parse de {}: {e}", wit_path.display());
            None
        }
    };

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

    let config = SidecarConfig {
        card,
        wit,
        ping_interval: Duration::from_secs(5),
    };

    let _handle = spawn_with_handle(config);

    eprintln!("[{label}] sidecar lanzado, durmiendo (Ctrl-C para salir)");
    std::thread::park();
}
