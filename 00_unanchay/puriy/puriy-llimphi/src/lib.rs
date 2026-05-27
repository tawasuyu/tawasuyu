//! `puriy-llimphi` — chrome + viewport del navegador sobre Llimphi.
//!
//! Punto de entrada: [`run`]. Toma una URL inicial, abre ventana Llimphi
//! con una pestaña, y delega al motor `puriy-engine` para parsear y
//! computar el [`BoxTree`](puriy_engine::BoxTree). El chrome cablea:
//!
//! - Address bar editable por pestaña (Enter navega, Esc cancela).
//! - Scroll vertical (wheel + PgUp/Dn + ArrowUp/Dn + Home/End).
//! - Links `<a href>` clickeables — disparan `Msg::Navigate`.
//! - Historial por pestaña: Alt+←/Alt+→ (back/forward).
//! - Pestañas múltiples: Ctrl+T (nueva), Ctrl+W (cerrar),
//!   Ctrl+Tab / Ctrl+Shift+Tab (rotar), click en pestaña la activa.
//!
//! Bold se simula con `font_size × 1.1` mientras `llimphi-text` no exponga
//! el eje weight.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use llimphi_layout::taffy::prelude::{
    auto, length, percent, AlignItems, FlexDirection, FlexWrap, Position, Rect, Size, Style,
};
use llimphi_raster::kurbo::{Affine, RoundedRect, Stroke};
use llimphi_raster::peniko::{Blob, Color, Image as PenikoImage, ImageFormat};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use puriy_engine::{BoxNode, BoxTree, Display, Engine, LengthVal, TextAlign};

const HEADER_H: f32 = 78.0;
const TABS_H: f32 = 30.0;
const LINE_PX: f32 = 24.0;
const NEW_TAB_URL: &str = "about:blank";

/// Punto de entrada — abre ventana Llimphi con una pestaña en `url`.
pub fn run(url: String) {
    PURIY_URL.with(|cell| *cell.borrow_mut() = Some(url));
    llimphi_ui::run::<Puriy>();
}

thread_local! {
    static PURIY_URL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// ID de pestaña incremental — los mensajes async (Loaded/Failed)
/// llevan el id de origen para que si la pestaña ya fue cerrada o
/// pisada por otra navegación, el resultado se descarte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

static NEXT_TAB: AtomicU64 = AtomicU64::new(1);

fn fresh_tab_id() -> TabId {
    TabId(NEXT_TAB.fetch_add(1, Ordering::Relaxed))
}

pub struct Puriy;

pub struct TabState {
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub status: String,
    pub scroll_y: f32,
    pub addr: TextInputState,
    pub addr_focused: bool,
    /// Stack de URLs visitadas. `history[cursor]` es la actual.
    pub history: Vec<String>,
    pub cursor: usize,
    pub box_tree: Option<BoxTree>,
    /// Generación monótona — Loaded de generaciones viejas se descarta.
    pub gen: u64,
}

impl TabState {
    fn new(url: String) -> Self {
        let mut addr = TextInputState::new();
        addr.set_text(url.clone());
        Self {
            id: fresh_tab_id(),
            url: url.clone(),
            title: String::new(),
            status: "cargando…".into(),
            scroll_y: 0.0,
            addr,
            addr_focused: false,
            history: vec![url],
            cursor: 0,
            box_tree: None,
            gen: 0,
        }
    }

    fn can_back(&self) -> bool {
        self.cursor > 0
    }
    fn can_fwd(&self) -> bool {
        self.cursor + 1 < self.history.len()
    }
}

pub struct Model {
    pub tabs: Vec<TabState>,
    pub active: usize,
}

impl Model {
    fn active(&self) -> &TabState {
        &self.tabs[self.active]
    }
    fn active_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active]
    }
    fn tab_idx(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }
}

