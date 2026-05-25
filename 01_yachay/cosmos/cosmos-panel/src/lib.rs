//! `cosmobiologia-panel` — control panel inferior de la app.
//!
//! Lee los módulos disponibles para la carta activa (vía
//! [`cosmobiologia_modules::Registry::for_kind`]) y pinta sus
//! [`Control`]s como toggles / sliders / selects. Cada cambio emite
//! [`PanelEvent`] que la app traduce a mutaciones de visibilidad sobre
//! el canvas y al `module_configs` del shell.
//!
//! ## Estado interno
//!
//! - `toggle_state: HashMap<(module_id, key), bool>` — valor actual de
//!   cada toggle. Se inicializa lazy desde los defaults del módulo al
//!   cambiar de `ChartKind`.
//! - `slider_state: HashMap<(module_id, key), f64>` — valor actual de
//!   cada slider. El shell puede sobreescribirlo via [`Self::set_slider`]
//!   cuando cambia la carta activa (ej. inicializar `target_age_years`
//!   con la edad actual del sujeto).
//! - `slider_drag: Option<SliderDrag>` — slider que está bajo drag
//!   activo. Mutuamente excluyente: solo se arrastra un slider a la vez.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::collections::HashMap;

use gpui::{
    Bounds, ClickEvent, Context, EventEmitter, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, SharedString, Styled,
    Window, canvas, div, prelude::*, px,
};

use cosmobiologia_model::ChartKind;
use cosmobiologia_modules::{Control, Registry, SelectOption};
use nahual_theme::Theme;

// =====================================================================
// Eventos
// =====================================================================

#[derive(Clone, Debug)]
pub enum PanelEvent {
    ModuleToggled { module_id: String, enabled: bool },
    ControlChanged {
        module_id: String,
        key: String,
        value: serde_json::Value,
    },
    /// Click sobre un `Control::Action`. El shell decide qué hacer
    /// (típicamente: capturar la carta derivada del overlay como
    /// `FreeChart`).
    Action {
        module_id: String,
        key: String,
    },
}

/// Opción que el host inyecta al panel para que los `Control::ChartPicker`
/// puedan mostrar el dropdown. El `id` es el ULID stringificado de la
/// carta; el `label` es lo que se muestra en el dropdown.
#[derive(Clone, Debug)]
pub struct ChartOption {
    pub id: String,
    pub label: String,
}

// =====================================================================
// Estado interno
// =====================================================================

#[derive(Clone, Debug)]
struct SliderDrag {
    module_id: String,
    key: String,
    min: f64,
    max: f64,
}

// =====================================================================
// Widget
// =====================================================================

pub struct ControlPanel {
    active_kind: Option<ChartKind>,
    toggle_state: HashMap<(String, String), bool>,
    slider_state: HashMap<(String, String), f64>,
    slider_drag: Option<SliderDrag>,
    /// Opciones globales para todos los `ChartPicker` — las inyecta el
    /// shell vía [`Self::set_chart_options`]. Compartido entre todos
    /// los pickers porque típicamente representan "todas las cartas
    /// del DB" sin filtros por módulo.
    chart_options: Vec<ChartOption>,
    /// Valor actual de cualquier control basado en string (ChartPicker
    /// y Select comparten storage). `None` = sin selección — el render
    /// muestra placeholder ("automático" en picker, default-label en
    /// select).
    string_state: HashMap<(String, String), Option<String>>,
    /// Si hay un dropdown abierto, su (module_id, key). Mutuamente
    /// excluyente: solo uno abierto a la vez en todo el panel.
    dropdown_open: Option<(String, String)>,
    /// Overrides explícitos del estado expanded/collapsed por módulo.
    /// La semántica del default (sin override) está en
    /// [`Self::is_collapsed`]: natal y módulos enabled = expanded;
    /// el resto collapsed.
    collapse_overrides: HashMap<String, bool>,
    registry: Registry,
}

