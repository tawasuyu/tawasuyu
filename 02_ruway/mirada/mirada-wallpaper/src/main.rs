//! `mirada-wallpaper` — pone el fondo de escritorio desde un servicio público.
//!
//! ```sh
//! mirada-wallpaper now        # un refresco ya (baja la imagen y la aplica)
//! mirada-wallpaper daemon     # refresca cada interval_secs, en bucle
//! mirada-wallpaper sources    # muestra la fuente configurada y las disponibles
//! ```
//!
//! La config vive en `~/.config/mirada/wallpaper.ron` (se escribe una
//! plantilla la primera vez). No habla con el compositor por ningún socket:
//! sólo edita `config.ron`, que mirada recarga en caliente.

use std::process::ExitCode;
use std::time::Duration;

use mirada_wallpaper::{run_once, Config, Outcome};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mirada-wallpaper: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        None | Some("now") => {
            let cfg = load_cfg();
            apply_once(&cfg)
        }
        Some("daemon") => {
            let cfg = load_cfg();
            daemon(&cfg)
        }
        Some("sources") => {
            print_sources(&load_cfg());
            Ok(())
        }
        Some("-h" | "--help" | "help") => {
            print_help();
            Ok(())
        }
        Some(other) => {
            anyhow::bail!("subcomando desconocido «{other}» (usa now, daemon, sources, help)");
        }
    }
}

/// Carga la config del daemon (escribe la plantilla si no existe).
fn load_cfg() -> Config {
    match Config::default_path() {
        Some(p) => Config::load_or_default(&p),
        None => {
            eprintln!("mirada-wallpaper · sin HOME para la config; uso defaults.");
            Config::default()
        }
    }
}

/// Un refresco, con reporte legible.
fn apply_once(cfg: &Config) -> anyhow::Result<()> {
    match run_once(cfg)? {
        Outcome::Changed(p) => println!("mirada-wallpaper · fondo → {}", p.display()),
        Outcome::Unchanged(p) => {
            println!("mirada-wallpaper · sin cambios (ya estaba {})", p.display())
        }
    }
    Ok(())
}

/// Bucle del daemon: refresca, duerme `interval_secs`, repite. Un error de red
/// no lo tumba — lo loguea y reintenta en el próximo ciclo.
fn daemon(cfg: &Config) -> anyhow::Result<()> {
    let interval = Duration::from_secs(cfg.interval_secs.max(1));
    println!(
        "mirada-wallpaper · daemon arriba (cada {}s, fuente: {})",
        interval.as_secs(),
        cfg.build_source().label()
    );
    loop {
        match run_once(cfg) {
            Ok(Outcome::Changed(p)) => println!("mirada-wallpaper · fondo → {}", p.display()),
            Ok(Outcome::Unchanged(_)) => {}
            Err(e) => eprintln!("mirada-wallpaper · refresco falló: {e:#} (reintento luego)"),
        }
        std::thread::sleep(interval);
    }
}

fn print_sources(cfg: &Config) {
    println!("Fuente configurada: {}", cfg.build_source().label());
    println!("Config: {}", Config::default_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<sin HOME>".into()));
    println!("\nFuentes disponibles (editá `source:` en wallpaper.ron):");
    println!("  Bing(market: \"en-US\", resolution: \"1920x1080\")  — foto del día, sin API key");
    println!("  Nasa(api_key: \"DEMO_KEY\")                        — astrofoto del día (APOD)");
    println!("  Folder(dir: \"/home/yo/fondos\")                   — rota una carpeta local, offline");
}

fn print_help() {
    println!(
        "mirada-wallpaper — fondo de escritorio desde un servicio público.\n\n\
         USO:\n\
         \x20 mirada-wallpaper now        un refresco ya (default)\n\
         \x20 mirada-wallpaper daemon     refresca en bucle cada interval_secs\n\
         \x20 mirada-wallpaper sources    muestra la fuente y las disponibles\n\
         \x20 mirada-wallpaper help       esta ayuda\n\n\
         Config: ~/.config/mirada/wallpaper.ron (se crea con una plantilla).\n\
         Cambia el wallpaper_path de ~/.config/mirada/config.ron, que el\n\
         compositor mirada recarga en caliente. No toca el compositor."
    );
}
