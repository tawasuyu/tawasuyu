//! `wawactl` — CLI sobre `wawa-config`.
//!
//! Subcomandos:
//!
//!   wawactl path [--system]            imprime el path del archivo de la capa
//!   wawactl show [--json] [--layer …]  dump completo de una capa o la efectiva
//!   wawactl get <key>                  imprime el valor efectivo de una key
//!   wawactl set <key> <value> [--system]   escribe (defaults a capa usuario)
//!   wawactl reset [--system]           restablece defaults y persiste
//!   wawactl watch                      sigue ambas capas y muestra cada cambio
//!   wawactl module <id> on|off|toggle [--system]   conmuta un módulo
//!
//! Las keys aceptadas en get/set son los nombres de campos del struct:
//! `theme_variant`, `accent`, `lang`, `timefmt_24h`. Para los módulos
//! existe el subcomando dedicado `module`.
//!
//! Capas:
//!   - **user** (default) → `$XDG_CONFIG_HOME/wawa/config.json`.
//!   - **system** (`--system`) → `/etc/wawa/config.json` (Linux,
//!     requiere root para escribir).
//!   - **effective** → unión mergeada usuario sobre sistema, para
//!     leer lo que realmente ven las apps.

use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use wawa_config::{ConfigWatcher, Layer, WawaConfig};

#[derive(Parser)]
#[command(name = "wawactl", about = "Cliente CLI del bus de configuración wawa-config")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Imprime el path absoluto del archivo de configuración. Por
    /// defecto, el de la capa de usuario; con `--system` el de
    /// `/etc/wawa/config.json`.
    Path {
        #[arg(long)]
        system: bool,
    },
    /// Muestra la configuración. Por defecto la efectiva (sistema +
    /// usuario mergeada). `--layer system` o `--layer user` para
    /// inspeccionar una capa concreta sin mergear.
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long, value_enum, default_value_t = ShowLayer::Effective)]
        layer: ShowLayer,
    },
    /// Devuelve el valor efectivo de una key (post-merge sistema +
    /// usuario).
    Get {
        /// `theme_variant`, `accent`, `lang`, `timefmt_24h`.
        key: String,
    },
    /// Cambia una key y persiste atómicamente en la capa de usuario.
    /// Con `--system` escribe en `/etc/wawa/config.json` (requiere
    /// root).
    Set {
        /// `theme_variant`, `accent`, `lang`, `timefmt_24h`.
        key: String,
        /// Para `timefmt_24h` acepta `true`/`false`/`24`/`12`.
        value: String,
        #[arg(long)]
        system: bool,
    },
    /// Restablece la capa indicada a defaults. Por defecto la de
    /// usuario; con `--system` la de sistema (requiere root).
    Reset {
        #[arg(long)]
        system: bool,
    },
    /// Subscribe al bus (ambas capas) e imprime cada cambio. Útil
    /// para debugging: dejar corriendo en una terminal mientras se
    /// prueba el panel.
    Watch,
    /// Conmuta un módulo del SO. Por defecto en la capa de usuario;
    /// con `--system` en la de sistema (requiere root).
    Module {
        /// id del módulo (`mirada`, `shuma`, …)
        id: String,
        /// `on`, `off`, o `toggle`.
        op: String,
        #[arg(long)]
        system: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ShowLayer {
    /// Sólo la capa de sistema (`/etc/wawa/config.json`), sin merge.
    System,
    /// Sólo la capa de usuario, sin merge.
    User,
    /// Unión mergeada usuario sobre sistema — lo que ven las apps.
    Effective,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("wawactl: {msg}");
            ExitCode::from(1)
        }
    }
}

fn run(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Path { system } => cmd_path(layer_of(system)),
        Cmd::Show { json, layer } => cmd_show(json, layer),
        Cmd::Get { key } => cmd_get(&key),
        Cmd::Set { key, value, system } => cmd_set(&key, &value, layer_of(system)),
        Cmd::Reset { system } => cmd_reset(layer_of(system)),
        Cmd::Watch => cmd_watch(),
        Cmd::Module { id, op, system } => cmd_module(&id, &op, layer_of(system)),
    }
}

fn layer_of(system: bool) -> Layer {
    if system { Layer::System } else { Layer::User }
}

fn layer_label(l: Layer) -> &'static str {
    match l {
        Layer::System => "system",
        Layer::User => "user",
    }
}

fn cmd_path(layer: Layer) -> Result<(), String> {
    let path = WawaConfig::path_for(layer)
        .ok_or_else(|| format!("la capa {} no aplica en esta plataforma", layer_label(layer)))?;
    println!("{}", path.display());
    Ok(())
}

