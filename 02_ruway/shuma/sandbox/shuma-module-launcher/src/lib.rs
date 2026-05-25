//! `shuma-module-launcher` — barra superior fija con apps/shortcuts.
//!
//! Vive en el slot [`Placement::TopBar`] del chasis: una tira corta
//! con accesos directos (apps, comandos del shumarc) que el usuario
//! puede pulsar para invocar. Placeholder por ahora — la integración
//! real con `mirada-launcher` y los `[apps]` del shumarc llega aparte.
//!
//! En el placeholder solo se muestra un label "shuma · launcher"
//! con tres botones de ejemplo (Files, Shell, Matilda) que no hacen
//! nada todavía.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::Theme;
use shuma_module::{ModuleContributions, Placement, ShortcutSpec};

/// `id` canónico del módulo.
pub const ID: &str = "launcher";

/// `Placement` por defecto del módulo. El shumarc puede overrideearlo
/// (p. ej. ponerlo como `DrawerTab` para tenerlo dentro del overlay
/// Quake), pero su lugar natural es la barra superior.
pub const DEFAULT_PLACEMENT: Placement = Placement::TopBar;

/// Estado del módulo. En el placeholder lleva un buffer mínimo: la
/// app que se está hovereando, si hay. Cuando llegue la integración
/// real, aquí vivirán los `[apps]` cargados del shumarc.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// Lista de entradas del launcher (label + acción al click).
    pub entries: Vec<LauncherEntry>,
}

/// Una entrada del launcher: un label y la acción que dispara al click.
/// El chasis traduce la acción a su propio `HostMsg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherEntry {
    pub label: String,
    /// Acción opaca al chasis (lo que tipea el shumarc en `action_id`),
    /// que se enrutará al módulo destino o se interpretará como
    /// comando. El placeholder no la usa.
    pub action_id: String,
}

impl LauncherEntry {
    pub fn new(label: impl Into<String>, action_id: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            action_id: action_id.into(),
        }
    }
}

impl State {
    /// State de demo con entries fijas: Files / Shell / Matilda. El
    /// shumarc real las reemplaza.
    pub fn demo() -> Self {
        Self {
            entries: vec![
                LauncherEntry::new("Files", "open:files"),
                LauncherEntry::new("Shell", "focus:shell"),
                LauncherEntry::new("Matilda", "focus:matilda"),
            ],
        }
    }
}

/// Mensajes del módulo. Por ahora sólo el click en una entry (que el
/// chasis traducirá a un `ShortcutAction::Command` o `FocusTab`).
#[derive(Debug, Clone)]
pub enum Msg {
    /// Click en una entry; lleva el `action_id` para que el chasis lo
    /// resuelva (típicamente buscando un módulo con ese id, o lanzando
    /// el comando si es `cmd:...`).
    EntryClicked(String),
}

pub fn update(state: State, _msg: Msg) -> State {
    state
}

/// Mapea `action_id` a `Msg`. El launcher expone `launcher.toggle` como
/// acción global que el chasis consume directamente (toggle de la
/// TopBar autohide); ningún `action_id` produce un `Msg` propio del
/// launcher todavía.
pub fn dispatch(_action_id: &str) -> Option<Msg> {
    None
}

/// Renderiza la barra superior: el label "shuma" a la izquierda y los
/// botones de entries a la derecha (compactos, alto fijo). Aplica el
/// alto de la app-header global (40 px) para que cuadre con el resto
/// de las apps Llimphi.
pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + 'static + Clone,
) -> View<HostMsg> {
    let brand = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned("shuma".to_string(), 13.0, theme.fg_text, Alignment::Start);

    let mut children: Vec<View<HostMsg>> = vec![brand];
    for entry in &state.entries {
        let lift = lift.clone();
        let action_id = entry.action_id.clone();
        children.push(entry_button(entry.label.clone(), theme, move || {
            lift(Msg::EntryClicked(action_id.clone()))
        }));
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(children)
}

fn entry_button<HostMsg: Clone + 'static>(
    label: String,
    theme: &Theme,
    on_click: impl FnOnce() -> HostMsg,
) -> View<HostMsg> {
    let msg = on_click();
    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .radius(4.0)
    .text_aligned(label, 12.0, theme.fg_text, Alignment::Center)
    .on_click(msg)
}

/// Por consistencia con `Color::accent`. No usado en el placeholder
/// pero referenciado para que pase clippy si el bloque siguiente lo
/// llama desde un panel de "recent apps" o similar.
#[allow(dead_code)]
fn _accent_unused(theme: &Theme) -> Color {
    theme.accent
}

/// Contribuciones: el launcher mismo aporta un shortcut al toolbar
/// general ("Apps") que es redundante con la TopBar pero útil cuando
/// el launcher está oculto (TopBar autohide).
pub fn contributions(_state: &State) -> ModuleContributions {
    ModuleContributions {
        monitors: Vec::new(),
        shortcuts: vec![ShortcutSpec::module_action("Apps", "launcher.toggle")
            .with_hint("Abrir el launcher de apps")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "launcher");
    }

    #[test]
    fn default_placement_is_topbar() {
        assert_eq!(DEFAULT_PLACEMENT, Placement::TopBar);
    }

    #[test]
    fn demo_state_has_three_entries() {
        let s = State::demo();
        assert_eq!(s.entries.len(), 3);
        assert_eq!(s.entries[0].label, "Files");
        assert_eq!(s.entries[1].action_id, "focus:shell");
    }

    #[test]
    fn contributions_expose_apps_shortcut() {
        let s = State::default();
        let c = contributions(&s);
        assert_eq!(c.shortcuts.len(), 1);
        assert_eq!(c.shortcuts[0].label, "Apps");
    }

    #[test]
    fn entry_clicked_message_carries_action_id() {
        let m = Msg::EntryClicked("focus:matilda".into());
        match m {
            Msg::EntryClicked(id) => assert_eq!(id, "focus:matilda"),
        }
    }
}
