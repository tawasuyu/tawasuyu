//! `pacha` — CLI de contextos de usuario.
//!
//! ```text
//! pacha daemon              # levanta el activador (sirve el socket)
//! pacha list                # contextos definidos + estado
//! pacha switch juegos       # cambia de contexto (default por on_leave)
//! pacha switch juegos --fresh   # ignora last_session, usa la receta
//! pacha close oficina       # libera un contexto sin cambiar el foco
//! ```
//!
//! Sin daemon corriendo, los subcomandos de cliente fallan con un mensaje
//! claro — primero hay que `pacha daemon` (lo arranca la sesión de escritorio).

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use pacha_manager::proto::{self, Req, Resp};
use pacha_manager::{linux::LinuxSurfaces, paths, server, Manager};

#[derive(Parser)]
#[command(name = "pacha", about = "Contextos de usuario — modos de uso con nombre")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Levanta el activador: carga catálogo+estado y sirve el socket.
    Daemon,
    /// Lista los contextos definidos y su estado.
    List,
    /// Cambia al contexto `id` (aplica el on_leave del saliente).
    Switch {
        id: String,
        /// Ignora `last_session` y abre la receta desde cero.
        #[arg(long)]
        fresh: bool,
    },
    /// Cierra el contexto `id` (libera recursos) sin cambiar el foco.
    Close { id: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon => run_daemon().await,
        Cmd::List => client(Req::List).await,
        Cmd::Switch { id, fresh } => client(Req::Switch { to: id, fresh }).await,
        Cmd::Close { id } => client(Req::Close { id }).await,
    }
}

/// Arranca el daemon: catálogo de disco + estado runtime + superficies reales.
async fn run_daemon() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let socket = paths::socket_path().context("sin runtime dir para el socket")?;
    let catalog = paths::load_catalog();
    let runtime = paths::load_runtime();
    let surf = LinuxSurfaces::connect().await;
    let manager = Manager::new(catalog, runtime, surf);
    println!("pacha-manager escuchando en {}", socket.display());
    server::serve(manager, &socket).await?;
    Ok(())
}

/// Cliente: manda un `Req` al daemon y formatea la respuesta.
async fn client(req: Req) -> Result<()> {
    let socket = paths::socket_path().context("sin runtime dir para el socket")?;
    let resp = proto::request(&socket, &req)
        .await
        .with_context(|| format!("no pude hablar con el daemon en {} (¿corre `pacha daemon`?)", socket.display()))?;
    match resp {
        Resp::Ok => println!("ok"),
        Resp::Switched { active, warnings } => {
            println!("activo: {}", active.as_deref().unwrap_or("(ninguno)"));
            for w in warnings {
                eprintln!("aviso: {w}");
            }
        }
        Resp::List(list) => {
            if list.is_empty() {
                println!("(sin contextos definidos — editá ~/.config/pacha/pachas.ron)");
            }
            for p in list {
                let mark = if p.active { "●" } else { " " };
                println!("{mark} {:<12} {:<10?} {}", p.id, p.lifecycle, p.label);
            }
        }
        Resp::Err(e) => anyhow::bail!("{e}"),
    }
    Ok(())
}
