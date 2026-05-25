//! `StatusPanel` — panel lateral con info de la app, switcher de tema y
//! controles de swap del kind del contenedor objetivo (Fase 5).
//!
//! Recibe el `Entity<LayoutModel>` para mutarlo: cuando el usuario clickea
//! "Tabs", el StatusPanel hace
//! `model.update(cx, |m, cx| m.set_kind(target_id, "Tabs", cx))`. El
//! LayoutHost está suscripto al model y rebuildea preservando los hijos
//! del contenedor por NodeId — eso es lo que demuestra que swappear el
//! contenedor no resetea el contenido (las pestañas activas, expansiones
//! del FileExplorer, scroll position, todo se mantiene).

use gpui::{
    ClickEvent, Context, Entity, IntoElement, Render, SharedString, Window, div, prelude::*, px,
};

use nahual_core::{LayerConfig, NodeId};
use nahual_theme::Theme;

use crate::layout_model::LayoutModel;

/// Id del contenedor que el StatusPanel controla con sus botones de swap.
/// Hardcoded por ahora; en el futuro lo podríamos leer de un param del
/// JSON (e.g. `target_id`) para que cualquier StatusPanel apunte al
/// container que querramos.
const SWAP_TARGET_ID: &str = "explorers";

const SWAPPABLE_KINDS: &[&str] = &["Split", "Tabs", "Tiled"];

/// Path del JSON que el botón "Reload" relee. Mantenemos consistencia con
/// `main.rs` — si llegamos a parametrizarlo, hacerlo en un solo lugar.
const LAYOUT_PATH: &str = "layout.json";

pub struct StatusPanel {
    model: Entity<LayoutModel>,
    last_event: SharedString,
}

impl StatusPanel {
    pub fn new(model: Entity<LayoutModel>, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        // Subscripción al model para refrescar el "kind activo" del swap.
        cx.observe(&model, |_, _, cx| cx.notify()).detach();
        Self {
            model,
            last_event: "(esperando bus app-level — Fase 6)".into(),
        }
    }

    /// Reservado para Fase 6: el AppBus va a actualizar este texto.
    #[allow(dead_code)]
    pub fn set_status(&mut self, text: SharedString, cx: &mut Context<Self>) {
        self.last_event = text;
        cx.notify();
    }

    fn cycle_theme(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let current = Theme::global(cx).name.to_string();
        Theme::set(cx, Theme::next_after(&current));
        cx.notify();
    }

    fn pick_theme(
        &mut self,
        name: SharedString,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(theme) = Theme::by_name(&name) {
            Theme::set(cx, theme);
            cx.notify();
        }
    }

    fn swap_container(
        &mut self,
        kind: SharedString,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = NodeId::new(SWAP_TARGET_ID);
        self.model.update(cx, |m, cx| {
            m.set_kind(&target, &kind, cx);
        });
    }

    fn reload_from_disk(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Releemos el JSON. Si el parsing falla, `load_or_default` cae al
        // árbol default — no panicamos. La preservación de hijos
        // funciona vía el id JSON: lo que matchee, persiste; lo nuevo se
        // instancia.
        let tree = LayerConfig::load_or_default(LAYOUT_PATH);
        self.model.update(cx, |m, cx| m.replace_tree(tree, cx));
    }

    /// Lee el `kind` actual del contenedor objetivo desde el model. Si el
    /// id no existe (alguien cambió el JSON), devuelve `None` y no
    /// resaltamos ningún chip.
    fn current_target_kind(&self, cx: &Context<Self>) -> Option<String> {
        find_kind(self.model.read(cx).tree(), SWAP_TARGET_ID)
    }
}

fn find_kind(node: &nahual_core::LayerConfig, target: &str) -> Option<String> {
    if let Some(id) = &node.id {
        if id == target {
            return Some(node.kind.clone());
        }
    }
    for child in &node.children {
        if let Some(k) = find_kind(child, target) {
            return Some(k);
        }
    }
    None
}

impl Render for StatusPanel {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let active_theme = theme.name;
        let theme_chips: Vec<_> = Theme::all()
            .into_iter()
            .map(|t| (t.name, t.name == active_theme))
            .collect();

        let current_kind = self.current_target_kind(cx);