#[derive(Clone)]
pub enum Msg {
    Reload,
    Loaded { tab: TabId, gen: u64, title: String, box_tree: BoxTree },
    LoadFailed { tab: TabId, gen: u64, err: String },
    Navigate(String),
    Scroll(f32),
    FocusAddr,
    AddrKey(KeyEvent),
    Back,
    Forward,
    NewTab,
    CloseTab(usize),
    SelectTab(usize),
    NextTab,
    PrevTab,
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
            .unwrap_or_else(|| NEW_TAB_URL.to_string());
        let mut tab = TabState::new(url.clone());
        tab.gen = 1;
        spawn_load(tab.id, tab.gen, url, handle.clone());
        Model { tabs: vec![tab], active: 0 }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        let mods = e.modifiers;
        // Atajos con Ctrl — toman precedencia incluso sobre el address bar.
        if mods.ctrl {
            match &e.key {
                Key::Character(s) if s.eq_ignore_ascii_case("t") => return Some(Msg::NewTab),
                Key::Character(s) if s.eq_ignore_ascii_case("w") => {
                    return Some(Msg::CloseTab(model.active));
                }
                Key::Named(NamedKey::Tab) if mods.shift => return Some(Msg::PrevTab),
                Key::Named(NamedKey::Tab) => return Some(Msg::NextTab),
                _ => {}
            }
        }
        if mods.alt {
            match &e.key {
                Key::Named(NamedKey::ArrowLeft) => return Some(Msg::Back),
                Key::Named(NamedKey::ArrowRight) => return Some(Msg::Forward),
                _ => {}
            }
        }
        // Si la address bar tiene foco, redirige las teclas al input.
        if model.active().addr_focused && !matches!(&e.key, Key::Named(NamedKey::F5)) {
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
        Some(Msg::Scroll(delta.y * LINE_PX * 3.0))
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Reload => {
                let url = m.active().url.clone();
                start_load(&mut m, url, /* push_history */ false, handle);
            }
            Msg::Loaded { tab, gen, title, box_tree } => {
                if let Some(idx) = m.tab_idx(tab) {
                    let t = &mut m.tabs[idx];
                    if t.gen == gen {
                        t.title = title;
                        let n = box_tree.descendants_count();
                        t.status = format!("OK · {n} boxes");
                        t.box_tree = Some(box_tree);
                    }
                }
            }
            Msg::LoadFailed { tab, gen, err } => {
                if let Some(idx) = m.tab_idx(tab) {
                    let t = &mut m.tabs[idx];
                    if t.gen == gen {
                        t.status = format!("error: {err}");
                        t.box_tree = None;
                    }
                }
            }
            Msg::Navigate(target) => {
                start_load(&mut m, target, /* push_history */ true, handle);
            }
            Msg::Scroll(dy) => {
                let t = m.active_mut();
                t.scroll_y = (t.scroll_y + dy).max(0.0);
            }
            Msg::FocusAddr => {
                m.active_mut().addr_focused = true;
            }
            Msg::AddrKey(e) => {
                if matches!(&e.key, Key::Named(NamedKey::Enter)) {
                    let target = m.active().addr.text().trim().to_string();
                    if !target.is_empty() {
                        return Self::update(m, Msg::Navigate(target), handle);
                    }
                } else if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                    let t = m.active_mut();
                    t.addr_focused = false;
                    t.addr.set_text(t.url.clone());
                } else {
                    m.active_mut().addr.apply_key(&e);
                }
            }
            Msg::Back => {
                let t = m.active_mut();
                if t.can_back() {
                    t.cursor -= 1;
                    let url = t.history[t.cursor].clone();
                    start_load(&mut m, url, /* push_history */ false, handle);
                }
            }
            Msg::Forward => {
                let t = m.active_mut();
                if t.can_fwd() {
                    t.cursor += 1;
                    let url = t.history[t.cursor].clone();
                    start_load(&mut m, url, /* push_history */ false, handle);
                }
            }
            Msg::NewTab => {
                let mut t = TabState::new(NEW_TAB_URL.into());
                t.status = "nueva pestaña".into();
                t.box_tree = None;
                m.tabs.push(t);
                m.active = m.tabs.len() - 1;
                m.active_mut().addr_focused = true;
            }
            Msg::CloseTab(idx) => {
                if idx < m.tabs.len() {
                    m.tabs.remove(idx);
                }
                if m.tabs.is_empty() {
                    let t = TabState::new(NEW_TAB_URL.into());
                    m.tabs.push(t);
                    m.active = 0;
                } else if m.active >= m.tabs.len() {
                    m.active = m.tabs.len() - 1;
                }
            }
            Msg::SelectTab(idx) => {
                if idx < m.tabs.len() {
                    m.active = idx;
                }
            }
            Msg::NextTab => {
                if !m.tabs.is_empty() {
                    m.active = (m.active + 1) % m.tabs.len();
                }
            }
            Msg::PrevTab => {
                if !m.tabs.is_empty() {
                    m.active = (m.active + m.tabs.len() - 1) % m.tabs.len();
                }
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let tabs_bar = tabs_bar(model);
        let header = header_bar(model.active());
        let body = viewport(model.active());

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(Color::from_rgb8(245, 245, 248))
        .children(vec![tabs_bar, header, body])
    }
}

