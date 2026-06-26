//! El binario: conecta al Cuerpo por `MIRADA_SOCKET`, carga los plugins y corre
//! el bucle `evento → comandos`, espejando a `mirada-app-llimphi` sin la UI.
//!
//! Directorio de plugins: `$MIRADA_PLUGINS` o, por defecto,
//! `$XDG_CONFIG_HOME/mirada/plugins` (`~/.config/mirada/plugins`).
//!
//! v1 usa un `Desktop` con la config por defecto. Cargar el keymap/reglas/caps
//! del usuario (como hace mirada-app-llimphi) es un seguimiento directo.

use std::path::PathBuf;

use mirada_brain::Desktop;
use mirada_link::BrainLink;
use mirada_plugin_host::{load_plugins_dir, Conductor};

fn plugins_dir() -> PathBuf {
    if let Ok(p) = std::env::var("MIRADA_PLUGINS") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("mirada").join("plugins")
}

fn main() {
    let path = match std::env::var("MIRADA_SOCKET") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("MIRADA_SOCKET no está puesto — arrancá el Cuerpo con él.");
            std::process::exit(1);
        }
    };
    let mut link = match BrainLink::connect(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("no se pudo conectar al Cuerpo en {path}: {e}");
            std::process::exit(1);
        }
    };

    let plugins = load_plugins_dir(&plugins_dir());
    let mut conductor = Conductor::new(Desktop::new(), plugins);

    // Handshake de arranque (atajos + decoración + permisos).
    for cmd in conductor.startup() {
        let _ = link.send(&cmd);
    }

    // Bucle: `recv` bloquea hasta un evento o el cierre del Cuerpo (None → salir).
    while let Some(first) = link.recv() {
        let mut batch = vec![first];
        batch.extend(link.drain());
        for ev in batch {
            for cmd in conductor.on_body_event(ev) {
                let _ = link.send(&cmd);
            }
        }
    }
    eprintln!("[host] el Cuerpo cerró la conexión; saliendo.");
}