impl EventEmitter<PanelEvent> for ControlPanel {}

impl ControlPanel {
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        Self {
            active_kind: None,
            toggle_state: HashMap::new(),
            slider_state: HashMap::new(),
            slider_drag: None,
            chart_options: Vec::new(),
            string_state: HashMap::new(),
            dropdown_open: None,
            collapse_overrides: HashMap::new(),
            registry: Registry::with_builtins(),
        }
    }

    /// Decide si el card de un módulo debe pintarse collapsed (solo
    /// header) o expanded (header + controles). La regla: si el usuario
    /// puso un override explícito lo respetamos; sino, natal va
    /// expanded siempre y el resto solo si su toggle "enabled" es true.
    fn is_collapsed(&self, module_id: &str) -> bool {
        if let Some(v) = self.collapse_overrides.get(module_id) {
            return *v;
        }
        if module_id == "natal" {
            return false;
        }
        !self
            .toggle_state
            .get(&(module_id.to_string(), "enabled".to_string()))
            .copied()
            .unwrap_or(false)
    }

    fn toggle_collapsed(&mut self, module_id: String, cx: &mut Context<'_, Self>) {
        let current = self.is_collapsed(&module_id);
        self.collapse_overrides.insert(module_id, !current);
        cx.notify();
    }

    pub fn set_active_kind(&mut self, kind: Option<ChartKind>, cx: &mut Context<'_, Self>) {
        if self.active_kind != kind {
            if let Some(k) = kind {
                for m in self.registry.for_kind(k) {
                    for c in m.controls() {
                        match c {
                            Control::Toggle { key, default, .. } => {
                                self.toggle_state
                                    .entry((m.id().to_string(), key))
                                    .or_insert(default);
                            }
                            Control::Slider { key, default, .. } => {
                                self.slider_state
                                    .entry((m.id().to_string(), key))
                                    .or_insert(default);
                            }
                            Control::ChartPicker { key, .. } => {
                                self.string_state
                                    .entry((m.id().to_string(), key))
                                    .or_insert(None);
                            }
                            Control::Select { key, default, .. } => {
                                self.string_state
                                    .entry((m.id().to_string(), key))
                                    .or_insert(Some(default));
                            }
                            // `TextInput` es un campo de sólo-display que el
                            // shell escribe (resultados, etiquetas) vía
                            // `set_string`; su estado vive en `string_state`.
                            Control::TextInput { key, default, .. } => {
                                self.string_state
                                    .entry((m.id().to_string(), key))
                                    .or_insert(Some(default));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        self.active_kind = kind;
        // Cerrar cualquier dropdown abierto al cambiar de carta.
        self.dropdown_open = None;
        cx.notify();
    }

    /// Setea un toggle desde afuera (sin emitir evento). Usado por el
    /// shell para sincronizar cuando el canvas se autotoggleó via hotkey.
    pub fn set_toggle(&mut self, module_id: &str, key: &str, value: bool, cx: &mut Context<'_, Self>) {
        self.toggle_state
            .insert((module_id.to_string(), key.to_string()), value);
        cx.notify();
    }

    /// Setea un slider desde afuera (sin emitir evento). El shell la
    /// usa, por ejemplo, para inicializar `progression.target_age_years`
    /// con la edad actual del sujeto al cargar una carta nueva.
    pub fn set_slider(&mut self, module_id: &str, key: &str, value: f64, cx: &mut Context<'_, Self>) {
        self.slider_state
            .insert((module_id.to_string(), key.to_string()), value);
        cx.notify();
    }

    /// Reemplaza el catálogo de opciones que muestran los
    /// `Control::ChartPicker`. El shell la llama cada vez que la
    /// jerarquía de cartas cambia (crear/borrar) para que el dropdown
    /// quede al día sin necesidad de re-instanciar el panel.
    pub fn set_chart_options(&mut self, options: Vec<ChartOption>, cx: &mut Context<'_, Self>) {
        self.chart_options = options;
        cx.notify();
    }

    /// Setea el valor de un control basado en string (ChartPicker o
    /// Select) desde afuera, sin emitir. El shell la usa para restaurar
    /// el valor persistido al cargar una carta.
    pub fn set_string(
        &mut self,
        module_id: &str,
        key: &str,
        value: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.string_state
            .insert((module_id.to_string(), key.to_string()), value);
        cx.notify();
    }

    /// Alias retrocompatible — los call-sites antiguos del shell usaban
    /// `set_chart_picker`. Funcionalmente idéntico a [`Self::set_string`].
    pub fn set_chart_picker(
        &mut self,
        module_id: &str,
        key: &str,
        chart_id: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_string(module_id, key, chart_id, cx);
    }

    // ----- internos: handlers -----

    fn on_toggle_click(&mut self, module_id: String, key: String, cx: &mut Context<'_, Self>) {
        let entry = self
            .toggle_state
            .entry((module_id.clone(), key.clone()))
            .or_insert(true);
        *entry = !*entry;
        let new_val = *entry;
        cx.emit(PanelEvent::ControlChanged {
            module_id,
            key,
            value: serde_json::Value::Bool(new_val),
        });
        cx.notify();
    }

    fn start_slider_drag(
        &mut self,
        module_id: String,
        key: String,
        min: f64,
        max: f64,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        cx: &mut Context<'_, Self>,
    ) {
        self.slider_drag = Some(SliderDrag {
            module_id: module_id.clone(),
            key: key.clone(),
            min,
            max,
        });
        self.apply_slider_position(bounds, position, cx);
    }

    fn continue_slider_drag(
        &mut self,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.slider_drag.is_some() {
            self.apply_slider_position(bounds, position, cx);
        }
    }

    fn end_slider_drag(&mut self, cx: &mut Context<'_, Self>) {
        if self.slider_drag.take().is_some() {
            cx.notify();
        }
    }

    fn toggle_dropdown_open(&mut self, module_id: String, key: String, cx: &mut Context<'_, Self>) {
        let key_pair = (module_id, key);
        let new_state = match self.dropdown_open.as_ref() {
            Some(open) if open == &key_pair => None,
            _ => Some(key_pair),
        };
        self.dropdown_open = new_state;
        cx.notify();
    }

    fn select_string_value(
        &mut self,
        module_id: String,
        key: String,
        value: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.string_state
            .insert((module_id.clone(), key.clone()), value.clone());
        self.dropdown_open = None;
        let json_value = match value {
            Some(s) => serde_json::Value::String(s),
            None => serde_json::Value::Null,
        };
        cx.emit(PanelEvent::ControlChanged {
            module_id,
            key,
            value: json_value,
        });
        cx.notify();
    }

    fn apply_slider_position(
        &mut self,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(drag) = self.slider_drag.as_ref().cloned() else {
            return;
        };
        let track_x: f32 = bounds.origin.x.into();
        let track_w: f32 = bounds.size.width.into();
        let mouse_x: f32 = position.x.into();
        let fraction = if track_w > 0.0 {
            ((mouse_x - track_x) / track_w).clamp(0.0, 1.0) as f64
        } else {
            0.0
        };
        let value = drag.min + fraction * (drag.max - drag.min);
        self.slider_state
            .insert((drag.module_id.clone(), drag.key.clone()), value);
        cx.emit(PanelEvent::ControlChanged {
            module_id: drag.module_id,
            key: drag.key,
            value: serde_json::json!(value),
        });
        cx.notify();
    }
}

// =====================================================================
// Render
// =====================================================================

const SLIDER_TRACK_W: f32 = 140.0;
const SLIDER_TRACK_H: f32 = 8.0;
const SLIDER_THUMB: f32 = 12.0;

impl Render for ControlPanel {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let modules: Vec<(String, String, String, Vec<Control>)> = match self.active_kind {
            Some(k) => self
                .registry
                .for_kind(k)
                .iter()
                .map(|m| {
                    (
                        m.id().to_string(),
                        m.label().to_string(),
                        m.description().to_string(),
                        m.controls(),
                    )
                })
                .collect(),
            None => Vec::new(),
        };

        let header = div()
            .h(px(28.0))
            .px(px(12.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.fg_muted)
                    .child("Panel de control"),
            )
            .child(
                div()
                    .ml_auto()
                    .text_size(px(10.0))
                    .text_color(theme.fg_disabled)
                    .child(match self.active_kind {
                        Some(k) => SharedString::from(format!("{:?}", k)),
                        None => SharedString::from("sin carta activa"),
                    }),
            );

        let mut body = div()
            .id("tts-panel-body")
            .flex_grow()
            // `min_h(0)` libera al body de la altura intrínseca de su
            // contenido — sin esto el flex_col padre lo expandiría hasta
            // fit-content y el scroll nunca aparecería.
            .min_h(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(16.0))
            .px(px(12.0))
            .py(px(8.0));

        if modules.is_empty() {
            body = body.child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.fg_disabled)
                    .child("Seleccioná una carta para ver sus controles."),
            );
        } else {
            for (id, label, desc, controls) in &modules {
                body = body.child(self.render_module(&theme, id, label, desc, controls, cx));
            }
        }

        div()
            .size_full()
            .bg(theme.bg_panel.clone())
            .flex()
            .flex_col()
            .child(header)
            .child(body)
    }
}

impl ControlPanel {
    fn render_module(
        &self,
        theme: &Theme,
        module_id: &str,
        label: &str,
        description: &str,
        controls: &[Control],
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let collapsed = self.is_collapsed(module_id);
        let chevron = if collapsed { "▸" } else { "▾" };
        let header_id: SharedString =
            SharedString::from(format!("tts-module-header-{}", module_id));
        let module_id_for_listener = module_id.to_string();
        let header = div()
            .id(gpui::ElementId::from(header_id))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .hover(|s| s.bg(theme.bg_row_hover))
            .rounded(px(4.0))
            .px(px(4.0))
            .py(px(2.0))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.fg_muted)
                    .child(chevron),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_grow()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.fg_text)
                            .child(SharedString::from(label.to_string())),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(theme.fg_muted)
                            .child(SharedString::from(description.to_string())),
                    ),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle_collapsed(module_id_for_listener.clone(), cx);
            }));

        let mut card = div()
            .min_w(px(260.0))
            .p(px(8.0))
            .rounded(px(6.0))
            .bg(theme.bg_panel_alt.clone())
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(header);
        if !collapsed {
            let mut body = div().flex().flex_col().gap(px(4.0));
            for c in controls {
                body = body.child(self.render_control(theme, module_id, c, cx));
            }
            card = card.child(body);
        }
        card
    }

    fn render_control(
        &self,
        theme: &Theme,
        module_id: &str,
        c: &Control,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        match c {
            Control::Toggle {
                key,
                label,
                default,
                hotkey,
            } => self.render_toggle(theme, module_id, key, label, *default, hotkey.as_deref(), cx),
            Control::Slider {
                key,
                label,
                min,
                max,
                default,
                ..
            } => self.render_slider(theme, module_id, key, label, *min, *max, *default, cx),
            Control::ChartPicker { key, label } => {
                self.render_chart_picker(theme, module_id, key, label, cx)
            }
            Control::Select {
                key,
                label,
                options,
                default,
            } => self.render_select(theme, module_id, key, label, options, default, cx),
            Control::TextInput { key, label, default } => {
                // Sólo-display: muestra lo último que el shell escribió
                // con `set_string`, o el `default` si nada se escribió.
                let valor = self
                    .string_state
                    .get(&(module_id.to_string(), key.to_string()))
                    .and_then(|o| o.clone())
                    .unwrap_or_else(|| default.clone());
                display_row(theme, label, &valor)
            }
            Control::Action { key, label } => {
                self.render_action(theme, module_id, key, label, cx)
            }
        }
    }

    fn render_action(
        &self,
        theme: &Theme,
        module_id: &str,
        key: &str,
        label: &str,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let id_str: SharedString =
            SharedString::from(format!("tts-action-{}-{}", module_id, key));
        let id_for_listener = (module_id.to_string(), key.to_string());
        let btn = div()
            .id(gpui::ElementId::from(id_str))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .px(px(10.0))
            .py(px(5.0))
            .rounded(px(6.0))
            .bg(theme.bg_button())
            .hover(|s| s.bg(theme.bg_button_hover()))
            .border_1()
            .border_color(theme.border)
            .text_size(px(11.0))
            .text_color(theme.fg_text)
            .child(SharedString::from(label.to_string()))
            .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                let (m, k) = id_for_listener.clone();
                cx.emit(PanelEvent::Action {
                    module_id: m,
                    key: k,
                });
            }));
        // Wrap en Div plano para que el tipo coincida con el resto
        // de los renderers (`render_control` espera `gpui::Div`).
        div().child(btn)
    }

    fn render_toggle(
        &self,
        theme: &Theme,
        module_id: &str,
        key: &str,
        label: &str,
        default: bool,
        hotkey: Option<&str>,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let active = self
            .toggle_state
            .get(&(module_id.to_string(), key.to_string()))
            .copied()
            .unwrap_or(default);
        let dot_color = if active {
            theme.accent
        } else {
            theme.fg_disabled
        };
        let id_str: SharedString =
            SharedString::from(format!("tts-toggle-{}-{}", module_id, key));
        let id_for_listener = (module_id.to_string(), key.to_string());
        let hotkey_str = hotkey
            .map(|h| format!("[{}]", h))
            .unwrap_or_default();
        let row = div()
            .id(gpui::ElementId::from(id_str))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(6.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .hover(|s| s.bg(theme.bg_row_hover))
            .child(div().size(px(8.0)).rounded(px(4.0)).bg(dot_color))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.fg_text)
                    .child(SharedString::from(label.to_string())),
            )
            .child(
                div()
                    .ml_auto()
                    .text_size(px(10.0))
                    .text_color(theme.fg_muted)
                    .child(SharedString::from(hotkey_str)),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let (m, k) = id_for_listener.clone();
                this.on_toggle_click(m, k, cx);
            }));
        div().child(row)
    }

    fn render_slider(
        &self,
        theme: &Theme,
        module_id: &str,
        key: &str,
        label: &str,
        min: f64,
        max: f64,
        default: f64,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let value = self
            .slider_state
            .get(&(module_id.to_string(), key.to_string()))
            .copied()
            .unwrap_or(default);
        let range = (max - min).max(f64::EPSILON);
        let fraction = ((value - min) / range).clamp(0.0, 1.0) as f32;
        let filled_w = fraction * SLIDER_TRACK_W;
        let thumb_x = (fraction * SLIDER_TRACK_W) - SLIDER_THUMB / 2.0;

        let entity = cx.entity();
        let mod_for_mouse = module_id.to_string();
        let key_for_mouse = key.to_string();
        let canvas_overlay = canvas(
            move |_bounds: Bounds<Pixels>, _w, _cx| (),
            move |bounds: Bounds<Pixels>, _, window, _| {
                // MouseDown sobre el track → start drag + valor inmediato.
                let entity_d = entity.clone();
                let mod_d = mod_for_mouse.clone();
                let key_d = key_for_mouse.clone();
                window.on_mouse_event(move |ev: &MouseDownEvent, _, _w, cx| {
                    if ev.button != MouseButton::Left {
                        return;
                    }
                    if !bounds.contains(&ev.position) {
                        return;
                    }
                    entity_d.update(cx, |this, cx| {
                        this.start_slider_drag(
                            mod_d.clone(),
                            key_d.clone(),
                            min,
                            max,
                            bounds,
                            ev.position,
                            cx,
                        );
                    });
                });

                // MouseMove (durante drag) → continuar solo si ESTE
                // slider es el que está bajo drag.
                let entity_m = entity.clone();
                let mod_m = mod_for_mouse.clone();
                let key_m = key_for_mouse.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, _, _w, cx| {
                    if !ev.dragging() {
                        return;
                    }
                    entity_m.update(cx, |this, cx| {
                        let is_mine = this
                            .slider_drag
                            .as_ref()
                            .map(|d| d.module_id == mod_m && d.key == key_m)
                            .unwrap_or(false);
                        if is_mine {
                            this.continue_slider_drag(bounds, ev.position, cx);
                        }
                    });
                });

                // MouseUp anywhere → terminar drag.
                let entity_u = entity.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, _, _w, cx| {
                    entity_u.update(cx, |this, cx| this.end_slider_drag(cx));
                });
            },
        )
        .absolute()
        .w(px(SLIDER_TRACK_W))
        .h(px(SLIDER_TRACK_H));

        let track = div()
            .relative()
            .w(px(SLIDER_TRACK_W))
            .h(px(SLIDER_TRACK_H))
            .bg(theme.bg_input())
            .rounded(px(SLIDER_TRACK_H / 2.0))
            .child(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(0.0))
                    .h(px(SLIDER_TRACK_H))
                    .w(px(filled_w))
                    .bg(theme.accent)
                    .rounded(px(SLIDER_TRACK_H / 2.0)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(thumb_x))
                    .top(px(-2.0))
                    .w(px(SLIDER_THUMB))
                    .h(px(SLIDER_THUMB))
                    .rounded(px(SLIDER_THUMB / 2.0))
                    .bg(theme.fg_text)
                    .border_1()
                    .border_color(theme.border_strong),
            )
            .child(canvas_overlay);

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .px(px(6.0))
            .py(px(3.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.fg_text)
                            .child(SharedString::from(label.to_string())),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .text_size(px(10.0))
                            .text_color(theme.fg_muted)
                            .child(SharedString::from(format!(
                                "{:.1}  ({}…{})",
                                value, min, max
                            ))),
                    ),
            )
            .child(track)
    }
}

