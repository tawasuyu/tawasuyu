//! `sandokan` — CLI de prueba del orquestador.
//!
//! Uso típico (dos terminales):
//!   terminal 1:  sandokan daemon
//!   terminal 2:  sandokan run /bin/sleep 300
//!                sandokan list
//!                sandokan status <card-id>
//!                sandokan stop <card-id>
//!
//! Sin daemon, `run` igual encarna el proceso, pero el registro vive en
//! el proceso del CLI y se pierde al salir — `list` no lo verá. Para
//! probar el lifecycle completo, corré el daemon primero.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use card_core::{Card, Payload};
use clap::{Parser, Subcommand};
use sandokan::{auto, default_socket_path, serve, Intent, LocalEngine};
use ulid::Ulid;

#[derive(Parser)]
#[command(name = "sandokan", about = "Orquestador brahman — CLI de prueba")]
struct Cli {
    /// Socket del daemon (default: $XDG_RUNTIME_DIR/sandokan.sock).
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Corre el daemon: sirve un LocalEngine sobre el socket.
    Daemon,
    /// Encarna un ejecutable como Card y lo orquesta.
    Run {
        /// Ruta del ejecutable.
        exec: String,
        /// Argumentos del ejecutable.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Lista las entidades activas.
    List,
    /// Estado de una entidad.
    Status { card_id: String },
    /// Telemetría puntual de una entidad.
    Telemetry { card_id: String },
    /// Detiene una entidad (SIGTERM + gracia + SIGKILL).
    Stop {
        card_id: String,
        #[arg(long, default_value = "1000")]
        grace_ms: u64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(default_socket_path);

    match cli.cmd {
        Cmd::Daemon => {
            if let Some(parent) = socket.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            println!("sandokan-daemon escuchando en {}", socket.display());
            println!("(Ctrl-C para salir)");
            let engine = Arc::new(LocalEngine::new());
            serve(engine, &socket).await?;
        }
        Cmd::Run { exec, args } => {
            let mut card = Card::new(format!("run:{exec}"));
            card.payload = Payload::Native {
                exec,
                argv: args,
                envp: vec![],
            };
            let engine = auto(&socket).await;
            let handle = engine.run(Intent::new(card)).await?;
            println!("encarnado:");
            println!("  card_id : {}", handle.card_id);
            println!("  label   : {}", handle.label);
        }
        Cmd::List => {
            let engine = auto(&socket).await;
            let list = engine.list().await?;
            if list.is_empty() {
                println!("(sin entidades activas)");
            }
            for h in list {
                println!("{}  {}", h.card_id, h.label);
            }
        }
        Cmd::Status { card_id } => {
            let engine = auto(&socket).await;
            let state = engine.status(parse_id(&card_id)?).await?;
            println!("{state:?}");
        }
        Cmd::Telemetry { card_id } => {
            let engine = auto(&socket).await;
            let t = engine.telemetry(parse_id(&card_id)?).await?;
            println!(
                "mem={} KiB  nproc={}  cpu={:.1}%",
                t.mem_bytes / 1024,
                t.nproc,
                t.cpu_pct
            );
        }
        Cmd::Stop { card_id, grace_ms } => {
            let engine = auto(&socket).await;
            engine
                .stop(parse_id(&card_id)?, Duration::from_millis(grace_ms))
                .await?;
            println!("detenido");
        }
    }
    Ok(())
}

fn parse_id(s: &str) -> anyhow::Result<Ulid> {
    Ulid::from_string(s).map_err(|e| anyhow::anyhow!("card-id inválido `{s}`: {e}"))
}
