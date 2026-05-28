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

/// Estado per-`<select>` en una pestaña: opción seleccionada + si está
/// abierto.
#[derive(Debug, Clone)]
pub struct SelectState {
    pub selected: usize,
    pub open: bool,
}

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
    /// HTML crudo de la respuesta. Lo usamos para `Ctrl+U` (page source).
    /// `None` si la pestaña todavía no cargó.
    pub source: Option<String>,
    /// Generación monótona — Loaded de generaciones viejas se descarta.
    pub gen: u64,
    /// Estado de los `<input>`/`<textarea>` por DFS index. Cada slot lleva
    /// el `TextInputState` de un input/textarea — se crea con su `value`/
    /// contenido inicial cuando llega el `Msg::Loaded` y persiste hasta la
    /// próxima navegación de la pestaña.
    pub inputs: Vec<TextInputState>,
    /// Estado `checked` paralelo a `inputs` para los slots cuyo
    /// `input_kind` es Checkbox/Radio. Para slots Text/Search/Password/
    /// TextArea/Submit el bool se ignora; pero el indexing es 1:1 para
    /// que sumar otro Vec no agregue otra fuente de drift.
    pub input_checks: Vec<bool>,
    /// Estado por `<select>` del documento (mismo orden DFS que el
    /// `select` walk del Loaded). Cada slot lleva el índice seleccionado
    /// y si está abierto (dropdown expandido).
    pub selects: Vec<SelectState>,
    /// Índice DFS del input/textarea focado (clave en `inputs`). `None` =
    /// sin foco; el chrome rutea teclas al resto del flow.
    pub focused_input: Option<usize>,
    /// Estado open/closed por `<details>` en orden DFS. Se inicializa al
    /// recibir `Msg::Loaded` walkeando el box tree y consultando
    /// `details_open_attr` de cada `<details>`. Subsiguientes
    /// `Msg::ToggleDetails(idx)` flippean el bool. Reset en cada
    /// navegación para evitar índices stale.
    pub details_open: Vec<bool>,
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
            source: None,
            gen: 0,
            inputs: Vec::new(),
            input_checks: Vec::new(),
            selects: Vec::new(),
            focused_input: None,
            details_open: Vec::new(),
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
    /// Match "actual" (1-based) cuando el usuario navega con
    /// Enter/Shift+Enter. `0` = sin nav todavía (todos los matches en
    /// amarillo); `>= 1` = ese match pinta en naranja para destacarse.
    pub find_current: usize,
    /// `Ctrl+B`/`Ctrl+H` abren un panel que reemplaza el viewport con la
    /// lista de bookmarks o el historial. `None` = panel cerrado y el
    /// documento se renderea normal. Sólo aplica cuando el chrome corre
    /// con un Profile cableado (sino las listas están vacías).
    pub panel: Option<PanelKind>,
    /// URL del link bajo el cursor (preview en status bar). `None` =
    /// cursor no está sobre ningún link.
    pub hover_link: Option<String>,
    /// Filtro de texto del panel (Bookmarks/History). Substring
    /// case-insensitive contra `title` y `url` de cada item. Vacío =
    /// muestra todo. Persistente entre toggle del panel pero se limpia
    /// si el usuario cambia de pestaña (no — por ahora se conserva).
    pub panel_filter: TextInputState,
}