fn cmd_show(json: bool, layer: ShowLayer) -> Result<(), String> {
    let cfg = match layer {
        ShowLayer::Effective => WawaConfig::load(),
        ShowLayer::System => WawaConfig::load_layer(Layer::System).ok_or_else(|| {
            "capa de sistema ausente (no existe /etc/wawa/config.json)".to_string()
        })?,
        ShowLayer::User => WawaConfig::load_layer(Layer::User)
            .ok_or_else(|| "capa de usuario ausente".to_string())?,
    };
    let s = if json {
        serde_json::to_string(&cfg).map_err(|e| e.to_string())?
    } else {
        serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?
    };
    println!("{s}");
    Ok(())
}

fn cmd_get(key: &str) -> Result<(), String> {
    let cfg = WawaConfig::load();
    let val = match key {
        "theme_variant" => cfg.theme_variant,
        "accent" => cfg.accent,
        "lang" => cfg.lang,
        "timefmt_24h" => cfg.timefmt_24h.to_string(),
        _ => return Err(format!("key desconocida: {key}")),
    };
    println!("{val}");
    Ok(())
}

fn cmd_set(key: &str, value: &str, layer: Layer) -> Result<(), String> {
    // Para set/reset/module sobre una capa, tomamos como base la
    // config **de esa capa** (si existe) — no la efectiva — para no
    // promover valores del usuario al archivo de sistema (o
    // viceversa) por accidente.
    let mut cfg = WawaConfig::load_layer(layer).unwrap_or_default();
    match key {
        "theme_variant" => {
            validate_theme(value)?;
            cfg.theme_variant = value.into();
        }
        "accent" => cfg.accent = value.into(),
        "lang" => {
            validate_lang(value)?;
            cfg.lang = value.into();
        }
        "timefmt_24h" => {
            cfg.timefmt_24h = parse_bool_or_clock(value)?;
        }
        _ => return Err(format!("key desconocida: {key}")),
    }
    let path = cfg.save_to(layer).map_err(|e| e.to_string())?;
    println!(
        "✓ [{}] {} = {}  →  {}",
        layer_label(layer),
        key,
        value,
        path.display()
    );
    Ok(())
}

fn cmd_reset(layer: Layer) -> Result<(), String> {
    let cfg = WawaConfig::default();
    let path = cfg.save_to(layer).map_err(|e| e.to_string())?;
    println!("✓ [{}] reset  →  {}", layer_label(layer), path.display());
    Ok(())
}

fn cmd_module(id: &str, op: &str, layer: Layer) -> Result<(), String> {
    let mut cfg = WawaConfig::load_layer(layer).unwrap_or_default();
    let before = cfg.module_enabled(id);
    let after = match op {
        "on" => true,
        "off" => false,
        "toggle" => !before,
        other => return Err(format!("op inválida: {other} (usar on|off|toggle)")),
    };
    cfg.modules.insert(id.into(), after);
    let path = cfg.save_to(layer).map_err(|e| e.to_string())?;
    println!(
        "✓ [{}] module {} = {}  ({})  →  {}",
        layer_label(layer),
        id,
        if after { "on" } else { "off" },
        if before == after { "sin cambios" } else { "cambiado" },
        path.display(),
    );
    Ok(())
}

fn cmd_watch() -> Result<(), String> {
    let user = WawaConfig::path_for(Layer::User).ok_or("no hay ProjectDirs")?;
    match WawaConfig::path_for(Layer::System) {
        Some(sys) => println!("watching {} y {}…", sys.display(), user.display()),
        None => println!("watching {}…", user.display()),
    }
    let (tx, rx) = mpsc::channel::<WawaConfig>();
    let _watcher = ConfigWatcher::spawn(move |cfg| {
        let _ = tx.send(cfg);
    })
    .map_err(|e| format!("watcher: {e}"))?;

    // Imprimimos la versión inicial como ancla.
    let initial = WawaConfig::load();
    print_change(&initial, true);

    let mut last = initial;
    loop {
        match rx.recv_timeout(Duration::from_secs(60 * 60 * 24)) {
            Ok(cfg) => {
                if cfg != last {
                    print_change(&cfg, false);
                    last = cfg;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn print_change(cfg: &WawaConfig, initial: bool) {
    let prefix = if initial { "·" } else { "↻" };
    let json = serde_json::to_string(cfg).unwrap_or_else(|_| String::from("?"));
    println!("{prefix} {json}");
}

// =====================================================================
// Validadores
// =====================================================================

fn validate_theme(v: &str) -> Result<(), String> {
    match v {
        "dark" | "light" | "aurora" | "sunset" => Ok(()),
        other => Err(format!(
            "theme_variant inválido: {other} (usar dark|light|aurora|sunset)"
        )),
    }
}

fn validate_lang(v: &str) -> Result<(), String> {
    match v {
        "es-PE" | "en-US" | "qu-PE" => Ok(()),
        other => Err(format!(
            "lang inválido: {other} (usar es-PE|en-US|qu-PE)"
        )),
    }
}

fn parse_bool_or_clock(v: &str) -> Result<bool, String> {
    match v {
        "true" | "24" | "24h" => Ok(true),
        "false" | "12" | "12h" => Ok(false),
        other => Err(format!("bool inválido: {other}")),
    }
}
