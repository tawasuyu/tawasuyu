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
    /// Versionado de dotfiles (local, sin daemon): respaldar/restaurar/compartir
    /// un puñado de archivos de `$HOME` por el grafo direccionado por contenido.
    #[command(subcommand)]
    Dotfiles(DotCmd),
}

/// Subcomandos de `pacha dotfiles`. Operan **localmente** sobre el almacén
/// persistente (`~/.local/share/pacha/dotfiles`); si la seed de identidad está
/// desbloqueada en el llavero de sesión, el almacén va cifrado en reposo.
#[derive(Subcommand)]
enum DotCmd {
    /// Agrega una ruta de `$HOME` a un set (lo crea si no existe).
    Add {
        set: String,
        /// Ruta relativa a `$HOME` (ej. `.zshrc`, `.config/nvim`).
        ruta: String,
        /// Clavada (read-only): el splice la conserva al recapturar. Por defecto
        /// es rastreada (se snapshotea al cambiar).
        #[arg(long)]
        fijado: bool,
    },
    /// Captura+commitea el estado actual del set (avanza su cabeza).
    Snapshot { set: String },
    /// Restaura en `$HOME` la cabeza (último snapshot) del set.
    Restore { set: String },
    /// Lista los sets, sus rutas y si tienen snapshot.
    List,
    /// Mi clave pública para compartir (X25519 de la identidad de sesión).
    Pubkey,
    /// Publica el set cifrado a destinatarios; escribe el sobre a `--out`.
    Publish {
        set: String,
        /// Clave(s) pública(s) hex (64) de destinatario. Repetible.
        #[arg(long = "to", value_name = "PUBKEY_HEX", required = true)]
        to: Vec<String>,
        /// Archivo de salida del sobre.
        #[arg(long)]
        out: std::path::PathBuf,
    },
    /// Empuja el set a un almacén remoto (otra ruta/disco), set-difference.
    Push {
        set: String,
        /// Directorio del almacén destino.
        #[arg(long = "to", value_name = "STORE_DIR")]
        to: std::path::PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon => run_daemon().await,
        Cmd::List => client(Req::List).await,
        Cmd::Switch { id, fresh } => client(Req::Switch { to: id, fresh }).await,
        Cmd::Close { id } => client(Req::Close { id }).await,
        Cmd::Dotfiles(op) => run_dotfiles(op),
    }
}

// =====================================================================
// `pacha dotfiles` — versionado local (sin daemon)
// =====================================================================

use pacha_dotfiles::{ConjuntoDotfiles, RutaGestionada, StoreObjetos};
use pacha_llavero::{Llavero, LlaveroKernel};
use pacha_manager::linux::DotfilesCtx;

/// Nombre de la seed de identidad en el llavero de sesión (Fase 3).
const SEED_KEY: &str = "id:default";

/// Construye el ctx local desde las rutas persistentes, cifrando si hay seed
/// desbloqueada. Devuelve `(ctx, sets_path, heads_path)`.
fn abrir_local() -> Result<(DotfilesCtx, std::path::PathBuf, std::path::PathBuf)> {
    let dir = pacha_manager::paths::dotfiles_dir().context("sin data dir para dotfiles")?;
    let store_dir = dir.join("objetos");
    let sets_path = dir.join("sets.ron");
    let heads_path = dir.join("heads.ron");
    let home = dirs_home().context("sin $HOME")?;

    // Catálogo de sets persistido (vacío si es el primer uso).
    let sets: Vec<ConjuntoDotfiles> = match std::fs::read_to_string(&sets_path) {
        Ok(s) => ron::from_str(&s).context("sets.ron corrupto")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };

    // Cifrado si la identidad está desbloqueada en el llavero de sesión.
    let mut ctx = match LlaveroKernel::new().recuperar(SEED_KEY) {
        Ok(Some(seed)) => DotfilesCtx::new_cifrado(&store_dir, &home, sets, &seed),
        _ => DotfilesCtx::new(&store_dir, &home, sets),
    }
    .map_err(anyhow::Error::msg)?;
    ctx.cargar_estado(&heads_path).map_err(anyhow::Error::msg)?;
    Ok((ctx, sets_path, heads_path))
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

fn guardar_sets(ctx: &DotfilesCtx, sets_path: &std::path::Path) -> Result<()> {
    if let Some(dir) = sets_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let ron = ron::ser::to_string(&ctx.sets()).context("serializar sets")?;
    std::fs::write(sets_path, ron)?;
    Ok(())
}

fn parse_pubkey(hex: &str) -> Result<[u8; 32]> {
    anyhow::ensure!(hex.len() == 64, "pubkey hex debe tener 64 chars, tiene {}", hex.len());
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).context("pubkey hex inválida")?;
    }
    Ok(out)
}