/// Tipo de panel auxiliar que reemplaza el viewport cuando está abierto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    Bookmarks,
    History,
    /// Vista de "Page Source" — HTML crudo de la pestaña activa.
    Source,
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
    Loaded {
        tab: TabId,
        gen: u64,
        /// URL final tras seguir redirects (3xx). Si el server redirigió,
        /// difiere de la URL solicitada — el chrome actualiza
        /// `t.url`/`t.addr` y reemplaza el último entry de history.
        final_url: String,
        title: String,
        box_tree: BoxTree,
        source: String,
        /// `<meta http-equiv="refresh">` del head, si existía. El chrome
        /// programa un thread con sleep y dispatcha `MetaRefreshFire`.
        meta_refresh: Option<puriy_engine::MetaRefresh>,
    },
    /// Disparo del temporizador de `<meta refresh>`. El handler valida
    /// `tab` y `gen` para descartar refreshes pendientes de pestañas
    /// cerradas o pisadas por otra navegación. `url=None` significa
    /// recargar; `Some(u)` navega a la URL ya resuelta contra base.
    MetaRefreshFire { tab: TabId, gen: u64, url: Option<String> },
    LoadFailed { tab: TabId, gen: u64, err: String },
    Navigate(String),
    /// Navega vía POST con body application/x-www-form-urlencoded.
    /// Usado por form submit con `method=post`.
    NavigatePost { url: String, body: String },
    /// Igual que Navigate pero arranca en una pestaña nueva — usado por
    /// `<a target="_blank">` y middle-click sobre links (ese día llega).
    NavigateNewTab(String),
    /// Click en `<a download>` — el chrome descarga el target a
    /// `$XDG_DOWNLOAD_DIR/puriy/` (o `~/Downloads/puriy/`) en lugar de
    /// navegar. `filename_hint` viene del attr `download="..."`; vacío
    /// = usar el último segmento del path de la URL.
    DownloadLink { url: String, filename_hint: String },
    /// Resultado del thread que descarga: cuenta de bytes escritos al
    /// disco o mensaje de error. Mismo gen check que `Loaded`.
    DownloadDone {
        tab: TabId,
        gen: u64,
        path: String,
        result: Result<usize, String>,
    },
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
    /// Enter en la find bar — avanza al próximo match (wrap a 1 en el
    /// extremo). Si no hay matches, no-op.
    FindNext,
    /// Shift+Enter — retrocede un match (wrap a N).
    FindPrev,
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
    /// Teclas redirigidas al input del filtro de panel (Bookmarks/History).
    PanelFilterKey(KeyEvent),
    /// Ctrl+U — toggle del panel "Page Source" para la pestaña activa.
    ViewSource,
    /// Pointer entered un `<a>` (cursor sobre link). `Some(url)` levanta
    /// el preview en la status bar; `None` lo limpia (pointer leave).
    HoverLink(Option<String>),
    /// Click en el "header" de un `<select>` (la barra siempre visible)
    /// — toggle abre/cierra el dropdown.
    SelectToggle(usize),
    /// Click en una de las opciones del dropdown — setea selected y cierra.
    SelectPick(usize, usize),
    /// Click en un `<input type=checkbox>` — flippea el bool.
    ToggleCheckbox(usize),
    /// Click en un `<input type=radio>` — setea sólo éste como checked
    /// en su grupo (mismo `name`).
    SelectRadio(usize),
    /// Click en un `<input type=submit>` — submitea su form.
    SubmitForm(usize),
    /// Click sobre un `<input>`/`<textarea>` del documento — foca ese
    /// input por índice DFS.
    FocusInput(usize),
    /// Teclas redirigidas al input focado (cuando `focused_input` es Some).
    InputKey(KeyEvent),
    /// Tab — foco al próximo input/textarea/select (con wrap).
    FocusNext,
    /// Shift+Tab — foco al input anterior.
    FocusPrev,
    /// Click en `<summary>` (o en la flecha que lo precede): toggle del
    /// `<details>` cuyo índice DFS es `idx`. Si el índice excede el
    /// `details_open` actual, el msg es no-op (ej: re-render durante una
    /// carga nueva).
    ToggleDetails(usize),
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
        spawn_load(tab.id, tab.gen, url, /* referer */ None, handle.clone());
        Model {
            tabs: vec![tab],
            active: 0,
            zoom: 1.0,
            find_active: false,
            find_input: TextInputState::new(),
            find_current: 0,
            panel: None,
            panel_filter: TextInputState::new(),
            hover_link: None,
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
                Key::Character(s) if s.eq_ignore_ascii_case("u") => {
                    return Some(Msg::ViewSource);
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
        // Si la find bar está activa, intercepta Esc (cerrar), Enter
        // (avanza match) y Shift+Enter (retrocede), y redirige el resto
        // al input. Tiene prioridad sobre el address bar.
        if model.find_active {
            if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::FindClose);
            }
            if matches!(&e.key, Key::Named(NamedKey::Enter)) {
                return Some(if mods.shift { Msg::FindPrev } else { Msg::FindNext });
            }
            return Some(Msg::FindKey(e.clone()));
        }
        // Panel abierto (bookmarks/history): Esc cierra; resto va al
        // input del filtro. F5 no se intercepta (es semánticamente la
        // pestaña activa, no el panel).
        if model.panel.is_some() {
            if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::ClosePanel);
            }
            if !matches!(&e.key, Key::Named(NamedKey::F5)) {
                return Some(Msg::PanelFilterKey(e.clone()));
            }
        }
        // Si la address bar tiene foco, redirige las teclas al input.
        if model.active().addr_focused && !matches!(&e.key, Key::Named(NamedKey::F5)) {
            return Some(Msg::AddrKey(e.clone()));
        }
        // Si hay un input/textarea del documento focado, las teclas van
        // ahí. Esc blurea (foco vuelve a la página). F5 se respeta como
        // recargar para no perder un atajo crítico. Tab/Shift+Tab cicla
        // entre inputs sin pisar el typing.
        if model.active().focused_input.is_some() {
            if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::FocusInput(usize::MAX)); // sentinel = blur
            }
            if matches!(&e.key, Key::Named(NamedKey::Tab)) {
                return Some(if mods.shift { Msg::FocusPrev } else { Msg::FocusNext });
            }
            if !matches!(&e.key, Key::Named(NamedKey::F5)) {
                return Some(Msg::InputKey(e.clone()));
            }
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
            Msg::Loaded { tab, gen, final_url, title, box_tree, source, meta_refresh } => {
                if let Some(idx) = m.tab_idx(tab) {
                    let t = &mut m.tabs[idx];
                    if t.gen == gen {
                        // Si hubo redirect, propaga la URL final a la
                        // tab, la address bar y reemplaza el slot
                        // actual de history (no empuja uno nuevo — el
                        // back debe saltar a la página *anterior* a la
                        // que pidió el redirect, no al request fallido).
                        if final_url != t.url {
                            t.url = final_url.clone();
                            t.addr.set_text(final_url.clone());
                            if let Some(slot) = t.history.get_mut(t.cursor) {
                                *slot = final_url.clone();
                            }
                        }
                        t.title = title.clone();
                        let n = box_tree.descendants_count();
                        t.status = format!("OK · {n} boxes");
                        t.source = Some(source);
                        // Prefill el estado de los <details> walkeando el
                        // árbol nuevo en orden DFS — cada `<details>`
                        // aporta un bool inicializado desde su
                        // `open` attribute.
                        let mut details_open = Vec::new();
                        let mut inputs: Vec<TextInputState> = Vec::new();
                        let mut input_checks: Vec<bool> = Vec::new();
                        let mut selects: Vec<SelectState> = Vec::new();
                        let mut autofocus_idx: Option<usize> = None;
                        box_tree.walk(|b| {
                            if b.tag.as_deref() == Some("details") {
                                details_open.push(b.details_open_attr);
                            }
                            if b.input_kind.is_some() {
                                let mut s = TextInputState::new();
                                if let Some(initial) = &b.input_initial {
                                    s.set_text(initial.clone());
                                }
                                let idx = inputs.len();
                                inputs.push(s);
                                input_checks.push(b.input_checked_initial);
                                if b.input_autofocus && autofocus_idx.is_none() {
                                    autofocus_idx = Some(idx);
                                }
                            }
                            if let Some(sel) = &b.select {
                                selects.push(SelectState {
                                    selected: sel.initial,
                                    open: false,
                                });
                            }
                        });
                        t.details_open = details_open;
                        t.inputs = inputs;
                        t.input_checks = input_checks;
                        t.selects = selects;
                        t.focused_input = autofocus_idx;
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
                        // <meta http-equiv="refresh"> — programa un thread
                        // que duerme N segundos y dispatcha
                        // MetaRefreshFire. El gen counter lo invalida si
                        // el usuario navegó manualmente antes de que vence.
                        if let Some(mr) = meta_refresh {
                            let resolved = mr.url.as_deref().and_then(|u| {
                                url::Url::parse(&t.url)
                                    .ok()
                                    .and_then(|base| base.join(u).ok())
                                    .map(|abs| abs.to_string())
                            });
                            t.status = match (mr.delay_secs, resolved.as_deref()) {
                                (0, Some(u)) => format!("→ refresh inmediato a {u}"),
                                (n, Some(u)) => format!("→ refresh en {n}s a {u}"),
                                (0, None) => "↻ reload inmediato".to_string(),
                                (n, None) => format!("↻ reload en {n}s"),
                            };
                            let h = handle.clone();
                            let delay = mr.delay_secs;
                            std::thread::spawn(move || {
                                if delay > 0 {
                                    std::thread::sleep(std::time::Duration::from_secs(
                                        delay as u64,
                                    ));
                                }
                                h.dispatch(Msg::MetaRefreshFire { tab, gen, url: resolved });
                            });
                        }
                    }
                }
            }
            Msg::MetaRefreshFire { tab, gen, url } => {
                // Sólo dispara si la pestaña sigue existiendo y no fue
                // pisada por otra navegación manual (gen counter).
                if let Some(idx) = m.tab_idx(tab) {
                    if m.tabs[idx].gen == gen {
                        m.active = idx;
                        let target = url.unwrap_or_else(|| m.tabs[idx].url.clone());
                        return Self::update(m, Msg::Navigate(target), handle);
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
                m.panel_filter.clear();
                // Same-page fragment navigation: si la URL solicitada
                // sólo difiere de la actual en el fragment, scrolleamos
                // al elemento con id matching y NO recargamos. Esto
                // matchea el comportamiento estándar de browsers para
                // `<a href="#sección">` y para typear `URL#frag` en la
                // barra estando ya en `URL`.
                let t = m.active();
                let same_doc_frag = same_doc_with_fragment(&t.url, &target);
                if let Some(frag) = same_doc_frag {
                    let y = t
                        .box_tree
                        .as_ref()
                        .and_then(|bt| bt.find_element_y(&frag));
                    let t = m.active_mut();
                    t.url = target.clone();
                    t.addr.set_text(target.clone());
                    t.history.truncate(t.cursor + 1);
                    if t.history.last() != Some(&target) {
                        t.history.push(target);
                        t.cursor = t.history.len() - 1;
                    }
                    if let Some(y) = y {
                        t.scroll_y = y.max(0.0);
                        t.status = format!("↟ #{frag}");
                    } else {
                        t.status = format!("(sin id #{frag})");
                    }
                    return m;
                }
                start_load(&mut m, target, /* push_history */ true, handle);
            }
            Msg::NavigatePost { url, body } => {
                m.panel = None;
                m.panel_filter.clear();
                start_load_post(&mut m, url, body, handle);
            }
            Msg::DownloadLink { url, filename_hint } => {
                let filename = pick_download_filename(&url, &filename_hint);
                let path = download_path(&filename);
                let status_path = path.display().to_string();
                m.active_mut().status = format!("⬇ descargando {filename}…");
                let h = handle.clone();
                let active_tab_id = m.active().id;
                let active_gen = m.active().gen;
                let url_clone = url.clone();
                std::thread::spawn(move || {
                    let result = puriy_engine::fetch::fetch_bytes(&url_clone)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| {
                            if let Some(parent) = path.parent() {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
                            Ok(bytes.len())
                        });
                    h.dispatch(Msg::DownloadDone {
                        tab: active_tab_id,
                        gen: active_gen,
                        path: status_path,
                        result,
                    });
                });
            }
            Msg::DownloadDone { tab, gen, path, result } => {
                if let Some(idx) = m.tab_idx(tab) {
                    if m.tabs[idx].gen == gen {
                        m.tabs[idx].status = match result {
                            Ok(n) => format!("⬇ {path} · {n} bytes"),
                            Err(e) => format!("⬇ fallo: {e}"),
                        };
                    }
                }
            }
            Msg::NavigateNewTab(target) => {
                // `target="_blank"` debe enviar Referer del padre.
                let referer = {
                    let cur = m.active().url.clone();
                    if cur == NEW_TAB_URL || cur.is_empty() { None } else { Some(cur) }
                };
                let mut tab = TabState::new(target.clone());
                tab.gen = 1;
                spawn_load(tab.id, tab.gen, target, referer, handle.clone());
                m.tabs.push(tab);
                m.active = m.tabs.len() - 1;
                m.panel = None;
                m.panel_filter.clear();
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
                m.find_current = 0;
            }
            Msg::FindClose => {
                m.find_active = false;
                m.find_input.clear();
                m.find_current = 0;
            }
            Msg::FindKey(e) => {
                let before = m.find_input.text();
                m.find_input.apply_key(&e);
                let after = m.find_input.text();
                if before != after {
                    // Query cambió → cualquier "match actual" previo
                    // queda inválido. Esperamos el primer Enter para
                    // arrancar la navegación.
                    m.find_current = 0;
                }
            }
            Msg::FindNext => {
                let q = m.find_input.text().to_lowercase();
                let total = count_matches(m.active().box_tree.as_ref(), &q);
                if total > 0 {
                    m.find_current = if m.find_current >= total {
                        1
                    } else {
                        m.find_current + 1
                    };
                    scroll_to_find_match(&mut m, &q);
                }
            }
            Msg::FindPrev => {
                let q = m.find_input.text().to_lowercase();
                let total = count_matches(m.active().box_tree.as_ref(), &q);
                if total > 0 {
                    m.find_current = if m.find_current <= 1 {
                        total
                    } else {
                        m.find_current - 1
                    };
                    scroll_to_find_match(&mut m, &q);
                }
            }
            Msg::ToggleBookmarks => {
                m.panel = match m.panel {
                    Some(PanelKind::Bookmarks) => None,
                    _ => Some(PanelKind::Bookmarks),
                };
                m.panel_filter.clear();
            }
            Msg::ToggleHistory => {
                m.panel = match m.panel {
                    Some(PanelKind::History) => None,
                    _ => Some(PanelKind::History),
                };
                m.panel_filter.clear();
            }
            Msg::ClosePanel => {
                m.panel = None;
                m.panel_filter.clear();
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
            Msg::ToggleDetails(idx) => {
                let t = m.active_mut();
                if let Some(slot) = t.details_open.get_mut(idx) {
                    *slot = !*slot;
                }
            }
            Msg::PanelFilterKey(e) => {
                m.panel_filter.apply_key(&e);
            }
            Msg::ViewSource => {
                m.panel = match m.panel {
                    Some(PanelKind::Source) => None,
                    _ => Some(PanelKind::Source),
                };
                m.panel_filter.clear();
            }
            Msg::HoverLink(url) => {
                m.hover_link = url;
            }
            Msg::SelectToggle(idx) => {
                let t = m.active_mut();
                if let Some(s) = t.selects.get_mut(idx) {
                    s.open = !s.open;
                }
            }
            Msg::SelectPick(idx, opt) => {
                let t = m.active_mut();
                if let Some(s) = t.selects.get_mut(idx) {
                    s.selected = opt;
                    s.open = false;
                }
            }
            Msg::ToggleCheckbox(idx) => {
                let t = m.active_mut();
                if let Some(c) = t.input_checks.get_mut(idx) {
                    *c = !*c;
                }
            }
            Msg::SelectRadio(idx) => {
                // Encontrá el `name` de este radio + form_idx; los radios
                // del mismo grupo se desmarcan, éste queda marcado.
                let tree_opt = m.active().box_tree.clone();
                let Some(tree) = tree_opt else { return m };
                let mut my_name: Option<String> = None;
                let mut my_form: Option<usize> = None;
                let mut i = 0usize;
                tree.walk(|b| {
                    if b.input_kind.is_some() {
                        if i == idx {
                            my_name = b.input_name.clone();
                            my_form = b.form_idx;
                        }
                        i += 1;
                    }
                });
                let mut counter = 0usize;
                let t = m.active_mut();
                tree.walk(|b| {
                    if b.input_kind == Some(puriy_engine::InputKind::Radio)
                        && b.input_name == my_name
                        && b.form_idx == my_form
                    {
                        if let Some(slot) = t.input_checks.get_mut(counter) {
                            *slot = counter == idx;
                        }
                    }
                    if b.input_kind.is_some() {
                        counter += 1;
                    }
                });
            }
            Msg::SubmitForm(idx) => {
                // Tratamos como si el input idx estuviera focado.
                m.active_mut().focused_input = Some(idx);
                if let Some(msg) = build_form_submit_url(&m) {
                    return Self::update(m, msg, handle);
                }
            }
            Msg::FocusInput(idx) => {
                let t = m.active_mut();
                if idx == usize::MAX {
                    // sentinel = blur
                    t.focused_input = None;
                } else if idx < t.inputs.len() {
                    t.focused_input = Some(idx);
                    // Blur address bar para que las teclas no compitan.
                    t.addr_focused = false;
                }
            }
            Msg::FocusNext => {
                let t = m.active_mut();
                if !t.inputs.is_empty() {
                    let n = t.inputs.len();
                    let next = match t.focused_input {
                        Some(i) => (i + 1) % n,
                        None => 0,
                    };
                    t.focused_input = Some(next);
                }
            }
            Msg::FocusPrev => {
                let t = m.active_mut();
                if !t.inputs.is_empty() {
                    let n = t.inputs.len();
                    let prev = match t.focused_input {
                        Some(0) | None => n - 1,
                        Some(i) => i - 1,
                    };
                    t.focused_input = Some(prev);
                }
            }
            Msg::InputKey(e) => {
                if matches!(&e.key, Key::Named(NamedKey::Enter)) {
                    // Submit (GET o POST según el form method).
                    if let Some(submit_msg) = build_form_submit_url(&m) {
                        return Self::update(m, submit_msg, handle);
                    } else {
                        m.active_mut().status =
                            "↵ submit: el input no está dentro de un <form action> conocido".into();
                    }
                } else {
                    let t = m.active_mut();
                    if let Some(idx) = t.focused_input {
                        if let Some(input) = t.inputs.get_mut(idx) {
                            input.apply_key(&e);
                        }
                    }
                }
            }
        }
        m
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // Si algún `<select>` está abierto, mostramos su lista como un
        // overlay centrado. Sin layout positioning real (no sabemos
        // dónde quedó el header del select en pantalla), el centrado
        // es la opción menos sorprendente. Click en una opción cierra;
        // backdrop transparente cierra también.
        let t = model.active();
        let (sel_idx, sel_state) = t
            .selects
            .iter()
            .enumerate()
            .find(|(_, s)| s.open)?;
        // Busca el SelectInfo correspondiente en el box tree por DFS idx.
        let tree = t.box_tree.as_ref()?;
        let mut info: Option<puriy_engine::SelectInfo> = None;
        let mut counter = 0usize;
        tree.walk(|b| {
            if let Some(s) = &b.select {
                if counter == sel_idx {
                    info = Some(s.clone());
                }
                counter += 1;
            }
        });
        let info = info?;
        Some(select_overlay_view(sel_idx, sel_state.selected, info))
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let tabs_bar = tabs_bar(model);
        let header = header_bar(model.active(), model.zoom, model.hover_link.as_deref());
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
            Some(kind) => panel_view(
                kind,
                &model.panel_filter,
                model.active().source.as_deref(),
                model.zoom,
            ),
            None => viewport(model.active(), model.zoom, &query_lc, model.find_current),
        };

        let mut children: Vec<View<Msg>> = vec![tabs_bar, header];
        if model.find_active {
            children.push(find_bar(&model.find_input, find_count, model.find_current));
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
fn find_bar(input: &TextInputState, count: usize, current: usize) -> View<Msg> {
    let palette = TextInputPalette::default();
    // Siempre focado mientras está abierta — Ctrl+F fue la última acción
    // explícita del usuario, no tiene sentido que el input no acepte teclas.
    let entry = text_input_view(input, "buscar en página…", true, &palette, Msg::FindOpen);

    let count_label = if input.text().is_empty() {
        "(escribí algo · Enter avanza)".to_string()
    } else if count == 0 {
        "sin matches".to_string()
    } else if current > 0 && current <= count {
        format!("{current} de {count}")
    } else if count == 1 {
        "1 match · Enter".to_string()
    } else {
        format!("{count} matches · Enter")
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
/// el chrome corre sin profile (modo efímero) muestra un mensaje. El
/// filtro substring (case-insensitive) se aplica al title y url de cada
/// item; vacío = sin filtro.
fn panel_view(
    kind: PanelKind,
    filter: &TextInputState,
    source: Option<&str>,
    zoom: f32,
) -> View<Msg> {
    // Source: render directo, sin items / filtro relevante.
    if kind == PanelKind::Source {
        return source_panel(source, zoom);
    }
    let (title, all_items) = match kind {
        PanelKind::Bookmarks => collect_bookmarks(),
        PanelKind::History => collect_history(),
        PanelKind::Source => unreachable!(),
    };
    let q = filter.text();
    let q_lc = q.to_lowercase();
    let items: Vec<PanelItem> = if q_lc.is_empty() {
        all_items
    } else {
        all_items
            .into_iter()
            .filter(|it| {
                it.title.to_lowercase().contains(&q_lc)
                    || it.url.to_lowercase().contains(&q_lc)
            })
            .collect()
    };
    let title = if q_lc.is_empty() {
        title
    } else {
        format!("{title} · filtrado: {} items", items.len())
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
            PanelKind::Source => unreachable!(),
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

    let palette = TextInputPalette::default();
    let placeholder = match kind {
        PanelKind::Bookmarks => "filtrar bookmarks por title o url…",
        PanelKind::History => "filtrar historial por title o url…",
        PanelKind::Source => unreachable!(),
    };
    let filter_input = text_input_view(filter, placeholder, true, &palette, Msg::ClosePanel);
    let filter_row = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(32.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(45, 45, 55))
    .children(vec![View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        ..Default::default()
    })
    .children(vec![filter_input])]);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![header, filter_row, body])
}

/// Panel "Page Source" — muestra el HTML crudo de la pestaña activa.
/// Línea por línea, prefijada por número (1-based, 4 dígitos). Mono
/// tamaño (12px × zoom), color foreground gris claro sobre fondo
/// oscuro estilo terminal. Sin scroll por ahora — Llimphi clipea
/// vertical; el usuario ve las primeras líneas.
fn source_panel(source: Option<&str>, zoom: f32) -> View<Msg> {
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
        .text_aligned(
            "Page Source · Ctrl+U cierra · Esc también".to_string(),
            13.0,
            Color::from_rgb8(230, 230, 240),
            Alignment::Start,
        ),
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

    let lines: Vec<View<Msg>> = match source {
        Some(src) if !src.is_empty() => src
            .lines()
            .enumerate()
            .take(2000) // cap protección — sources gigantes no destruyen el frame
            .map(|(i, line)| source_line_view(i + 1, line, zoom))
            .collect(),
        Some(_) => vec![source_empty_row("(la respuesta no tenía cuerpo)")],
        None => vec![source_empty_row("(la pestaña todavía no cargó)")],
    };

    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::from_rgb8(24, 24, 30))
    .clip(true)
    .children(lines);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![header, body])
}