        // Theme chips.
        let mut theme_row = div().flex().flex_row().flex_wrap().gap(px(6.0));
        for (name, is_active) in theme_chips {
            let bg = if is_active {
                theme.bg_row_active
            } else {
                theme.bg_row_hover
            };
            let border = if is_active {
                theme.accent_strong
            } else {
                theme.border
            };
            let chip_id = SharedString::from(format!("theme-chip-{}", name));
            let chip_name = SharedString::from(name.to_string());
            theme_row = theme_row.child(
                div()
                    .id(chip_id)
                    .px(px(10.0))
                    .py(px(5.0))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(border)
                    .bg(bg)
                    .text_size(px(11.0))
                    .text_color(theme.fg_text)
                    .hover(|s| s.opacity(0.85))
                    .child(SharedString::from(name.to_string()))
                    .on_click(cx.listener(move |this, click, w, cx| {
                        this.pick_theme(chip_name.clone(), click, w, cx);
                    })),
            );
        }

        // Swap chips — uno por kind (Split / Tabs / Tiled). El activo
        // refleja el `kind` actual del nodo objetivo.
        let mut swap_row = div().flex().flex_row().gap(px(6.0));
        for &kind in SWAPPABLE_KINDS {
            let is_active = current_kind.as_deref() == Some(kind);
            let bg = if is_active {
                theme.bg_row_active
            } else {
                theme.bg_row_hover
            };
            let border = if is_active {
                theme.accent_strong
            } else {
                theme.border
            };
            let chip_id = SharedString::from(format!("swap-chip-{}", kind));
            let kind_str = SharedString::from(kind.to_string());
            swap_row = swap_row.child(
                div()
                    .id(chip_id)
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(border)
                    .bg(bg)
                    .text_size(px(12.0))
                    .text_color(theme.fg_text)
                    .hover(|s| s.opacity(0.85))
                    .child(SharedString::from(kind.to_string()))
                    .on_click(cx.listener(move |this, click, w, cx| {
                        this.swap_container(kind_str.clone(), click, w, cx);
                    })),
            );
        }

        div()
            .size_full()
            .bg(theme.bg_panel_alt.clone())
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(
                div()
                    .text_size(px(22.0))
                    .text_color(theme.accent_strong)
                    .child("Yahweh — Fase 5"),
            )
            .child(
                div()
                    .text_color(theme.fg_muted)
                    .text_size(px(12.0))
                    .child("Contenedores intercambiables sin perder hijos."),
            )
            // ----- persistencia + reload -----
            .child(
                div()
                    .mt(px(14.0))
                    .text_color(theme.fg_muted)
                    .text_size(px(11.0))
                    .child("layout.json:"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(theme.fg_muted)
                            .child(
                                "auto-save al swap o al soltar un divisor.",
                            ),
                    )
                    .child(
                        div()
                            .id("reload-from-disk")
                            .px(px(10.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(theme.border_strong)
                            .bg(theme.bg_panel.clone())
                            .text_color(theme.fg_text)
                            .text_size(px(11.0))
                            .hover(|s| s.opacity(0.85))
                            .on_click(cx.listener(Self::reload_from_disk))
                            .child("⤓ reload"),
                    ),
            )
            // ----- swap del contenedor 'explorers' -----
            .child(
                div()
                    .mt(px(14.0))
                    .text_color(theme.fg_muted)
                    .text_size(px(11.0))
                    .child(SharedString::from(format!(
                        "contenedor '{}' — kind:",
                        SWAP_TARGET_ID
                    ))),
            )
            .child(swap_row)
            .child(
                div()
                    .mt(px(2.0))
                    .text_color(theme.fg_muted)
                    .text_size(px(10.0))
                    .child(
                        "click ⇒ swappea el kind del contenedor padre. Los hijos \
                         (FileExplorer, DatabaseExplorer) se preservan: cualquier \
                         folder expandido o entry seleccionado sigue así tras el swap.",
                    ),
            )
            // ----- log de evento (placeholder Fase 6) -----
            .child(
                div()
                    .mt(px(16.0))
                    .text_color(theme.fg_muted)
                    .text_size(px(11.0))
                    .child("último evento (bus Fase 6):"),
            )
            .child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .bg(theme.bg_panel.clone())
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(4.0))
                    .text_size(px(11.0))
                    .text_color(theme.fg_text)
                    .child(self.last_event.clone()),
            )
            // ----- theme switcher -----
            .child(
                div()
                    .mt(px(20.0))
                    .text_color(theme.fg_muted)
                    .text_size(px(11.0))
                    .child("tema activo:"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_color(theme.accent)
                            .text_size(px(15.0))
                            .child(SharedString::from(theme.name.to_string())),
                    )
                    .child(
                        div()
                            .id("cycle-theme")
                            .px(px(10.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(theme.border_strong)
                            .bg(theme.bg_panel.clone())
                            .text_color(theme.fg_text)
                            .text_size(px(11.0))
                            .hover(|s| s.opacity(0.85))
                            .on_click(cx.listener(Self::cycle_theme))
                            .child("⇄ siguiente"),
                    ),
            )
            .child(theme_row)
    }
}