/// Inicia la carga de `url` en la pestaña activa. Si `push_history` es
/// `true`, se trunca y empuja al stack — útil para Navigate; back/fwd/
/// reload pasan `false`.
fn start_load(m: &mut Model, url: String, push_history: bool, handle: &Handle<Msg>) {
    let t = m.active_mut();
    t.url = url.clone();
    t.addr.set_text(url.clone());
    t.addr_focused = false;
    t.status = format!("cargando {url}…");
    t.scroll_y = 0.0;
    t.box_tree = None;
    if push_history {
        // Trunca lo que esté adelante del cursor — convención estándar.
        t.history.truncate(t.cursor + 1);
        if t.history.last() != Some(&url) {
            t.history.push(url.clone());
            t.cursor = t.history.len() - 1;
        }
    }
    t.gen = t.gen.wrapping_add(1);
    let (id, gen) = (t.id, t.gen);
    spawn_load(id, gen, url, handle.clone());
}

fn spawn_load(tab: TabId, gen: u64, url: String, handle: Handle<Msg>) {
    if url == NEW_TAB_URL {
        // No fetch para about:blank.
        return;
    }
    std::thread::spawn(move || {
        let engine = Engine::new();
        match engine.load(&url) {
            Ok(doc) => {
                let title = if doc.title.is_empty() { doc.url.clone() } else { doc.title.clone() };
                handle.dispatch(Msg::Loaded { tab, gen, title, box_tree: doc.box_tree });
            }
            Err(e) => handle.dispatch(Msg::LoadFailed { tab, gen, err: e.to_string() }),
        }
    });
}

