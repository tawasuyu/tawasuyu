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
