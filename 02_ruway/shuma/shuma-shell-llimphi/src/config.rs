//! Configuración del chasis: lectura del `shumarc-modules.toml`.
//!
//! El config controla qué módulos ocupan cada slot y con qué
//! parámetros. Cualquier `id` que no esté compilado en este binario
//! se ignora con un warning a stderr — un shumarc no debe romper el
//! arranque.
//!
//! Esquema:
//!
//! ```toml
//! [topbar]
//! module = "launcher"
//!
//! [bottombar]
//! module = "command-bar"
//!
//! [main]
//! module = "matilda"
//! source = { kind = "local" }
//!
//! [[drawer.tabs]]
//! id = "shell"
//! source = { kind = "local" }
//! label = "Shell"
//!
//! [[drawer.tabs]]
//! id = "matilda"
//! source = { kind = "remote", host = "edge-1", user = "ops" }
//! label = "edge-1"
//!
//! [drawer.trigger]
//! key = "F12"
//! hover = false
//! height_fraction = 0.4
//! ```
//!
//! Defaults aplicables:
//! - Sin `[topbar]` → launcher (demo entries).
//! - Sin `[bottombar]` → command-bar local.
//! - Sin `[main]` → vacío (placeholder).
//! - Sin `[[drawer.tabs]]` → shell + matilda locales.
//! - Sin `[drawer.trigger]` → F12, no hover, 40 %.

use serde::Deserialize;
use shuma_module::{DrawerTrigger, Source};
use std::path::{Path, PathBuf};

/// Una entrada simple "qué módulo + opciones" para los slots TopBar/
/// Main/BottomBar. Sin label porque la barra superior/inferior no las
/// muestra y el Main usa el label canónico del módulo.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SlotEntry {
    /// `id` del módulo a activar.
    pub module: String,
    #[serde(default)]
    pub source: Source,
    /// Override de label (donde aplique).
    #[serde(default)]
    pub label: Option<String>,
}

/// Una entrada del array `[[drawer.tabs]]`. Mismo shape pero con el
/// `id` separado del campo `module` por convención del shumarc.
#[derive(Debug, Clone, Deserialize)]
pub struct DrawerTabEntry {
    /// `id` del módulo a activar como tab.
    pub id: String,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DrawerSection {
    #[serde(default)]
    pub tabs: Vec<DrawerTabEntry>,
    #[serde(default)]
    pub trigger: Option<DrawerTrigger>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ShumaConfig {
    pub topbar: Option<SlotEntry>,
    pub bottombar: Option<SlotEntry>,
    pub main: Option<SlotEntry>,
    #[serde(default)]
    pub drawer: DrawerSection,
}

impl ShumaConfig {
    /// Ruta canónica: `$XDG_CONFIG_HOME/shuma/shumarc-modules.toml`.
    /// Es **distinto** del `shumarc.toml` clásico (aliases/env/prompt)
    /// para que el chasis no acople su parseo al de `shuma-config`.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "shuma")
            .map(|d| d.config_dir().join("shumarc-modules.toml"))
    }

    /// Lee el config del path. Si no existe, devuelve `Self::default()`
    /// sin error. Si está mal formado, log a stderr y devuelve default
    /// (un shumarc roto no debe impedir arrancar el shell).
    pub fn load(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!(
                        "shuma: {} mal formado ({e}), uso defaults",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("shuma: no se pudo leer {} ({e})", path.display());
                Self::default()
            }
        }
    }

    /// Lee el config del path por defecto. Si no hay `ProjectDirs`
    /// (caso raro), devuelve defaults.
    pub fn load_default() -> Self {
        match Self::default_path() {
            Some(p) => Self::load(&p),
            None => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_yields_default() {
        let d = tempdir().unwrap();
        let c = ShumaConfig::load(&d.path().join("nope.toml"));
        assert!(c.topbar.is_none());
        assert!(c.drawer.tabs.is_empty());
    }

    #[test]
    fn parses_a_full_shumarc() {
        let d = tempdir().unwrap();
        let path = d.path().join("shumarc-modules.toml");
        std::fs::write(
            &path,
            r#"
[topbar]
module = "launcher"

[bottombar]
module = "command-bar"

[main]
module = "matilda"
source = { kind = "local" }
label = "Servidores"

[[drawer.tabs]]
id = "shell"
source = { kind = "local" }
label = "Shell"

[[drawer.tabs]]
id = "matilda"
source = { kind = "remote", host = "edge-1.example", user = "deploy" }
label = "edge-1"

[drawer.trigger]
key = "F11"
hover = true
height_fraction = 0.5
"#,
        )
        .unwrap();

        let c = ShumaConfig::load(&path);
        assert_eq!(c.topbar.unwrap().module, "launcher");
        assert_eq!(c.bottombar.unwrap().module, "command-bar");
        let main = c.main.unwrap();
        assert_eq!(main.module, "matilda");
        assert_eq!(main.label.as_deref(), Some("Servidores"));
        assert_eq!(c.drawer.tabs.len(), 2);
        assert_eq!(c.drawer.tabs[0].id, "shell");
        match &c.drawer.tabs[1].source {
            Source::Remote { host, user, .. } => {
                assert_eq!(host, "edge-1.example");
                assert_eq!(user, "deploy");
            }
            _ => panic!("expected Remote"),
        }
        let t = c.drawer.trigger.unwrap();
        assert_eq!(t.key.as_deref(), Some("F11"));
        assert!(t.hover);
        assert!((t.height_fraction - 0.5).abs() < 1e-6);
    }

    #[test]
    fn broken_toml_returns_default_without_panic() {
        let d = tempdir().unwrap();
        let path = d.path().join("shumarc-modules.toml");
        std::fs::write(&path, "this = is { broken").unwrap();
        let c = ShumaConfig::load(&path);
        assert!(c.topbar.is_none()); // default
    }

    #[test]
    fn partial_config_falls_back_to_defaults_per_field() {
        let d = tempdir().unwrap();
        let path = d.path().join("p.toml");
        std::fs::write(
            &path,
            r#"
[main]
module = "shell"
"#,
        )
        .unwrap();
        let c = ShumaConfig::load(&path);
        assert!(c.topbar.is_none());
        assert!(c.bottombar.is_none());
        assert_eq!(c.main.as_ref().unwrap().module, "shell");
        assert!(c.drawer.tabs.is_empty());
        assert!(c.drawer.trigger.is_none());
    }
}
