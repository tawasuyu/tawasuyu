//! `select_demo` — recorre el gradiente de complejidad del select en una
//! sola ventana. Tres controles, de tonto a listo:
//!
//! 1. **Simple** — estado de una tarea, sin búsqueda.
//! 2. **Buscable + badges** — asignar a una persona: icono, sublabel y
//!    badge de conteo; teclear filtra, ↑/↓ navega, Enter elige.
//! 3. **Async** — el primer load *falla* (mirá el error + Reintentar); el
//!    reintento trae los datos tras ~900 ms vía `Handle::spawn`, con guard
//!    de generación para descartar respuestas viejas.
//!
//! Corré con:
//!   cargo run -p llimphi-widget-select --example select_demo --release

use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_select::{
    filter, resolve, select_menu_view, select_trigger_view, step_active, BadgeKind, SelectBadge,
    SelectItem, SelectMenuSpec, SelectPalette, SelectPhase,
};

const X: f32 = 48.0;
const Y0: f32 = 96.0;
const ROW: f32 = 96.0;
const W: f32 = 340.0;
const TRIGGER_H: f32 = 36.0;

/// Cuál de los tres selects está abierto.
const SEL_ESTADO: usize = 0;
const SEL_PERSONA: usize = 1;
const SEL_ASYNC: usize = 2;

#[derive(Clone)]
enum Msg {
    Toggle(usize),
    Dismiss,
    Pick(usize, usize), // (cuál select, índice original)
    Hover(usize),       // posición en visible del select abierto
    Key(KeyEvent),
    Retry,
    // Resultado del worker async: (generación, Ok(items) | Err(mensaje))
    AsyncLoaded(u64, Result<Vec<SelectItem>, String>),
}

enum AsyncState {
    Idle,
    Loading,
    Error(String),
    Ready(Vec<SelectItem>),
}

struct Model {
    theme: Theme,
    open: Option<usize>,
    active: usize, // posición en visible
    query: String,

    estado_items: Vec<SelectItem>,
    estado_sel: Option<usize>,

    persona_items: Vec<SelectItem>,
    persona_sel: Option<usize>,

    async_state: AsyncState,
    async_sel: Option<usize>,
    async_gen: u64,
    async_attempts: u32,
}

impl Model {
    /// Ítems del select abierto (para filtro/navegación), si aplica.
    fn open_items(&self) -> Option<&[SelectItem]> {
        match self.open? {
            SEL_ESTADO => Some(&self.estado_items),
            SEL_PERSONA => Some(&self.persona_items),
            SEL_ASYNC => match &self.async_state {
                AsyncState::Ready(items) => Some(items),
                _ => None,
            },
            _ => None,
        }
    }

    fn is_searchable(open: usize) -> bool {
        open == SEL_PERSONA || open == SEL_ASYNC
    }

    fn visible(&self) -> Vec<usize> {
        match self.open_items() {
            Some(items) => filter(items, &self.query),
            None => Vec::new(),
        }
    }
}

fn estado_items() -> Vec<SelectItem> {
    vec![
        SelectItem::new("Pendiente").icon("\u{25CB}").badge(SelectBadge::dot(BadgeKind::Warning)),
        SelectItem::new("En curso").icon("\u{25D0}").badge(SelectBadge::dot(BadgeKind::Info)),
        SelectItem::new("Bloqueado").icon("\u{25A0}").disabled(),
        SelectItem::new("Hecho").icon("\u{25CF}").badge(SelectBadge::dot(BadgeKind::Success)),
    ]
}