fn source_line_view(num: usize, text: &str, zoom: f32) -> View<Msg> {
    let line_h = 16.0_f32 * zoom;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(line_h) },
        flex_direction: FlexDirection::Row,
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size { width: length(48.0_f32 * zoom), height: length(line_h) },
            margin: Rect {
                left: length(0.0_f32),
                right: length(8.0_f32 * zoom),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            format!("{num:>4}"),
            11.0 * zoom,
            Color::from_rgb8(110, 110, 130),
            Alignment::End,
        ),
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(line_h) },
            ..Default::default()
        })
        .text_aligned(
            text.to_string(),
            12.0 * zoom,
            Color::from_rgb8(220, 220, 230),
            Alignment::Start,
        ),
    ])
}

fn source_empty_row(msg: &str) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(msg.to_string(), 12.0, Color::from_rgb8(140, 140, 150), Alignment::Start)
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
    // El referer es la URL desde la que se navega — útil para que el
    // server sepa de dónde viene el click. Capturado ANTES de pisar t.url.
    let referer = if t.url == NEW_TAB_URL || t.url.is_empty() { None } else { Some(t.url.clone()) };
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
    spawn_load(id, gen, url, referer, handle.clone());
}

fn spawn_load(tab: TabId, gen: u64, url: String, referer: Option<String>, handle: Handle<Msg>) {
    if url == NEW_TAB_URL {
        // No fetch para about:blank.
        return;
    }
    std::thread::spawn(move || {
        let engine = Engine::new();
        match engine.load_with_referer(&url, referer.as_deref()) {
            Ok(doc) => {
                let title = if doc.title.is_empty() { doc.url.clone() } else { doc.title.clone() };
                handle.dispatch(Msg::Loaded {
                    tab,
                    gen,
                    final_url: doc.url.clone(),
                    title,
                    box_tree: doc.box_tree,
                    source: doc.source,
                    meta_refresh: doc.meta_refresh,
                });
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

fn start_load_post(m: &mut Model, url: String, body: String, handle: &Handle<Msg>) {
    let t = m.active_mut();
    let referer = if t.url == NEW_TAB_URL || t.url.is_empty() { None } else { Some(t.url.clone()) };
    t.url = url.clone();
    t.addr.set_text(url.clone());
    t.addr_focused = false;
    t.status = format!("POST {url}…");
    t.scroll_y = 0.0;
    t.box_tree = None;
    t.history.truncate(t.cursor + 1);
    if t.history.last() != Some(&url) {
        t.history.push(url.clone());
        t.cursor = t.history.len() - 1;
    }
    t.gen = t.gen.wrapping_add(1);
    let (id, gen) = (t.id, t.gen);
    let h = handle.clone();
    std::thread::spawn(move || {
        let engine = Engine::new();
        match engine.load_post_with_referer(&url, &body, referer.as_deref()) {
            Ok(doc) => {
                let title = if doc.title.is_empty() { doc.url.clone() } else { doc.title.clone() };
                h.dispatch(Msg::Loaded {
                    tab: id,
                    gen,
                    final_url: doc.url.clone(),
                    title,
                    box_tree: doc.box_tree,
                    source: doc.source,
                    meta_refresh: doc.meta_refresh,
                });
            }
            Err(e) => h.dispatch(Msg::LoadFailed { tab: id, gen, err: e.to_string() }),
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

fn header_bar(t: &TabState, zoom: f32, hover_link: Option<&str>) -> View<Msg> {
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
    // Si el cursor está sobre un link, el preview de la URL reemplaza
    // la línea de status normal (estilo browser tradicional).
    let status_line = if let Some(href) = hover_link {
        format!("→ {}", truncate(href, 220))
    } else {
        format!(
            "{}    ·    status: {}{}    ·    [Ctrl+T/W/Tab · Alt+←/→ · F5 · Ctrl+= / Ctrl+- / Ctrl+0 zoom · Ctrl+F buscar · Ctrl+B bookmarks · Ctrl+H historial · Ctrl+U source]",
            title_line, t.status, zoom_tag,
        )
    };

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

/// Estado por-frame que el render walk hila por toda la jerarquía. Lo
/// agrupamos en un struct para que `render_box`/`render_link_subtree`
/// no tengan 10 params; los `*_counter` se mutan por referencia.
struct RenderCtx<'a> {
    zoom: f32,
    find_query_lc: &'a str,
    find_current: usize,
    find_counter: usize,
    details_open: &'a [bool],
    details_counter: usize,
    inputs: &'a [TextInputState],
    input_checks: &'a [bool],
    focused_input: Option<usize>,
    input_counter: usize,
    selects: &'a [SelectState],
    select_counter: usize,
}

fn viewport(t: &TabState, zoom: f32, find_query_lc: &str, find_current: usize) -> View<Msg> {
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
    let mut ctx = RenderCtx {
        zoom,
        find_query_lc,
        find_current,
        find_counter: 0,
        details_open: &t.details_open,
        details_counter: 0,
        inputs: &t.inputs,
        input_checks: &t.input_checks,
        focused_input: t.focused_input,
        input_counter: 0,
        selects: &t.selects,
        select_counter: 0,
    };
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
    .children(vec![render_box(&tree.root, &mut ctx)]);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(Color::WHITE)
    .clip(true)
    .children(vec![content])
}

fn render_box(b: &BoxNode, ctx: &mut RenderCtx<'_>) -> View<Msg> {
    let zoom = ctx.zoom;
    // <input>/<textarea>: reservar slot y devolver un text_input_view
    // independiente del flujo normal.
    if let Some(kind) = b.input_kind {
        let my_idx = ctx.input_counter;
        ctx.input_counter += 1;
        return render_input(b, kind, my_idx, ctx);
    }
    // <select>: reservar slot y devolver el dropdown (header + opciones).
    if let Some(info) = &b.select {
        let my_idx = ctx.select_counter;
        ctx.select_counter += 1;
        return render_select(b, info, my_idx, ctx);
    }
    // <svg>: bypass del flujo normal — pinta primitivas con vello.
    if let Some(scene) = &b.svg {
        return render_svg(scene, zoom);
    }
    let style = box_style(b, zoom);
    let mut view = View::new(style);
    // Si este nodo es un <details>, reservamos su slot de estado y
    // renderizamos sólo `<summary>` (precedido de la flecha clickeable)
    // si está cerrado. La rama de `<details>` retorna acá para no caer
    // en el flujo normal de children.
    if b.tag.as_deref() == Some("details") {
        let my_idx = ctx.details_counter;
        ctx.details_counter += 1;
        let open = ctx.details_open.get(my_idx).copied().unwrap_or(false);
        let mut kids: Vec<View<Msg>> = Vec::new();
        for child in &b.children {
            let is_summary = child.tag.as_deref() == Some("summary");
            if is_summary {
                let arrow = if open { "▼ " } else { "▶ " };
                let arrow_view = View::new(Style {
                    size: Size {
                        width: length(16.0_f32 * zoom),
                        height: length(child.font_size * zoom * 1.2),
                    },
                    margin: Rect {
                        left: length(0.0_f32),
                        right: length(2.0_f32 * zoom),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    ..Default::default()
                })
                .text_aligned(
                    arrow.to_string(),
                    child.font_size * zoom,
                    Color::from_rgb8(80, 80, 95),
                    Alignment::Start,
                )
                .on_click(Msg::ToggleDetails(my_idx));
                let summary_view = render_box(child, ctx).on_click(Msg::ToggleDetails(my_idx));
                kids.push(
                    View::new(Style {
                        flex_direction: FlexDirection::Row,
                        align_items: Some(AlignItems::Center),
                        size: Size { width: percent(1.0_f32), height: auto() },
                        ..Default::default()
                    })
                    // Hover feedback sobre toda la fila (flecha + summary)
                    // para que sea evidente que es clickeable. El CSS no
                    // suele estilar `<summary>:hover`, así que es nuestra
                    // contribución de chrome — un gris muy suave.
                    .hover_fill(Color::from_rgba8(0, 0, 0, 18))
                    .on_click(Msg::ToggleDetails(my_idx))
                    .children(vec![arrow_view, summary_view]),
                );
            } else if open {
                kids.push(render_box(child, ctx));
            } else {
                // Cerrado y no-summary: no renderizamos, pero sí
                // avanzamos el counter por cada `<details>` anidado
                // adentro para no desalinear los índices con el vector
                // `details_open` que el Loaded prefilló en orden DFS
                // completo. Sin esto, abrir un parent cerrado le daría
                // a sus hijos índices que el state vector pensaba que
                // correspondían a `<details>` posteriores.
                skip_count_details(child, &mut ctx.details_counter);
            }
        }
        return view.children(kids);
    }
    // Find-in-page: si la query no es vacía y este nodo es una hoja de
    // texto que la contiene (case-insensitive), pintamos su background
    // con un highlight. El N-ésimo match en orden DFS es el "actual"
    // (find_current, 1-based) y pinta en naranja para destacarse —
    // el resto en amarillo. El paint del fill normal del nodo
    // (background CSS) se sobrescribe si hay match.
    let find_hit = !ctx.find_query_lc.is_empty()
        && b.text
            .as_ref()
            .map(|s| s.to_lowercase().contains(ctx.find_query_lc))
            .unwrap_or(false);
    let find_hit_color: Option<Color> = if find_hit {
        ctx.find_counter += 1;
        let is_current = ctx.find_current != 0 && ctx.find_counter == ctx.find_current;
        Some(if is_current {
            Color::from_rgba8(255, 140, 0, 240)
        } else {
            Color::from_rgba8(255, 230, 0, 200)
        })
    } else {
        None
    };

    // visibility:hidden ocupa espacio pero no pinta. Devolvemos la view
    // con su layout pero sin children/text/fill — sus descendientes
    // serían computados pero también deberían ser hidden por inheritance.
    let hidden = matches!(b.visibility, Visibility::Hidden);

    // opacity multiplica el alpha del background sólido. text/border
    // se manejan en apply_decorations/render del texto.
    let alpha_mul = b.opacity.clamp(0.0, 1.0);

    if !hidden {
        if let Some(c) = find_hit_color {
            view = view.fill(c);
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
            // `<a download>` descarga el target en lugar de navegar. El
            // filename hint queda en `b.link_download` (String vacío =
            // usar nombre del path).
            let msg = if let Some(filename_hint) = &b.link_download {
                Msg::DownloadLink {
                    url: target.clone(),
                    filename_hint: filename_hint.clone(),
                }
            } else if b.link_new_tab {
                Msg::NavigateNewTab(target.clone())
            } else {
                Msg::Navigate(target.clone())
            };
            view = view
                .on_click(msg)
                .on_middle_click(Msg::NavigateNewTab(target.clone()))
                .on_pointer_enter(Msg::HoverLink(Some(target.clone())))
                .on_pointer_leave(Msg::HoverLink(None));
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
        let italic = matches!(b.font_style, puriy_engine::FontStyle::Italic);
        return view.text_aligned_full(
            text.clone(),
            size,
            display_color,
            Alignment::Start,
            italic,
            b.font_family.clone(),
        );
    }

    if !b.children.is_empty() {
        let kids: Vec<View<Msg>> = if let Some(target) = &b.link {
            // Dentro de un <a>, los descendientes son no-interactive por
            // contagio (ya enlazan al target del <a>). No esperamos
            // <details> dentro de links — pero contamos por las dudas
            // para no romper el invariante del counter.
            let target = target.clone();
            let new_tab = b.link_new_tab;
            b.children
                .iter()
                .map(|c| render_link_subtree(c, &target, link_color, new_tab, ctx))
                .collect()
        } else {
            render_children_z_ordered(&b.children, ctx)
        };
        view = view.children(kids);
    }
    view
}

/// Renderea los children aplicando z-index: in-flow primero (orden
/// DOM), luego out-of-flow (position absolute/fixed) ordenados por
/// z-index ascendente — mayor pinta encima de los demás. Reordenar
/// los out-of-flow es seguro porque su layout depende de insets, no
/// de su posición en el Vec.
fn render_children_z_ordered(children: &[BoxNode], ctx: &mut RenderCtx<'_>) -> Vec<View<Msg>> {
    let mut in_flow_idx: Vec<usize> = Vec::new();
    let mut out_of_flow_idx: Vec<usize> = Vec::new();
    for (i, c) in children.iter().enumerate() {
        match c.position {
            puriy_engine::Position::Absolute | puriy_engine::Position::Fixed => {
                out_of_flow_idx.push(i)
            }
            _ => in_flow_idx.push(i),
        }
    }
    // Sort estable por z-index ascending; ties mantienen orden DOM.
    out_of_flow_idx.sort_by_key(|&i| children[i].z_index);
    in_flow_idx
        .into_iter()
        .chain(out_of_flow_idx)
        .map(|i| render_box(&children[i], ctx))
        .collect()
}

/// Recorre `b` y avanza `*counter` por cada `<details>` descendiente.
/// Usado por el chrome cuando un `<details>` padre está cerrado: aunque
/// no rendereamos los hijos non-summary, sí tenemos que consumir sus
/// índices para que no se desalineen con el vector `details_open` que
/// el Loaded prefilló en DFS completo.
/// Si el input focado está dentro de un `<form>`, arma la URL `action?
/// n1=v1&n2=v2&…` con los inputs que tienen `name` no vacío,
/// urlencodeados de manera mínima. Devuelve `None` si no hay form
/// asociado o si el form no tiene action navegable.
fn build_form_submit_url(m: &Model) -> Option<Msg> {
    let t = m.active();
    let focused_idx = t.focused_input?;
    let tree = t.box_tree.as_ref()?;
    // Primer pase: identificá el form_idx del input focado.
    let mut focused_form: Option<usize> = None;
    let mut counter: usize = 0;
    tree.walk(|b| {
        if b.input_kind.is_some() {
            if counter == focused_idx {
                focused_form = b.form_idx;
            }
            counter += 1;
        }
    });
    let form_idx = focused_form?;
    // Segundo pase: junta los pares (name, value) de los inputs y
    // `<select>`s del mismo form que tengan `name`. Texto del input vive
    // en `t.inputs[idx]`; valor del select en `t.selects[idx].selected`
    // → SelectInfo.options[i].value.
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut input_idx: usize = 0;
    let mut select_idx: usize = 0;
    tree.walk(|b| {
        if let Some(kind) = b.input_kind {
            let my_idx = input_idx;
            input_idx += 1;
            if b.form_idx == Some(form_idx) {
                if let Some(name) = &b.input_name {
                    match kind {
                        puriy_engine::InputKind::Checkbox
                        | puriy_engine::InputKind::Radio => {
                            let checked = t.input_checks.get(my_idx).copied().unwrap_or(false);
                            if checked {
                                let val = b
                                    .input_initial
                                    .clone()
                                    .unwrap_or_else(|| "on".to_string());
                                pairs.push((name.clone(), val));
                            }
                            // No-checked checkbox/radio: NO se manda
                            // (HTML spec).
                        }
                        puriy_engine::InputKind::Submit => {
                            // Submit con name: contribuye su `value`/label.
                            let val = b
                                .input_initial
                                .clone()
                                .unwrap_or_else(|| "Submit".to_string());
                            pairs.push((name.clone(), val));
                        }
                        _ => {
                            let value = t
                                .inputs
                                .get(my_idx)
                                .map(|s| s.text())
                                .unwrap_or_default();
                            pairs.push((name.clone(), value));
                        }
                    }
                }
            }
        }
        if let Some(info) = &b.select {
            let my_idx = select_idx;
            select_idx += 1;
            if b.form_idx == Some(form_idx) {
                if let Some(name) = &b.input_name {
                    let sel = t
                        .selects
                        .get(my_idx)
                        .map(|s| s.selected)
                        .unwrap_or(info.initial);
                    let value = info
                        .options
                        .get(sel)
                        .map(|o| o.value.clone())
                        .unwrap_or_default();
                    pairs.push((name.clone(), value));
                }
            }
        }
    });
    let form = tree.forms.get(form_idx)?;
    let action = form.action.clone()?;
    // URL-encoder mínimo (espacios → '+', resto de chars unsafe → %HH).
    fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for &b in s.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char);
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    }
    let qs: Vec<String> = pairs.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect();
    let body = qs.join("&");
    match form.method {
        puriy_engine::FormMethod::Get => {
            // Concatena action con `?…`. Si action ya tiene `?`, usamos `&`.
            let sep = if action.contains('?') { '&' } else { '?' };
            Some(Msg::Navigate(format!("{}{}{}", action, sep, body)))
        }
        puriy_engine::FormMethod::Post => Some(Msg::NavigatePost { url: action, body }),
    }
}

fn skip_count_details(b: &BoxNode, counter: &mut usize) {
    if b.tag.as_deref() == Some("details") {
        *counter += 1;
    }
    for c in &b.children {
        skip_count_details(c, counter);
    }
}

/// View dimensionada para una imagen — ancho hasta `width_px` pero
/// nunca más que el contenedor (`max_width: 100%`), altura proporcional
/// vía aspect ratio inverso (`width / height`).
fn image_view(width: u32, height: u32, zoom: f32) -> View<Msg> {
    let w = (width.max(1)) as f32 * zoom;
    let h = (height.max(1)) as f32 * zoom;
    let ratio = if height > 0 { Some(width as f32 / height as f32) } else { None };
    View::new(Style {
        size: Size { width: length(w), height: length(h) },
        // `max-width: 100%` clampa el ancho al contenedor (responsive
        // por default — sin esto, imágenes grandes rompen layouts narrow);
        // `aspect_ratio` deja que taffy preserve la proporción cuando el
        // ancho real (post-clamp) sea menor que `length(w)`.
        max_size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        aspect_ratio: ratio,
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
    new_tab: bool,
    ctx: &mut RenderCtx<'_>,
) -> View<Msg> {
    let zoom = ctx.zoom;
    // <details> dentro de un <a> es HTML inválido en la práctica; pero
    // si aparece, contamos el slot igualmente para no desalinear el
    // counter global. No reescribimos el comportamiento interactivo:
    // dentro de un link el subtree colapsado se ignora.
    if b.tag.as_deref() == Some("details") {
        skip_count_details(b, &mut ctx.details_counter);
    }
    let nav_msg = |t: &str| {
        if new_tab {
            Msg::NavigateNewTab(t.to_string())
        } else {
            Msg::Navigate(t.to_string())
        }
    };
    let mut view = View::new(box_style(b, zoom))
        .on_click(nav_msg(target))
        .on_middle_click(Msg::NavigateNewTab(target.to_string()));
    let find_hit = !ctx.find_query_lc.is_empty()
        && b.text
            .as_ref()
            .map(|s| s.to_lowercase().contains(ctx.find_query_lc))
            .unwrap_or(false);
    let find_hit_color: Option<Color> = if find_hit {
        ctx.find_counter += 1;
        let is_current = ctx.find_current != 0 && ctx.find_counter == ctx.find_current;
        Some(if is_current {
            Color::from_rgba8(255, 140, 0, 240)
        } else {
            Color::from_rgba8(255, 230, 0, 200)
        })
    } else {
        None
    };
    if let Some(c) = find_hit_color {
        view = view.fill(c);
    } else if let Some(bg) = b.background {
        view = view.fill(Color::from_rgb8(bg.r, bg.g, bg.b));
    }
    if let Some(img) = &b.image {
        let blob = Blob::from(img.rgba.clone());
        let peniko = PenikoImage::new(blob, ImageFormat::Rgba8, img.width, img.height);
        return image_view(img.width, img.height, zoom)
            .image(peniko)
            .on_click(nav_msg(target));
    }
    if let Some(text) = &b.text {
        let base = if b.font_weight >= 600 { b.font_size * 1.1 } else { b.font_size };
        let size = base * zoom;
        let italic = matches!(b.font_style, puriy_engine::FontStyle::Italic);
        return view.text_aligned_full(
            text.clone(),
            size,
            color,
            Alignment::Start,
            italic,
            b.font_family.clone(),
        );
    }
    if !b.children.is_empty() {
        let target_owned = target.to_string();
        view = view.children(
            b.children
                .iter()
                .map(|c| render_link_subtree(c, &target_owned, color, new_tab, ctx))
                .collect(),
        );
    }
    view
}

/// `<input type=text>` / `<input type=search>` / `<input type=password>` /
/// `<textarea>`: arma un widget `text_input_view` ligado al
/// `TextInputState` del slot DFS `idx`. Click→focus dispara
/// `Msg::FocusInput(idx)`. El estilo del input se mantiene básico
/// (border gris claro + padding); el font-size hereda del nodo. Sin
/// soporte de submit/Enter por ahora — Enter en un input single-line
/// no hace nada (en un textarea inserta newline via apply_key).
fn render_input(
    b: &BoxNode,
    kind: puriy_engine::InputKind,
    idx: usize,
    ctx: &mut RenderCtx<'_>,
) -> View<Msg> {
    let zoom = ctx.zoom;
    // Checkbox / radio / submit: widgets a parte (no text_input_view).
    match kind {
        puriy_engine::InputKind::Checkbox => {
            return render_checkbox_radio(b, idx, ctx, /* radio */ false);
        }
        puriy_engine::InputKind::Radio => {
            return render_checkbox_radio(b, idx, ctx, /* radio */ true);
        }
        puriy_engine::InputKind::Submit => {
            return render_submit_button(b, idx, ctx);
        }
        _ => {}
    }
    let focused = ctx.focused_input == Some(idx);
    // Estado por slot — usamos un blank si todavía no hay (no debería
    // pasar tras Loaded, pero defensivo).
    let blank = TextInputState::new();
    let state = ctx.inputs.get(idx).unwrap_or(&blank);

    let placeholder = b
        .input_placeholder
        .as_deref()
        .unwrap_or(match kind {
            puriy_engine::InputKind::Search => "buscar…",
            puriy_engine::InputKind::Password => "contraseña",
            puriy_engine::InputKind::TextArea => "",
            _ => "",
        });

    let palette = TextInputPalette::default();
    let input = text_input_view(state, placeholder, focused, &palette, Msg::FocusInput(idx));

    // Tamaño: ancho 100% del contenedor por default (los autores suelen
    // poner `width: 200px` o similar; el CSS engine ya lo materializa
    // como `b.width`). El alto: una línea para text/search/password, un
    // textarea recibe ~5 líneas.
    let line_h = (b.font_size * zoom).max(14.0_f32 * zoom) + 12.0;
    let height = match kind {
        puriy_engine::InputKind::TextArea => line_h * 5.0,
        _ => line_h,
    };
    let css_width = length_to_taffy(b.width, zoom);

    // Background base: CSS background-color del nodo si lo seteó; sino
    // blanco. Cuando está focado y el autor escribió `:focus { background:
    // X }`, aplicamos X.
    let base_bg = b
        .background
        .map(|c| Color::from_rgba8(c.r, c.g, c.b, c.a))
        .unwrap_or(Color::WHITE);
    let bg = if focused {
        b.focus_background
            .map(|c| Color::from_rgba8(c.r, c.g, c.b, c.a))
            .unwrap_or(base_bg)
    } else {
        base_bg
    };
    let outline = b
        .outline
        .color
        .filter(|_| focused && b.outline.style_active && b.outline.width > 0.0);

    let mut wrapper = View::new(Style {
        size: Size {
            width: css_width.unwrap_or_else(|| length(220.0_f32 * zoom)),
            height: length(height),
        },
        padding: Rect {
            left: length(6.0_f32 * zoom),
            right: length(6.0_f32 * zoom),
            top: length(4.0_f32 * zoom),
            bottom: length(4.0_f32 * zoom),
        },
        margin: Rect {
            left: length(b.margin.left * zoom),
            right: length(b.margin.right * zoom),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .on_click(Msg::FocusInput(idx));
    // Ring de focus visible: si el autor no proveyó outline, lo damos
    // gratis para feedback. Stroke azul accent estándar.
    if focused && outline.is_none() {
        wrapper = wrapper.paint_with(|scene, _ts, rect| {
            let stroke = Stroke::new(2.0);
            let half = stroke.width * 0.5;
            let r = RoundedRect::new(
                rect.x as f64 - half,
                rect.y as f64 - half,
                (rect.x + rect.w) as f64 + half,
                (rect.y + rect.h) as f64 + half,
                3.0 + half,
            );
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                Color::from_rgba8(40, 110, 220, 255),
                None,
                &r,
            );
        });
    }
    wrapper.children(vec![input])
}

/// `<input type=checkbox|radio>`: caja chica con `☐`/`☑` (o circle
/// vacío/lleno para radio) clickeable. Sin label asociada — el `<label
/// for="...">` no se cablea todavía, pero el click sobre el widget
/// alcanza para toggle.
fn render_checkbox_radio(
    b: &BoxNode,
    idx: usize,
    ctx: &mut RenderCtx<'_>,
    radio: bool,
) -> View<Msg> {
    let zoom = ctx.zoom;
    let checked = ctx.input_checks.get(idx).copied().unwrap_or(false);
    let glyph = if radio {
        if checked { "●" } else { "○" }
    } else if checked {
        "☑"
    } else {
        "☐"
    };
    let msg = if radio { Msg::SelectRadio(idx) } else { Msg::ToggleCheckbox(idx) };
    let size_px = (b.font_size * zoom).max(14.0 * zoom);
    View::new(Style {
        size: Size {
            width: length(size_px + 4.0),
            height: length(size_px + 4.0),
        },
        margin: Rect {
            left: length(b.margin.left * zoom),
            right: length(b.margin.right * zoom + 4.0),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .on_click(msg)
    .text_aligned(
        glyph.to_string(),
        size_px,
        Color::from_rgb8(40, 40, 50),
        Alignment::Center,
    )
}

/// `<input type=submit|button>` — botón con label desde `value` o
/// default `Submit`. Click submitea el form.
fn render_submit_button(b: &BoxNode, idx: usize, ctx: &mut RenderCtx<'_>) -> View<Msg> {
    let zoom = ctx.zoom;
    let label = b
        .input_initial
        .clone()
        .unwrap_or_else(|| "Submit".to_string());
    let css_width = length_to_taffy(b.width, zoom);
    let h = (b.font_size * zoom).max(14.0 * zoom) + 12.0;
    View::new(Style {
        size: Size {
            width: css_width.unwrap_or_else(|| length(120.0 * zoom)),
            height: length(h),
        },
        padding: Rect {
            left: length(10.0 * zoom),
            right: length(10.0 * zoom),
            top: length(6.0 * zoom),
            bottom: length(6.0 * zoom),
        },
        margin: Rect {
            left: length(b.margin.left * zoom),
            right: length(b.margin.right * zoom),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(230, 230, 240))
    .hover_fill(Color::from_rgb8(215, 220, 235))
    .radius(3.0)
    .on_click(Msg::SubmitForm(idx))
    .text_aligned(
        label,
        b.font_size * zoom,
        Color::from_rgb8(30, 30, 40),
        Alignment::Center,
    )
}

/// `<select>` con `<option>`s: renderea un header click-toggle con la
/// opción elegida + flecha; cuando está abierto, expande la lista
/// debajo. Click en una opción la selecciona y cierra el dropdown.
fn render_select(
    b: &BoxNode,
    info: &puriy_engine::SelectInfo,
    idx: usize,
    ctx: &mut RenderCtx<'_>,
) -> View<Msg> {
    let zoom = ctx.zoom;
    let state = ctx.selects.get(idx);
    let selected = state.map(|s| s.selected).unwrap_or(info.initial);
    let open = state.map(|s| s.open).unwrap_or(false);
    let current_label = info
        .options
        .get(selected)
        .map(|o| o.label.clone())
        .unwrap_or_default();

    let css_width = length_to_taffy(b.width, zoom);
    let header_h = (b.font_size * zoom).max(14.0_f32 * zoom) + 10.0;
    let header = View::new(Style {
        size: Size {
            width: css_width.clone().unwrap_or_else(|| length(220.0_f32 * zoom)),
            height: length(header_h),
        },
        padding: Rect {
            left: length(8.0_f32 * zoom),
            right: length(8.0_f32 * zoom),
            top: length(4.0_f32 * zoom),
            bottom: length(4.0_f32 * zoom),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::WHITE)
    .radius(3.0)
    .on_click(Msg::SelectToggle(idx))
    .children(vec![
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(header_h - 8.0) },
            ..Default::default()
        })
        .text_aligned(
            truncate(&current_label, 80),
            b.font_size * zoom,
            Color::from_rgb8(30, 30, 40),
            Alignment::Start,
        ),
        View::new(Style {
            size: Size {
                width: length(14.0_f32 * zoom),
                height: length(header_h - 8.0),
            },
            ..Default::default()
        })
        .text_aligned(
            if open { "▲".to_string() } else { "▼".to_string() },
            b.font_size * zoom * 0.8,
            Color::from_rgb8(80, 80, 95),
            Alignment::End,
        ),
    ]);

    // El header se rendera siempre; la lista expandida ahora vive en
    // `view_overlay` (popup flotante) cuando `open=true`. Esto evita
    // empujar el flow del documento al abrir un select.
    let _ = (selected, info, open); // ya consumidos en el overlay
    let all: Vec<View<Msg>> = vec![header];

    View::new(Style {
        size: Size {
            width: css_width.unwrap_or_else(|| length(220.0_f32 * zoom)),
            height: auto(),
        },
        margin: Rect {
            left: length(b.margin.left * zoom),
            right: length(b.margin.right * zoom),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::from_rgb8(220, 220, 230))
    .radius(3.0)
    .children(all)
}

/// Overlay flotante con la lista de opciones del `<select>` abierto.
/// Centrado en la ventana; backdrop semitransparente que cierra el
/// dropdown al clickear fuera de la lista.
fn select_overlay_view(idx: usize, selected: usize, info: puriy_engine::SelectInfo) -> View<Msg> {
    let row_h = 28.0_f32;
    let total_h = (info.options.len() as f32 * row_h).min(360.0);
    let rows: Vec<View<Msg>> = info
        .options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            let is_sel = i == selected;
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(row_h) },
                padding: Rect {
                    left: length(12.0_f32),
                    right: length(12.0_f32),
                    top: length(4.0_f32),
                    bottom: length(4.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(if is_sel {
                Color::from_rgb8(220, 230, 250)
            } else {
                Color::WHITE
            })
            .hover_fill(Color::from_rgb8(238, 240, 248))
            .on_click(Msg::SelectPick(idx, i))
            .text_aligned(
                truncate(&opt.label, 80),
                13.0,
                Color::from_rgb8(30, 30, 40),
                Alignment::Start,
            )
        })
        .collect();

    let list = View::new(Style {
        size: Size { width: length(320.0_f32), height: length(total_h) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::from_rgb8(245, 245, 250))
    .radius(4.0)
    .clip(true)
    .children(rows);

    // Backdrop fullscreen con flex centering del list. Click en el
    // backdrop cierra el dropdown via SelectToggle.
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 60))
    .on_click(Msg::SelectToggle(idx))
    .children(vec![list])
}

/// Pinta las primitivas de un `<svg>` dentro de un rect del tamaño
/// `scene.width × scene.height` (escalado por zoom). Si `view_box` está
/// definido, las primitivas se mapean a [0..1] vía viewBox y luego se
/// escalan al rect del nodo (preservando aspect ratio, "meet").
fn render_svg(scene: &puriy_engine::SvgScene, zoom: f32) -> View<Msg> {
    use llimphi_raster::kurbo::{Circle as KurboCircle, Line as KurboLine};
    let w = scene.width * zoom;
    let h = scene.height * zoom;
    let prims = scene.prims.clone();
    let view_box = scene.view_box;
    let svg_w = scene.width;
    let svg_h = scene.height;
    View::new(Style {
        size: Size { width: length(w), height: length(h) },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        // Mapping local → pantalla. Si hay viewBox, normalizamos por él
        // y escalamos al rect; sino usamos directamente width/height del
        // svg como dominio.
        let (src_x, src_y, src_w, src_h) = view_box.unwrap_or((0.0, 0.0, svg_w, svg_h));
        let sx = if src_w > 0.0 { rect.w as f64 / src_w as f64 } else { 1.0 };
        let sy = if src_h > 0.0 { rect.h as f64 / src_h as f64 } else { 1.0 };
        let s = sx.min(sy).max(0.001);
        let to_x = |x: f32| rect.x as f64 + ((x - src_x) as f64) * s;
        let to_y = |y: f32| rect.y as f64 + ((y - src_y) as f64) * s;
        let to_color = |c: puriy_engine::Color| {
            Color::from_rgba8(c.r, c.g, c.b, c.a)
        };
        for p in &prims {
            match *p {
                puriy_engine::SvgPrim::Rect {
                    x, y, w, h, rx, fill, stroke, stroke_w,
                } => {
                    let r = RoundedRect::new(
                        to_x(x),
                        to_y(y),
                        to_x(x + w),
                        to_y(y + h),
                        (rx as f64) * s,
                    );
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &r);
                    }
                    if let Some(st) = stroke {
                        let stroke = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke, Affine::IDENTITY, to_color(st), None, &r);
                    }
                }
                puriy_engine::SvgPrim::Circle { cx, cy, r, fill, stroke, stroke_w } => {
                    let c = KurboCircle::new((to_x(cx), to_y(cy)), r as f64 * s);
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &c);
                    }
                    if let Some(st) = stroke {
                        let stroke = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke, Affine::IDENTITY, to_color(st), None, &c);
                    }
                }
                puriy_engine::SvgPrim::Line { x1, y1, x2, y2, stroke, stroke_w } => {
                    let l = KurboLine::new((to_x(x1), to_y(y1)), (to_x(x2), to_y(y2)));
                    let stroke_obj = Stroke::new(stroke_w as f64 * s);
                    scene.stroke(&stroke_obj, Affine::IDENTITY, to_color(stroke), None, &l);
                }
                puriy_engine::SvgPrim::Polyline {
                    ref points, closed, fill, stroke, stroke_w,
                } => {
                    use llimphi_raster::kurbo::{BezPath, PathEl, Point as KurboPoint};
                    let mut path = BezPath::new();
                    let mut iter = points.iter();
                    if let Some(&(x, y)) = iter.next() {
                        path.push(PathEl::MoveTo(KurboPoint::new(to_x(x), to_y(y))));
                        for &(x, y) in iter {
                            path.push(PathEl::LineTo(KurboPoint::new(to_x(x), to_y(y))));
                        }
                        if closed {
                            path.push(PathEl::ClosePath);
                        }
                    }
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &path);
                    }
                    if let Some(st) = stroke {
                        let stroke_obj = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke_obj, Affine::IDENTITY, to_color(st), None, &path);
                    }
                }
                puriy_engine::SvgPrim::Path { ref d, fill, stroke, stroke_w } => {
                    use llimphi_raster::kurbo::{BezPath, PathEl, Point as KurboPoint};
                    let mut path = BezPath::new();
                    for cmd in d {
                        match *cmd {
                            puriy_engine::PathCmd::MoveTo(x, y) => {
                                path.push(PathEl::MoveTo(KurboPoint::new(to_x(x), to_y(y))));
                            }
                            puriy_engine::PathCmd::LineTo(x, y) => {
                                path.push(PathEl::LineTo(KurboPoint::new(to_x(x), to_y(y))));
                            }
                            puriy_engine::PathCmd::CubicTo(x1, y1, x2, y2, x, y) => {
                                path.push(PathEl::CurveTo(
                                    KurboPoint::new(to_x(x1), to_y(y1)),
                                    KurboPoint::new(to_x(x2), to_y(y2)),
                                    KurboPoint::new(to_x(x), to_y(y)),
                                ));
                            }
                            puriy_engine::PathCmd::QuadTo(x1, y1, x, y) => {
                                path.push(PathEl::QuadTo(
                                    KurboPoint::new(to_x(x1), to_y(y1)),
                                    KurboPoint::new(to_x(x), to_y(y)),
                                ));
                            }
                            puriy_engine::PathCmd::ClosePath => {
                                path.push(PathEl::ClosePath);
                            }
                        }
                    }
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &path);
                    }
                    if let Some(st) = stroke {
                        let stroke_obj = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke_obj, Affine::IDENTITY, to_color(st), None, &path);
                    }
                }
            }
        }
    })
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
    // Radio del clip del view: usamos el máximo de las 4 esquinas (Llimphi
    // `View::radius` toma un escalar). Cuando las 4 esquinas son iguales
    // el resultado es exacto; cuando difieren, el clip queda con la
    // esquina más redonda — el border per-side dibujado abajo seguirá
    // marcando las corners individuales.
    let radii = b.border_radii;
    let radius_max =
        radii.top_left.max(radii.top_right).max(radii.bottom_right).max(radii.bottom_left);
    if radius_max > 0.0 {
        view = view.radius((radius_max * z) as f64);
    }
    let radius = (radius_max * z) as f64;
    let shadow = b.box_shadow.map(|s| BoxShadow {
        offset_x: s.offset_x * z,
        offset_y: s.offset_y * z,
        blur_px: s.blur_px * z,
        spread_px: s.spread_px * z,
        color: s.color,
    });
    let alpha_mul = b.opacity.clamp(0.0, 1.0);
    // Border uniforme = los 4 lados con mismo width y color. Lo
    // dibujamos como RoundedRect stroke para que las corners radius
    // queden suaves. Si los lados difieren, pintamos cada uno como
    // segmento independiente (Border::Sides) — las corners en ese caso
    // van en chaflán cuadrado, que matchea el look estándar de browsers
    // cuando se mezclan widths/colors por lado.
    let bw = b.border_widths;
    let bc = b.border_colors;
    let uniform_border = if bw.top == bw.right
        && bw.right == bw.bottom
        && bw.bottom == bw.left
        && bc.top == bc.right
        && bc.right == bc.bottom
        && bc.bottom == bc.left
        && bw.top > 0.0
    {
        bc.top.map(|c| (c, bw.top * z))
    } else {
        None
    };
    let per_side_border = if uniform_border.is_none() {
        let s_top = bc.top.filter(|_| bw.top > 0.0).map(|c| (c, bw.top * z));
        let s_right = bc.right.filter(|_| bw.right > 0.0).map(|c| (c, bw.right * z));
        let s_bottom = bc.bottom.filter(|_| bw.bottom > 0.0).map(|c| (c, bw.bottom * z));
        let s_left = bc.left.filter(|_| bw.left > 0.0).map(|c| (c, bw.left * z));
        if s_top.is_some() || s_right.is_some() || s_bottom.is_some() || s_left.is_some() {
            Some((s_top, s_right, s_bottom, s_left))
        } else {
            None
        }
    } else {
        None
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
    // `background-image: url(...)`: si el engine pudo descargarla, la
    // envolvemos en peniko::Image para que el closure de paint_with la
    // tile/escale dentro del rect. Por ahora un sólo modo: cover sin tile,
    // anclada arriba-izquierda y escalada a llenar el ancho del box.
    let bg_image = b.background_image.as_ref().map(|img| {
        let blob = Blob::from(img.rgba.clone());
        let peniko = PenikoImage::new(blob, ImageFormat::Rgba8, img.width, img.height);
        (peniko, img.width as f64, img.height as f64)
    });
    if shadow.is_none()
        && uniform_border.is_none()
        && per_side_border.is_none()
        && deco.is_none()
        && outline.is_none()
        && gradient.is_none()
        && bg_image.is_none()
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
        // background-image: escala la imagen a "cover" del rect (la
        // dimensión más chica al lado del rect, el sobrante se clipea).
        // Anchor: arriba-izquierda. Sin `background-size`/`-position`
        // CSS por ahora — alcanza para hero images simples.
        if let Some((img, iw, ih)) = &bg_image {
            if *iw > 0.0 && *ih > 0.0 {
                let sx = rect.w as f64 / *iw;
                let sy = rect.h as f64 / *ih;
                let s = sx.max(sy);
                let transform = Affine::translate((rect.x as f64, rect.y as f64))
                    * Affine::scale(s);
                scene.draw_image(img, transform);
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
        if let Some((bc, w)) = uniform_border {
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
        if let Some((s_top, s_right, s_bottom, s_left)) = per_side_border {
            // Per-side: pintamos cada lado como una línea recta del color
            // y grosor correspondientes. Corners en chaflán cuadrado —
            // matchea el look de browsers cuando border-{top,right,...}
            // difieren entre sí.
            let x0 = rect.x as f64;
            let y0 = rect.y as f64;
            let x1 = x0 + rect.w as f64;
            let y1 = y0 + rect.h as f64;
            // Cada lado se inseta por w/2 para que el trazo caiga dentro
            // del rect del nodo (vello pinta centrado al path). Pintamos
            // inline (sin closure) para evitar capturas raras del scene.
            if let Some((c, w)) = s_top {
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x0, y0 + h), (x1, y0 + h));
                scene.stroke(&Stroke::new(w as f64), Affine::IDENTITY, color, None, &line);
            }
            if let Some((c, w)) = s_bottom {
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x0, y1 - h), (x1, y1 - h));
                scene.stroke(&Stroke::new(w as f64), Affine::IDENTITY, color, None, &line);
            }
            if let Some((c, w)) = s_left {
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x0 + h, y0), (x0 + h, y1));
                scene.stroke(&Stroke::new(w as f64), Affine::IDENTITY, color, None, &line);
            }
            if let Some((c, w)) = s_right {
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x1 - h, y0), (x1 - h, y1));
                scene.stroke(&Stroke::new(w as f64), Affine::IDENTITY, color, None, &line);
            }
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
    let lh_mult = b.line_height.unwrap_or(1.2);
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

/// Decide qué nombre usar para una descarga. El hint del attr
/// `download="..."` gana si no está vacío y no contiene `/` (un attr
/// `download="../etc/passwd"` debe rechazarse — es vector de path
/// traversal). Sino, usamos el último segmento del path de la URL; si
/// la URL no tiene path significativo, fallback a `descarga`.
fn pick_download_filename(url: &str, hint: &str) -> String {
    let hint = hint.trim();
    if !hint.is_empty() && !hint.contains('/') && !hint.contains('\\') {
        return hint.to_string();
    }
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(seg) = parsed.path_segments().and_then(|s| s.last()) {
            let seg = seg.trim();
            if !seg.is_empty() {
                return seg.to_string();
            }
        }
    }
    "descarga".to_string()
}

/// Path absoluto donde la descarga termina. Convención: `$XDG_DOWNLOAD_DIR/
/// puriy/<filename>` o, sin xdg, `~/Downloads/puriy/<filename>`. Si
/// ningún path conocido es accesible, cae a `/tmp/`.
fn download_path(filename: &str) -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DOWNLOAD_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Downloads"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join("puriy").join(filename)
}

/// Centra el viewport sobre el match actual de la find bar. Llama a
/// `find_y_of_match` del box tree con el contador 1-based; si encuentra
/// la y aproximada, setea `scroll_y` ~80px arriba del match para dar
/// contexto visual. No-op si no hay box tree o el match no se encuentra.
fn scroll_to_find_match(m: &mut Model, query_lower: &str) {
    if m.find_current == 0 {
        return;
    }
    let nth = m.find_current;
    let y = m
        .active()
        .box_tree
        .as_ref()
        .and_then(|bt| bt.find_y_of_match(query_lower, nth));
    if let Some(y) = y {
        let t = m.active_mut();
        t.scroll_y = (y - 80.0).max(0.0);
    }
}

/// Si `target` representa la misma URL que `current` excepto por el
/// fragment, devuelve el fragment (sin `#`). Si difieren en algo más
/// (scheme/host/path/query) o el target no tiene fragment, devuelve
/// `None` — el caller debe recargar normal.
fn same_doc_with_fragment(current: &str, target: &str) -> Option<String> {
    let cur = url::Url::parse(current).ok()?;
    // El target puede venir absoluto (`https://x/y#z`) o ya resuelto
    // contra base (`current_url#z`). Ambos casos los soporta Url::parse
    // directo porque el engine resuelve los `#x` puros antes de pasarlos.
    let tgt = url::Url::parse(target).ok()?;
    let frag = tgt.fragment()?;
    if frag.is_empty() {
        return None;
    }
    // Comparamos URLs sin fragment. Cheap: clonamos y limpiamos.
    let mut a = cur.clone();
    a.set_fragment(None);
    let mut b = tgt.clone();
    b.set_fragment(None);
    if a == b {
        Some(frag.to_string())
    } else {
        None
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
    fn pick_download_filename_usa_hint_si_es_seguro() {
        assert_eq!(
            pick_download_filename("https://x/y/z.pdf", "doc.pdf"),
            "doc.pdf"
        );
        // Path traversal en el hint → cae a path de la URL.
        assert_eq!(
            pick_download_filename("https://x/y/z.pdf", "../etc/passwd"),
            "z.pdf"
        );
        assert_eq!(
            pick_download_filename("https://x/y/z.pdf", "a\\b"),
            "z.pdf"
        );
        // Hint vacío → path de la URL.
        assert_eq!(
            pick_download_filename("https://x/file.tar.gz", ""),
            "file.tar.gz"
        );
        // URL sin path significativo + hint vacío → fallback.
        assert_eq!(pick_download_filename("https://x/", ""), "descarga");
    }

    #[test]
    fn same_doc_with_fragment_detecta_solo_fragment() {
        assert_eq!(
            same_doc_with_fragment("https://x/p", "https://x/p#top"),
            Some("top".to_string())
        );
        // Sin fragment en target → recargar normal.
        assert_eq!(same_doc_with_fragment("https://x/p", "https://x/p"), None);
        // Path distinto → recargar normal.
        assert_eq!(
            same_doc_with_fragment("https://x/p", "https://x/q#top"),
            None
        );
        // Query distinta → recargar normal.
        assert_eq!(
            same_doc_with_fragment("https://x/p", "https://x/p?q=1#top"),
            None
        );
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

    #[test]
    fn skip_count_details_avanza_por_cada_details_anidado() {
        let tree = parse(
            "<details><summary>A</summary><details><summary>B</summary><p>x</p></details></details>\
             <details><summary>C</summary></details>",
        );
        // Pre-cuenta total via walk (mismo orden que el Loaded llena).
        let mut total = 0_usize;
        tree.walk(|b| {
            if b.tag.as_deref() == Some("details") {
                total += 1;
            }
        });
        assert!(total >= 3, "esperaba >= 3 <details>, conseguí {total}");

        let mut counter = 0_usize;
        skip_count_details(&tree.root, &mut counter);
        assert_eq!(counter, total, "skip_count_details debe contar todos los <details>");
    }

    #[test]
    fn skip_count_details_no_cuenta_otros_tags() {
        let tree = parse("<p>foo</p><h1>bar</h1><div><span>baz</span></div>");
        let mut counter = 0_usize;
        skip_count_details(&tree.root, &mut counter);
        assert_eq!(counter, 0);
    }
}
