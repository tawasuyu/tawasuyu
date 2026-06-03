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
    pub general: GeneralConfig,
    #[serde(default)]
    pub panel: PanelConfig,
}

/// Settings transversales a todos los widgets: zona horaria, etc. NTP
/// sync no se implementa acá — es responsabilidad del SO (en wawa, un
/// daemon de akasha; en Linux, systemd-timesyncd / chrony).
#[derive(Debug, Clone, Deserialize)]
pub struct GeneralConfig {
    /// "auto" detecta del sistema (`TZ` env / `/etc/timezone` /
    /// `/etc/localtime`). Si auto falla, cae a UTC.
    /// Acepta también nombres IANA (`America/Lima`) cuando chrono-tz
    /// esté disponible; en MVP sólo procesamos "auto" y "UTC".
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self { timezone: default_timezone() }
    }
}

fn default_timezone() -> String { "auto".into() }

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
    /// Tarjetas flotantes tipo conky en el área debajo de la barra.
    /// Cada una tiene posición absoluta en píxeles desde la esquina
    /// superior-izquierda del área libre.
    #[serde(default)]
    pub floating: Vec<FloatingCard>,
    /// Barra inferior opcional. Pensada para alojar un único widget
    /// grande tipo launcher (e.g. `shuma_bar`) o varios que se
    /// distribuyen con flex.
    #[serde(default)]
    pub bottom: Option<BottomBar>,
}

/// Barra inferior — sin slots, lista de widgets que se distribuyen
/// horizontalmente con flex_grow=1 cada uno (un único widget ocupa todo).
#[derive(Debug, Clone, Deserialize)]
pub struct BottomBar {
    #[serde(default = "default_bottom_height")]
    pub height: f32,
    /// Si `true`, la barra se autoesconde: en reposo sólo se ve una franja
    /// fina en el borde inferior que la revela al pasar el puntero; al subir
    /// el puntero al área libre se vuelve a esconder.
    #[serde(default)]
    pub autohide: bool,
    #[serde(default)]
    pub widgets: Vec<WidgetSpec>,
}

fn default_bottom_height() -> f32 { 40.0 }

/// Una tarjeta posicionada en píxeles dentro del área libre.
#[derive(Debug, Clone, Deserialize)]
pub struct FloatingCard {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Título opcional (chip arriba a la izquierda).
    #[serde(default)]
    pub title: Option<String>,
    /// Widgets dentro de la tarjeta, apilados verticalmente.
    #[serde(default)]
    pub widgets: Vec<WidgetSpec>,
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
        Self { general: GeneralConfig::default(), panel: PanelConfig::default() }
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
                WidgetSpec { kind: "brightness".into(), props: HashMap::new() },
                WidgetSpec { kind: "volume".into(), props: HashMap::new() },
                WidgetSpec { kind: "clipboard".into(), props: HashMap::new() },
                WidgetSpec { kind: "system_tray".into(), props: HashMap::new() },
                WidgetSpec { kind: "ram_meter".into(), props: HashMap::new() },
                WidgetSpec { kind: "cpu_meter".into(), props: HashMap::new() },
                WidgetSpec {
                    kind: "quake_input".into(),
                    props: [(
                        "hotkey".to_string(),
                        toml::Value::String("F12".to_string()),
                    )]
                    .into(),
                },
            ],
            floating: Vec::new(),
            bottom: None,
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
        for kind in ["brightness", "volume", "clipboard", "ram_meter", "cpu_meter", "quake_input"] {
            assert!(
                cfg.panel.right.iter().any(|w| w.kind == kind),
                "default no incluye widget {kind}",
            );
        }
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

    #[test]
    fn quake_default_carries_f12_hotkey() {
        let cfg = Config::default();
        let q = cfg.panel.right.iter().find(|w| w.kind == "quake_input").unwrap();
        assert_eq!(q.str_prop("hotkey", "?"), "F12");
    }

    #[test]
    fn general_defaults_timezone_auto() {
        let cfg = Config::default();
        assert_eq!(cfg.general.timezone, "auto");
    }

    #[test]
    fn parses_bottom_bar_with_shuma() {
        let src = r#"
            [panel.bottom]
            height = 48
            autohide = false

            [[panel.bottom.widgets]]
            kind = "shuma_bar"
            placeholder = "› shuma"
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        let b = cfg.panel.bottom.expect("debe haber bottom bar");
        assert_eq!(b.height, 48.0);
        assert!(!b.autohide);
        assert_eq!(b.widgets.len(), 1);
        assert_eq!(b.widgets[0].kind, "shuma_bar");
        assert_eq!(b.widgets[0].str_prop("placeholder", "?"), "› shuma");
    }

    #[test]
    fn parses_floating_card() {
        let src = r#"
            [[panel.floating]]
            x = 40
            y = 80
            w = 280
            h = 140
            title = "sistema"

            [[panel.floating.widgets]]
            kind = "ram_meter"

            [[panel.floating.widgets]]
            kind = "cpu_meter"
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert_eq!(cfg.panel.floating.len(), 1);
        let card = &cfg.panel.floating[0];
        assert_eq!(card.x, 40.0);
        assert_eq!(card.title.as_deref(), Some("sistema"));
        assert_eq!(card.widgets.len(), 2);
        assert_eq!(card.widgets[0].kind, "ram_meter");
    }
}
