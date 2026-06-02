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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Un `FileWatch` detecta una escritura del archivo vigilado. Es un test
    /// de integración con el SO (inotify): si el entorno no provee un backend
    /// de vigilancia (algunos sandboxes), `FileWatch::new` falla y el test se
    /// salta — no queremos un test frágil que rompa el smoke del workspace.
    #[test]
    fn detects_a_write_to_the_watched_file() {
        let dir = std::env::temp_dir().join(format!("mirada-watch-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("config.ron");
        std::fs::write(&file, b"(\n)\n").unwrap();

        let Ok(watch) = FileWatch::new(&file) else {
            eprintln!("watch: sin backend de vigilancia en este entorno; salto el test.");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        assert!(!watch.changed(), "recién creado: nada que reportar todavía");

        // Reescribe el archivo y espera (acotado) a que el evento llegue.
        std::fs::write(&file, b"( gap: 12 )\n").unwrap();
        let mut seen = false;
        for _ in 0..60 {
            if watch.changed() {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50)); // hasta ~3 s
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(seen, "el FileWatch no reportó la escritura en 3 s");
    }
}