impl ControlPanel {
    fn render_chart_picker(
        &self,
        theme: &Theme,
        module_id: &str,
        key: &str,
        label: &str,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let options: Vec<(String, String)> = self
            .chart_options
            .iter()
            .map(|o| (o.id.clone(), o.label.clone()))
            .collect();
        self.render_dropdown(
            theme,
            module_id,
            key,
            label,
            "(automático)",
            &options,
            true, // incluir opción "(automático)" en el popup
            cx,
        )
    }

    fn render_select(
        &self,
        theme: &Theme,
        module_id: &str,
        key: &str,
        label: &str,
        options: &[SelectOption],
        default: &str,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let opts: Vec<(String, String)> = options
            .iter()
            .map(|o| (o.value.clone(), o.label.clone()))
            .collect();
        let placeholder = options
            .iter()
            .find(|o| o.value == default)
            .map(|o| o.label.clone())
            .unwrap_or_else(|| default.to_string());
        self.render_dropdown(theme, module_id, key, label, &placeholder, &opts, false, cx)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_dropdown(
        &self,
        theme: &Theme,
        module_id: &str,
        key: &str,
        label: &str,
        placeholder: &str,
        options: &[(String, String)],
        include_auto: bool,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let current_value = self
            .string_state
            .get(&(module_id.to_string(), key.to_string()))
            .cloned()
            .flatten();
        let current_label = current_value
            .as_ref()
            .and_then(|v| options.iter().find(|(val, _)| val == v).map(|(_, l)| l.clone()))
            .unwrap_or_else(|| placeholder.to_string());

        let is_open = self
            .dropdown_open
            .as_ref()
            .map(|(m, k)| m == module_id && k == key)
            .unwrap_or(false);

        let module_id_btn = module_id.to_string();
        let key_btn = key.to_string();
        let btn_id: SharedString =
            SharedString::from(format!("tts-dropdown-btn-{}-{}", module_id, key));
        let button = div()
            .id(gpui::ElementId::from(btn_id))
            .px(px(10.0))
            .py(px(5.0))
            .rounded(px(4.0))
            .bg(theme.bg_button())
            .hover(|s| s.bg(theme.bg_button_hover()))
            .border_1()
            .border_color(if is_open {
                theme.accent_strong
            } else {
                theme.border
            })
            .text_size(px(11.0))
            .text_color(theme.fg_text)
            .child(SharedString::from(format!("▾ {}", current_label)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle_dropdown_open(module_id_btn.clone(), key_btn.clone(), cx);
            }));

        let mut wrapper = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(theme.fg_muted)
                    .child(SharedString::from(label.to_string())),
            )
            .child(button);

