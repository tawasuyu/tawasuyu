//! `launcher-core` — el layout de UN solo motor de launcher reusable.
//!
//! No tres launchers (mirada / shuma / wawa), sino **una** estructura
//! configurable que se monta donde sea. Lo que varía por entorno NO vive
//! acá: el render (Llimphi en host/shuma, compositor en wawa) y la
//! instrucción de ejecución (`app_bus::Launcher`) son adaptadores
//! inyectados. Acá vive sólo el *qué se dibuja y dónde*, como datos puros
//! `no_std` — el mismo TOML/JSON describe la superficie en los tres lados.
//!
//! Generaliza el `WidgetSpec { kind, props }` probado de
//! `mirada-launcher`: cada [`Module`] es un `kind` + props arbitrarios que
//! el render interpreta. Encima monta la estructura de alto nivel:
//! [`Surface`] = barras ([`Bar`], estilo eww, ancladas a un [`Edge`]) +
//! docks ([`Dock`], con tear-off) + flotantes ([`FloatingCard`]) + la
//! barra de menú global ([`AppMenuBar`], estilo mac).

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

// =====================================================================
// Primitivas
// =====================================================================

/// Borde de la pantalla donde se ancla una barra o un dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    /// `true` si la barra es horizontal (top/bottom) — el grosor es alto.
    /// `false` si es vertical (left/right) — el grosor es ancho.
    pub fn is_horizontal(self) -> bool {
        matches!(self, Edge::Top | Edge::Bottom)
    }
}

/// Valor de una prop de módulo. Reemplaza al `toml::Value` de
/// `mirada-launcher` por un tipo portátil (`no_std`), para que el mismo
/// schema sirva en wawa. `untagged`: en TOML/JSON un escalar entra como
/// el variante que matchee (bool / int / float / string).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Prop {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

// =====================================================================
// Módulo (kind + props) — la unidad que vive en un slot
// =====================================================================

/// Un módulo del launcher: `kind` + props. Mismo patrón que
/// `mirada-launcher::WidgetSpec`, generalizado y portátil. Kinds builtin
/// que el render conoce: `clock`, `cpu`, `ram`, `volume`, `brightness`,
/// `clipboard`, `quake`, `shuma_bar`, `app_menu` (slot del menú global),
/// `launch` (botón que lanza `app_id`), `dock` (inserta el dock `id`),
/// `spacer`. Cualquier otro `kind` es un widget propio del host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Module {
    pub kind: String,
    #[serde(default, flatten)]
    pub props: BTreeMap<String, Prop>,
}

impl Module {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            props: BTreeMap::new(),
        }
    }

    /// Agrega una prop (builder).
    pub fn with(mut self, key: impl Into<String>, value: Prop) -> Self {
        self.props.insert(key.into(), value);
        self
    }

    /// Lee una prop string; `default` si falta o no es string.
    pub fn str_prop<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        match self.props.get(key) {
            Some(Prop::Str(s)) => s,
            _ => default,
        }
    }

    /// Lee una prop numérica (acepta int o float); `default` si falta.
    pub fn f64_prop(&self, key: &str, default: f64) -> f64 {
        match self.props.get(key) {
            Some(Prop::Float(f)) => *f,
            Some(Prop::Int(i)) => *i as f64,
            _ => default,
        }
    }

    /// Lee una prop booleana; `default` si falta o no es bool.
    pub fn bool_prop(&self, key: &str, default: bool) -> bool {
        match self.props.get(key) {
            Some(Prop::Bool(b)) => *b,
            _ => default,
        }
    }
}

// =====================================================================
// Barra (estilo eww / mirada) — slots start/center/end
// =====================================================================

