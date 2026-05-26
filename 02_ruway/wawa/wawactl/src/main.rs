//! `wawactl` — CLI sobre `wawa-config`.
//!
//! Subcomandos:
//!
//!   wawactl path                       imprime $XDG_CONFIG_HOME/wawa/config.json
//!   wawactl show [--json]              dump completo (pretty / json compacto)
//!   wawactl get <key>                  imprime el valor de una key
//!   wawactl set <key> <value>          escribe y dispara watchers
//!   wawactl reset                      restablece defaults y persiste
//!   wawactl watch                      sigue el archivo y muestra cada cambio
//!   wawactl module <id> on|off|toggle  conmuta un módulo
//!
//! Las keys aceptadas en get/set son los nombres de campos del struct:
//! `theme_variant`, `accent`, `lang`, `timefmt_24h`. Para los módulos
//! existe el subcomando dedicado `module`.

use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use wawa_config::{ConfigWatcher, WawaConfig};

#[derive(Parser)]
#[command(name = "wawactl", about = "Cliente CLI del bus de configuración wawa-config")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Imprime el path absoluto del archivo de configuración.
    Path,
    /// Muestra la configuración entera. Por defecto pretty-print;
    /// pasar `--json` para una sola línea JSON.
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Devuelve el valor de una key.
    Get {
        /// `theme_variant`, `accent`, `lang`, `timefmt_24h`.
        key: String,
    },
    /// Cambia una key y persiste atómicamente.
    Set {
        /// `theme_variant`, `accent`, `lang`, `timefmt_24h`.
        key: String,
        /// Para `timefmt_24h` acepta `true`/`false`/`24`/`12`.
        value: String,
    },
    /// Restablece a defaults.
    Reset,
    /// Subscribe al bus e imprime cada cambio. Útil para debugging:
    /// dejar corriendo en una terminal mientras se prueba el panel.
    Watch,
    /// Conmuta un módulo del SO.
    Module {
        /// id del módulo (`mirada`, `shuma`, …)
        id: String,
        /// `on`, `off`, o `toggle`.
        op: String,
    },
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
        Cmd::Path => cmd_path(),
        Cmd::Show { json } => cmd_show(json),
        Cmd::Get { key } => cmd_get(&key),
        Cmd::Set { key, value } => cmd_set(&key, &value),
        Cmd::Reset => cmd_reset(),
        Cmd::Watch => cmd_watch(),
        Cmd::Module { id, op } => cmd_module(&id, &op),
    }
}

fn cmd_path() -> Result<(), String> {
    let path = WawaConfig::path().ok_or("no hay ProjectDirs")?;
    println!("{}", path.display());
    Ok(())
}

fn cmd_show(json: bool) -> Result<(), String> {
    let cfg = WawaConfig::load();
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

fn cmd_set(key: &str, value: &str) -> Result<(), String> {
    let mut cfg = WawaConfig::load();
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
    let path = cfg.save().map_err(|e| e.to_string())?;
    println!("✓ {} = {}  →  {}", key, value, path.display());
    Ok(())
}

fn cmd_reset() -> Result<(), String> {
    let cfg = WawaConfig::default();
    let path = cfg.save().map_err(|e| e.to_string())?;
    println!("✓ reset  →  {}", path.display());
    Ok(())
}

fn cmd_module(id: &str, op: &str) -> Result<(), String> {
    let mut cfg = WawaConfig::load();
    let before = cfg.module_enabled(id);
    let after = match op {
        "on" => true,
        "off" => false,
        "toggle" => !before,
        other => return Err(format!("op inválida: {other} (usar on|off|toggle)")),
    };
    cfg.modules.insert(id.into(), after);
    let path = cfg.save().map_err(|e| e.to_string())?;
    println!(
        "✓ module {} = {}  ({})  →  {}",
        id,
        if after { "on" } else { "off" },
        if before == after { "sin cambios" } else { "cambiado" },
        path.display(),
    );
    Ok(())
}

fn cmd_watch() -> Result<(), String> {
    let path = WawaConfig::path().ok_or("no hay ProjectDirs")?;
    println!("watching {}…", path.display());
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