fn persona_items() -> Vec<SelectItem> {
    vec![
        SelectItem::new("Sergio Luna")
            .icon("\u{25C9}")
            .with_sublabel("gerencia · dueño")
            .badge(SelectBadge::count(12, BadgeKind::Info)),
        SelectItem::new("Ana Quispe")
            .icon("\u{25C9}")
            .with_sublabel("backend")
            .badge(SelectBadge::count(3, BadgeKind::Neutral)),
        SelectItem::new("Beto Mamani")
            .icon("\u{25C9}")
            .with_sublabel("diseño")
            .badge(SelectBadge::label("beta", BadgeKind::Warning)),
        SelectItem::new("Carmen Rojas")
            .icon("\u{25C9}")
            .with_sublabel("infra · de licencia")
            .disabled(),
        SelectItem::new("Diego Flores")
            .icon("\u{25C9}")
            .with_sublabel("qa")
            .badge(SelectBadge::count(120, BadgeKind::Error)),
    ]
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            theme: Theme::dark(),
            open: None,
            active: usize::MAX,
            query: String::new(),
            estado_items: estado_items(),
            estado_sel: Some(0),
            persona_items: persona_items(),
            persona_sel: None,
            async_state: AsyncState::Idle,
            async_sel: None,
            async_gen: 0,
            async_attempts: 0,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Toggle(which) => {
                if model.open == Some(which) {
                    model.open = None;
                } else {
                    model.open = Some(which);
                    model.query.clear();
                    model.active = usize::MAX;
                    // Abrir el async dispara la carga si no hay datos.
                    if which == SEL_ASYNC && !matches!(model.async_state, AsyncState::Ready(_)) {
                        model = start_load(model, handle);
                    }
                }
            }
            Msg::Dismiss => model.open = None,
            Msg::Hover(pos) => model.active = pos,
            Msg::Pick(which, orig) => {
                match which {
                    SEL_ESTADO => model.estado_sel = Some(orig),
                    SEL_PERSONA => model.persona_sel = Some(orig),
                    SEL_ASYNC => model.async_sel = Some(orig),
                    _ => {}
                }
                model.open = None;
            }
            Msg::Retry => {
                if model.open == Some(SEL_ASYNC) {
                    model = start_load(model, handle);
                }
            }
            Msg::AsyncLoaded(gen, result) => {
                // Guard de generación: descartar respuestas de cargas viejas.
                if gen == model.async_gen {
                    model.async_state = match result {
                        Ok(items) => AsyncState::Ready(items),
                        Err(e) => AsyncState::Error(e),
                    };
                }
            }
            Msg::Key(ev) => {
                if ev.state != KeyState::Pressed {
                    return model;
                }
                let Some(which) = model.open else { return model };
                match &ev.key {
                    Key::Named(NamedKey::Escape) => model.open = None,
                    Key::Named(NamedKey::ArrowDown) => {
                        if let Some(items) = model.open_items() {
                            let vis = filter(items, &model.query);
                            model.active = step_active(items, &vis, model.active, 1);
                        }
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        if let Some(items) = model.open_items() {
                            let vis = filter(items, &model.query);
                            model.active = step_active(items, &vis, model.active, -1);
                        }
                    }
                    Key::Named(NamedKey::Enter) => {
                        let vis = model.visible();
                        if let Some(orig) = resolve(&vis, model.active) {
                            return Self::update(model, Msg::Pick(which, orig), handle);
                        }
                    }
                    Key::Named(NamedKey::Backspace) if Model::is_searchable(which) => {
                        model.query.pop();
                        model.active = usize::MAX;
                    }
                    _ => {
                        if Model::is_searchable(which) {
                            if let Some(text) = &ev.text {
                                if !text.is_empty() && !text.chars().any(|c| c.is_control()) {
                                    model.query.push_str(text);
                                    model.active = usize::MAX;
                                }
                            }
                        }
                    }
                }
            }
        }
        model
    }

    fn on_key(_: &Model, ev: &KeyEvent) -> Option<Msg> {
        Some(Msg::Key(ev.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        let pal = SelectPalette::from_theme(&model.theme);

        let title = View::new(Style {
            position: Position::Absolute,
            inset: Rect { left: length(X), top: length(40.0_f32), right: auto(), bottom: auto() },
            size: Size { width: length(640.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "llimphi-widget-select — simple · buscable+badges · async".to_string(),
            16.0,
            model.theme.fg_text,
            Alignment::Start,
        );

        let estado = labeled_trigger(
            "Estado (simple)",
            SEL_ESTADO,
            select_trigger_view(
                model.estado_sel.and_then(|i| model.estado_items.get(i)),
                "Elegí un estado…",
                model.open == Some(SEL_ESTADO),
                Some(W),
                &pal,
                Msg::Toggle(SEL_ESTADO),
            ),
            &model.theme,
            0,
        );

        let persona = labeled_trigger(
            "Asignar a (buscable · badges)",
            SEL_PERSONA,
            select_trigger_view(
                model.persona_sel.and_then(|i| model.persona_items.get(i)),
                "Buscar persona…",
                model.open == Some(SEL_PERSONA),
                Some(W),
                &pal,
                Msg::Toggle(SEL_PERSONA),
            ),
            &model.theme,
            1,
        );

        let async_selected = match (&model.async_state, model.async_sel) {
            (AsyncState::Ready(items), Some(i)) => items.get(i),
            _ => None,
        };
        let async_t = labeled_trigger(
            "Repositorio (carga async)",
            SEL_ASYNC,
            select_trigger_view(
                async_selected,
                "Cargar repos…",
                model.open == Some(SEL_ASYNC),
                Some(W),
                &pal,
                Msg::Toggle(SEL_ASYNC),
            ),
            &model.theme,
            2,
        );

        View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .children(vec![title, estado, persona, async_t])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        let which = model.open?;
        let pal = SelectPalette::from_theme(&model.theme);
        let anchor = (X, Y0 + which as f32 * ROW + TRIGGER_H + 6.0);
        let visible = model.visible();

        let phase = match which {
            SEL_ESTADO => SelectPhase::Ready(&model.estado_items),
            SEL_PERSONA => SelectPhase::Ready(&model.persona_items),
            SEL_ASYNC => match &model.async_state {
                AsyncState::Loading | AsyncState::Idle => SelectPhase::Loading,
                AsyncState::Error(e) => SelectPhase::Error(e),
                AsyncState::Ready(items) => SelectPhase::Ready(items),
            },
            _ => return None,
        };

        let selected: Vec<usize> = match which {
            SEL_ESTADO => model.estado_sel.into_iter().collect(),
            SEL_PERSONA => model.persona_sel.into_iter().collect(),
            SEL_ASYNC => model.async_sel.into_iter().collect(),
            _ => Vec::new(),
        };

        Some(select_menu_view(SelectMenuSpec {
            anchor,
            viewport: (980.0, 640.0),
            width: W,
            phase,
            visible: &visible,
            active: model.active,
            selected: &selected,
            query: &model.query,
            searchable: Model::is_searchable(which),
            empty_text: "Sin coincidencias",
            appear: 1.0,
            on_pick: std::sync::Arc::new(move |orig| Msg::Pick(which, orig)),
            on_hover: Some(std::sync::Arc::new(Msg::Hover)),
            on_dismiss: Msg::Dismiss,
            on_retry: Some(Msg::Retry),
            palette: &pal,
        }))
    }
}

/// Lanza la carga async: incrementa la generación, marca Loading y dispara
/// un worker que duerme ~900 ms. El primer intento falla a propósito (para
/// mostrar el estado de error + Reintentar); del segundo en adelante trae
/// los datos.
fn start_load(mut model: Model, handle: &Handle<Msg>) -> Model {
    model.async_state = AsyncState::Loading;
    model.async_gen = model.async_gen.wrapping_add(1);
    model.async_attempts += 1;
    let gen = model.async_gen;
    let attempt = model.async_attempts;
    handle.spawn(move || {
        std::thread::sleep(Duration::from_millis(900));
        if attempt == 1 {
            Msg::AsyncLoaded(gen, Err("No se pudo contactar el índice".to_string()))
        } else {
            Msg::AsyncLoaded(
                gen,
                Ok(vec![
                    SelectItem::new("tawasuyu")
                        .icon("\u{2756}")
                        .with_sublabel("rust · 210 crates")
                        .badge(SelectBadge::count(42, BadgeKind::Info)),
                    SelectItem::new("llimphi")
                        .icon("\u{2756}")
                        .with_sublabel("motor gráfico")
                        .badge(SelectBadge::label("ui", BadgeKind::Success)),
                    SelectItem::new("wawa")
                        .icon("\u{2756}")
                        .with_sublabel("SO bare-metal")
                        .badge(SelectBadge::dot(BadgeKind::Warning)),
                    SelectItem::new("sigma")
                        .icon("\u{2756}")
                        .with_sublabel("gestión escolar")
                        .badge(SelectBadge::count(7, BadgeKind::Neutral)),
                ]),
            )
        }
    });
    model
}

/// Envuelve un trigger en un bloque absoluto con rótulo arriba, en la
/// posición canónica del select `i`.
fn labeled_trigger(
    label: &str,
    _which: usize,
    trigger: View<Msg>,
    theme: &Theme,
    i: usize,
) -> View<Msg> {
    let y = Y0 + i as f32 * ROW;
    View::new(Style {
        position: Position::Absolute,
        inset: Rect { left: length(X), top: length(y - 22.0), right: auto(), bottom: auto() },
        size: Size { width: length(W), height: length(ROW) },
        flex_direction: FlexDirection::Column,
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
            ..Default::default()
        })
        .text_aligned(label.to_string(), 11.5, theme.fg_muted, Alignment::Start),
        trigger,
    ])
}

fn main() {
    llimphi_ui::run::<Demo>();
}
