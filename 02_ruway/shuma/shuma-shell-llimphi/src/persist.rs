//! Persistencia del chasis: sesiones, chrome, disposiciones y containers.
//!
//! Funciones para leer/guardar `sessions.json`, `chrome.json`,
//! `layouts.json` y `containers.json`.

use crate::types::{
    ChromeState, ContainerCfg, LayoutSnapshot, Model, ModuleState, SessionConfig, SessionKind,
};

// ─── Output por sesión (flag «Persistir sesión») ────────────────────

/// Tope de líneas persistidas por sesión — suficiente historial visible
/// sin que el JSON crezca sin techo.
const PERSIST_MAX_LINES: usize = 2000;

/// El directorio de datos del perfil de **sesión** activo (tipo Firefox). Todas
/// las rutas de estado cuelgan de acá: el perfil `default` usa el directorio
/// histórico `~/.config/shuma/`; otro perfil `<n>` usa `…/profiles/<n>/`.
fn data_dir() -> Option<std::path::PathBuf> {
    crate::perfiles::sessions::active_data_dir()
}

/// `<perfil>/outputs/<sesión>.json`.
pub(crate) fn session_output_path(name: &str) -> Option<std::path::PathBuf> {
    let sane: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    data_dir().map(|d| d.join("outputs").join(format!("{sane}.json")))
}

/// Guarda el output de TODAS las sesiones con `persist` activo. Barato:
/// snapshot capeado + write atómico sólo si hay algo que decir.
pub(crate) fn save_session_outputs(m: &Model) {
    for s in &m.sessions {
        if !s.persist || s.pending || s.kind == SessionKind::Draft {
            continue;
        }
        let ModuleState::Shell(st) = &s.shell().state else {
            continue;
        };
        let snap = st.output_snapshot(PERSIST_MAX_LINES);
        if snap.lines.is_empty() {
            continue;
        }
        let Some(path) = session_output_path(&s.name) else {
            continue;
        };
        if let Ok(json) = serde_json::to_string(&snap) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

/// Lee el output persistido de una sesión, si existe.
pub(crate) fn load_session_output(name: &str) -> Option<shuma_module_shell::OutputSnapshot> {
    let path = session_output_path(name)?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Lee un snapshot de salida desde una ruta arbitraria — el archivo de handoff
/// que escribe `pata` al desacoplar ("mover de verdad") una sesión a un shuma
/// standalone. Distinto de [`load_session_output`], que resuelve la ruta por
/// nombre de sesión persistida.
pub(crate) fn load_output_snapshot_file(path: &str) -> Option<shuma_module_shell::OutputSnapshot> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Borra el output persistido (al apagar el flag o cerrar la sesión).
pub(crate) fn remove_session_output(name: &str) {
    if let Some(p) = session_output_path(name) {
        let _ = std::fs::remove_file(p);
    }
}

/// mtime de `env.json` — para detectar cambios hechos por el builtin
/// `:env` (u otra instancia) y recargar los grupos del Model.
pub(crate) fn env_groups_mtime() -> Option<std::time::SystemTime> {
    shuma_config::env_groups_path()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|md| md.modified().ok())
}

// ─── Containers ────────────────────────────────────────────────────

pub(crate) fn containers_cfg_path() -> Option<std::path::PathBuf> {
    data_dir().map(|d| d.join("containers.json"))
}

pub(crate) fn load_container_cfgs() -> Vec<ContainerCfg> {
    containers_cfg_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<ContainerCfg>>(&s).ok())
        .unwrap_or_default()
}

pub(crate) fn save_container_cfgs(cfgs: &[ContainerCfg]) {
    let Some(path) = containers_cfg_path() else {
        return;
    };
    if let Ok(json) = serde_json::to_string_pretty(cfgs) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json);
    }
}

// ─── Sesiones ───────────────────────────────────────────────────────

/// `$XDG_CONFIG_HOME/shuma/sessions.json`.
pub(crate) fn sessions_path() -> Option<std::path::PathBuf> {
    data_dir().map(|d| d.join("sessions.json"))
}

/// Guarda las sesiones reales (no la draft).
pub(crate) fn save_sessions(m: &Model) {
    let Some(path) = sessions_path() else {
        return;
    };
    let cfgs: Vec<SessionConfig> = m
        .sessions
        .iter()
        .filter(|s| s.kind != SessionKind::Draft && !s.pending)
        .map(|s| s.to_config())
        .collect();
    if let Ok(json) = serde_json::to_string_pretty(&cfgs) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json);
    }
}

/// Lee las sesiones persistidas.
pub(crate) fn load_sessions() -> Vec<SessionConfig> {
    sessions_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<SessionConfig>>(&s).ok())
        .unwrap_or_default()
}

// ─── Chrome ─────────────────────────────────────────────────────────

/// `$XDG_CONFIG_HOME/shuma/chrome.json`.
pub(crate) fn chrome_path() -> Option<std::path::PathBuf> {
    data_dir().map(|d| d.join("chrome.json"))
}

/// Guarda el estado de chrome (paneles + pestaña activa).
pub(crate) fn save_chrome(m: &Model) {
    let Some(path) = chrome_path() else {
        return;
    };
    let state = ChromeState {
        active_tool: m.active_tool,
        session_panel_open: m.session_panel_open,
        active_session: m.active_session,
        session_w: m.session_w,
        monitors_width: m.monitors_width,
    };
    if let Ok(json) = serde_json::to_string_pretty(&state) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json);
    }
}

/// Lee el estado de chrome persistido.
pub(crate) fn load_chrome() -> ChromeState {
    chrome_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<ChromeState>(&s).ok())
        .unwrap_or_default()
}

// ─── Disposiciones ──────────────────────────────────────────────────

/// `$XDG_CONFIG_HOME/shuma/layouts.json`.
pub(crate) fn layouts_path() -> Option<std::path::PathBuf> {
    data_dir().map(|d| d.join("layouts.json"))
}

/// Lee las disposiciones guardadas.
pub(crate) fn load_layouts() -> Vec<LayoutSnapshot> {
    layouts_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<LayoutSnapshot>>(&s).ok())
        .unwrap_or_default()
}

/// Persiste la lista de disposiciones.
pub(crate) fn save_layouts(layouts: &[LayoutSnapshot]) {
    let Some(path) = layouts_path() else {
        return;
    };
    if let Ok(json) = serde_json::to_string_pretty(layouts) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json);
    }
}

/// Snapshot del espacio de trabajo actual.
pub(crate) fn snapshot_workspace(m: &Model, name: String) -> LayoutSnapshot {
    let sessions: Vec<SessionConfig> = m
        .sessions
        .iter()
        .filter(|s| s.kind != SessionKind::Draft && !s.pending)
        .map(|s| s.to_config())
        .collect();
    LayoutSnapshot {
        name,
        sessions,
        chrome: ChromeState {
            active_tool: m.active_tool,
            session_panel_open: m.session_panel_open,
            active_session: m.active_session,
            session_w: m.session_w,
            monitors_width: m.monitors_width,
        },
    }
}
