//! `puriy-llimphi` — chrome + viewport del navegador sobre Llimphi.
//!
//! Punto de entrada: [`run`]. Toma una URL, lanza el engine en init,
//! abre ventana Llimphi y dibuja el [`BoxTree`](puriy_engine::BoxTree)
//! como árbol de Views (column de blocks, row de inlines).
//!
//! Anti-objetivo de Fase 3: no hay JS, no hay scroll virtual, no hay
//! historial — esto es el MVP feo que muestra que el pipeline cierra.

#![forbid(unsafe_code)]

use llimphi_layout::taffy::prelude::{
    length, percent, AlignItems, FlexDirection, Rect, Size, Style,
};
use llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, KeyEvent, NamedKey, View};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::Key;

use puriy_engine::{BoxNode, BoxTree, Display, Engine};

/// Punto de entrada — abre ventana Llimphi cargando `url`.
pub fn run(url: String) {
    PURIY_URL.with(|cell| *cell.borrow_mut() = Some(url));
    llimphi_ui::run::<Puriy>();
}

// La trait `App` no permite parámetros — el url inicial viaja por TLS.
thread_local! {
    static PURIY_URL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub struct Puriy;

pub struct Model {
    pub url: String,
    pub title: String,
    pub status: String,
    /// Sólo el [`BoxTree`] viaja al UI thread — el `DomTree` (Rc-based)
    /// es `!Send`, así que se queda en el worker y muere ahí.
    pub box_tree: Option<BoxTree>,
}

#[derive(Clone)]
pub enum Msg {
    Reload,
    Loaded { title: String, box_tree: BoxTree },
    LoadFailed(String),
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
        Model {
            url,
            title: String::new(),
            status: "cargando…".into(),
            box_tree: None,
        }
    }

    fn on_key(_m: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != llimphi_ui::KeyState::Pressed {
            return None;
        }
        if matches!(&e.key, Key::Named(NamedKey::F5)) {
            return Some(Msg::Reload);
        }
        None
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Reload => {
                m.status = "recargando…".into();
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
                // Sólo el BoxTree cruza al UI thread; el DomTree (Rc) se
                // dropea junto con `doc` al salir del scope.
                handle.dispatch(Msg::Loaded { title, box_tree: doc.box_tree });
            }
            Err(e) => handle.dispatch(Msg::LoadFailed(e.to_string())),
        }
    });
}

fn header_bar(model: &Model) -> View<Msg> {
    let title_line = if model.title.is_empty() {
        model.url.clone()
    } else {
        format!("{}  ·  {}", model.title, model.url)
    };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(48.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::from_rgb8(28, 28, 36))
    .children(vec![
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
            ..Default::default()
        })
        .text_aligned(title_line, 13.0, Color::from_rgb8(220, 220, 230), Alignment::Start),
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(14.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            format!("status: {}    [F5 recargar]", model.status),
            11.0,
            Color::from_rgb8(150, 150, 165),
            Alignment::Start,
        ),
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

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(16.0_f32),
            bottom: length(16.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::WHITE)
    .children(vec![render_box(&tree.root)])
}

fn render_box(b: &BoxNode) -> View<Msg> {
    let style = box_style(b);
    let mut view = View::new(style);

    if let Some(bg) = b.background {
        view = view.fill(Color::from_rgb8(bg.r, bg.g, bg.b));
    }

    // Hojas de texto: el nodo no tiene children, pinta el texto.
    if let Some(text) = &b.text {
        let color = Color::from_rgb8(b.color.r, b.color.g, b.color.b);
        return view.text_aligned(text.clone(), b.font_size, color, Alignment::Start);
    }

    if !b.children.is_empty() {
        view = view.children(b.children.iter().map(render_box).collect());
    }
    view
}

fn box_style(b: &BoxNode) -> Style {
    let (flex_direction, width, height) = match b.display {
        Display::Block => (
            FlexDirection::Column,
            percent(1.0_f32),
            llimphi_layout::taffy::prelude::auto(),
        ),
        Display::InlineBlock | Display::Inline => (
            FlexDirection::Row,
            llimphi_layout::taffy::prelude::auto(),
            // ~ 1 línea por defecto cuando hay texto; layout taffy mide.
            llimphi_layout::taffy::prelude::auto(),
        ),
        Display::None => (FlexDirection::Column, length(0.0_f32), length(0.0_f32)),
    };

    Style {
        flex_direction,
        size: Size { width, height },
        margin: Rect {
            left: length(b.margin),
            right: length(b.margin),
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