fn tabs_bar(model: &Model) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::with_capacity(model.tabs.len() + 1);
    for (i, t) in model.tabs.iter().enumerate() {
        let active = i == model.active;
        let bg = if active { Color::from_rgb8(245, 245, 248) } else { Color::from_rgb8(40, 40, 50) };
        let fg = if active { Color::from_rgb8(20, 20, 24) } else { Color::from_rgb8(200, 200, 210) };
        let label = if t.title.is_empty() { t.url.as_str() } else { t.title.as_str() };
        let close = View::new(Style {
            size: Size { width: length(18.0_f32), height: length(18.0_f32) },
            margin: Rect {
                left: length(6.0_f32),
                right: length(2.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned("✕", 11.0, fg, Alignment::Center)
        .on_click(Msg::CloseTab(i));

        let tab_view = View::new(Style {
            size: Size { width: length(180.0_f32), height: percent(1.0_f32) },
            padding: Rect {
                left: length(10.0_f32),
                right: length(6.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            margin: Rect {
                left: length(0.0_f32),
                right: length(2.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(bg)
        .radius(3.0)
        .on_click(Msg::SelectTab(i))
        .children(vec![
            View::new(Style {
                size: Size { width: length(140.0_f32), height: length(18.0_f32) },
                ..Default::default()
            })
            .text_aligned(truncate(label, 22), 11.0, fg, Alignment::Start),
            close,
        ]);
        kids.push(tab_view);
    }
    kids.push(
        View::new(Style {
            size: Size { width: length(28.0_f32), height: percent(1.0_f32) },
            margin: Rect {
                left: length(4.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned("+", 16.0, Color::from_rgb8(200, 200, 210), Alignment::Center)
        .on_click(Msg::NewTab),
    );

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(TABS_H) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(0.0_f32),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(18, 18, 22))
    .children(kids)
}

fn header_bar(t: &TabState) -> View<Msg> {
    let palette = TextInputPalette::default();
    let addr = text_input_view(&t.addr, "ingresar URL…", t.addr_focused, &palette, Msg::FocusAddr);

    // Botones nav: ← → ⟳
    let back_color = if t.can_back() { Color::from_rgb8(220, 220, 230) } else { Color::from_rgb8(90, 90, 100) };
    let fwd_color = if t.can_fwd() { Color::from_rgb8(220, 220, 230) } else { Color::from_rgb8(90, 90, 100) };
    let nav_btn = |label: &str, color: Color, msg: Msg| {
        View::new(Style {
            size: Size { width: length(28.0_f32), height: length(28.0_f32) },
            margin: Rect {
                left: length(0.0_f32),
                right: length(4.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(Color::from_rgb8(40, 40, 50))
        .radius(3.0)
        .text_aligned(label.to_string(), 14.0, color, Alignment::Center)
        .on_click(msg)
    };

    let addr_row = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        nav_btn("◀", back_color, Msg::Back),
        nav_btn("▶", fwd_color, Msg::Forward),
        nav_btn("⟳", Color::from_rgb8(220, 220, 230), Msg::Reload),
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
            ..Default::default()
        })
        .children(vec![addr]),
    ]);

    let title_line = if t.title.is_empty() { t.url.as_str() } else { t.title.as_str() };
    let status_line = format!(
        "{}    ·    status: {}    ·    [Ctrl+T nueva · Ctrl+W cerrar · Ctrl+Tab rotar · Alt+←/→ back/fwd · F5 recargar]",
        title_line, t.status,
    );

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(HEADER_H - TABS_H) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::from_rgb8(28, 28, 36))
    .children(vec![
        addr_row,
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

fn viewport(t: &TabState) -> View<Msg> {
    let Some(tree) = t.box_tree.as_ref() else {
        let msg = if t.url == NEW_TAB_URL {
            "(pestaña vacía · escribí una URL arriba)"
        } else {
            "(cargando…)"
        };
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
        .text_aligned(msg.to_string(), 14.0, Color::from_rgb8(120, 120, 120), Alignment::Start);
    };

    let content = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(16.0_f32 - t.scroll_y),
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
    if let Some(hbg) = b.hover_background {
        view = view.hover_fill(Color::from_rgb8(hbg.r, hbg.g, hbg.b));
    }
    view = apply_border(view, b);

    let link_color = Color::from_rgb8(30, 90, 200);
    let display_color = if b.link.is_some() {
        link_color
    } else {
        Color::from_rgb8(b.color.r, b.color.g, b.color.b)
    };

    if let Some(target) = &b.link {
        view = view.on_click(Msg::Navigate(target.clone()));
    }

    // <img> con imagen decodificada: arma peniko::Image, ajusta el rect
    // del nodo al tamaño nativo (taffy luego lo clampa por el ancho del
    // contenedor). Llimphi escala preservando aspect ratio.
    if let Some(img) = &b.image {
        let blob = Blob::from(img.rgba.clone());
        let peniko = PenikoImage::new(blob, ImageFormat::Rgba8, img.width, img.height);
        return image_view(img.width, img.height).image(peniko);
    }

    if let Some(text) = &b.text {
        let size = if b.font_weight >= 600 { b.font_size * 1.1 } else { b.font_size };
        return view.text_aligned(text.clone(), size, display_color, Alignment::Start);
    }

    if !b.children.is_empty() {
        let kids: Vec<View<Msg>> = if let Some(target) = &b.link {
            b.children.iter().map(|c| render_link_subtree(c, target, link_color)).collect()
        } else {
            b.children.iter().map(render_box).collect()
        };
        view = view.children(kids);
    }
    view
}

/// View dimensionada para una imagen — ancho hasta `width_px` pero
/// nunca más que el contenedor (`max_width: 100%`), altura proporcional
/// vía aspect ratio inverso (`width / height`).
fn image_view(width: u32, height: u32) -> View<Msg> {
    let w = (width.max(1)) as f32;
    let h = (height.max(1)) as f32;
    View::new(Style {
        size: Size { width: length(w), height: length(h) },
        max_size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
}

fn render_link_subtree(b: &BoxNode, target: &str, color: Color) -> View<Msg> {
    let mut view = View::new(box_style(b)).on_click(Msg::Navigate(target.to_string()));
    if let Some(bg) = b.background {
        view = view.fill(Color::from_rgb8(bg.r, bg.g, bg.b));
    }
    if let Some(img) = &b.image {
        let blob = Blob::from(img.rgba.clone());
        let peniko = PenikoImage::new(blob, ImageFormat::Rgba8, img.width, img.height);
        return image_view(img.width, img.height)
            .image(peniko)
            .on_click(Msg::Navigate(target.to_string()));
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

/// Aplica `border-radius` (vía `View::radius`) y dibuja el contorno del
/// box con `paint_with(...)` si `border_width > 0 && border_color.is_some()`.
/// Vello stroke con `RoundedRect` da bordes redondeados consistentes con
/// el fill — ambos comparten el mismo radio.
fn apply_border(mut view: View<Msg>, b: &BoxNode) -> View<Msg> {
    if b.border_radius > 0.0 {
        view = view.radius(b.border_radius as f64);
    }
    let (Some(bc), w) = (b.border_color, b.border_width) else {
        return view;
    };
    if w <= 0.0 {
        return view;
    }
    let radius = b.border_radius as f64;
    let color = Color::from_rgba8(bc.r, bc.g, bc.b, 255);
    let stroke = Stroke::new(w as f64);
    view.paint_with(move |scene, _typesetter, rect| {
        // Inset por media línea para que el stroke caiga dentro del
        // rect — vello pinta el trazo centrado en el path.
        let half = stroke.width * 0.5;
        let r = RoundedRect::new(
            rect.x as f64 + half,
            rect.y as f64 + half,
            (rect.x + rect.w) as f64 - half,
            (rect.y + rect.h) as f64 - half,
            (radius - half).max(0.0),
        );
        scene.stroke(&stroke, Affine::IDENTITY, color, None, &r);
    })
}

fn box_style(b: &BoxNode) -> Style {
    // Si el nodo es una hoja de texto, le damos un height ≈ line-height
    // para que el row del padre tenga altura real — sin esto, taffy
    // colapsa los inlines al top del bloque. Para inlines con hijos
    // dejamos auto y que el padre mida.
    let is_text_leaf = b.text.is_some();
    let lh_mult = b.line_height.unwrap_or(1.4);
    let line_h = b.font_size * lh_mult;

    let (flex_direction, mut width, height) = match b.display {
        Display::Block => (FlexDirection::Column, percent(1.0_f32), auto()),
        Display::InlineBlock | Display::Inline => {
            let h = if is_text_leaf { length(line_h) } else { auto() };
            (FlexDirection::Row, auto(), h)
        }
        Display::None => (FlexDirection::Column, length(0.0_f32), length(0.0_f32)),
    };

    // Bloques con hijos inline: habilitamos flex_wrap para que los
    // tokens fluyan en múltiples líneas si exceden el ancho del bloque.
    // Bloques con hijos block siguen en column sin wrap.
    let flex_wrap = if matches!(b.display, Display::Block) && has_inline_children(b) {
        FlexWrap::Wrap
    } else {
        FlexWrap::NoWrap
    };

    // Para bloques con hijos inline, cambiamos a Row + wrap. Esto convierte
    // el `<p>foo <a>bar</a> baz</p>` en una fila con wrap en lugar de
    // apilar cada token en su propia línea.
    let (flex_direction, w_base) =
        if matches!(b.display, Display::Block) && has_inline_children(b) {
            (FlexDirection::Row, percent(1.0_f32))
        } else {
            (flex_direction, width)
        };
    width = w_base;

    // CSS `width` explícito gana sobre el default de display.
    if let Some(explicit) = length_to_taffy(b.width) {
        width = explicit;
    }
    // `max-width` se aplica vía max_size del Style.
    let max_size = Size {
        width: length_to_taffy(b.max_width).unwrap_or_else(auto),
        height: auto(),
    };

    // `text-align: center|right` sobre bloques con inlines mapea a
    // `justify_content` del row interno (axis main = row → horizontal).
    let justify_content =
        if matches!(b.display, Display::Block) && has_inline_children(b) {
            match b.text_align {
                TextAlign::Left | TextAlign::Justify => None,
                TextAlign::Center => Some(llimphi_layout::taffy::prelude::JustifyContent::Center),
                TextAlign::Right => Some(llimphi_layout::taffy::prelude::JustifyContent::End),
            }
        } else {
            None
        };

    Style {
        flex_direction,
        flex_wrap,
        justify_content,
        size: Size { width, height },
        max_size,
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

/// Traduce un `LengthVal` CSS al tipo de longitud que taffy entiende.
/// `Auto` queda como `None` (caller lo reemplaza con el default según
/// display o `auto()` para max-size).
fn length_to_taffy(v: LengthVal) -> Option<llimphi_layout::taffy::style::Dimension> {
    match v {
        LengthVal::Auto => None,
        LengthVal::Px(px) => Some(length(px)),
        LengthVal::Pct(pct) => Some(percent(pct / 100.0)),
    }
}

/// `true` si todos los hijos directos son inline o inline-block. Si los
/// hijos son block, el bloque sigue siendo column.
fn has_inline_children(b: &BoxNode) -> bool {
    !b.children.is_empty()
        && b.children
            .iter()
            .all(|c| matches!(c.display, Display::Inline | Display::InlineBlock))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
