//! ente-binfmt-compat: registra handlers de binfmt_misc al boot.
//!
//! systemd-binfmt lee `/usr/lib/binfmt.d/*.conf` y `/etc/binfmt.d/*.conf` y
//! escribe cada línea al kernel via `/proc/sys/fs/binfmt_misc/register`.
//! Esto habilita ejecución transparente de binarios no-ELF (qemu-user,
//! wine, etc).
//!
//! Formato de cada línea:
//!   :<name>:<type>:<offset>:<magic>:<mask>:<interpreter>:<flags>
//!
//! Líneas que empiezan con `#` o vacías se ignoran.

use std::fs;
use std::io::Write;
use std::path::Path;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const REGISTER_PATH: &str = "/proc/sys/fs/binfmt_misc/register";
const SEARCH_DIRS: &[&str] = &[
    "/usr/lib/binfmt.d",
    "/etc/binfmt.d",
    "/run/binfmt.d",
];

fn main() {
    init_tracing();
    info!("ente-binfmt-compat: registrando handlers binfmt_misc");

    if !Path::new(REGISTER_PATH).exists() {
        warn!(path = REGISTER_PATH, "binfmt_misc no montado — skip");
        std::process::exit(0);
    }

    let mut registered = 0;
    let mut errors = 0;
    let mut skipped = 0;

    for dir in SEARCH_DIRS {
        if !Path::new(dir).exists() { continue; }
        let mut entries: Vec<_> = match fs::read_dir(dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
            Err(_) => continue,
        };
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if path.extension().map(|e| e != "conf").unwrap_or(true) { continue; }
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => { warn!(?e, path = %path.display(), "read"); continue; }
            };
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') { continue; }
                match register(line) {
                    Ok(name) => {
                        info!(file = %path.display(), %name, "binfmt registrado");
                        registered += 1;
                    }
                    Err(e) => {
                        if e.is_already_exists() {
                            skipped += 1;
                        } else {
                            warn!(?e, file = %path.display(), "registro falló");
                            errors += 1;
                        }
                    }
                }
            }
        }
    }
    info!(registered, skipped, errors, "binfmt aplicado");
    if errors > 0 { std::process::exit(1); }
}

#[derive(Debug)]
struct RegError(std::io::Error);
impl std::fmt::Display for RegError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}
impl RegError {
    fn is_already_exists(&self) -> bool {
        // EEXIST = 17 en Linux.
        self.0.raw_os_error() == Some(17)
    }
}

/// Escribe la línea al register file. Devuelve el `name` extraído del
/// primer campo (entre `:` separators) si tuvo éxito.
fn register(line: &str) -> Result<String, RegError> {
    // Sintaxis: :<name>:<type>:<offset>:<magic>:<mask>:<interpreter>:<flags>
    // Field 0 (después del ':' inicial) es el name.
    let name = line.split(':').nth(1)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".into());
    let mut f = fs::OpenOptions::new()
        .write(true)
        .open(REGISTER_PATH)
        .map_err(RegError)?;
    f.write_all(line.as_bytes()).map_err(RegError)?;
    Ok(name)
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_binfmt_compat=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}
