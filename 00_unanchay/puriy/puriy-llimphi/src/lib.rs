//! `puriy-llimphi` — chrome + viewport del navegador sobre Llimphi.
//!
//! Punto de entrada: [`run`]. Toma una URL, lanza el engine en init,
//! abre ventana Llimphi y dibuja el [`BoxTree`](puriy_engine::BoxTree)
//! como árbol de Views (column de blocks, row de inlines).
//!
//! Características:
//! - Address bar editable (`text_input_view`) con Enter para navegar.
//! - F5 recarga.
//! - Rueda del mouse scrollea el viewport (clip + inset negativo).
//! - Links `<a href>` clickeables — disparan `Msg::Navigate`.
//! - Bold simulado: `font_weight >= 600` agranda 10 % el `font_size`
//!   (Llimphi text aún no expone el eje weight de la fuente).

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_layout::taffy::prelude::{
    auto, length, percent, AlignItems, FlexDirection, Position, Rect, Size, Style,
};
use llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use puriy_engine::{BoxNode, BoxTree, Display, Engine};

const HEADER_H: f32 = 56.0;
const LINE_PX: f32 = 24.0;

/// Punto de entrada — abre ventana Llimphi cargando `url`.
pub fn run(url: String) {
    PURIY_URL.with(|cell| *cell.borrow_mut() = Some(url));
    llimphi_ui::run::<Puriy>();
}

thread_local! {
    static PURIY_URL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub struct Puriy;

pub struct Model {
    pub url: String,
    pub title: String,
    pub status: String,
    pub scroll_y: f32,
    pub addr: TextInputState,
    pub addr_focused: bool,
    /// Sólo el [`BoxTree`] viaja al UI thread — el `DomTree` (Rc-based)
    /// es `!Send`, así que se queda en el worker y muere ahí.
    pub box_tree: Option<BoxTree>,
}

#[derive(Clone)]
pub enum Msg {
    Reload,
    Loaded { title: String, box_tree: BoxTree },
    LoadFailed(String),
    Navigate(String),
    Scroll(f32),
    FocusAddr,
    AddrKey(KeyEvent),
}

impl App for Puriy {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "puriy · navegador soberano"
    }

    fn app_id() -> Option<&'static str> {
        Some("net.gioser.puriy")
    }

    fn initial_size() -> (u32, u32) {
        (1100, 760)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let url = PURIY_URL
            .with(|c| c.borrow().clone())
            .unwrap_or_else(|| "about:blank".to_string());
        spawn_load(url.clone(), handle.clone());
        let mut addr = TextInputState::new();
        addr.set_text(url.clone());
        Model {
            url,
            title: String::new(),
            status: "cargando…".into(),
            scroll_y: 0.0,
            addr,
            addr_focused: false,
            box_tree: None,
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Si la address bar tiene foco, redirige las teclas al input
        // (excepto F5 que siempre recarga).
        if model.addr_focused && !matches!(&e.key, Key::Named(NamedKey::F5)) {
            return Some(Msg::AddrKey(e.clone()));
        }
        match &e.key {
            Key::Named(NamedKey::F5) => Some(Msg::Reload),
            Key::Named(NamedKey::PageDown) => Some(Msg::Scroll(LINE_PX * 12.0)),
            Key::Named(NamedKey::PageUp) => Some(Msg::Scroll(-LINE_PX * 12.0)),
            Key::Named(NamedKey::ArrowDown) => Some(Msg::Scroll(LINE_PX)),
            Key::Named(NamedKey::ArrowUp) => Some(Msg::Scroll(-LINE_PX)),
            Key::Named(NamedKey::Home) => Some(Msg::Scroll(-1.0e9)),
            Key::Named(NamedKey::End) => Some(Msg::Scroll(1.0e9)),
            _ => None,
        }
    }

    fn on_wheel(
        _model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Self::Msg> {
        // WheelDelta.y > 0 = contenido sube (CSS convention).
        Some(Msg::Scroll(delta.y * LINE_PX * 3.0))
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Reload => {
                m.status = "recargando…".into();
                m.scroll_y = 0.0;
                m.box_tree = None;
                spawn_load(m.url.clone(), handle.clone());
            }
            Msg::Loaded { title, box_tree } => {
                m.title = title;
                let boxes = box_tree.descendants_count();
                m.status = format!("OK · {boxes} boxes");
                m.box_tree = Some(box_tree);
            }
            Msg::LoadFailed(err) => {
                m.status = format!("error: {err}");
                m.box_tree = None;
            }
            Msg::Navigate(target) => {
                m.url = target.clone();
                m.addr.set_text(target.clone());
                m.addr_focused = false;
                m.status = format!("cargando {target}…");
                m.scroll_y = 0.0;
                m.box_tree = None;
                spawn_load(target, handle.clone());
            }
            Msg::Scroll(dy) => {
                m.scroll_y = (m.scroll_y + dy).max(0.0);
            }
            Msg::FocusAddr => {
                m.addr_focused = true;
            }
            Msg::AddrKey(e) => {
                if matches!(&e.key, Key::Named(NamedKey::Enter)) {
                    let target = m.addr.text().trim().to_string();
                    if !target.is_empty() {
                        return Self::update(m, Msg::Navigate(target), handle);
                    }
                } else if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                    m.addr_focused = false;
                    m.addr.set_text(m.url.clone());
                } else {
                    m.addr.apply_key(&e);
                }
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let header = header_bar(model);
        let body = viewport(model);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(Color::from_rgb8(245, 245, 248))
        .children(vec![header, body])
    }
}

fn spawn_load(url: String, handle: Handle<Msg>) {
    std::thread::spawn(move || {
        let engine = Engine::new();
        match engine.load(&url) {
            Ok(doc) => {
                let title = if doc.title.is_empty() {
                    doc.url.clone()
                } else {
                    doc.title.clone()
                };
                handle.dispatch(Msg::Loaded { title, box_tree: doc.box_tree });
            }
            Err(e) => handle.dispatch(Msg::LoadFailed(e.to_string())),
        }
    });
}

fn header_bar(model: &Model) -> View<Msg> {
    let palette = TextInputPalette::default();
    let addr = text_input_view(&model.addr, "ingresar URL…", model.addr_focused, &palette, Msg::FocusAddr);

    let status_line = format!(
        "{}    ·    status: {}    ·    [F5 recargar · Enter navegar]",
        if model.title.is_empty() { model.url.as_str() } else { model.title.as_str() },
        model.status,
    );

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(HEADER_H) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::from_rgb8(28, 28, 36))
    .children(vec![
        addr,
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(14.0_f32) },
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(2.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(status_line, 10.0, Color::from_rgb8(150, 150, 165), Alignment::Start),
    ])
}

