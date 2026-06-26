//! `mirada-plugin-host` — un Cerebro de mirada hecho de plugins WASM.
//!
//! Se conecta al Cuerpo (`mirada-compositor`) por `MIRADA_SOCKET` como un
//! cerebro más, pero su lógica embebe un [`mirada_brain::Desktop`] autoritativo
//! (foco, atajos, reglas, multi-monitor) que los plugins **aumentan**:
//!
//! - un plugin de **layout** refina la geometría del teselado;
//! - los plugins **reactores** reaccionan a los eventos y emiten comandos por
//!   capacidades gateadas a nivel de importación WASM.
//!
//! El [`Conductor`] orquesta y arbitra; [`wasm`] sandboxea con `wasmi`; [`caps`]
//! define el bitfield de capacidades; [`manifest`] lee los `.ron`.

pub mod caps;
pub mod conductor;
pub mod manifest;
pub mod wasm;

pub use caps::CapsPlugin;
pub use conductor::Conductor;
pub use manifest::{PluginKind, PluginManifest, ResolvedManifest};
pub use wasm::{HostCtx, LoadedPlugin};

use std::path::Path;

use mirada_brain::{Config, Desktop, Keymap, Permisos, Rules};

/// Construye un `Desktop` con la config del usuario (keymap, reglas, ajustes
/// generales y **permisos de capacidad**), cayendo a los valores por defecto si
/// falta cada archivo. Espeja el arranque de `mirada-app-llimphi` para que el
/// host honre el teclado, el tema y —clave— las denylists de seguridad
/// (portapapeles/screencopy) del usuario, en vez de partir de `Desktop::new()`.
///
/// Carga inicial; el hot-reload de estos archivos en caliente queda como
/// seguimiento (requiere sondear con `FileWatch` en el bucle).
pub fn configured_desktop() -> Desktop {
    let mut desktop = Desktop::with_keymap(load_keymap());
    desktop.set_rules(load_rules());
    if let Some(cfg) = load_config() {
        desktop.set_config(cfg);
    }
    let _ = desktop.set_caps(load_caps());
    desktop
}

/// Carga el keymap del usuario (creando el de arranque si falta), o el default.
pub fn load_keymap() -> Keymap {
    match Keymap::default_path() {
        Some(p) => Keymap::load_or_init(&p),
        None => Keymap::default(),
    }
}

/// Carga las reglas de ventana del usuario, o las default.
pub fn load_rules() -> Rules {
    match Rules::default_path() {
        Some(p) => Rules::load_or_default(&p),
        None => Rules::default(),
    }
}

/// Carga la config general del usuario, o `None` si no hay ruta.
pub fn load_config() -> Option<Config> {
    Config::default_path().map(|p| Config::load_or_default(&p))
}

/// Carga los permisos de capacidad del usuario, o los default (permitir todo).
pub fn load_caps() -> Permisos {
    match mirada_brain::permisos::default_path() {
        Some(p) => mirada_brain::permisos::load_or_default(&p),
        None => Permisos::default(),
    }
}

/// Carga todos los plugins declarados por archivos `*.ron` de un directorio.
/// Los manifests inválidos o los `.wasm` que no pasan el gateo de capacidades se
/// saltan con un aviso por `stderr` — un plugin roto nunca tumba al host.
pub fn load_plugins_dir(dir: &Path) -> Vec<LoadedPlugin> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[host] sin directorio de plugins {}: {e}", dir.display());
            return out;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("ron") {
            continue;
        }
        match manifest::PluginManifest::load(&path).and_then(|m| LoadedPlugin::load(&m)) {
            Ok(p) => {
                eprintln!("[host] plugin cargado: {} ({:?})", p.name, p.kind);
                out.push(p);
            }
            Err(e) => eprintln!("[host] plugin {} rechazado: {e}", path.display()),
        }
    }
    out
}
