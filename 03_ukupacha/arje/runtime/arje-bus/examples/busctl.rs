//! busctl: cliente CLI para el bus interno del fractal.
//!
//! Uso:
//!   cargo run --example busctl -- list-entes
//!   cargo run --example busctl -- announce
//!   cargo run --example busctl -- power-off
//!
//! Si `ENTE_BUS_SOCK` no está en el entorno, cae al path dev por defecto.

use arje_bus::{BusClient, BusRequest};
use std::env;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("list-entes");

    let mut client = match BusClient::from_env().await {
        Ok(c) => c,
        Err(_) => {
            let user = env::var("USER").unwrap_or_else(|_| "ente".into());
            let runtime = env::var("XDG_RUNTIME_DIR")
                .unwrap_or_else(|_| env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()));
            let path = format!("{runtime}/ente-bus-{user}.sock");
            eprintln!("ENTE_BUS_SOCK no definido, intentando {path}");
            BusClient::connect(&path).await?
        }
    };

    let req = match cmd {
        "list-entes" => BusRequest::ListEntes,
        "announce" => BusRequest::Announce { capabilities: vec![] },
        "power-off" => BusRequest::PowerOff { interactive: false },
        "reboot" => BusRequest::Reboot { interactive: false },
        "suspend" => BusRequest::Suspend { interactive: false },
        "invoke-echo" => {
            let msg = args.get(2).map(|s| s.as_str()).unwrap_or("hola");
            BusRequest::Invoke {
                cap: arje_echo::echo_capability(),
                blob: msg.as_bytes().to_vec(),
            }
        }
        other => {
            eprintln!("subcomando desconocido: {other}");
            eprintln!("válidos: list-entes, announce, power-off, reboot, suspend, invoke-echo <text>");
            std::process::exit(2);
        }
    };
    let resp = client.call(req).await?;
    println!("{resp:#?}");
    Ok(())
}
