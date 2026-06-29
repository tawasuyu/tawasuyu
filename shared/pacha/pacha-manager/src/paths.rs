//! Rutas canónicas de persistencia y del socket de control.

use std::path::PathBuf;

use pacha_core::{Catalog, Runtime};

/// Subdir de config/runtime.
pub const DIR: &str = "pacha";
/// Archivo de definiciones (editable por el usuario).
pub const CATALOG_FILE: &str = "pachas.ron";
/// Archivo de estado vivo (efímero, en runtime dir).
pub const STATE_FILE: &str = "state.ron";
/// Socket de control del daemon.
pub const SOCKET_FILE: &str = "pacha.sock";

/// `~/.config/pacha/pachas.ron`. `None` si no hay config dir.
pub fn catalog_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", DIR).map(|d| d.config_dir().join(CATALOG_FILE))
}

/// `$XDG_RUNTIME_DIR/pacha/state.ron` (fallback a config dir si no hay
/// runtime dir). El estado es efímero por sesión.
pub fn state_path() -> Option<PathBuf> {
    runtime_dir().map(|d| d.join(STATE_FILE))
}

/// `$XDG_RUNTIME_DIR/pacha/pacha.sock`.
pub fn socket_path() -> Option<PathBuf> {
    runtime_dir().map(|d| d.join(SOCKET_FILE))
}

/// Directorio runtime de pacha (`$XDG_RUNTIME_DIR/pacha`), con fallback al
/// config dir cuando no hay runtime dir (entornos sin sesión de login).
pub fn runtime_dir() -> Option<PathBuf> {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Some(PathBuf::from(rt).join(DIR));
    }
    directories::ProjectDirs::from("", "", DIR).map(|d| d.config_dir().to_path_buf())
}

/// Carga el catálogo de `pachas.ron`. Si no existe o está corrupto, devuelve
/// uno vacío (nunca falla el arranque; el error se loggea).
pub fn load_catalog() -> Catalog {
    let Some(p) = catalog_path() else { return Catalog::new() };
    match std::fs::read_to_string(&p) {
        Ok(s) => Catalog::from_ron(&s).unwrap_or_else(|e| {
            tracing::warn!(path = %p.display(), error = %e, "pachas.ron corrupto, uso vacío");
            Catalog::new()
        }),
        Err(_) => Catalog::new(),
    }
}

/// Persiste el catálogo atómicamente (tmp + rename).
pub fn save_catalog(cat: &Catalog) -> std::io::Result<()> {
    let Some(p) = catalog_path() else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "sin config dir"));
    };
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let s = cat.to_ron().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let tmp = p.with_extension("ron.tmp");
    std::fs::write(&tmp, s)?;
    std::fs::rename(&tmp, &p)
}

/// Carga el estado runtime; vacío si no existe.
pub fn load_runtime() -> Runtime {
    let Some(p) = state_path() else { return Runtime::new() };
    match std::fs::read_to_string(&p) {
        Ok(s) => Runtime::from_ron(&s).unwrap_or_default(),
        Err(_) => Runtime::new(),
    }
}

/// Persiste el estado runtime atómicamente.
pub fn save_runtime(rt: &Runtime) -> std::io::Result<()> {
    let Some(p) = state_path() else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "sin runtime dir"));
    };
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let s = rt.to_ron().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let tmp = p.with_extension("ron.tmp");
    std::fs::write(&tmp, s)?;
    std::fs::rename(&tmp, &p)
}
