//! Rutas canónicas de persistencia y del socket de control.

use std::collections::HashMap;
use std::path::PathBuf;

use pacha_core::{Catalog, Runtime};
use sandokan_monitor_core::reglas::ReglaMetrica;

/// Subdir de config/runtime.
pub const DIR: &str = "pacha";
/// Archivo de definiciones (editable por el usuario).
pub const CATALOG_FILE: &str = "pachas.ron";
/// Archivo de reglas de métrica por contexto (editable por el usuario): un mapa
/// `contexto → [ReglaMetrica]`. Aparte de `pachas.ron` para no acoplar el
/// catálogo (agnóstico de sandokan) a los tipos del plano de control.
pub const REGLAS_FILE: &str = "reglas.ron";
/// Archivo de estado vivo (efímero, en runtime dir).
pub const STATE_FILE: &str = "state.ron";
/// Socket de control del daemon.
pub const SOCKET_FILE: &str = "pacha.sock";

/// `~/.config/pacha/pachas.ron`. `None` si no hay config dir.
pub fn catalog_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", DIR).map(|d| d.config_dir().join(CATALOG_FILE))
}

/// `~/.config/pacha/reglas.ron`. `None` si no hay config dir.
pub fn reglas_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", DIR).map(|d| d.config_dir().join(REGLAS_FILE))
}

/// Tipo del binding que el daemon pasa a `Manager::con_reglas`.
pub type ReglasPorContexto = HashMap<String, Vec<ReglaMetrica>>;

/// Parsea el mapa `contexto → reglas` desde RON. **Pura** (testeable sin disco).
pub fn parse_reglas(s: &str) -> Result<ReglasPorContexto, String> {
    ron::from_str(s).map_err(|e| e.to_string())
}

/// Carga las reglas de métrica por contexto; vacío si no existe o está corrupto
/// (nunca falla el arranque, igual que `load_catalog`).
pub fn load_reglas() -> ReglasPorContexto {
    let Some(p) = reglas_path() else { return HashMap::new() };
    match std::fs::read_to_string(&p) {
        Ok(s) => parse_reglas(&s).unwrap_or_else(|e| {
            tracing::warn!(path = %p.display(), error = %e, "reglas.ron corrupto, uso vacío");
            HashMap::new()
        }),
        Err(_) => HashMap::new(),
    }
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

/// Directorio **persistente** del versionado de dotfiles
/// (`~/.local/share/pacha/dotfiles`): almacén de objetos + catálogo + cabezas.
/// A diferencia del state runtime, esto NO es efímero (el historial perdura).
pub fn dotfiles_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", DIR).map(|d| d.data_dir().join("dotfiles"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use sandokan_monitor_core::reglas::{AccionControl, Condicion};

    #[test]
    fn parse_reglas_round_trip() {
        let mut mapa: ReglasPorContexto = HashMap::new();
        mapa.insert(
            "presentando".into(),
            vec![ReglaMetrica {
                id: "congelar-fondo".into(),
                cuando: Condicion::CpuPctMin(50.0),
                durante: std::time::Duration::from_secs(15),
                entonces: AccionControl::Congelar { cgroup_path: "pacha/secundario".into(), frozen: true },
            }],
        );
        let ron = ron::to_string(&mapa).expect("serializa");
        let back = parse_reglas(&ron).expect("parsea");
        assert_eq!(back.len(), 1);
        assert_eq!(back["presentando"].len(), 1);
        assert_eq!(back["presentando"][0].id, "congelar-fondo");
    }

    #[test]
    fn parse_reglas_basura_es_error() {
        assert!(parse_reglas("no es ron {{{").is_err());
    }

    /// El `reglas.ron` que siembra el install DEBE parsear — si no, el daemon
    /// arranca sin reglas y el usuario no se entera.
    #[test]
    fn ejemplo_sembrado_parsea() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ejemplos/reglas.ron");
        let s = std::fs::read_to_string(&p).expect("leer ejemplos/reglas.ron");
        let mapa = parse_reglas(&s).expect("reglas.ron de ejemplo no parsea");
        assert!(mapa.contains_key("oficina"));
        assert!(mapa.contains_key("presentando"));
    }
}