        if is_open {
            wrapper = wrapper.child(self.render_dropdown_popup(
                theme,
                module_id,
                key,
                options,
                include_auto,
                cx,
            ));
        }

        div().px(px(6.0)).py(px(3.0)).child(wrapper)
    }

    fn render_dropdown_popup(
        &self,
        theme: &Theme,
        module_id: &str,
        key: &str,
        options: &[(String, String)],
        include_auto: bool,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let mut popup = div()
            .absolute()
            .top(px(48.0))
            .left(px(0.0))
            .min_w(px(240.0))
            .py(px(4.0))
            .bg(theme.bg_panel_alt.clone())
            .border_1()
            .border_color(theme.border_strong)
            .rounded(px(6.0))
            .flex()
            .flex_col();

        if include_auto {
            let module_id_clear = module_id.to_string();
            let key_clear = key.to_string();
            let clear_id: SharedString =
                SharedString::from(format!("tts-dropdown-clear-{}-{}", module_id, key));
            popup = popup.child(
                div()
                    .id(gpui::ElementId::from(clear_id))
                    .px(px(12.0))
                    .py(px(5.0))
                    .text_size(px(11.0))
                    .text_color(theme.fg_muted)
                    .hover(|s| s.bg(theme.bg_row_hover))
                    .child("(automático)")
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.select_string_value(
                            module_id_clear.clone(),
                            key_clear.clone(),
                            None,
                            cx,
                        );
                    })),
            );

            if !options.is_empty() {
                popup = popup.child(
                    div()
                        .my(px(3.0))
                        .h(px(1.0))
                        .w_full()
                        .bg(theme.border),
                );
            }
        }

        for (value, opt_label) in options {
            let module_id_pick = module_id.to_string();
            let key_pick = key.to_string();
            let opt_value = value.clone();
            let row_id: SharedString =
                SharedString::from(format!("tts-dropdown-opt-{}-{}-{}", module_id, key, value));
            popup = popup.child(
                div()
                    .id(gpui::ElementId::from(row_id))
                    .px(px(12.0))
                    .py(px(5.0))
                    .text_size(px(11.0))
                    .text_color(theme.fg_text)
                    .hover(|s| s.bg(theme.bg_row_hover))
                    .child(SharedString::from(opt_label.clone()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.select_string_value(
                            module_id_pick.clone(),
                            key_pick.clone(),
                            Some(opt_value.clone()),
                            cx,
                        );
                    })),
            );
        }

        popup
    }
}

fn display_row(theme: &Theme, label: &str, value: &str) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(6.0))
        .py(px(3.0))
        .child(
            div()
                .text_size(px(11.0))
                .text_color(theme.fg_text)
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .ml_auto()
                .text_size(px(10.0))
                .text_color(theme.fg_muted)
                .child(SharedString::from(value.to_string())),
        )
}