fn viewport(model: &Model) -> View<Msg> {
    let Some(tree) = model.box_tree.as_ref() else {
        return View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            padding: Rect {
                left: length(24.0_f32),
                right: length(24.0_f32),
                top: length(24.0_f32),
                bottom: length(24.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::WHITE)
        .text_aligned("(sin documento)", 14.0, Color::from_rgb8(120, 120, 120), Alignment::Start);
    };

    // Contenido absoluto desplazado verticalmente; el outer recorta.
    let content = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(16.0_f32 - model.scroll_y),
            bottom: auto(),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![render_box(&tree.root)]);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(Color::WHITE)
    .clip(true)
    .children(vec![content])
}

fn render_box(b: &BoxNode) -> View<Msg> {
    let style = box_style(b);
    let mut view = View::new(style);

    if let Some(bg) = b.background {
        view = view.fill(Color::from_rgb8(bg.r, bg.g, bg.b));
    }

    // Color azul + click handler para links (sólo si tienen href).
    let link_color = Color::from_rgb8(30, 90, 200);
    let display_color = if b.link.is_some() {
        link_color
    } else {
        Color::from_rgb8(b.color.r, b.color.g, b.color.b)
    };

    if let Some(target) = &b.link {
        view = view.on_click(Msg::Navigate(target.clone()));
    }

    // Hojas de texto: el nodo no tiene children, pinta el texto.
    if let Some(text) = &b.text {
        let size = if b.font_weight >= 600 { b.font_size * 1.1 } else { b.font_size };
        return view.text_aligned(text.clone(), size, display_color, Alignment::Start);
    }

    if !b.children.is_empty() {
        // Si el container es un link, propagamos el Navigate también
        // a los hijos via wrap (Llimphi sólo emite on_click sobre el
        // nodo cliqueado, y los hijos lo "ocultan"); recolorear el
        // texto interno se hace abajo con render_link_subtree.
        let kids: Vec<View<Msg>> = if let Some(target) = &b.link {
            b.children.iter().map(|c| render_link_subtree(c, target, link_color)).collect()
        } else {
            b.children.iter().map(render_box).collect()
        };
        view = view.children(kids);
    }
    view
}

fn render_link_subtree(b: &BoxNode, target: &str, color: Color) -> View<Msg> {
    let mut view = View::new(box_style(b)).on_click(Msg::Navigate(target.to_string()));
    if let Some(bg) = b.background {
        view = view.fill(Color::from_rgb8(bg.r, bg.g, bg.b));
    }
    if let Some(text) = &b.text {
        let size = if b.font_weight >= 600 { b.font_size * 1.1 } else { b.font_size };
        return view.text_aligned(text.clone(), size, color, Alignment::Start);
    }
    if !b.children.is_empty() {
        view = view.children(
            b.children
                .iter()
                .map(|c| render_link_subtree(c, target, color))
                .collect(),
        );
    }
    view
}

fn box_style(b: &BoxNode) -> Style {
    let (flex_direction, width, height) = match b.display {
        Display::Block => (FlexDirection::Column, percent(1.0_f32), auto()),
        Display::InlineBlock | Display::Inline => (FlexDirection::Row, auto(), auto()),
        Display::None => (FlexDirection::Column, length(0.0_f32), length(0.0_f32)),
    };

    Style {
        flex_direction,
        size: Size { width, height },
        margin: Rect {
            left: length(b.margin),
            right: length(b.margin * 0.25),
            top: length(b.margin * 0.25),
            bottom: length(b.margin * 0.25),
        },
        padding: Rect {
            left: length(b.padding),
            right: length(b.padding),
            top: length(b.padding * 0.5),
            bottom: length(b.padding * 0.5),
        },
        ..Default::default()
    }
}

// `Arc` queda traído por si futuras versiones envuelven handlers async.
#[doc(hidden)]
pub fn _arc_anchor<T>(v: Arc<T>) -> Arc<T> {
    v
}
