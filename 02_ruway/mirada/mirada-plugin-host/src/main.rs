//! El binario: conecta al Cuerpo por `MIRADA_SOCKET`, carga los plugins y corre
//! el bucle `evento → comandos`, espejando a `mirada-app-llimphi` sin la UI.
//!
//! Directorio de plugins: `$MIRADA_PLUGINS` o, por defecto,
//! `$XDG_CONFIG_HOME/mirada/plugins` (`~/.config/mirada/plugins`).
//!
//! El `Desktop` se construye con la config del usuario (keymap/reglas/ajustes/
//! permisos) vía [`configured_desktop`], y el bucle **recarga en caliente** esos
//! archivos: vigila `~/.config/mirada/` y reaplica el que cambie, reemitiendo
//! los comandos que correspondan al Cuerpo.

use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use mirada_brain::{Config, FileWatch, Keymap, Rules};
use mirada_link::BrainLink;
use mirada_plugin_host::{
    configured_desktop, load_caps, load_config, load_keymap, load_plugins_dir, load_rules, Conductor,
};
use mirada_protocol::BrainCommand;

/// Cadencia del sondeo: cada `recv_timeout` agotado revisa los watchers.
const POLL: Duration = Duration::from_millis(100);

/// Un `FileWatch` sobre `path` si se pudo abrir (en algunos sandboxes no hay
/// backend de inotify; ahí el hot-reload simplemente no opera).
fn watch_of(path: Option<PathBuf>) -> Option<FileWatch> {
    let p = path?;
    match FileWatch::new(&p) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("[host] sin vigilancia de {}: {e}", p.display());
            None
        }
    }
}

fn changed(w: &Option<FileWatch>) -> bool {
    w.as_ref().is_some_and(|w| w.changed())
}

fn send_all(link: &mut BrainLink, cmds: Vec<BrainCommand>) {
    for cmd in cmds {
        let _ = link.send(&cmd);
    }
}

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
    let mut conductor = Conductor::new(configured_desktop(), plugins);

    // Handshake de arranque (atajos + decoración + permisos).
    for cmd in conductor.startup() {
        let _ = link.send(&cmd);
    }

    // Vigías de los archivos de config (todos en ~/.config/mirada/).
    let km_w = watch_of(Keymap::default_path());
    let cfg_w = watch_of(Config::default_path());
    let rules_w = watch_of(Rules::default_path());
    let caps_w = watch_of(mirada_brain::permisos::default_path());

    // Bucle: intercala el sondeo del Cuerpo (con timeout) con la recarga en
    // caliente de la config. `Disconnected` (el Cuerpo cerró) sale; `Timeout`
    // sólo da otra vuelta a revisar los watchers.
    loop {
        if changed(&km_w) {
            send_all(&mut link, conductor.apply_keymap(load_keymap()));
        }
        if changed(&cfg_w) {
            if let Some(cfg) = load_config() {
                send_all(&mut link, conductor.apply_config(cfg));
            }
        }
        if changed(&caps_w) {
            send_all(&mut link, conductor.apply_caps(load_caps()));
        }
        if changed(&rules_w) {
            conductor.apply_rules(load_rules());
        }

        match link.recv_timeout(POLL) {
            Ok(ev) => {
                send_all(&mut link, conductor.on_body_event(ev));
                for ev in link.drain() {
                    send_all(&mut link, conductor.on_body_event(ev));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    eprintln!("[host] el Cuerpo cerró la conexión; saliendo.");
}
