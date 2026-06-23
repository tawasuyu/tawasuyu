//! Chrome del shell: barra de menú principal, árbol de navegación,
//! tira de pestañas, barra de estado, menús contextuales (overlay) y el
//! dispatch del contenido central según la vista activa.
//!
//! Los menús (principal y contextual) comparten una representación común
//! [`MenuEntry`]/[`MenuCmd`]: `menu::overlay_view` arma los `ContextMenuItem`
//! desde la lista y `main::update` resuelve el índice clickeado contra la
//! misma lista — una sola fuente de verdad para que no se desincronicen.
//!
//! Submódulos:
//! - `menu`     — entradas de menú + overlay
//! - `nav`      — árbol de navegación
//! - `dock`     — rail de dientes + paneles acoplables
//! - `estado`   — barra de menú y barra de estado
//! - `graficas` — gráficas centrales (rueda, dial, armónica, esfera, cielo)
//! - `impresion`— hoja imprimible
//! - `config`   — panel de configuración
//! - `rectificar` — panel del rectificador de hora

mod config;
mod dock;
mod estado;
mod graficas;
mod impresion;
pub(crate) mod menu;
mod nav;
mod rectificar;

// =====================================================================
// Re-exports para que los sitios de llamada (main.rs, tools.rs, etc.)
// no tengan que cambiar sus rutas de importación.
// =====================================================================

pub(crate) use menu::{ctx_entries, menu_entries, nav_ctx_entries, MenuCmd, MenuEntry, NavAct, NavCtxItem};
pub(crate) use nav::{nav_content_h, nav_viewport_h, NAV_HEADER_H, NAV_ROW_H, NAV_TOOLBAR_H};
pub(crate) use nav::visible_nav_nodes;
pub(crate) use nav::kind_label_es;
pub(crate) use dock::{dock_collapsed, dock_panel_for, dock_rail_for};
pub(crate) use estado::{menu_bar, status_bar};
pub(crate) use graficas::center_view;
pub(crate) use graficas::armonico::harmonic_flower_cmds;
pub(crate) use graficas::dial::uranian_dial_cmds;
pub(crate) use impresion::{print_page, print_page_content, print_sheet_h, print_viewport_h};
pub(crate) use config::config_view;
pub(crate) use rectificar::rectify_view;
pub(crate) use menu::overlay_view;

// =====================================================================
// Constantes compartidas entre submódulos
// =====================================================================

/// Lado del lienzo de cada carta en modo mosaico.
pub(super) const TILE_SIZE: f32 = 360.0;

// =====================================================================
// Helpers compartidos entre submódulos (color/paleta)
// =====================================================================

/// Marca de "activo" en las entradas de menú. Bullet (U+2022, presente en
/// las fuentes default) en vez de ✓ que cae como `.notdef`.
pub(super) fn check(label: &str, on: bool) -> String {
    if on {
        format!("•  {label}")
    } else {
        format!("     {label}")
    }
}