fn hex32(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn run_dotfiles(op: DotCmd) -> Result<()> {
    let (mut ctx, sets_path, heads_path) = abrir_local()?;
    match op {
        DotCmd::Add { set, ruta, fijado } => {
            let rg = if fijado { RutaGestionada::fijado(&ruta) } else { RutaGestionada::rastreado(&ruta) };
            ctx.agregar_ruta(&set, rg);
            guardar_sets(&ctx, &sets_path)?;
            println!("añadida {ruta} a «{set}» ({})", if fijado { "fijado" } else { "rastreado" });
        }
        DotCmd::Snapshot { set } => {
            let raiz = ctx.snapshot_set(&set).map_err(anyhow::Error::msg)?;
            ctx.guardar_estado(&heads_path).map_err(anyhow::Error::msg)?;
            println!("snapshot de «{set}»: {}", hex32(&raiz));
        }
        DotCmd::Restore { set } => {
            ctx.restaurar_set(&set).map_err(anyhow::Error::msg)?;
            println!("«{set}» restaurado en {}", ctx.home().display());
        }
        DotCmd::List => {
            let sets = ctx.sets();
            if sets.is_empty() {
                println!("(sin sets — `pacha dotfiles add <set> <ruta>`)");
            }
            for s in sets {
                let cab = if ctx.cabeza(&s.id).is_some() { "●" } else { "○" };
                println!("{cab} {} ({} rutas)", s.id, s.entradas.len());
                for e in &s.entradas {
                    let m = if e.modo == pacha_dotfiles::ModoGestion::Fijado { "fijado" } else { "rastreado" };
                    println!("    {} [{m}]", e.origen.display());
                }
            }
        }
        DotCmd::Pubkey => {
            let seed = LlaveroKernel::new()
                .recuperar(SEED_KEY)
                .ok()
                .flatten()
                .context("identidad bloqueada: no hay seed en el llavero de sesión")?;
            println!("{}", hex32(&pacha_dotfiles::clave_publica_de_seed(&seed)));
        }
        DotCmd::Publish { set, to, out } => {
            let pubs: Vec<[u8; 32]> = to.iter().map(|h| parse_pubkey(h)).collect::<Result<_>>()?;
            let sobre = ctx.publicar_set(&set, &pubs).map_err(anyhow::Error::msg)?;
            let bytes = sobre.serializar().map_err(anyhow::Error::msg)?;
            std::fs::write(&out, &bytes)?;
            println!("publicado «{set}» a {} destinatario(s) → {}", pubs.len(), out.display());
        }
        DotCmd::Push { set, to } => {
            let remoto = StoreObjetos::abrir(&to).map_err(anyhow::Error::msg)?;
            let stats = ctx.empujar_set(&set, &remoto).map_err(anyhow::Error::msg)?;
            println!("push «{set}» → {}: {} copiados, {} ya presentes", to.display(), stats.copiados, stats.ya_presentes);
        }
    }
    Ok(())
}

/// Arranca el daemon: catálogo de disco + estado runtime + superficies reales.
async fn run_daemon() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let socket = paths::socket_path().context("sin runtime dir para el socket")?;
    let catalog = paths::load_catalog();
    let runtime = paths::load_runtime();
    let reglas = paths::load_reglas();
    // Arranque vigilado: enciende el Vigilante (reglas de métrica) y lo cablea a
    // las superficies; `con_reglas` asocia el set de cada contexto, que el
    // switch arma al enfocarse (SDD §8 capa 2/4).
    let surf = LinuxSurfaces::connect_vigilado().await;
    let manager = Manager::new(catalog, runtime, surf).con_reglas(reglas);
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
