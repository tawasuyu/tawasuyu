//! Render del chasis de shuma: topbar, área central, tabs, monitores.
//!
//! El módulo raíz orquesta los sub-módulos y re-exporta lo que `main.rs`
//! necesita: `render_topbar`, `render_bottombar`, `render_main_area`,
//! `dropdown_overlay`, y los modales.

mod chrome;
mod modals;
mod monitors;
mod session;
mod tools;
mod widgets;

// Re-exportaciones públicas hacia main.rs (Shell App).
pub(crate) use chrome::{
    empty_bar, render_bottombar, render_main_area, render_topbar, status_bar,
};
pub(crate) use modals::{containers_modal, hosts_modal, layouts_modal};
pub(crate) use monitors::{curve_view, monitor_card, monitor_stack};
pub(crate) use widgets::placeholder;

use super::*;
use llimphi_ui::View;
use llimphi_theme::Theme;
use llimphi_widget_select::{
    select_menu_view, SelectItem, SelectMenuSpec, SelectPalette, SelectPhase,
};

/// Alto del disparador del select (debe seguir a `llimphi-widget-select`).
const TRIGGER_H: f32 = 34.0;

/// Ítems del dropdown de engine de aislamiento.
pub(crate) fn engine_items() -> Vec<SelectItem> {
    let mut out: Vec<SelectItem> = Vec::new();
    if unshare_disponible() {
        out.push(
            SelectItem::new("unshare".to_string())
                .with_sublabel("util-linux + chroot — sin instalar nada (recomendado)"),
        );
    }
    if bwrap_disponible() {
        out.push(
            SelectItem::new("bwrap".to_string())
                .with_sublabel("bubblewrap — sandbox liviano"),
        );
    }
    if podman_disponible() {
        out.push(
            SelectItem::new("podman".to_string())
                .with_sublabel("OCI completo (con storage.conf)"),
        );
    }
    if out.is_empty() {
        out.push(
            SelectItem::new("(ninguno)".to_string()).with_sublabel(
                "instalá util-linux + coreutils, bubblewrap o podman",
            ),
        );
    }
    out
}

/// Ítems del dropdown de aislamiento.
fn iso_items() -> Vec<SelectItem> {
    vec![
        SelectItem::new("Local").with_sublabel("Directo en esta máquina."),
        SelectItem::new("Remoto (SSH)").with_sublabel("En otra máquina por SSH."),
    ]
}

fn iso_index(iso: Isolation) -> usize {
    Isolation::ALL.iter().position(|x| *x == iso).unwrap_or(0)
}

/// `y` aproximado del disparador de un dropdown dentro del panel de sesión.
fn cfg_trigger_y(is_draft: bool, kind: DropKind) -> f32 {
    let iso_y = if is_draft { 134.0 } else { 92.0 };
    match kind {
        DropKind::Isolation => iso_y,
        DropKind::Engine => iso_y + 50.0,
        DropKind::Distro => iso_y + 98.0,
        DropKind::Container => iso_y + 98.0 + 64.0,
        DropKind::Host => iso_y,
    }
}

/// El menú del dropdown de config abierto (para `App::view_overlay`).
pub(crate) fn dropdown_overlay(model: &Model) -> Option<View<Msg>> {
    let kind = model.dropdown_open?;
    let session = model.active()?;
    if session.pending {
        return None;
    }
    let is_draft = session.kind == SessionKind::Draft;
    let pal = SelectPalette::from_theme(&model.theme);

    let (items, selected_vec): (Vec<SelectItem>, Vec<usize>) = match kind {
        DropKind::Isolation => (iso_items(), vec![iso_index(session.isolation)]),
        DropKind::Host | DropKind::Container | DropKind::Distro | DropKind::Engine => {
            return None
        }
    };
    let visible: Vec<usize> = (0..items.len()).collect();
    let anchor = (12.0, cfg_trigger_y(is_draft, kind) + TRIGGER_H + 4.0);
    let width = (model.session_w - 24.0).max(140.0);

    let n_containers = model.containers.len();
    let on_pick: std::sync::Arc<dyn Fn(usize) -> Msg + Send + Sync> = match kind {
        DropKind::Isolation => {
            std::sync::Arc::new(|i| Msg::SetIsolation(Isolation::ALL[i.min(1)]))
        }
        DropKind::Distro => std::sync::Arc::new(|i| Msg::SetDistro(Distro::ALL[i.min(3)])),
        DropKind::Engine => {
            let items_clone = engine_items();
            std::sync::Arc::new(move |i| {
                let label = items_clone
                    .get(i)
                    .map(|it| it.label.clone())
                    .unwrap_or_default();
                Msg::SetEngine(label)
            })
        }
        DropKind::Container => std::sync::Arc::new(move |i| {
            if i < n_containers {
                Msg::SubscribeContainer(i)
            } else {
                Msg::CreateContainer
            }
        }),
        DropKind::Host => std::sync::Arc::new(|_| Msg::DismissDropdown),
    };

    Some(select_menu_view(SelectMenuSpec {
        anchor,
        viewport: (1280.0, 800.0),
        width,
        phase: SelectPhase::Ready(&items),
        visible: &visible,
        active: usize::MAX,
        selected: &selected_vec,
        query: "",
        searchable: false,
        empty_text: "",
        appear: 1.0,
        on_pick,
        on_hover: None,
        on_dismiss: Msg::DismissDropdown,
        on_retry: None,
        palette: &pal,
    }))
}
