use super::*;

/// Fuente de candidatos del shell — implementa
/// [`shuma_line::CompletionSource`]:
///
/// - `commands()`: escanea `$PATH` la primera vez y cachea el resultado.
/// - `paths(prefix)`: listado del dir derivado del `prefix`, resolviendo
///   relativos contra `cwd`.
#[derive(Debug)]
pub struct ShellSource {
    cwd: PathBuf,
    /// Si la sesión es un contenedor unshare/bwrap, el path al rootfs en disco.
    /// Con esto el preview/completado mira los binarios y archivos de ADENTRO
    /// (escaneando el rootfs en el host) en vez del PATH/FS del host — antes el
    /// ghost marcaba como existentes comandos del host que no están en el
    /// contenedor (y viceversa).
    root: Option<PathBuf>,
    commands: std::sync::OnceLock<Vec<String>>,
}

impl ShellSource {
    pub fn new(cwd: &std::path::Path) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            root: None,
            commands: std::sync::OnceLock::new(),
        }
    }

    /// Variante para sesiones de contenedor: `root` es el rootfs en disco.
    pub fn new_in_rootfs(cwd: &std::path::Path, root: PathBuf) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            root: Some(root),
            commands: std::sync::OnceLock::new(),
        }
    }

    /// Traduce un path INTERIOR del contenedor (`/etc`, `/root/foo`) al path
    /// real en el host bajo el rootfs. Sin root, devuelve el path tal cual.
    fn host_path(&self, interior: &std::path::Path) -> PathBuf {
        match &self.root {
            Some(root) => {
                let rel = interior.strip_prefix("/").unwrap_or(interior);
                root.join(rel)
            }
            None => interior.to_path_buf(),
        }
    }
}

impl shuma_line::CompletionSource for ShellSource {
    fn commands(&self) -> Vec<String> {
        self.commands
            .get_or_init(|| {
                // Dirs de binarios a escanear. Container: los del rootfs en
                // disco. Host: los del PATH del proceso.
                let dirs: Vec<PathBuf> = match &self.root {
                    Some(root) => ["usr/bin", "bin", "usr/local/bin", "sbin", "usr/sbin"]
                        .iter()
                        .map(|d| root.join(d))
                        .collect(),
                    None => std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                        .collect(),
                };
                let mut out: Vec<String> = Vec::new();
                for dir in dirs {
                    if let Ok(rd) = std::fs::read_dir(&dir) {
                        for ent in rd.flatten() {
                            if let Some(name) = ent.file_name().to_str() {
                                out.push(name.to_string());
                            }
                        }
                    }
                }
                out.sort();
                out.dedup();
                out
            })
            .clone()
    }
    fn paths(&self, prefix: &str) -> Vec<String> {
        let (dir_part, file_part) = match prefix.rfind('/') {
            Some(i) => (&prefix[..=i], &prefix[i + 1..]),
            None => ("", prefix),
        };
        let dir: PathBuf = if dir_part.is_empty() {
            self.cwd.clone()
        } else if dir_part.starts_with('/') {
            PathBuf::from(dir_part)
        } else if let Some(stripped) = dir_part.strip_prefix("~/") {
            if let Ok(home) = std::env::var("HOME") {
                PathBuf::from(home).join(stripped)
            } else {
                self.cwd.join(dir_part)
            }
        } else {
            self.cwd.join(dir_part)
        };
        // En un contenedor, `dir` es un path INTERIOR — lo listamos desde el
        // rootfs real en el host. (`host_path` es no-op sin rootfs.)
        let Ok(rd) = std::fs::read_dir(self.host_path(&dir)) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for ent in rd.flatten() {
            let name = match ent.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !name.starts_with(file_part) {
                continue;
            }
            // Ocultos: sólo aparecen si el prefix los pidió explícito.
            if name.starts_with('.') && !file_part.starts_with('.') {
                continue;
            }
            let mut full = format!("{dir_part}{name}");
            if ent.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                full.push('/');
            }
            out.push(full);
        }
        out.sort();
        out
    }
}

/// Construye el `ShellSource` adecuado para `source`: un contenedor
/// unshare/bwrap mira los binarios/archivos del rootfs en disco (preview y
/// completado correctos adentro); cualquier otro source mira el host.
pub(crate) fn completion_source_for(
    source: &Source,
    cwd: &std::path::Path,
) -> Arc<ShellSource> {
    match source {
        Source::Container { engine, name, .. }
            if matches!(engine.as_str(), "unshare" | "bwrap") =>
        {
            Arc::new(ShellSource::new_in_rootfs(cwd, PathBuf::from(name)))
        }
        _ => Arc::new(ShellSource::new(cwd)),
    }
}
