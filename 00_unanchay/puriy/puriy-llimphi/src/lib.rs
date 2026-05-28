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
    auto, fr, length, percent, AlignItems, AlignSelf, BoxSizing, Dimension, FlexDirection,
    FlexWrap, JustifyContent, LengthPercentageAuto, Position as TaffyPosition, Rect, Size, Style,
};
use llimphi_layout::taffy::{Display as TaffyDisplay, GridTemplateComponent, TrackSizingFunction};
use llimphi_raster::kurbo::{Affine, Line, Point, Rect as KurboRect, RoundedRect, Stroke};
use llimphi_raster::peniko::{
    Blob, Color, ColorStop, ColorStops, Fill, Gradient, GradientKind, Image as PenikoImage,
    ImageFormat,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use puriy_engine::{
    AlignItems as CssAlignItems, AlignSelf as CssAlignSelf, BoxNode, BoxShadow,
    BoxSizing as CssBoxSizing, BoxTree, Display, Engine, FlexDirection as CssFlexDirection,
    FlexWrap as CssFlexWrap, GridTrackSize, JustifyContent as CssJustifyContent, LengthVal,
    LinearGradient, Overflow, PointerEvents, Position as CssPosition, TextAlign,
    TextDecorationLine, VerticalAlign, Visibility,
};

const HEADER_H: f32 = 78.0;
const TABS_H: f32 = 30.0;
const LINE_PX: f32 = 24.0;
const NEW_TAB_URL: &str = "about:blank";

/// Punto de entrada — abre ventana Llimphi con una pestaña en `url` sin
/// profile (caches/historial efímeros). Prefiere `run_with_profile` si
/// el caller ya levantó un Profile.
pub fn run(url: String) {
    PURIY_URL.with(|cell| *cell.borrow_mut() = Some(url));
    llimphi_ui::run::<Puriy>();
}

/// Punto de entrada con Profile cableado. El chrome graba en
/// `profile.history` cada navegación exitosa, deja Ctrl+D para
/// bookmarkear, y persiste a `profile_path` después de cada cambio
/// (best-effort, errores silenciosos).
pub fn run_with_profile(
    url: String,
    profile: std::sync::Arc<std::sync::Mutex<puriy_core::Profile>>,
    profile_path: std::path::PathBuf,
) {
    PURIY_URL.with(|cell| *cell.borrow_mut() = Some(url));
    PURIY_PROFILE.with(|cell| *cell.borrow_mut() = Some(profile));
    PURIY_PROFILE_PATH.with(|cell| *cell.borrow_mut() = Some(profile_path));
    llimphi_ui::run::<Puriy>();
}

thread_local! {
    static PURIY_URL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static PURIY_PROFILE: std::cell::RefCell<Option<std::sync::Arc<std::sync::Mutex<puriy_core::Profile>>>> = const { std::cell::RefCell::new(None) };
    static PURIY_PROFILE_PATH: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Devuelve la handle al Profile compartido si el chrome se arrancó vía
/// `run_with_profile`. `None` en el path `run(url)` (efímero).
fn profile_handle() -> Option<std::sync::Arc<std::sync::Mutex<puriy_core::Profile>>> {
    PURIY_PROFILE.with(|c| c.borrow().clone())
}

fn profile_path() -> Option<std::path::PathBuf> {
    PURIY_PROFILE_PATH.with(|c| c.borrow().clone())
}

/// Persiste el Profile a disco si está cableado. Silencioso ante I/O
/// errors — el usuario no necesita ver mensajes del flush.
fn persist_profile() {
    let (Some(handle), Some(path)) = (profile_handle(), profile_path()) else {
        return;
    };
    let Ok(p) = handle.lock() else { return };
    let _ = puriy_core::store::save(&path, &p);
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
    /// Factor de zoom de la página (1.0 = 100%). `Ctrl+=` lo sube,
    /// `Ctrl+-` lo baja, `Ctrl+0` lo resetea. Clampado a 0.5..3.0.
    pub zoom: f32,
    /// `Ctrl+F` levanta la find bar arriba del viewport; Esc la cierra.
    pub find_active: bool,
    /// Texto a buscar (se redacta vía `TextInputState`). Comparación
    /// case-insensitive contra cada hoja de texto del box tree del
    /// documento activo. Vacío = sin highlight.
    pub find_input: TextInputState,
    /// `Ctrl+B`/`Ctrl+H` abren un panel que reemplaza el viewport con la
    /// lista de bookmarks o el historial. `None` = panel cerrado y el
    /// documento se renderea normal. Sólo aplica cuando el chrome corre
    /// con un Profile cableado (sino las listas están vacías).
    pub panel: Option<PanelKind>,
}

/// Tipo de panel auxiliar que reemplaza el viewport cuando está abierto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    Bookmarks,
    History,
}

const ZOOM_MIN: f32 = 0.5;
const ZOOM_MAX: f32 = 3.0;
const ZOOM_STEP: f32 = 1.1;

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
    /// Ctrl+D — agrega la URL de la pestaña activa al BookmarkStore del
    /// Profile. Si el chrome corre sin profile, no-op.
    Bookmark,
    /// Ctrl+= / Ctrl++ — sube el zoom por `ZOOM_STEP` clamp a `ZOOM_MAX`.
    ZoomIn,
    /// Ctrl+- — baja el zoom por `ZOOM_STEP` clamp a `ZOOM_MIN`.
    ZoomOut,
    /// Ctrl+0 — reset a 1.0.
    ZoomReset,
    /// Ctrl+F — abre la find bar y focaliza el input.
    FindOpen,
    /// Esc (con find bar activa) — cierra la find bar y limpia la query.
    FindClose,
    /// Teclas redirigidas al input de la find bar mientras está activa.
    FindKey(KeyEvent),
    /// Ctrl+B — toggle del panel de bookmarks. Si el panel está abierto
    /// en bookmarks, lo cierra; sino lo abre con bookmarks.
    ToggleBookmarks,
    /// Ctrl+H — toggle del panel de historial.
    ToggleHistory,
    /// Esc cuando hay panel abierto (y la find bar no está activa).
    ClosePanel,
    /// Click en el botón ✕ de un bookmark — lo borra del profile y
    /// persiste.
    RemoveBookmark(puriy_core::BookmarkId),
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
        Model {
            tabs: vec![tab],
            active: 0,
            zoom: 1.0,
            find_active: false,
            find_input: TextInputState::new(),
            panel: None,
        }
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
                Key::Character(s) if s.eq_ignore_ascii_case("d") => return Some(Msg::Bookmark),
                Key::Character(s) if s.eq_ignore_ascii_case("f") => return Some(Msg::FindOpen),
                Key::Character(s) if s.eq_ignore_ascii_case("b") => {
                    return Some(Msg::ToggleBookmarks);
                }
                Key::Character(s) if s.eq_ignore_ascii_case("h") => {
                    return Some(Msg::ToggleHistory);
                }
                Key::Named(NamedKey::Tab) if mods.shift => return Some(Msg::PrevTab),
                Key::Named(NamedKey::Tab) => return Some(Msg::NextTab),
                // Zoom: Ctrl+= / Ctrl++ / Ctrl+- / Ctrl+0. El charset depende
                // del layout — aceptamos `=`/`+` para zoom in y `-`/`_` para
                // zoom out por compat con teclados sin numpad.
                Key::Character(s) if s.as_str() == "=" || s.as_str() == "+" => {
                    return Some(Msg::ZoomIn);
                }
                Key::Character(s) if s.as_str() == "-" || s.as_str() == "_" => {
                    return Some(Msg::ZoomOut);
                }
                Key::Character(s) if s.as_str() == "0" => return Some(Msg::ZoomReset),
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
        // Si la find bar está activa, intercepta Esc (cerrar) y redirige
        // el resto al input. Tiene prioridad sobre el address bar.
        if model.find_active {
            if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::FindClose);
            }
            return Some(Msg::FindKey(e.clone()));
        }
        // Esc cierra el panel (bookmarks/history) si está abierto.
        if model.panel.is_some() && matches!(&e.key, Key::Named(NamedKey::Escape)) {
            return Some(Msg::ClosePanel);
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
                        t.title = title.clone();
                        let n = box_tree.descendants_count();
                        t.status = format!("OK · {n} boxes");
                        t.box_tree = Some(box_tree);
                        // Registra en la history global del Profile (no
                        // confundir con TabState.history, que es el
                        // stack back/fwd de la pestaña).
                        let url_for_history = t.url.clone();
                        if let Some(handle) = profile_handle() {
                            if let Ok(mut p) = handle.lock() {
                                p.history.record(&url_for_history, &title, puriy_core::now());
                            }
                        }
                        persist_profile();
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
                // Cualquier navegación cierra el panel — el usuario quiere
                // ver la página, no la lista de bookmarks/history.
                m.panel = None;
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
            Msg::Bookmark => {
                let t = m.active();
                let url = t.url.clone();
                let title = if t.title.is_empty() { t.url.clone() } else { t.title.clone() };
                if let Some(handle) = profile_handle() {
                    if let Ok(mut p) = handle.lock() {
                        let already = p
                            .bookmarks
                            .items()
                            .iter()
                            .any(|b| b.url == url);
                        if !already {
                            p.bookmarks.add(&url, &title, None, puriy_core::now());
                            m.active_mut().status = format!("⭐ guardado · {} bookmarks", p.bookmarks.len());
                        } else {
                            m.active_mut().status = "⭐ ya estaba guardado".into();
                        }
                    }
                }
                persist_profile();
            }
            Msg::ZoomIn => {
                let new_zoom = (m.zoom * ZOOM_STEP).min(ZOOM_MAX);
                m.zoom = new_zoom;
                m.active_mut().status = format!("zoom: {}%", (new_zoom * 100.0).round() as i32);
            }
            Msg::ZoomOut => {
                let new_zoom = (m.zoom / ZOOM_STEP).max(ZOOM_MIN);
                m.zoom = new_zoom;
                m.active_mut().status = format!("zoom: {}%", (new_zoom * 100.0).round() as i32);
            }
            Msg::ZoomReset => {
                m.zoom = 1.0;
                m.active_mut().status = "zoom: 100%".into();
            }
            Msg::FindOpen => {
                m.find_active = true;
                // Re-abrir limpia query previa para que el usuario arranque fresh.
                m.find_input.clear();
            }
            Msg::FindClose => {
                m.find_active = false;
                m.find_input.clear();
            }
            Msg::FindKey(e) => {
                m.find_input.apply_key(&e);
            }
            Msg::ToggleBookmarks => {
                m.panel = match m.panel {
                    Some(PanelKind::Bookmarks) => None,
                    _ => Some(PanelKind::Bookmarks),
                };
            }
            Msg::ToggleHistory => {
                m.panel = match m.panel {
                    Some(PanelKind::History) => None,
                    _ => Some(PanelKind::History),
                };
            }
            Msg::ClosePanel => {
                m.panel = None;
            }
            Msg::RemoveBookmark(id) => {
                if let Some(handle) = profile_handle() {
                    if let Ok(mut p) = handle.lock() {
                        if p.bookmarks.remove(id) {
                            m.active_mut().status =
                                format!("⭐ borrado · {} bookmarks", p.bookmarks.len());
                        }
                    }
                }
                persist_profile();
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let tabs_bar = tabs_bar(model);
        let header = header_bar(model.active(), model.zoom);
        let query = model.find_input.text();
        let query_lc = query.to_lowercase();
        // Pre-cuenta los matches del documento contra la query para
        // mostrarlos en la find bar. Si find_active=false o query vacía,
        // count=0 y el viewport rendea sin highlight.
        let find_count = if model.find_active && !query_lc.is_empty() {
            count_matches(model.active().box_tree.as_ref(), &query_lc)
        } else {
            0
        };
        let body = match model.panel {
            Some(kind) => panel_view(kind),
            None => viewport(model.active(), model.zoom, &query_lc),
        };

        let mut children: Vec<View<Msg>> = vec![tabs_bar, header];
        if model.find_active {
            children.push(find_bar(&model.find_input, find_count));
        }
        children.push(body);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(Color::from_rgb8(245, 245, 248))
        .children(children)
    }
}