/// Una barra anclada a un borde, con tres slots. `start`/`center`/`end`
/// en vez de `left`/`right` para que funcione igual en barras verticales.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    #[serde(default)]
    pub edge: Edge,
    /// Alto (barras horizontales) o ancho (verticales), en px.
    #[serde(default = "default_thickness")]
    pub thickness: f32,
    #[serde(default = "default_padding")]
    pub padding: f32,
    #[serde(default = "default_gap")]
    pub gap: f32,
    /// Si se autoesconde y aparece al hover/hotkey. Aceptado por el schema;
    /// que el render lo respete es cosa del render.
    #[serde(default)]
    pub autohide: bool,
    #[serde(default)]
    pub start: Vec<Module>,
    #[serde(default)]
    pub center: Vec<Module>,
    #[serde(default)]
    pub end: Vec<Module>,
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            edge: Edge::default(),
            thickness: default_thickness(),
            padding: default_padding(),
            gap: default_gap(),
            autohide: false,
            start: Vec::new(),
            center: Vec::new(),
            end: Vec::new(),
        }
    }
}

// =====================================================================
// Dock (con tear-off)
// =====================================================================

/// Un ítem del dock: referencia a una app del registro (`app-bus`) por id,
/// con label/ícono opcionales para override visual.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockEntry {
    pub app_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

impl DockEntry {
    pub fn new(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            label: None,
            icon: None,
        }
    }
}

/// Un dock: fila/columna de apps anclada a un borde. `tear_off` habilita
/// arrancar un ítem y dejarlo como tarjeta flotante (estilo mac). Se
/// referencia desde una barra con un módulo `dock { id }`, o se ancla solo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dock {
    pub id: String,
    #[serde(default = "default_dock_edge")]
    pub edge: Edge,
    #[serde(default = "default_thickness")]
    pub thickness: f32,
    #[serde(default)]
    pub tear_off: bool,
    #[serde(default)]
    pub entries: Vec<DockEntry>,
}

impl Dock {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            edge: default_dock_edge(),
            thickness: default_thickness(),
            tear_off: false,
            entries: Vec::new(),
        }
    }
}

// =====================================================================
// Tarjeta flotante (conky / tear-off)
// =====================================================================

/// Tarjeta posicionada en píxeles. La base de los tear-off: un ítem
/// arrancado del dock se materializa como una de éstas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloatingCard {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub modules: Vec<Module>,
}

// =====================================================================
// Barra de menú global (estilo mac)
// =====================================================================

/// La barra que adopta el menú global de la app focuseada. Cuando está
/// presente en la [`Surface`], las apps no pintan su menú propio: lo
/// publican por el bus de `app-bus` y esta barra lo muestra.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppMenuBar {
    #[serde(default)]
    pub edge: Edge,
    #[serde(default = "default_thickness")]
    pub thickness: f32,
    /// Módulos extra a la derecha del menú (reloj, status…), igual que la
    /// barra de menú de mac tiene los indicadores a la derecha.
    #[serde(default)]
    pub trailing: Vec<Module>,
}

impl Default for AppMenuBar {
    fn default() -> Self {
        Self {
            edge: Edge::Top,
            thickness: default_thickness(),
            trailing: Vec::new(),
        }
    }
}

// =====================================================================
// Surface — la superficie completa, UNA estructura
// =====================================================================

/// La superficie completa del launcher: las barras, los docks, las
/// flotantes y (opcional) la barra de menú global. Esto es lo que se
/// describe en `~/.config/gioser/launcher.toml` y se monta idéntico en
/// host, shuma o wawa.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    #[serde(default)]
    pub bars: Vec<Bar>,
    #[serde(default)]
    pub docks: Vec<Dock>,
    #[serde(default)]
    pub floating: Vec<FloatingCard>,
    #[serde(default)]
    pub app_menu: Option<AppMenuBar>,
}

impl Surface {
    /// Devuelve el dock con ese id, si existe (para resolver un módulo
    /// `dock { id }` o un tear-off).
    pub fn dock(&self, id: &str) -> Option<&Dock> {
        self.docks.iter().find(|d| d.id == id)
    }

