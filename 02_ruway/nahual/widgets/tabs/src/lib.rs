//! `nahual_widget_tabs` — `TabContainer`.
//!
//! `n` hijos `AnyView`, **uno visible** por vez (la pestaña activa). Header
//! horizontal con un botón por hijo; click cambia la pestaña activa. La
//! identidad del hijo activo se preserva por `NodeId`, así que swappear de
//! Split → Tabs y volver no resetea cuál está abierto.
//!
//! API alineada con `SplitContainer` (mismo `set_children`) para que el
//! LayoutHost los use intercambiablemente.

use gpui::{
    ClickEvent, Context, EventEmitter, IntoElement, Render, SharedString, Window, div, prelude::*,
    px,
};

use nahual_core::NodeId;
use nahual_theme::Theme;
use nahual_widget_container_core::ChildSlot;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum TabsEvent {
    /// Una pestaña distinta quedó activa (por click o `set_active`).
    TabActivated { id: NodeId, index: usize },
}

pub struct TabContainer {
    children: Vec<ChildSlot>,
    /// Id del hijo activo. Lo guardamos por id (no por índice) para que
    /// reorders/inserts no rompan la selección.
    active_id: Option<NodeId>,
}

impl EventEmitter<TabsEvent> for TabContainer {}

impl TabContainer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        Self {
            children: Vec::new(),
            active_id: None,
        }
    }

    pub fn set_children(&mut self, children: Vec<ChildSlot>, cx: &mut Context<Self>) {
        // Si el id activo previo sigue presente, preservarlo. Si no, caer
        // al primero (o None si vacío).
        let still_present = self
            .active_id
            .as_ref()
            .map(|id| children.iter().any(|c| &c.id == id))
            .unwrap_or(false);
        if !still_present {
            self.active_id = children.first().map(|c| c.id.clone());
        }
        self.children = children;
        cx.notify();
    }

    pub fn set_active(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if self.children.iter().any(|c| c.id == id) && self.active_id.as_ref() != Some(&id) {
            let index = self.children.iter().position(|c| c.id == id).unwrap_or(0);
            self.active_id = Some(id.clone());
            cx.emit(TabsEvent::TabActivated { id, index });
            cx.notify();
        }
    }

    pub fn active_id(&self) -> Option<&NodeId> {
        self.active_id.as_ref()
    }

    fn active_index(&self) -> Option<usize> {
        let id = self.active_id.as_ref()?;
        self.children.iter().position(|c| &c.id == id)
    }

    fn on_tab_click(
        &mut self,
        id: NodeId,
        _click: &ClickEvent,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_active(id, cx);
    }
}

const TAB_HEADER_HEIGHT: f32 = 30.0;

impl Render for TabContainer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let active_idx = self.active_index();

        // Header — una "pestaña" por hijo. Cada tab usa una stripe inferior
        // (un div hijo de 2px de alto) como indicador de "activa", porque
        // gpui no expone `border_b_color` por separado del border global.
        let mut header = div()
            .h(px(TAB_HEADER_HEIGHT))
            .w_full()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.bg_panel.clone())
            .flex()
            .flex_row();

        for (i, child) in self.children.iter().enumerate() {
            let is_active = active_idx == Some(i);
            let label_text = child
                .label
                .clone()
                .unwrap_or_else(|| child.id.as_str().to_string());
            let id_for_click = child.id.clone();
            let tab_id: SharedString =
                SharedString::from(format!("tab-{}", child.id));

            let bg = if is_active {
                theme.bg_panel_alt.clone()
            } else {
                theme.bg_panel.clone()
            };
            let fg = if is_active {
                theme.fg_text
            } else {
                theme.fg_muted
            };
            let stripe_color = if is_active {
                theme.accent_strong
            } else {
                gpui::hsla(0.0, 0.0, 0.0, 0.0)
            };

            header = header.child(
                div()
                    .id(tab_id)
                    .h_full()
                    .border_r_1()
                    .border_color(theme.border)
                    .bg(bg)
                    .text_color(fg)
                    .text_size(px(12.0))
                    .hover(|s| s.opacity(0.85))
                    .flex()
                    .flex_col()
                    .child(
                        // Etiqueta + padding centrado.
                        div()
                            .flex_grow()
                            .px(px(14.0))
                            .flex()
                            .items_center()
                            .child(SharedString::from(label_text)),
                    )
                    .child(
                        // Stripe inferior de 2px — indicador de activa.
                        div().h(px(2.0)).w_full().bg(stripe_color),
                    )
                    .on_click(cx.listener(move |this, click, w, cx| {
                        this.on_tab_click(id_for_click.clone(), click, w, cx);
                    })),
            );
        }

        // Cuerpo — solo el child activo. Si no hay ninguno (children
        // vacío), pintamos un mensaje neutro.
        let body = match active_idx.and_then(|i| self.children.get(i)) {
            Some(child) => div()
                .flex_grow()
                .min_h(px(0.0))
                .bg(theme.bg_panel_alt.clone())
                .child(child.view.clone())
                .into_any_element(),
            None => div()
                .flex_grow()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.fg_muted)
                .text_size(px(11.0))
                .child("(sin hijos)")
                .into_any_element(),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
    }
}