/// Walk del box tree contando hojas de texto cuyo contenido (lowercased)
/// contiene `query_lc`. La query ya viene en minúsculas para evitar
/// pagar el cast por hoja.
fn count_matches(tree: Option<&BoxTree>, query_lc: &str) -> usize {
    let Some(t) = tree else { return 0 };
    if query_lc.is_empty() {
        return 0;
    }
    let mut count = 0_usize;
    t.walk(|b| {
        if let Some(txt) = &b.text {
            if txt.to_lowercase().contains(query_lc) {
                count += 1;
            }
        }
    });
    count
}

/// Find bar — input + contador + close. Sticky entre header y viewport
/// mientras `find_active`.
fn find_bar(input: &TextInputState, count: usize) -> View<Msg> {
    let palette = TextInputPalette::default();
    // Siempre focado mientras está abierta — Ctrl+F fue la última acción
    // explícita del usuario, no tiene sentido que el input no acepte teclas.
    let entry = text_input_view(input, "buscar en página…", true, &palette, Msg::FindOpen);

    let count_label = if input.text().is_empty() {
        "(escribí algo)".to_string()
    } else if count == 0 {
        "sin matches".to_string()
    } else if count == 1 {
        "1 match".to_string()
    } else {
        format!("{count} matches")
    };

    let close = View::new(Style {
        size: Size { width: length(22.0_f32), height: length(22.0_f32) },
        margin: Rect {
            left: length(8.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(80, 80, 95))
    .radius(3.0)
    .text_aligned("✕", 12.0, Color::from_rgb8(220, 220, 230), Alignment::Center)
    .on_click(Msg::FindClose);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(50, 50, 62))
    .children(vec![
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            ..Default::default()
        })
        .children(vec![entry]),
        View::new(Style {
            size: Size { width: length(120.0_f32), height: length(20.0_f32) },
            margin: Rect {
                left: length(8.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(count_label, 11.0, Color::from_rgb8(200, 200, 215), Alignment::Start),
        close,
    ])
}

/// Panel auxiliar que reemplaza el viewport con la lista de bookmarks o
/// el historial. Lee directamente del Profile vía `profile_handle()`; si
/// el chrome corre sin profile (modo efímero) muestra un mensaje.
fn panel_view(kind: PanelKind) -> View<Msg> {
    let (title, items) = match kind {
        PanelKind::Bookmarks => collect_bookmarks(),
        PanelKind::History => collect_history(),
    };

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(38.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(35, 35, 45))
    .children(vec![
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(title, 13.0, Color::from_rgb8(230, 230, 240), Alignment::Start),
        View::new(Style {
            size: Size { width: length(22.0_f32), height: length(22.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(Color::from_rgb8(80, 80, 95))
        .radius(3.0)
        .text_aligned("✕", 12.0, Color::from_rgb8(220, 220, 230), Alignment::Center)
        .on_click(Msg::ClosePanel),
    ]);

    let list: Vec<View<Msg>> = if items.is_empty() {
        let msg = match kind {
            PanelKind::Bookmarks => "(no hay bookmarks · Ctrl+D guarda la pestaña activa)",
            PanelKind::History => "(historial vacío)",
        };
        vec![View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(48.0_f32) },
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(16.0_f32),
                bottom: length(16.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(msg.to_string(), 12.0, Color::from_rgb8(140, 140, 150), Alignment::Start)]
    } else {
        items.into_iter().map(panel_item_row).collect()
    };

    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::WHITE)
    .clip(true)
    .children(list);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![header, body])
}

/// Item de panel: title arriba, url abajo (más chico/gris), click→navega.
/// `removable` con Some(id) agrega un botón ✕ que dispara
/// `Msg::RemoveBookmark(id)`.
struct PanelItem {
    title: String,
    url: String,
    removable: Option<puriy_core::BookmarkId>,
}

fn panel_item_row(item: PanelItem) -> View<Msg> {
    let nav_msg = Msg::Navigate(item.url.clone());
    let title_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        truncate(&item.title, 80),
        13.0,
        Color::from_rgb8(30, 30, 40),
        Alignment::Start,
    );
    let url_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        truncate(&item.url, 100),
        10.0,
        Color::from_rgb8(110, 110, 130),
        Alignment::Start,
    );
    let mut col_children = vec![title_view, url_view];
    let text_col = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(44.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .on_click(nav_msg)
    .children(std::mem::take(&mut col_children));

    let mut row_children = vec![text_col];
    if let Some(id) = item.removable {
        row_children.push(
            View::new(Style {
                size: Size { width: length(24.0_f32), height: length(24.0_f32) },
                margin: Rect {
                    left: length(8.0_f32),
                    right: length(0.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(Color::from_rgb8(220, 220, 230))
            .radius(3.0)
            .text_aligned("✕", 11.0, Color::from_rgb8(80, 80, 95), Alignment::Center)
            .on_click(Msg::RemoveBookmark(id)),
        );
    }

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(54.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(1.0_f32),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::WHITE)
    .hover_fill(Color::from_rgb8(238, 238, 245))
    .children(row_children)
}

/// Lee los bookmarks del Profile (si está cableado) y los devuelve como
/// items de panel con botón de borrar.
fn collect_bookmarks() -> (String, Vec<PanelItem>) {
    let Some(handle) = profile_handle() else {
        return ("Bookmarks · (sin profile)".to_string(), Vec::new());
    };
    let Ok(p) = handle.lock() else {
        return ("Bookmarks".to_string(), Vec::new());
    };
    let items: Vec<PanelItem> = p
        .bookmarks
        .items()
        .iter()
        .map(|b| PanelItem {
            title: if b.title.is_empty() { b.url.clone() } else { b.title.clone() },
            url: b.url.clone(),
            removable: Some(b.id),
        })
        .collect();
    let title = format!("Bookmarks · {} items", items.len());
    (title, items)
}

/// Lee el historial del Profile y lo devuelve descendente (más reciente
/// primero), sin botón de borrado individual por ahora.
fn collect_history() -> (String, Vec<PanelItem>) {
    let Some(handle) = profile_handle() else {
        return ("Historial · (sin profile)".to_string(), Vec::new());
    };
    let Ok(p) = handle.lock() else {
        return ("Historial".to_string(), Vec::new());
    };
    let items: Vec<PanelItem> = p
        .history
        .entries()
        .iter()
        .rev()
        .map(|h| PanelItem {
            title: if h.title.is_empty() { h.url.clone() } else { h.title.clone() },
            url: h.url.clone(),
            removable: None,
        })
        .collect();
    let title = format!("Historial · {} entradas", items.len());
    (title, items)
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
                // Best-effort: persistimos la cache después de cada
                // navegación exitosa. Si el proceso muere por SIGKILL o
                // panic, sólo se pierde la navegación en vuelo — las
                // anteriores ya quedaron en disco.
                puriy_engine::cache::flush();
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

fn header_bar(t: &TabState, zoom: f32) -> View<Msg> {
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
    let zoom_tag = if (zoom - 1.0).abs() > 0.005 {
        format!("    ·    zoom: {}%", (zoom * 100.0).round() as i32)
    } else {
        String::new()
    };
    let status_line = format!(
        "{}    ·    status: {}{}    ·    [Ctrl+T/W/Tab · Alt+←/→ · F5 · Ctrl+= / Ctrl+- / Ctrl+0 zoom · Ctrl+F buscar · Ctrl+B bookmarks · Ctrl+H historial]",
        title_line, t.status, zoom_tag,
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

fn viewport(t: &TabState, zoom: f32, find_query_lc: &str) -> View<Msg> {
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
        .text_aligned(msg.to_string(), 14.0 * zoom, Color::from_rgb8(120, 120, 120), Alignment::Start);
    };

    // Margen del viewport y scroll: el margen interior (24 px / 16 px) no
    // se escala para que el "marco" del documento sea estable; lo que
    // escala es el contenido (font_size + spacing del box tree).
    let content = View::new(Style {
        position: TaffyPosition::Absolute,
        inset: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(16.0_f32 - t.scroll_y),
            bottom: auto(),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![render_box(&tree.root, zoom, find_query_lc)]);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(Color::WHITE)
    .clip(true)
    .children(vec![content])
}

fn render_box(b: &BoxNode, zoom: f32, find_query_lc: &str) -> View<Msg> {
    let style = box_style(b, zoom);
    let mut view = View::new(style);
    // Find-in-page: si la query no es vacía y este nodo es una hoja de
    // texto que la contiene (case-insensitive), pintamos su background
    // con un highlight amarillo. El paint del fill normal del nodo
    // (background CSS) lo aplicamos abajo — si hay match, lo
    // sobrescribimos para que se note.
    let find_hit = !find_query_lc.is_empty()
        && b.text
            .as_ref()
            .map(|s| s.to_lowercase().contains(find_query_lc))
            .unwrap_or(false);

    // visibility:hidden ocupa espacio pero no pinta. Devolvemos la view
    // con su layout pero sin children/text/fill — sus descendientes
    // serían computados pero también deberían ser hidden por inheritance.
    let hidden = matches!(b.visibility, Visibility::Hidden);

    // opacity multiplica el alpha del background sólido. text/border
    // se manejan en apply_decorations/render del texto.
    let alpha_mul = b.opacity.clamp(0.0, 1.0);

    if !hidden {
        if find_hit {
            // Amarillo Chrome-ish — predomina sobre el background CSS.
            view = view.fill(Color::from_rgba8(255, 230, 0, 200));
        } else if let Some(bg) = b.background {
            let a = ((bg.a as f32) * alpha_mul) as u8;
            view = view.fill(Color::from_rgba8(bg.r, bg.g, bg.b, a));
        }
        if let Some(hbg) = b.hover_background {
            let a = ((hbg.a as f32) * alpha_mul) as u8;
            view = view.hover_fill(Color::from_rgba8(hbg.r, hbg.g, hbg.b, a));
        }
        view = apply_decorations(view, b, zoom);
    }
    if hidden {
        // Sin children/text — el subárbol queda invisible pero ocupando
        // su layout. Devolvemos acá para evitar pintar nada.
        return view;
    }
    // `overflow: hidden` aplica clip(true) — recorta el subárbol al
    // borde del rect del nodo.
    if matches!(b.overflow, Overflow::Hidden) {
        view = view.clip(true);
    }

    let link_color = Color::from_rgb8(30, 90, 200);
    let display_color = if b.link.is_some() {
        link_color
    } else {
        Color::from_rgb8(b.color.r, b.color.g, b.color.b)
    };

    // pointer-events:none deshabilita on_click (también propaga por
    // inheritance, así que los descendientes ya lo tienen marcado).
    let pe_active = matches!(b.pointer_events, PointerEvents::Auto);

    if let Some(target) = &b.link {
        if pe_active {
            view = view.on_click(Msg::Navigate(target.clone()));
        }
    }

    // <img> con imagen decodificada: arma peniko::Image, ajusta el rect
    // del nodo al tamaño nativo (taffy luego lo clampa por el ancho del
    // contenedor). Llimphi escala preservando aspect ratio.
    if let Some(img) = &b.image {
        let blob = Blob::from(img.rgba.clone());
        let peniko = PenikoImage::new(blob, ImageFormat::Rgba8, img.width, img.height);
        return image_view(img.width, img.height, zoom).image(peniko);
    }

    if let Some(text) = &b.text {
        let base = if b.font_weight >= 600 { b.font_size * 1.1 } else { b.font_size };
        let size = base * zoom;
        // text-shadows: paint_with previo al texto. Cada shadow se pinta
        // como una segunda capa de texto desplazada y semitransparente —
        // peniko no expone draw text directo desde el callback, así que
        // usamos un rect aproximado proporcional al tamaño de fuente.
        // Aproximación suficiente para hero text decorativo.
        if !b.text_shadows.is_empty() {
            let shadows = b.text_shadows.clone();
            let z = zoom as f64;
            view = view.paint_with(move |scene, _ts, rect| {
                for sh in &shadows {
                    // Banda horizontal centrada de altura ≈ font_size,
                    // desplazada por (offset_x, offset_y), expandida por
                    // blur. Alpha proporcional al blur (más blur = más
                    // difuso = menos opaco).
                    let extra = sh.blur_px as f64 * 0.5 * z;
                    let mid_y = rect.y as f64 + rect.h as f64 * 0.55;
                    let h = size as f64 * 0.55;
                    let r = KurboRect::new(
                        rect.x as f64 + sh.offset_x as f64 * z - extra,
                        mid_y - h * 0.5 + sh.offset_y as f64 * z - extra,
                        (rect.x + rect.w) as f64 + sh.offset_x as f64 * z + extra,
                        mid_y + h * 0.5 + sh.offset_y as f64 * z + extra,
                    );
                    let alpha = if sh.blur_px > 0.0 { 0.35 } else { 0.6 };
                    let c = Color::from_rgba8(
                        sh.color.r,
                        sh.color.g,
                        sh.color.b,
                        (sh.color.a as f64 * alpha) as u8,
                    );
                    scene.fill(Fill::NonZero, Affine::IDENTITY, c, None, &r);
                }
            });
        }
        return view.text_aligned(text.clone(), size, display_color, Alignment::Start);
    }

    if !b.children.is_empty() {
        let kids: Vec<View<Msg>> = if let Some(target) = &b.link {
            b.children.iter().map(|c| render_link_subtree(c, target, link_color, zoom, find_query_lc)).collect()
        } else {
            b.children.iter().map(|c| render_box(c, zoom, find_query_lc)).collect()
        };
        view = view.children(kids);
    }
    view
}

/// View dimensionada para una imagen — ancho hasta `width_px` pero
/// nunca más que el contenedor (`max_width: 100%`), altura proporcional
/// vía aspect ratio inverso (`width / height`).
fn image_view(width: u32, height: u32, zoom: f32) -> View<Msg> {
    let w = (width.max(1)) as f32 * zoom;
    let h = (height.max(1)) as f32 * zoom;
    View::new(Style {
        size: Size { width: length(w), height: length(h) },
        max_size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(4.0_f32 * zoom),
            bottom: length(4.0_f32 * zoom),
        },
        ..Default::default()
    })
}

fn render_link_subtree(
    b: &BoxNode,
    target: &str,
    color: Color,
    zoom: f32,
    find_query_lc: &str,
) -> View<Msg> {
    let mut view = View::new(box_style(b, zoom)).on_click(Msg::Navigate(target.to_string()));
    let find_hit = !find_query_lc.is_empty()
        && b.text
            .as_ref()
            .map(|s| s.to_lowercase().contains(find_query_lc))
            .unwrap_or(false);
    if find_hit {
        view = view.fill(Color::from_rgba8(255, 230, 0, 200));
    } else if let Some(bg) = b.background {
        view = view.fill(Color::from_rgb8(bg.r, bg.g, bg.b));
    }
    if let Some(img) = &b.image {
        let blob = Blob::from(img.rgba.clone());
        let peniko = PenikoImage::new(blob, ImageFormat::Rgba8, img.width, img.height);
        return image_view(img.width, img.height, zoom)
            .image(peniko)
            .on_click(Msg::Navigate(target.to_string()));
    }
    if let Some(text) = &b.text {
        let base = if b.font_weight >= 600 { b.font_size * 1.1 } else { b.font_size };
        let size = base * zoom;
        return view.text_aligned(text.clone(), size, color, Alignment::Start);
    }
    if !b.children.is_empty() {
        view = view.children(
            b.children
                .iter()
                .map(|c| render_link_subtree(c, target, color, zoom, find_query_lc))
                .collect(),
        );
    }
    view
}

/// Aplica `border-radius` y dibuja, en una sola pasada de `paint_with`,
/// la sombra (si la hay) y el contorno del border (si lo hay). Vello
/// pinta el callback entre el `fill` y la `image`/`text` del view, así
/// que la sombra cae detrás del contenido pero encima del fondo del
/// parent. Aproximación: sin gaussian blur — el `blur_px` se mapea
/// como expansión adicional del rect con alpha proporcional, lo cual
/// da una sombra "dura" pero proporcionada.
fn apply_decorations(mut view: View<Msg>, b: &BoxNode, zoom: f32) -> View<Msg> {
    let z = zoom;
    if b.border_radius > 0.0 {
        view = view.radius((b.border_radius * z) as f64);
    }
    let radius = (b.border_radius * z) as f64;
    let shadow = b.box_shadow.map(|s| BoxShadow {
        offset_x: s.offset_x * z,
        offset_y: s.offset_y * z,
        blur_px: s.blur_px * z,
        spread_px: s.spread_px * z,
        color: s.color,
    });
    let alpha_mul = b.opacity.clamp(0.0, 1.0);
    let border = match (b.border_color, b.border_width) {
        (Some(c), w) if w > 0.0 => Some((c, w * z)),
        _ => None,
    };
    // outline se pinta fuera del border + offset, sin afectar layout. Si
    // `style_active` es false (none/hidden) o falta color, no pinta.
    let outline = if b.outline.style_active
        && b.outline.width > 0.0
        && b.outline.color.is_some()
    {
        Some((
            b.outline.color.unwrap(),
            b.outline.width * z,
            b.outline.offset * z,
        ))
    } else {
        None
    };
    // text-decoration sólo tiene efecto visual sobre hojas de texto. En
    // un nodo container, la línea ya la pinta cada hoja descendiente.
    let deco = if b.text.is_some() && b.text_decoration != TextDecorationLine::None {
        Some((b.text_decoration, b.color, b.font_size * z))
    } else {
        None
    };
    let gradient = b.background_gradient.clone();
    if shadow.is_none()
        && border.is_none()
        && deco.is_none()
        && outline.is_none()
        && gradient.is_none()
    {
        return view;
    }
    view.paint_with(move |scene, _typesetter, rect| {
        // linear-gradient: se pinta como fill rectangular alineado al
        // ángulo CSS. peniko interpreta `Linear { start, end }` como
        // las dos puntas — calculamos el segmento atravesando el rect
        // en la dirección dada.
        if let Some(g) = &gradient {
            if let Some(brush) = build_linear_gradient_brush(g, rect, alpha_mul) {
                let r = RoundedRect::new(
                    rect.x as f64,
                    rect.y as f64,
                    (rect.x + rect.w) as f64,
                    (rect.y + rect.h) as f64,
                    radius,
                );
                scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &r);
            }
        }
        if let Some(BoxShadow { offset_x, offset_y, blur_px, spread_px, color }) = shadow {
            let extra = (blur_px + spread_px) as f64;
            let half_alpha = if blur_px > 0.0 { 0.55 } else { 0.85 };
            let sc = Color::from_rgba8(
                color.r,
                color.g,
                color.b,
                (color.a as f64 * half_alpha) as u8,
            );
            let r = RoundedRect::new(
                (rect.x + offset_x) as f64 - extra,
                (rect.y + offset_y) as f64 - extra,
                (rect.x + rect.w + offset_x) as f64 + extra,
                (rect.y + rect.h + offset_y) as f64 + extra,
                (radius + extra).max(0.0),
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, sc, None, &r);
        }
        if let Some((bc, w)) = border {
            let stroke = Stroke::new(w as f64);
            let half = stroke.width * 0.5;
            let r = RoundedRect::new(
                rect.x as f64 + half,
                rect.y as f64 + half,
                (rect.x + rect.w) as f64 - half,
                (rect.y + rect.h) as f64 - half,
                (radius - half).max(0.0),
            );
            let a = (bc.a as f32 * alpha_mul) as u8;
            let color = Color::from_rgba8(bc.r, bc.g, bc.b, a);
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &r);
        }
        if let Some((oc, ow, off)) = outline {
            let stroke = Stroke::new(ow as f64);
            let half = stroke.width * 0.5;
            // outline se dibuja FUERA del border, separado por `offset`.
            let outset = (off as f64) + half;
            let r = RoundedRect::new(
                rect.x as f64 - outset,
                rect.y as f64 - outset,
                (rect.x + rect.w) as f64 + outset,
                (rect.y + rect.h) as f64 + outset,
                radius + outset,
            );
            let a = (oc.a as f32 * alpha_mul) as u8;
            let color = Color::from_rgba8(oc.r, oc.g, oc.b, a);
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &r);
        }
        if let Some((line_kind, c, font_size)) = deco {
            // Posición vertical relativa al rect (sin baseline real). El
            // rect del leaf de texto tiene height = font_size * line_height
            // (≈1.4 default), así que el texto vive arriba-centro:
            //   overline    → top + line_height*0.10
            //   line-through → mid (≈ 0.55)
            //   underline   → ~ baseline (≈ 0.85)
            let y_frac = match line_kind {
                TextDecorationLine::Overline => 0.10,
                TextDecorationLine::LineThrough => 0.55,
                TextDecorationLine::Underline => 0.88,
                TextDecorationLine::None => return,
            };
            let y = rect.y as f64 + rect.h as f64 * y_frac;
            let thickness = ((font_size * 0.07) as f64).max(1.0);
            let stroke = Stroke::new(thickness);
            let dec_color = Color::from_rgba8(c.r, c.g, c.b, 255);
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                dec_color,
                None,
                &Line::new((rect.x as f64, y), ((rect.x + rect.w) as f64, y)),
            );
        }
    })
}

fn box_style(b: &BoxNode, zoom: f32) -> Style {
    // Si el nodo es una hoja de texto, le damos un height ≈ line-height
    // para que el row del padre tenga altura real — sin esto, taffy
    // colapsa los inlines al top del bloque. Para inlines con hijos
    // dejamos auto y que el padre mida.
    let is_text_leaf = b.text.is_some();
    let lh_mult = b.line_height.unwrap_or(1.4);
    let line_h = b.font_size * lh_mult * zoom;

    let is_flex = matches!(b.display, Display::Flex | Display::InlineFlex);

    let is_grid = matches!(b.display, Display::Grid | Display::InlineGrid);

    // Defaults según display: Block fila completa columnar, Inline en row
    // con altura auto, Flex toma sus props del nodo. None: cero.
    let (default_direction, mut width, height) = match b.display {
        Display::Block => (FlexDirection::Column, percent(1.0_f32), auto()),
        Display::Flex => (map_flex_direction(b.flex_direction), percent(1.0_f32), auto()),
        Display::InlineFlex => (map_flex_direction(b.flex_direction), auto(), auto()),
        Display::Grid => (FlexDirection::Column, percent(1.0_f32), auto()),
        Display::InlineGrid => (FlexDirection::Column, auto(), auto()),
        Display::InlineBlock | Display::Inline => {
            let h = if is_text_leaf { length(line_h) } else { auto() };
            (FlexDirection::Row, auto(), h)
        }
        Display::None => (FlexDirection::Column, length(0.0_f32), length(0.0_f32)),
    };

    // Para bloques con hijos inline conmutamos a Row + Wrap (igual que
    // antes — el hack original que hace que `<p>` flowee tokens). Para
    // Flex respetamos las props del autor sin tocar.
    let block_inline_wrap =
        matches!(b.display, Display::Block) && has_inline_children(b);

    let flex_wrap = if is_flex {
        map_flex_wrap(b.flex_wrap)
    } else if block_inline_wrap {
        FlexWrap::Wrap
    } else {
        FlexWrap::NoWrap
    };

    let (flex_direction, w_base) = if block_inline_wrap {
        (FlexDirection::Row, percent(1.0_f32))
    } else {
        (default_direction, width)
    };
    width = w_base;

    // CSS `width` explícito gana sobre el default de display.
    if let Some(explicit) = length_to_taffy(b.width, zoom) {
        width = explicit;
    }
    let max_size = Size {
        width: length_to_taffy(b.max_width, zoom).unwrap_or_else(auto),
        height: length_to_taffy(b.max_height, zoom).unwrap_or_else(auto),
    };
    let min_size = Size {
        width: length_to_taffy(b.min_width, zoom).unwrap_or_else(|| length(0.0_f32)),
        height: length_to_taffy(b.min_height, zoom).unwrap_or_else(|| length(0.0_f32)),
    };

    // justify/align: si es flex, vienen del autor; sino, sólo derivamos
    // `justify_content` de `text-align` sobre bloques con inlines (el
    // viejo comportamiento heredado).
    let justify_content = if is_flex {
        Some(map_justify(b.justify_content))
    } else if block_inline_wrap {
        match b.text_align {
            TextAlign::Left | TextAlign::Justify => None,
            TextAlign::Center => Some(JustifyContent::Center),
            TextAlign::Right => Some(JustifyContent::End),
        }
    } else {
        None
    };

    let align_items = if is_flex {
        Some(map_align(b.align_items))
    } else {
        None
    };

    // gap: aplica a flex (y a futuros grid). Taffy lo expone como
    // `Size { width: column-gap, height: row-gap }`.
    let gap = if is_flex {
        Size {
            width: length(b.gap_column * zoom),
            height: length(b.gap_row * zoom),
        }
    } else {
        Size { width: length(0.0_f32), height: length(0.0_f32) }
    };

    // box-sizing default CSS = ContentBox; los resets modernos lo
    // fuerzan a BorderBox. Taffy 0.9 default es BorderBox así que
    // mapeamos explícito en ambos sentidos.
    let box_sizing = match b.box_sizing {
        CssBoxSizing::ContentBox => BoxSizing::ContentBox,
        CssBoxSizing::BorderBox => BoxSizing::BorderBox,
    };
    // vertical-align mapea a align_self (con prioridad sobre el de
    // align-self CSS) cuando es inline/inline-block — no es lo mismo en
    // CSS spec pero alcanza para el subset que nos importa.
    let align_self = match b.vertical_align {
        VerticalAlign::Baseline => map_align_self(b.align_self),
        VerticalAlign::Top => Some(AlignSelf::Start),
        VerticalAlign::Middle => Some(AlignSelf::Center),
        VerticalAlign::Bottom | VerticalAlign::Sub => Some(AlignSelf::End),
        VerticalAlign::Super => Some(AlignSelf::Start),
    };
    let flex_basis: Dimension = length_to_taffy(b.flex_basis, zoom).unwrap_or_else(auto);

    // Position + insets (top/right/bottom/left).
    let position_kind = match b.position {
        CssPosition::Static => TaffyPosition::Relative, // = layout normal
        CssPosition::Relative | CssPosition::Sticky => TaffyPosition::Relative,
        CssPosition::Absolute | CssPosition::Fixed => TaffyPosition::Absolute,
    };
    let inset = Rect {
        top: length_to_inset(b.inset_top, zoom),
        right: length_to_inset(b.inset_right, zoom),
        bottom: length_to_inset(b.inset_bottom, zoom),
        left: length_to_inset(b.inset_left, zoom),
    };

    // Taffy Display: Block/Flex/Grid/None. Inline/InlineBlock las
    // tratamos como Flex (row) por las hacks de inlines.
    let taffy_display = match b.display {
        Display::None => TaffyDisplay::None,
        Display::Grid | Display::InlineGrid => TaffyDisplay::Grid,
        _ => TaffyDisplay::Flex,
    };

    // Grid templates — sólo se aplican si display es grid. Las pistas Px
    // se escalan con zoom; fr/auto/pct quedan intactas.
    let grid_template_columns: Vec<GridTemplateComponent<String>> =
        if is_grid { b.grid_template_columns.iter().map(|t| map_grid_track(t, zoom)).collect() } else { Vec::new() };
    let grid_template_rows: Vec<GridTemplateComponent<String>> =
        if is_grid { b.grid_template_rows.iter().map(|t| map_grid_track(t, zoom)).collect() } else { Vec::new() };

    Style {
        display: taffy_display,
        flex_direction,
        flex_wrap,
        justify_content,
        align_items,
        align_self,
        flex_grow: b.flex_grow,
        flex_shrink: b.flex_shrink,
        flex_basis,
        box_sizing,
        position: position_kind,
        inset,
        gap,
        size: Size { width, height },
        min_size,
        max_size,
        margin: Rect {
            left: length(b.margin.left * zoom),
            right: length(b.margin.right * zoom),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        padding: Rect {
            left: length(b.padding.left * zoom),
            right: length(b.padding.right * zoom),
            top: length(b.padding.top * zoom),
            bottom: length(b.padding.bottom * zoom),
        },
        grid_template_columns: grid_template_columns.into(),
        grid_template_rows: grid_template_rows.into(),
        ..Default::default()
    }
}

fn map_grid_track(t: &GridTrackSize, zoom: f32) -> GridTemplateComponent<String> {
    let single: TrackSizingFunction = match t {
        GridTrackSize::Auto => auto(),
        GridTrackSize::Px(v) => length(*v * zoom),
        GridTrackSize::Pct(v) => percent(*v / 100.0),
        GridTrackSize::Fr(v) => fr(*v),
    };
    GridTemplateComponent::Single(single)
}

/// `length-percentage-auto`: para insets (top/right/bottom/left) que
/// aceptan `auto` además de px/%. `zoom` escala sólo el valor Px;
/// los porcentajes se resuelven contra el contenedor (que también escala).
fn length_to_inset(v: LengthVal, zoom: f32) -> LengthPercentageAuto {
    match v {
        LengthVal::Auto => auto(),
        LengthVal::Px(px) => length(px * zoom),
        LengthVal::Pct(pct) => percent(pct / 100.0),
    }
}

fn map_align_self(a: CssAlignSelf) -> Option<AlignSelf> {
    match a {
        CssAlignSelf::Auto => None,
        CssAlignSelf::Start => Some(AlignSelf::Start),
        CssAlignSelf::Center => Some(AlignSelf::Center),
        CssAlignSelf::End => Some(AlignSelf::End),
        CssAlignSelf::Stretch => Some(AlignSelf::Stretch),
        CssAlignSelf::Baseline => Some(AlignSelf::Baseline),
    }
}

/// Calcula el segmento (start, end) que cruza el rect en la dirección
/// CSS (0deg = up, 90deg = right, etc.) y arma un peniko::Gradient
/// linear con los stops del nodo. Aplica `alpha_mul` (opacity) a cada
/// stop. Devuelve None si los stops no se pueden representar.
fn build_linear_gradient_brush(
    g: &LinearGradient,
    rect: llimphi_ui::PaintRect,
    alpha_mul: f32,
) -> Option<Gradient> {
    if g.stops.len() < 2 {
        return None;
    }
    // CSS: 0deg = up (negative y), 90 = right (+x), 180 = down (+y),
    // 270 = left (-x). Convertimos a radianes y direccion en
    // espacio de pantalla (y crece hacia abajo).
    let theta = (g.angle_deg).to_radians();
    let dx = theta.sin() as f64;
    let dy = -theta.cos() as f64;
    let w = rect.w as f64;
    let h = rect.h as f64;
    // Largo del segmento que cubre el rect en la dirección (dx, dy):
    // proyectamos cada esquina sobre el eje y tomamos el rango.
    let cx = rect.x as f64 + w * 0.5;
    let cy = rect.y as f64 + h * 0.5;
    let half_len = (dx.abs() * w + dy.abs() * h) * 0.5;
    let start = Point::new(cx - dx * half_len, cy - dy * half_len);
    let end = Point::new(cx + dx * half_len, cy + dy * half_len);

    // Stops: si pos es None, distribuir uniformemente.
    let n = g.stops.len();
    let mut peniko_stops: Vec<ColorStop> = Vec::with_capacity(n);
    for (i, s) in g.stops.iter().enumerate() {
        let pos = s.pos.unwrap_or_else(|| {
            if n == 1 { 0.0 } else { i as f32 / (n - 1) as f32 }
        });
        let a = ((s.color.a as f32) * alpha_mul) as u8;
        let c = Color::from_rgba8(s.color.r, s.color.g, s.color.b, a);
        peniko_stops.push(ColorStop::from((pos, c)));
    }
    Some(Gradient {
        kind: GradientKind::Linear { start, end },
        stops: ColorStops(peniko_stops.into()),
        ..Default::default()
    })
}

fn map_flex_direction(d: CssFlexDirection) -> FlexDirection {
    match d {
        CssFlexDirection::Row => FlexDirection::Row,
        CssFlexDirection::RowReverse => FlexDirection::RowReverse,
        CssFlexDirection::Column => FlexDirection::Column,
        CssFlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
    }
}

fn map_flex_wrap(w: CssFlexWrap) -> FlexWrap {
    match w {
        CssFlexWrap::NoWrap => FlexWrap::NoWrap,
        CssFlexWrap::Wrap => FlexWrap::Wrap,
        CssFlexWrap::WrapReverse => FlexWrap::WrapReverse,
    }
}

fn map_justify(j: CssJustifyContent) -> JustifyContent {
    match j {
        CssJustifyContent::Start => JustifyContent::Start,
        CssJustifyContent::Center => JustifyContent::Center,
        CssJustifyContent::End => JustifyContent::End,
        CssJustifyContent::SpaceBetween => JustifyContent::SpaceBetween,
        CssJustifyContent::SpaceAround => JustifyContent::SpaceAround,
        CssJustifyContent::SpaceEvenly => JustifyContent::SpaceEvenly,
    }
}

fn map_align(a: CssAlignItems) -> AlignItems {
    match a {
        CssAlignItems::Start => AlignItems::Start,
        CssAlignItems::Center => AlignItems::Center,
        CssAlignItems::End => AlignItems::End,
        CssAlignItems::Stretch => AlignItems::Stretch,
        CssAlignItems::Baseline => AlignItems::Baseline,
    }
}

/// Traduce un `LengthVal` CSS al tipo de longitud que taffy entiende.
/// `Auto` queda como `None` (caller lo reemplaza con el default según
/// display o `auto()` para max-size).
fn length_to_taffy(v: LengthVal, zoom: f32) -> Option<llimphi_layout::taffy::style::Dimension> {
    match v {
        LengthVal::Auto => None,
        LengthVal::Px(px) => Some(length(px * zoom)),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: parsea un snippet HTML offline y devuelve el BoxTree.
    fn parse(html: &str) -> BoxTree {
        let engine = Engine::new();
        engine.load_html("about:test", html).box_tree
    }

    #[test]
    fn count_matches_devuelve_cero_cuando_query_vacia() {
        let tree = parse("<p>hola mundo</p>");
        assert_eq!(count_matches(Some(&tree), ""), 0);
    }

    #[test]
    fn count_matches_devuelve_cero_cuando_tree_none() {
        assert_eq!(count_matches(None, "algo"), 0);
    }

    #[test]
    fn count_matches_es_case_insensitive() {
        let tree = parse("<p>Hola MUNDO</p><p>mundO repetido</p>");
        // La query ya viene lowercased — emula lo que hace `view()`.
        let n = count_matches(Some(&tree), "mundo");
        assert!(n >= 2, "esperaba >= 2 matches, conseguí {n}");
    }

    #[test]
    fn count_matches_busca_dentro_de_hojas() {
        let tree = parse(
            "<article><h1>Tutorial</h1><p>Este tutorial cubre Rust</p><p>Otra cosa</p></article>",
        );
        // La query "tutorial" matchea el <h1> y el primer <p> (ambos como hojas).
        let n = count_matches(Some(&tree), "tutorial");
        assert_eq!(n, 2);
    }

    #[test]
    fn count_matches_query_sin_hits_devuelve_cero() {
        let tree = parse("<p>foo bar baz</p>");
        assert_eq!(count_matches(Some(&tree), "qwerty"), 0);
    }
}
