//! `matilda` — CLI de administración de servidores.
//!
//! Carga un inventario declarativo (JSON), lo reconcilia contra el
//! estado actual y aplica los cambios — localmente, en seco, o en un
//! servidor remoto por SSH:
//!
//! ```text
//!   matilda example                 imprime un inventario de ejemplo
//!   matilda plan    inv.json        muestra el plan de reconciliación
//!   matilda script  inv.json        emite el script de aplicación
//!   matilda apply   inv.json        aplica localmente
//!   matilda apply   inv.json --dry-run            simula
//!   matilda apply   inv.json --host deploy@srv    aplica por SSH
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use matilda_apply::{plan_to_steps, steps_to_script, ApplyStep};
use matilda_core::{Container, Host, Inventory, RestartPolicy, VHost};
use matilda_ghost::ApplyReport;
use matilda_linker::{Linker, SshAuth, SshConfig};
use matilda_plan::{plan, Op};

#[derive(Parser)]
#[command(name = "matilda", about = "Administración declarativa de servidores")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Imprime un inventario de ejemplo para editar.
    Example,
    /// Muestra el plan de reconciliación del inventario.
    Plan {
        inventory: PathBuf,
        /// Estado actual del servidor (por defecto: vacío).
        #[arg(long)]
        current: Option<PathBuf>,
        /// Descubre el estado actual de esta máquina (docker + nginx).
        #[arg(long)]
        discover: bool,
    },
    /// Emite el script de shell que aplicaría el plan.
    Script {
        inventory: PathBuf,
        #[arg(long)]
        current: Option<PathBuf>,
        #[arg(long)]
        discover: bool,
    },
    /// Aplica el plan: local, en seco, o remoto por SSH.
    Apply {
        inventory: PathBuf,
        #[arg(long)]
        current: Option<PathBuf>,
        /// Descubre el estado actual de esta máquina antes de reconciliar.
        #[arg(long)]
        discover: bool,
        /// Simula sin tocar nada.
        #[arg(long)]
        dry_run: bool,
        /// Aplica en un host remoto, `usuario@host`.
        #[arg(long)]
        host: Option<String>,
        /// Contraseña SSH (si no se da, se usa la clave por defecto).
        #[arg(long)]
        password: Option<String>,
    },
}

/// Carga un inventario JSON desde un archivo.
fn load(path: &PathBuf) -> Result<Inventory, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("no se pudo leer {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("JSON inválido en {}: {e}", path.display()))
}

/// Resuelve el inventario "actual" contra el que reconciliar:
/// `--discover` observa esta máquina; `--current` lee un archivo; si no,
/// se parte de un inventario vacío (todo es creación).
fn current_inventory(
    discover: bool,
    current: &Option<PathBuf>,
    desired: &Inventory,
) -> Result<Inventory, String> {
    if discover {
        // Descubrimiento detallado: `docker inspect` detecta el drift.
        Ok(matilda_discover::discover_inventory(desired))
    } else {
        match current {
            Some(p) => load(p),
            None => Ok(Inventory::new()),
        }
    }
}

/// Construye un inventario de ejemplo.
fn example_inventory() -> Inventory {
    let mut inv = Inventory::new();
    inv.add_host(Host::new("edge-1", "10.0.0.1").with_tag("prod"));
    inv.add_container(
        Container::new("web", "nginx:1.27")
            .with_port(8080, 80)
            .with_volume("/srv/site", "/usr/share/nginx/html")
            .with_restart(RestartPolicy::Always),
    );
    inv.add_container(
        Container::new("api", "ghcr.io/ejemplo/api:1.0")
            .with_port(9000, 9000)
            .with_env("DATABASE_URL", "postgres://db/app")
            .with_restart(RestartPolicy::UnlessStopped),
    );
    inv.add_vhost(
        VHost::to_container("sitio.com", "web", 80)
            .with_alias("www.sitio.com")
            .with_tls(),
    );
    inv
}

/// Imprime un `ApplyReport` legible.
fn print_report(report: &ApplyReport) {
    for r in &report.results {
        println!("\n{} {}", if r.ok { "✔" } else { "✘" }, r.describe);
        for l in &r.log {
            println!("   {l}");
        }
    }
    println!(
        "\n{} de {} pasos aplicados.",
        report.applied(),
        report.results.len()
    );
    if !report.all_ok() {
        println!("✘ se detuvo en el primer error.");
    }
}

/// Aplica los pasos en un host remoto por SSH.
async fn apply_remote(
    target: &str,
    password: Option<String>,
    steps: &[ApplyStep],
) -> Result<ApplyReport, String> {
    let (user, host) = target
        .split_once('@')
        .ok_or_else(|| format!("host inválido (esperaba usuario@host): {target}"))?;
    let auth = match password {
        Some(pw) => SshAuth::Password(pw),
        None => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            SshAuth::Key {
                path: PathBuf::from(format!("{home}/.ssh/id_ed25519")),
                passphrase: None,
            }
        }
    };
    let config = SshConfig::new(host, user, auth);
    let linker = Linker::connect(&config)
        .await
        .map_err(|e| format!("conexión SSH: {e}"))?;
    Ok(linker.apply(steps).await)
}

fn run() -> Result<(), String> {
    match Cli::parse().cmd {
        Cmd::Example => {
            let json = serde_json::to_string_pretty(&example_inventory())
                .map_err(|e| e.to_string())?;
            println!("{json}");
        }

        Cmd::Plan { inventory, current, discover } => {
            let desired = load(&inventory)?;
            let p = plan(&current_inventory(discover, &current, &desired)?, &desired);
            if p.is_empty() {
                println!("Sin cambios: el servidor ya está al día.");
            } else {
                for (i, action) in p.actions.iter().enumerate() {
                    println!("{:>2}. {}", i + 1, action.describe());
                }
                println!(
                    "\n{} acciones — {} crear, {} actualizar, {} eliminar.",
                    p.len(),
                    p.count(Op::Create),
                    p.count(Op::Update),
                    p.count(Op::Remove),
                );
            }
        }

        Cmd::Script { inventory, current, discover } => {
            let desired = load(&inventory)?;
            let p = plan(&current_inventory(discover, &current, &desired)?, &desired);
            print!("{}", steps_to_script(&plan_to_steps(&p, &desired)));
        }

        Cmd::Apply { inventory, current, discover, dry_run, host, password } => {
            let desired = load(&inventory)?;
            let p = plan(&current_inventory(discover, &current, &desired)?, &desired);
            let steps = plan_to_steps(&p, &desired);
            if steps.is_empty() {
                println!("Sin cambios: nada que aplicar.");
                return Ok(());
            }
            let report = if dry_run {
                println!("— simulación (no se toca nada) —");
                matilda_ghost::dry_run(&steps)
            } else if let Some(target) = host {
                println!("— aplicando en {target} por SSH —");
                let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
                rt.block_on(apply_remote(&target, password, &steps))?
            } else {
                println!("— aplicando localmente —");
                matilda_ghost::apply(&steps)
            };
            print_report(&report);
            if !report.all_ok() {
                return Err("la aplicación falló".into());
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    bitacora::abrir("shuma");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
