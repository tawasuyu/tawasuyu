//! Persistencia del chasis: sesiones, chrome, disposiciones y containers.
//!
//! Funciones para leer/guardar `sessions.json`, `chrome.json`,
//! `layouts.json` y `containers.json`.

use crate::types::{
    ChromeState, ContainerCfg, LayoutSnapshot, Model, SessionConfig, SessionKind,
};

// ─── Containers ────────────────────────────────────────────────────

pub(crate) fn containers_cfg_path() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.config_dir().join("shuma").join("containers.json"))
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
    directories::BaseDirs::new().map(|b| b.config_dir().join("shuma").join("sessions.json"))
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
    directories::BaseDirs::new().map(|b| b.config_dir().join("shuma").join("chrome.json"))
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
    directories::BaseDirs::new().map(|b| b.config_dir().join("shuma").join("layouts.json"))
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
