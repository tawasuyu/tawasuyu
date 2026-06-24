//! Lectura/escritura del perfil de **atajos de terminal de shuma** desde el
//! panel de control. shuma guarda su biblioteca de keymaps (shuma/terminal/
//! hyprland/tmux/zellij/vim + propios) en `~/.config/shuma/shortcuts.ron`; acá
//! sólo conmutamos el **perfil activo** y leemos los nombres disponibles, sin
//! depender del crate de shuma. shuma re-siembra los presets que falten al
//! cargar, así que escribir `(active: "x", profiles: {})` es seguro aunque shuma
//! nunca haya corrido.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Presets de fábrica de shuma, en orden — el fallback cuando el RON no existe
/// todavía. Debe seguir a `shuma_shell_llimphi::perfiles::shortcuts::PRESET_NAMES`.
pub const PRESET_NAMES: &[&str] = &["shuma", "terminal", "hyprland", "tmux", "zellij", "vim"];

/// Espejo mínimo del RON de shuma: el activo + los perfiles como valores opacos
/// (no nos interesa su contenido, sólo preservarlos al reescribir).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShortcutsRon {
    active: String,
    #[serde(default)]
    profiles: BTreeMap<String, ron::Value>,
}

/// `~/.config/shuma/shortcuts.ron` (respeta `XDG_CONFIG_HOME`).
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("shuma").join("shortcuts.ron"))
}

/// El perfil activo y la lista de nombres disponibles. Si el RON no existe o no
/// parsea, cae a los presets de fábrica con `shuma` activo.
pub fn load() -> (String, Vec<String>) {
    let fallback = || {
        (
            "shuma".to_string(),
            PRESET_NAMES.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
    };
    let Some(p) = path() else { return fallback() };
    let Ok(text) = std::fs::read_to_string(&p) else { return fallback() };
    match ron::from_str::<ShortcutsRon>(&text) {
        Ok(r) => {
            // Unimos los presets de fábrica con los del disco (propios), sin
            // duplicar, así el selector siempre ofrece los builtins aunque el
            // RON viejo no los tenga todos.
            let mut names: Vec<String> =
                PRESET_NAMES.iter().map(|s| s.to_string()).collect();
            for k in r.profiles.keys() {
                if !names.contains(k) {
                    names.push(k.clone());
                }
            }
            (r.active, names)
        }
        Err(_) => fallback(),
    }
}

/// Conmuta el perfil activo y reescribe el RON, preservando los perfiles que ya
/// hubiera. Crea el directorio padre si falta.
pub fn set_active(name: &str) -> std::io::Result<()> {
    let Some(p) = path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no pude resolver ~/.config",
        ));
    };
    let mut ron_doc = std::fs::read_to_string(&p)
        .ok()
        .and_then(|t| ron::from_str::<ShortcutsRon>(&t).ok())
        .unwrap_or(ShortcutsRon {
            active: "shuma".to_string(),
            profiles: BTreeMap::new(),
        });
    ron_doc.active = name.to_string();
    let text = ron::ser::to_string_pretty(&ron_doc, ron::ser::PrettyConfig::default())
        .unwrap_or_else(|_| format!("(active: \"{name}\", profiles: {{}})"));
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p, text)
}
