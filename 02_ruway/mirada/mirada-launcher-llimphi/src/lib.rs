//! mirada-launcher-llimphi — núcleo del launcher/panel configurable.
//!
//! Brutos: editan el TOML. Expertos: enchufan un Rhai por widget (defer).
//! Cada widget builtin es un struct con `tick` (refresh de datos) y `view`
//! (cómo se pinta dentro de la barra). El panel los agrupa en left/center/right.

pub mod config;
pub mod keys;
pub mod panel;
pub mod tray;
pub mod widget;
pub mod widgets;

pub use config::{Config, PanelConfig, WidgetSpec};
pub use widget::{Msg, Widget};
