//! Vigía genérico de un archivo de configuración, para la recarga en
//! caliente. Lo comparten el keymap, la config y las reglas: los tres son
//! RON en `~/.config/mirada/` que el usuario edita a mano y mirada
//! recarga sin reiniciar.
//!
//! El patrón: se vigila el **directorio** (los editores reescriben el
//! archivo por *rename*, no editándolo en sitio) y se filtra al archivo de
//! interés. Una ráfaga de eventos de un solo guardado se *coalesce* en un
//! único [`changed`](FileWatch::changed).

use std::path::Path;
use std::sync::mpsc;

/// Vigía de un archivo para la recarga en caliente.
///
/// Mantenlo vivo mientras quieras recargas; al soltarlo, la vigilancia
/// cesa. Consulta [`changed`](FileWatch::changed) en tu bucle de eventos.
pub struct FileWatch {
    _watcher: notify::RecommendedWatcher,
    rx: mpsc::Receiver<()>,
}

impl FileWatch {
    /// Empieza a vigilar `path`. Vigila su directorio padre (si existe) y
    /// filtra los eventos al archivo concreto, así capta los guardados por
    /// *rename* de los editores.
    pub fn new(path: &Path) -> notify::Result<FileWatch> {
        use notify::{RecursiveMode, Watcher};

        let target = path.to_path_buf();
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if event.paths.iter().any(|p| p == &target) {
                    let _ = tx.send(());
                }
            }
        })?;
        let dir = path.parent().filter(|d| d.exists());
        watcher.watch(dir.unwrap_or(path), RecursiveMode::NonRecursive)?;
        Ok(FileWatch { _watcher: watcher, rx })
    }

    /// `true` si el archivo cambió desde la última consulta. Coalesce una
    /// ráfaga de eventos (un guardado dispara varios) en un solo `true`.
    pub fn changed(&self) -> bool {
        self.rx.try_iter().count() > 0
    }
}
