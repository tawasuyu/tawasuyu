//! Schema TOML del launcher.
//!
//! El TOML manda; el código no asume cosas que el config no nombre. La
//! resolución por defecto vive en [`Config::load_or_default`].

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Config completa de un launcher. Tope de jerarquía: un solo panel por
/// instancia (varios paneles = varios procesos, simplifica todo).
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub panel: PanelConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PanelConfig {
    /// `top` | `bottom` | `left` | `right` — orientación de la barra.
    /// (Floating tipo conky queda para una iteración posterior.)
    #[serde(default = "default_position")]
    pub position: String,
    /// Alto en píxeles para barras horizontales (ancho para verticales).
    #[serde(default = "default_height")]
    pub height: f32,
    /// Padding interno (px) en el eje principal — entre el borde y el primer widget.
    #[serde(default = "default_padding")]
    pub padding: f32,
    /// Separación entre widgets adyacentes (px).
    #[serde(default = "default_gap")]
    pub gap: f32,
    #[serde(default)]
    pub left: Vec<WidgetSpec>,
    #[serde(default)]
    pub center: Vec<WidgetSpec>,
    #[serde(default)]
    pub right: Vec<WidgetSpec>,
}

/// Una entrada del config: `kind` + props arbitrarios. Los props los lee
/// cada widget builtin como mejor le venga (helpers en [`WidgetSpec`]).
#[derive(Debug, Clone, Deserialize)]
pub struct WidgetSpec {
    pub kind: String,
    #[serde(default, flatten)]
    pub props: HashMap<String, toml::Value>,
}

impl WidgetSpec {
    /// Lee una prop string. Devuelve `default` si falta o no es string.
    pub fn str_prop<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.props.get(key).and_then(|v| v.as_str()).unwrap_or(default)
    }

    /// Lee una prop float. Devuelve `default` si falta o no es número.
    pub fn float_prop(&self, key: &str, default: f64) -> f64 {
        self.props.get(key).and_then(|v| v.as_float()).unwrap_or(default)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self { panel: PanelConfig::default() }
    }
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            position: default_position(),
            height: default_height(),
            padding: default_padding(),
            gap: default_gap(),
            left: vec![WidgetSpec { kind: "clock".into(), props: HashMap::new() }],
            center: vec![],
            right: vec![
                WidgetSpec { kind: "ram_meter".into(), props: HashMap::new() },
                WidgetSpec { kind: "cpu_meter".into(), props: HashMap::new() },
                WidgetSpec { kind: "quake_input".into(), props: HashMap::new() },
            ],
        }
    }
}

fn default_position() -> String { "top".into() }
fn default_height() -> f32 { 32.0 }
fn default_padding() -> f32 { 12.0 }
fn default_gap() -> f32 { 16.0 }

impl Config {
    /// Busca el TOML en los lugares estándar; cae al default si no hay.
    pub fn load_or_default() -> Self {
        for path in candidate_paths() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                match toml::from_str::<Config>(&text) {
                    Ok(cfg) => {
                        eprintln!("mirada-launcher · cargué {}", path.display());
                        return cfg;
                    }
                    Err(e) => {
                        eprintln!(
                            "mirada-launcher · {} no parsea ({e}); intento siguiente",
                            path.display()
                        );
                    }
                }
            }
        }
        eprintln!("mirada-launcher · sin TOML; uso default");
        Config::default()
    }
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        out.push(PathBuf::from(xdg).join("mirada/launcher.toml"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        out.push(PathBuf::from(home).join(".config/mirada/launcher.toml"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_panel_has_clock_and_meters() {
        let cfg = Config::default();
        assert_eq!(cfg.panel.position, "top");
        assert_eq!(cfg.panel.left[0].kind, "clock");
        assert!(cfg.panel.right.iter().any(|w| w.kind == "ram_meter"));
        assert!(cfg.panel.right.iter().any(|w| w.kind == "cpu_meter"));
        assert!(cfg.panel.right.iter().any(|w| w.kind == "quake_input"));
    }

    #[test]
    fn parses_minimal_toml() {
        let src = r#"
            [panel]
            position = "bottom"
            height = 24

            [[panel.left]]
            kind = "clock"
            format = "%H:%M"

            [[panel.right]]
            kind = "ram_meter"
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert_eq!(cfg.panel.position, "bottom");
        assert_eq!(cfg.panel.height, 24.0);
        assert_eq!(cfg.panel.left.len(), 1);
        assert_eq!(cfg.panel.left[0].str_prop("format", "?"), "%H:%M");
        assert_eq!(cfg.panel.right[0].kind, "ram_meter");
    }

    #[test]
    fn unknown_props_are_kept_as_toml_values() {
        let src = r#"
            [[panel.left]]
            kind = "custom"
            color = "rebeccapurple"
            ratio = 0.42
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        let w = &cfg.panel.left[0];
        assert_eq!(w.str_prop("color", "?"), "rebeccapurple");
        assert!((w.float_prop("ratio", 0.0) - 0.42).abs() < 1e-9);
    }
}