    /// Un escritorio sensato de arranque: barra de menú global arriba con
    /// reloj a la derecha, barra inferior con reloj/medidores, y un dock
    /// abajo con tear-off. Sirve de default y de demo.
    pub fn desktop_default() -> Self {
        let dock = Dock {
            id: "principal".into(),
            edge: Edge::Bottom,
            thickness: 56.0,
            tear_off: true,
            entries: Vec::new(), // se llena del AppRegistry en runtime
        };
        Self {
            bars: alloc::vec![Bar {
                edge: Edge::Bottom,
                start: alloc::vec![Module::new("ram"), Module::new("cpu")],
                center: alloc::vec![Module::new("dock").with("id", Prop::Str("principal".into()))],
                end: alloc::vec![Module::new("volume"), Module::new("clock")],
                ..Bar::default()
            }],
            docks: alloc::vec![dock],
            floating: Vec::new(),
            app_menu: Some(AppMenuBar::default()),
        }
    }
}

// ----- defaults -----

fn default_thickness() -> f32 {
    32.0
}
fn default_padding() -> f32 {
    12.0
}
fn default_gap() -> f32 {
    16.0
}
fn default_dock_edge() -> Edge {
    Edge::Bottom
}

// =====================================================================
// Tests (corren con default features = std)
// =====================================================================

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn edge_orientacion() {
        assert!(Edge::Top.is_horizontal());
        assert!(!Edge::Left.is_horizontal());
        assert_eq!(Edge::default(), Edge::Top);
    }

    #[test]
    fn module_props_accesores() {
        let m = Module::new("clock")
            .with("format", Prop::Str("%H:%M".into()))
            .with("scale", Prop::Float(1.5))
            .with("big", Prop::Int(3))
            .with("on", Prop::Bool(true));
        assert_eq!(m.str_prop("format", "?"), "%H:%M");
        assert!((m.f64_prop("scale", 0.0) - 1.5).abs() < 1e-9);
        // Int se lee como f64 también.
        assert!((m.f64_prop("big", 0.0) - 3.0).abs() < 1e-9);
        assert!(m.bool_prop("on", false));
        assert_eq!(m.str_prop("falta", "def"), "def");
    }

    #[test]
    fn surface_default_tiene_menu_y_dock() {
        let s = Surface::desktop_default();
        assert!(s.app_menu.is_some());
        assert_eq!(s.app_menu.as_ref().unwrap().edge, Edge::Top);
        assert!(s.dock("principal").unwrap().tear_off);
        // La barra inferior referencia el dock por módulo.
        let bar = &s.bars[0];
        assert!(bar
            .center
            .iter()
            .any(|m| m.kind == "dock" && m.str_prop("id", "") == "principal"));
    }

    #[test]
    fn parse_surface_desde_toml() {
        // El mismo schema que describiría ~/.config/gioser/launcher.toml.
        let src = r#"
            [[bars]]
            edge = "top"
            thickness = 28

            [[bars.start]]
            kind = "app_menu"

            [[bars.end]]
            kind = "clock"
            format = "%H:%M"

            [[bars.end]]
            kind = "ram"

            [[docks]]
            id = "principal"
            edge = "bottom"
            tear_off = true

            [[docks.entries]]
            app_id = "cosmos"
            icon = "✶"

            [[docks.entries]]
            app_id = "nada"

            [app_menu]
            edge = "top"
        "#;
        let s: Surface = toml::from_str(src).expect("parsea surface");
        assert_eq!(s.bars.len(), 1);
        assert_eq!(s.bars[0].edge, Edge::Top);
        assert_eq!(s.bars[0].thickness, 28.0);
        assert_eq!(s.bars[0].start[0].kind, "app_menu");
        assert_eq!(s.bars[0].end[0].str_prop("format", "?"), "%H:%M");
        let dock = s.dock("principal").expect("dock principal");
        assert!(dock.tear_off);
        assert_eq!(dock.entries.len(), 2);
        assert_eq!(dock.entries[0].app_id, "cosmos");
        assert_eq!(dock.entries[0].icon.as_deref(), Some("✶"));
        assert!(s.app_menu.is_some());
    }

    #[test]
    fn surface_roundtrip_json() {
        let s = Surface::desktop_default();
        let json = serde_json::to_string(&s).unwrap();
        let back: Surface = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
