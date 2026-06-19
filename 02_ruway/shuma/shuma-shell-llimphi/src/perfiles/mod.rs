//! Perfiles de shuma — tres bibliotecas conmutables, al estilo de
//! `mirada-brain::profiles`:
//!
//! - [`shortcuts`] — **atajos** del workspace (globales, un clic): `shuma`,
//!   `hyprland`, `tmux`, `zellij`, `vim`.
//! - [`appearance`] — **apariencia** estilo konsole (tema, acento, fuente,
//!   transparencia, wallpaper). Activo global = default de toda ventana; cada
//!   sesión puede fijar el suyo.
//! - [`sessions`] — **perfiles de sesión** estilo Firefox (contextos completos
//!   con su propio juego de sesiones/workspaces, aislados por directorio).

pub mod appearance;
pub mod sessions;
pub mod shortcuts;

use llimphi_theme::Theme;

use crate::types::Model;

/// Aplica al `Model` la apariencia que corresponde ahora: la del perfil fijado
/// por la **sesión activa** si lo tiene, o el **default global** si no. El
/// perfil `Sistema` sigue el tema de `wawa-config`.
///
/// Llamar tras conmutar de sesión, conmutar el perfil de apariencia global, o
/// recibir un cambio de tema del sistema.
pub(crate) fn apply_active_appearance(model: &mut Model) {
    // ¿La sesión activa fija una apariencia propia?
    let per_session = model
        .sessions
        .get(model.active_session)
        .and_then(|s| s.appearance.clone());
    let name = per_session.unwrap_or_else(|| model.appearance.active().to_string());
    model.theme = resolve_named(model, &name);
}

/// Resuelve un nombre de apariencia a un `Theme`. `Sistema` (o un nombre
/// desconocido que caiga ahí) toma el tema de wawa; el resto, su preset.
fn resolve_named(model: &Model, name: &str) -> Theme {
    if name == appearance::SYSTEM_NAME {
        let wawa = wawa_config::WawaConfig::load();
        return wawa_config_llimphi::theme_from_wawa(&wawa, &Theme::dark());
    }
    match model.appearance.get(name) {
        Some(ap) => ap.resolve(),
        None => {
            let wawa = wawa_config::WawaConfig::load();
            wawa_config_llimphi::theme_from_wawa(&wawa, &Theme::dark())
        }
    }
}

/// `true` si la apariencia efectiva ahora mismo sigue al sistema (`Sistema`):
/// entonces los cambios de `wawa-config` deben propagarse al tema. Si una
/// sesión o el default fijan un perfil concreto, wawa **no** debe pisarlo.
pub(crate) fn follows_system(model: &Model) -> bool {
    let per_session = model
        .sessions
        .get(model.active_session)
        .and_then(|s| s.appearance.clone());
    match per_session {
        Some(name) => name == appearance::SYSTEM_NAME,
        None => model.appearance.active() == appearance::SYSTEM_NAME,
    }
}
