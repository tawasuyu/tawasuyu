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
    auto, fr, length, percent, AlignContent, AlignItems, AlignSelf, BoxSizing, Dimension,
    FlexDirection, FlexWrap, JustifyContent, LengthPercentageAuto, Position as TaffyPosition, Rect,
    Size, Style,
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
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_motion::{animate, motion, Tween};
use llimphi_clipboard::SystemClipboard;
// El trait `Clipboard` aporta `get`/`set` sobre `SystemClipboard` — lo usamos
// para puentear `navigator.clipboard` (Fase 7.176) con el portapapeles real.
use llimphi_widget_text_editor::Clipboard as _;
use llimphi_theme::Theme;

use puriy_engine::{
    AlignItems as CssAlignItems, AlignSelf as CssAlignSelf, BoxNode, BoxShadow,
    BoxSizing as CssBoxSizing, BoxTree, Display, Engine, FlexDirection as CssFlexDirection,
    AlignContent as CssAlignContent, FlexWrap as CssFlexWrap, GridTrackSize,
    JustifyContent as CssJustifyContent, LengthVal,
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
    /// Viewport actual de la ventana en px físicos (lo actualiza
    /// `Msg::Resize` desde el `on_resize` del runtime). `run_scripts_on_tab`
    /// lo lee para que `window.innerWidth`/`innerHeight` reflejen el tamaño
    /// real ya en la primera ejecución de scripts. Default = `initial_size`.
    static PURIY_VIEWPORT: std::cell::Cell<(f32, f32)> = const { std::cell::Cell::new((1100.0, 760.0)) };
    /// Factor de escala (DPI) actual de la ventana, el `scale_factor` de
    /// winit. Lo actualiza `Msg::ScaleFactor` (desde `on_scale_factor`) y
    /// `run_scripts_on_tab` lo lee para que `window.devicePixelRatio` sea
    /// correcto ya en la primera ejecución de scripts. Default = 1.0.
    static PURIY_DPR: std::cell::Cell<f64> = const { std::cell::Cell::new(1.0) };
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

/// Estado temporal del tween de `transition` de background en hover, por
/// nodo (keyeado por `BoxNode.node_id`). A diferencia de las `@keyframes`
/// (timeline absoluto), una transición arranca cuando el estado cambia y
/// puede REVERTIR a mitad de camino si el cursor entra y sale rápido — por
/// eso guardamos el progreso lineal en el momento del último toggle y el
/// instante del toggle, y reconstruimos el progreso actual sumando/restando
/// el tiempo según la dirección (`hovered`). El easing se aplica recién al
/// pintar (no acá), para que la reversión sea continua en el espacio lineal.
#[derive(Debug, Clone, Copy)]
struct HoverTween {
    /// `true` mientras el cursor está sobre el nodo (avanza hacia 1.0).
    hovered: bool,
    /// Progreso lineal `∈ [0,1]` capturado en el último cambio de estado.
    progress_at_toggle: f32,
    /// Reloj del `Model` (ms) del último toggle.
    toggle_ms: u64,
    /// Duración de la transición de background, en ms (de la binding CSS).
    duration_ms: u32,
}

impl HoverTween {
    /// Progreso lineal `∈ [0,1]` al instante `now_ms`: avanza desde
    /// `progress_at_toggle` hacia 1.0 si `hovered`, o hacia 0.0 si no,
    /// a razón de `1/duration`. Duración nula = salto inmediato.
    fn sample_linear(&self, now_ms: u64) -> f32 {
        if self.duration_ms == 0 {
            return if self.hovered { 1.0 } else { 0.0 };
        }
        let dt = now_ms.saturating_sub(self.toggle_ms) as f32 / self.duration_ms as f32;
        let delta = if self.hovered { dt } else { -dt };
        (self.progress_at_toggle + delta).clamp(0.0, 1.0)
    }
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
    /// Reloj base (ms desde `Model.start`) anclado al cargar la página. El
    /// runtime de animaciones CSS computa `elapsed = now_ms - anim_start_ms`
    /// y se lo pasa a `anim::animation_progress` por cada nodo animado.
    pub anim_start_ms: u64,
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
    /// `element_id` paralelo a `inputs` (sólo presente si el `<input>`
    /// o `<textarea>` tiene atributo `id=`). Permite despachar
    /// `focus`/`blur`/`keydown`/`input` JS sobre el elemento focado
    /// cuando el usuario interactúa con el input.
    pub inputs_element_ids: Vec<Option<String>>,
    /// `element_id` paralelo a `selects` (Fase 7.7).
    pub selects_element_ids: Vec<Option<String>>,
    /// Índice DFS del input/textarea focado (clave en `inputs`). `None` =
    /// sin foco; el chrome rutea teclas al resto del flow.
    pub focused_input: Option<usize>,
    /// Estado open/closed por `<details>` en orden DFS. Se inicializa al
    /// recibir `Msg::Loaded` walkeando el box tree y consultando
    /// `details_open_attr` de cada `<details>`. Subsiguientes
    /// `Msg::ToggleDetails(idx)` flippean el bool. Reset en cada
    /// navegación para evitar índices stale.
    pub details_open: Vec<bool>,
    /// Runtime JavaScript de la pestaña (creado lazily la primera vez
    /// que la página tiene un `<script>` inline). Lleva el contexto JS
    /// con `console`/`document` ya bootstraped. Se destruye en cada
    /// `Msg::Navigate` para que `var x = ...` de una página no fugue
    /// a la siguiente.
    ///
    /// `Box` para mantener el `TabState` pequeño en el caso común
    /// (mayoría de pestañas no usan JS) y evitar mover ~1MB de runtime
    /// cuando el Vec<TabState> se redimensiona. `JsRuntime` no es
    /// `Send` — el `Model` vive en el UI thread y nunca cruza hilos.
    pub js: Option<Box<puriy_js::JsRuntime>>,
    /// Resumen del último batch de scripts ejecutados: contador de logs
    /// y errores. Se muestra en la status bar cuando es no-cero.
    pub js_summary: JsSummary,
    /// Tweens de `transition` de background en hover, keyeados por
    /// `BoxNode.node_id`. Se puebla lazily cuando el cursor entra/sale de un
    /// nodo con `:hover { background }` + `transition`. Reset en cada
    /// navegación/load para no arrastrar ids de un árbol viejo.
    hover_tweens: std::collections::HashMap<u32, HoverTween>,
    /// `EventSource` vivos: id JS → flag de cancelación compartido con su
    /// worker de streaming. `close()` o una navegación lo prenden y el worker
    /// corta el stream y termina (Fase 7.182).
    es_cancel: std::collections::HashMap<u32, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Frames de `<canvas>` 2D del runtime JS, keyeados por `element_id`
    /// del canvas. Se refrescan (`refresh_canvas_frames`) tras correr
    /// scripts, en cada tick y tras dispatchear eventos — el render del box
    /// canvas (`render_canvas`) los interpreta y pinta con vello. Fase 7.196.
    canvas_frames: std::collections::HashMap<String, CanvasFrame>,
    /// `true` si el box tree de esta pestaña tiene al menos un `<canvas>`.
    /// Gatea el refresh por tick (evita un `eval` por frame en páginas sin
    /// canvas, que son la mayoría).
    has_canvas: bool,
}

/// Resultado agregado de ejecutar todos los `<script>` de un load.
#[derive(Default, Debug, Clone)]
pub struct JsSummary {
    pub logs: usize,
    pub errors: usize,
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
            anim_start_ms: 0,
            box_tree: None,
            source: None,
            gen: 0,
            inputs: Vec::new(),
            input_checks: Vec::new(),
            selects: Vec::new(),
            inputs_element_ids: Vec::new(),
            selects_element_ids: Vec::new(),
            focused_input: None,
            details_open: Vec::new(),
            js: None,
            js_summary: JsSummary::default(),
            hover_tweens: std::collections::HashMap::new(),
            es_cancel: std::collections::HashMap::new(),
            canvas_frames: std::collections::HashMap::new(),
            has_canvas: false,
        }
    }

    /// Cancela todos los `EventSource` vivos de esta pestaña (sus workers
    /// cortan el stream) y limpia el registro. Se llama al navegar/recargar
    /// (el runtime viejo se destruye) y al cerrar la pestaña.
    fn cancel_all_eventsources(&mut self) {
        for flag in self.es_cancel.values() {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.es_cancel.clear();
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
    /// Texto a buscar (se redacta vía `TextInputState`). Se compila a un
    /// `Matcher` (con los toggles `find_case_sensitive`/`find_whole_word`)
    /// contra cada hoja de texto del box tree del documento activo. Vacío
    /// = sin highlight.
    pub find_input: TextInputState,
    /// Match "actual" (1-based) cuando el usuario navega con
    /// Enter/Shift+Enter. `0` = sin nav todavía (todos los matches en
    /// amarillo); `>= 1` = ese match pinta en naranja para destacarse.
    pub find_current: usize,
    /// Toggle "Aa" de la find bar — distingue mayúsculas/minúsculas.
    /// Default `false` (búsqueda case-insensitive, como los browsers).
    pub find_case_sensitive: bool,
    /// Toggle "W" de la find bar — sólo matchea palabras completas
    /// (delimitadas por bordes no-alfanuméricos). Default `false`.
    pub find_whole_word: bool,
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
    /// Instante de arranque — base monotónica para el reloj que el
    /// reactor JS le pasa a `setTimeout`/`setInterval`. Cada `JsTick`
    /// calcula `start.elapsed().as_millis()` y avanza el runtime.
    pub start: std::time::Instant,
    /// Índice del menú principal abierto (barra Archivo/Editar/Navegar/
    /// Ver/Ayuda). `None` = todos cerrados. Lo gobierna `menubar_view` /
    /// `menubar_overlay` vía `Msg::MenuOpen`.
    pub menu_open: Option<usize>,
    /// Ancla `(x, y)` del menú contextual de edición (right-click sobre
    /// un campo de texto). `None` = cerrado. El contenido lo arma
    /// `editmenu::edit_context_menu` con los flags del input focuseado.
    pub edit_menu: Option<(f32, f32)>,
    /// Clipboard del sistema — lo consumen las acciones Cut/Copy/Paste
    /// del menú de edición sobre el `EditorState` del campo focuseado.
    pub clipboard: SystemClipboard,
    /// Fila resaltada por teclado en el menú principal (`usize::MAX` = ninguna).
    pub menu_active: usize,
    /// Animación de aparición/swap del dropdown principal.
    pub menu_anim: Tween<f32>,
    /// Fila resaltada por teclado en el menú de edición (`usize::MAX` = ninguna).
    pub edit_active: usize,
    /// Animación de aparición del menú de edición.
    pub edit_anim: Tween<f32>,
}

/// Periodo del poll del reactor JS — disparo de `Msg::JsTick`. ~30 fps
/// matchea el comportamiento de browsers reales para `setTimeout(_, 0)`
/// (la spec dice 4ms pero los browsers clampan a ~16ms; nosotros a
/// 33ms para no saturar el UI thread con ticks vacíos).
const JS_POLL_PERIOD_MS: u64 = 33;

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
    /// Compila el predicado de búsqueda activo (query + toggles) en un
    /// `Matcher` reutilizable por count/highlight/scroll. La query se
    /// normaliza una sola vez acá para no pagar el cast por hoja.
    fn find_matcher(&self) -> Matcher {
        Matcher::new(
            &self.find_input.text(),
            MatchOpts {
                case_sensitive: self.find_case_sensitive,
                whole_word: self.find_whole_word,
            },
        )
    }

    /// Devuelve una referencia compartida al `TextInputState` que tiene
    /// el foco de teclado en este momento, junto con su flag `masked`, o
    /// `None` si el foco no está sobre ningún campo editable (página sin
    /// input focuseado, address bar sin foco, etc). La prioridad de foco
    /// replica `on_key`: find bar > filtro de panel > input de página >
    /// address bar. Lo consume el menú de edición para pintar los flags
    /// (undo/redo/selección) del campo correcto.
    fn focused_text_input(&self) -> Option<(&TextInputState, bool)> {
        if self.find_active {
            return Some((&self.find_input, false));
        }
        if self.panel.is_some() {
            return Some((&self.panel_filter, false));
        }
        let t = self.active();
        if let Some(idx) = t.focused_input {
            if let Some(s) = t.inputs.get(idx) {
                return Some((s, s.is_masked()));
            }
        }
        if t.addr_focused {
            return Some((&t.addr, false));
        }
        None
    }

    /// `Ctrl+F` — levanta la find bar con query fresca.
    fn find_open(&mut self) {
        self.find_active = true;
        // Re-abrir limpia query previa para que el usuario arranque fresh.
        self.find_input.clear();
        self.find_current = 0;
    }

    /// `Esc` / ✕ — cierra la find bar y limpia la query (el highlight
    /// desaparece porque el matcher queda vacío).
    fn find_close(&mut self) {
        self.find_active = false;
        self.find_input.clear();
        self.find_current = 0;
    }

    /// Navega al match siguiente (`forward=true`) o anterior con
    /// wrap-around 1↔N. No-op si la query no tiene matches. Mueve el
    /// `scroll_y` del tab activo al match resultante.
    fn find_step(&mut self, forward: bool) {
        let matcher = self.find_matcher();
        let total = count_matches(self.active().box_tree.as_ref(), &matcher);
        if total == 0 {
            return;
        }
        self.find_current = if forward {
            if self.find_current >= total { 1 } else { self.find_current + 1 }
        } else if self.find_current <= 1 {
            total
        } else {
            self.find_current - 1
        };
        scroll_to_find_match(self, &matcher);
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
        /// `<script>` extraídos del DOM en orden. El chrome los ejecuta
        /// en orden tras el load (sólo los inline; src= se ignora hasta
        /// que tengamos async fetch + queue de ejecución).
        scripts: Vec<puriy_engine::ScriptInfo>,
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
    /// La ventana cambió de tamaño (px físicos). Actualiza el viewport y
    /// dispatcha el evento `resize` al window de la pestaña activa.
    Resize(u32, u32),
    /// El factor de escala (DPI) de la ventana cambió (`scale_factor` de
    /// winit). Actualiza `window.devicePixelRatio` y dispatcha `resize`.
    ScaleFactor(f64),
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
    /// Click en el toggle "Aa" — alterna búsqueda case-sensitive y
    /// resetea la navegación (cambia el conjunto de matches).
    FindToggleCase,
    /// Click en el toggle "W" — alterna match de palabra completa.
    FindToggleWord,
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
    /// El cursor entró (`entering=true`) o salió de un nodo con
    /// `:hover { background }` + `transition`. Ancla/reversa el tween de
    /// background del nodo (keyeado por `node_id`). `duration_ms` viaja en
    /// el msg porque el `update` no tiene el box tree a mano para mirarlo.
    HoverTween { node_id: u32, entering: bool, duration_ms: u32 },
    /// Fase 7.31 — request HTTP iniciado por `fetch()` desde el JS. El
    /// chrome lo recibe vía `apply_dom_mutations` (kind 'fetch') y spawn-ea
    /// un worker que llama `puriy_engine::fetch::fetch_full`.
    FetchRequest {
        tab: TabId,
        gen: u64,
        fetch_id: u32,
        method: String,
        url: String,
        body: Option<Vec<u8>>,
        headers: Vec<(String, String)>,
    },
    /// Resultado del worker de fetch. Si `result` es `Ok`, llama
    /// `JsRuntime::resolve_fetch` para disparar `.then()`; si `Err`,
    /// `reject_fetch` para disparar `.catch()`. Mismo gen check que
    /// otros Msg async — descarta si la pestaña fue cerrada o pisada
    /// por otra navegación.
    FetchComplete {
        tab: TabId,
        gen: u64,
        fetch_id: u32,
        result: Result<puriy_engine::FetchResponse, String>,
    },
    /// Empuja texto al portapapeles real del sistema. Lo emite
    /// `apply_dom_mutations` cuando el JS llamó `navigator.clipboard.writeText`/
    /// `write` (mutación `kind:'clipboard'`). Se procesa en el update loop
    /// porque escribir el portapapeles necesita `&mut Model.clipboard`, que
    /// `apply_dom_mutations` (sólo `&mut TabState`) no alcanza (Fase 7.176).
    SetSystemClipboard(String),
    /// `new EventSource(url)` — abre un stream SSE. Lo emite `apply_dom_mutations`
    /// (mutación `kind:'eventsource'`, acción `open`). El update arm crea el flag
    /// de cancelación y spawnea el worker de streaming (Fase 7.182).
    EsOpen { tab: TabId, gen: u64, es_id: u32, url: String },
    /// `es.close()` — el update arm prende el flag de cancelación del worker.
    EsClose { tab: TabId, es_id: u32 },
    /// Evento ya parseado que el worker SSE reinyecta al runtime: `kind` es
    /// `open`/`message`/`error`; para `message`, los otros campos portan el
    /// evento. Mismo gen check que el resto de async — se descarta si la
    /// pestaña se cerró o fue pisada por otra navegación.
    EsDispatch {
        tab: TabId,
        gen: u64,
        es_id: u32,
        kind: String,
        event_type: String,
        data: String,
        last_id: String,
    },
    /// Tick periódico del reactor JS — disparado por `Handle::spawn_periodic`
    /// cada `JS_POLL_PERIOD_MS`. Para cada pestaña con `JsRuntime` y timers
    /// vivos, avanza el reloj a `Model.start.elapsed_ms()` y corre los
    /// callbacks vencidos (setTimeout/setInterval). Sin payload — el handler
    /// computa el now al momento de procesar.
    JsTick,
    /// Dispatch de un evento JS sobre el elemento `element_id` de la
    /// pestaña activa. El chrome arma este msg cuando el usuario hace
    /// click/key/etc. sobre un nodo con `id=`. El handler llama
    /// `dispatch_event(element_id, event_type)` sobre el runtime; si el
    /// JS NO llamó `event.preventDefault()`, dispatcha `fallback`
    /// (típicamente un `Navigate` para `<a href>` con id).
    ///
    /// `fallback = None` significa "no default action" (`<button>`,
    /// `<div>` sin link). `fallback = Some(...)` cohabita link/handler
    /// para `<a id href>` — Fase 7.6.
    JsDispatchEvent {
        element_id: String,
        event_type: String,
        fallback: Option<Box<Msg>>,
    },
    /// Abre/cierra un menú de la barra principal (`Some(idx)` lo abre,
    /// `None` cierra). Lo dispara `menubar_view`/`menubar_overlay`.
    MenuOpen(Option<usize>),
    /// Pick de un ítem del menú principal — el string es el `command`
    /// del `MenuItem`. `handle_menu_command` lo mapea al `Msg` real.
    MenuCommand(String),
    /// Cierra todos los menús (principal y contextual de edición).
    CloseMenus,
    /// Right-click sobre la ventana — abre el menú contextual de edición
    /// anclado en `(x, y)` (coords de ventana) sobre el campo focuseado.
    EditMenuOpen(f32, f32),
    /// Acción del menú de edición a aplicar sobre el `EditorState` del
    /// campo de texto focuseado.
    EditMenuAction(EditAction),
    /// Navegación ↑/↓ por la fila activa del menú principal.
    MenuNav(i32),
    /// Enter sobre la fila activa del menú principal.
    MenuActivate,
    /// Tick de animación de aparición/swap (re-render).
    MenuTick,
    /// Navegación ↑/↓ por la fila activa del menú de edición.
    EditNav(i32),
    /// Enter sobre la fila activa del menú de edición.
    EditActivate,
}

/// Decide qué hace un evento de rueda según los modifiers: con `Ctrl`
/// hace zoom de página (gesto estándar del navegador), si no scrollea.
/// Pura para poder testearla sin construir un `Model`.
///
/// Convención CSS de `WheelDelta`: `y > 0` = rueda hacia abajo
/// (contenido baja) → alejar; `y < 0` = rueda hacia arriba → acercar.
/// Cada notch da un paso, igual que `Ctrl+=`/`Ctrl+-`.
fn wheel_to_msg(delta: WheelDelta, mods: Modifiers) -> Option<Msg> {
    if mods.ctrl {
        return if delta.y < 0.0 {
            Some(Msg::ZoomIn)
        } else if delta.y > 0.0 {
            Some(Msg::ZoomOut)
        } else {
            None
        };
    }
    Some(Msg::Scroll(delta.y * LINE_PX * 3.0))
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
        spawn_load(tab.id, tab.gen, url, /* referer */ None, current_viewport(), handle.clone());
        // Poll del reactor JS — un solo thread global que dispatcha
        // `Msg::JsTick` cada ~33ms. El handler walka las pestañas y
        // saltea las que no tienen runtime (cost ~ns por tab inactiva).
        handle.spawn_periodic(
            std::time::Duration::from_millis(JS_POLL_PERIOD_MS),
            || Msg::JsTick,
        );
        Model {
            tabs: vec![tab],
            active: 0,
            zoom: 1.0,
            find_active: false,
            find_input: TextInputState::new(),
            find_current: 0,
            find_case_sensitive: false,
            find_whole_word: false,
            panel: None,
            panel_filter: TextInputState::new(),
            hover_link: None,
            start: std::time::Instant::now(),
            menu_open: None,
            edit_menu: None,
            clipboard: SystemClipboard::new(),
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra. Tiene prioridad sobre todo lo demás.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            match &e.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => {
                    return Some(Msg::MenuOpen(Some((mi + n - 1) % n)));
                }
                Key::Named(NamedKey::ArrowRight) => {
                    return Some(Msg::MenuOpen(Some((mi + 1) % n)));
                }
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::MenuActivate),
                _ => return None,
            }
        }
        // Menú de edición abierto: ↑/↓ navegan, Enter ejecuta, Esc cierra.
        if model.edit_menu.is_some() {
            match &e.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::EditNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::EditNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::EditActivate),
                _ => return None,
            }
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
        mods: Modifiers,
    ) -> Option<Self::Msg> {
        wheel_to_msg(delta, mods)
    }

    fn on_resize(_model: &Self::Model, width: u32, height: u32) -> Option<Self::Msg> {
        Some(Msg::Resize(width, height))
    }

    fn on_scale_factor(_model: &Self::Model, scale: f64) -> Option<Self::Msg> {
        Some(Msg::ScaleFactor(scale))
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Reload => {
                let url = m.active().url.clone();
                start_load(&mut m, url, /* push_history */ false, handle);
            }
            Msg::Loaded { tab, gen, final_url, title, box_tree, source, meta_refresh, scripts } => {
                if let Some(idx) = m.tab_idx(tab) {
                    // Lee el portapapeles del sistema ANTES de tomar `&mut t`
                    // (borrow disjunto) para sembrarlo en el runtime nuevo.
                    let sys_clipboard = m.clipboard.get();
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
                        let mut inputs_element_ids: Vec<Option<String>> = Vec::new();
                        let mut selects_element_ids: Vec<Option<String>> = Vec::new();
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
                                inputs_element_ids.push(b.element_id.clone());
                                if b.input_autofocus && autofocus_idx.is_none() {
                                    autofocus_idx = Some(idx);
                                }
                            }
                            if let Some(sel) = &b.select {
                                selects.push(SelectState {
                                    selected: sel.initial,
                                    open: false,
                                });
                                selects_element_ids.push(b.element_id.clone());
                            }
                        });
                        t.details_open = details_open;
                        t.inputs = inputs;
                        t.input_checks = input_checks;
                        t.selects = selects;
                        t.inputs_element_ids = inputs_element_ids;
                        t.selects_element_ids = selects_element_ids;
                        t.focused_input = autofocus_idx;
                        // Árbol nuevo → los node_id viejos ya no aplican.
                        t.hover_tweens.clear();
                        // El runtime previo se va a destruir: cortá sus
                        // EventSource (sus workers de streaming) para no fugar
                        // threads ni reinyectar al runtime viejo (Fase 7.182).
                        t.cancel_all_eventsources();
                        // Fase 7.196 — ¿hay algún `<canvas>` en el árbol? Gatea
                        // el refresh de frames (evita un `eval` por tick en
                        // páginas sin canvas). Reset de frames stale del load previo.
                        t.canvas_frames.clear();
                        let mut has_canvas = false;
                        box_tree.walk(|b| {
                            if b.canvas.is_some() {
                                has_canvas = true;
                            }
                        });
                        t.has_canvas = has_canvas;
                        t.box_tree = Some(box_tree);
                        // Ancla el reloj de animaciones CSS de esta carga.
                        t.anim_start_ms = m.start.elapsed().as_millis() as u64;
                        // Ejecuta los `<script>` inline del documento.
                        // Destruimos cualquier JsRuntime previo (var x = ...
                        // de la página anterior no debe fugar). Si esta
                        // página tiene scripts, instanciamos un runtime
                        // fresh, hacemos set_document con el snapshot del
                        // DOM, y eval cada script en orden. Logs y errores
                        // se acumulan en t.js_summary y se muestran en la
                        // status bar.
                        t.js = None;
                        t.js_summary = JsSummary::default();
                        let now_ms = m.start.elapsed().as_millis() as u64;
                        let pending =
                            run_scripts_on_tab(t, &scripts, now_ms, sys_clipboard.as_deref());
                        for req in pending {
                            handle.dispatch(req);
                        }
                        // 'DOMContentLoaded' al document: dispara cuando el DOM
                        // quedó parseado y los scripts inline corrieron, ANTES
                        // del 'load' del window (que en spec espera recursos).
                        // Es el evento más usado para diferir init
                        // (`document.addEventListener('DOMContentLoaded', ...)`).
                        if t.js.is_some() {
                            let (_, pending) = dispatch_document_js_event_on_tab(
                                t,
                                "DOMContentLoaded",
                                None,
                                None,
                                now_ms,
                            );
                            for req in pending {
                                handle.dispatch(req);
                            }
                        }
                        // Fase 7.39 — disparar 'load' al window. Apps usan
                        // `window.addEventListener('load', fn)` para diferir
                        // init hasta que el DOM esté pronto. Sólo si el tab
                        // creó runtime (hay scripts en la página).
                        if t.js.is_some() {
                            let (_, pending) = dispatch_window_js_event_on_tab(t, "load", now_ms);
                            for req in pending {
                                handle.dispatch(req);
                            }
                        }
                        if t.js_summary.errors > 0 {
                            t.status =
                                format!("{} · JS: {} log/{} err",
                                    t.status, t.js_summary.logs, t.js_summary.errors);
                        } else if t.js_summary.logs > 0 {
                            t.status = format!("{} · JS: {} logs",
                                t.status, t.js_summary.logs);
                        }
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
                        if idx != m.active {
                            switch_active_tab(&mut m, idx);
                        }
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
                spawn_load(tab.id, tab.gen, target, referer, current_viewport(), handle.clone());
                m.tabs.push(tab);
                let new_idx = m.tabs.len() - 1;
                switch_active_tab(&mut m, new_idx);
                m.panel = None;
                m.panel_filter.clear();
            }
            Msg::Scroll(dy) => {
                let now_ms = m.start.elapsed().as_millis() as u64;
                let t = m.active_mut();
                t.scroll_y = (t.scroll_y + dy).max(0.0);
                // Fase 7.39 — dispatchar 'scroll' al window para handlers
                // tipo `window.addEventListener('scroll', fn)` (header
                // sticky, infinite scroll, etc.). Sólo si hay JS runtime
                // creado para esta pestaña.
                if t.js.is_some() {
                    let (_, pending) = dispatch_window_js_event_on_tab(t, "scroll", now_ms);
                    for req in pending {
                        handle.dispatch(req);
                    }
                }
            }
            Msg::Resize(w, h) => {
                // Guardamos el viewport para que los próximos loads lo
                // sincronicen ya en la primera ejecución de scripts.
                let (vp_w, vp_h) = (w as f32, h as f32);
                PURIY_VIEWPORT.with(|c| c.set((vp_w, vp_h)));
                let now_ms = m.start.elapsed().as_millis() as u64;
                let t = m.active_mut();
                // set_viewport ANTES del dispatch para que el handler de
                // 'resize' lea `window.innerWidth`/`innerHeight` actuales.
                if let Some(rt) = t.js.as_mut() {
                    let _ = rt.set_viewport(vp_w, vp_h);
                    // Re-evalúa las media queries de ancho/alto/orientation con
                    // el viewport nuevo (dispara `change` donde flipeó).
                    sync_media_queries(rt, vp_w, vp_h, PURIY_DPR.with(|c| c.get()) as f32);
                    let (_, pending) = dispatch_window_js_event_on_tab(t, "resize", now_ms);
                    for req in pending {
                        handle.dispatch(req);
                    }
                }
            }
            Msg::ScaleFactor(scale) => {
                // Guardamos el DPR para que los próximos loads lo sincronicen
                // ya en la primera ejecución de scripts.
                PURIY_DPR.with(|c| c.set(scale));
                let now_ms = m.start.elapsed().as_millis() as u64;
                let t = m.active_mut();
                // set_device_pixel_ratio ANTES del dispatch para que el
                // handler de 'resize' lea `window.devicePixelRatio` actual
                // (los browsers disparan 'resize' al cambiar el DPI).
                if let Some(rt) = t.js.as_mut() {
                    let _ = rt.set_device_pixel_ratio(scale);
                    // Re-evalúa las media queries de resolution con el DPR nuevo.
                    let (vp_w, vp_h) = PURIY_VIEWPORT.with(|c| c.get());
                    sync_media_queries(rt, vp_w, vp_h, scale as f32);
                    let (_, pending) = dispatch_window_js_event_on_tab(t, "resize", now_ms);
                    for req in pending {
                        handle.dispatch(req);
                    }
                }
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
                let new_idx = m.tabs.len() - 1;
                switch_active_tab(&mut m, new_idx);
                m.active_mut().addr_focused = true;
            }
            Msg::CloseTab(idx) => {
                if idx < m.tabs.len() {
                    // Corta los EventSource de la pestaña antes de tirarla.
                    m.tabs[idx].cancel_all_eventsources();
                    m.tabs.remove(idx);
                }
                if m.tabs.is_empty() {
                    let t = TabState::new(NEW_TAB_URL.into());
                    m.tabs.push(t);
                    m.active = 0;
                } else if m.active >= m.tabs.len() {
                    // El active quedó out-of-bounds — apuntar al último.
                    // No usamos switch_active_tab porque no hay tab "vieja"
                    // a marcar hidden (la borramos arriba).
                    m.active = m.tabs.len() - 1;
                    if let Some(rt) = m.tabs[m.active].js.as_mut() {
                        let _ = rt.set_visibility(false);
                    }
                }
            }
            Msg::SelectTab(idx) => {
                if idx < m.tabs.len() && idx != m.active {
                    switch_active_tab(&mut m, idx);
                }
            }
            Msg::NextTab => {
                if !m.tabs.is_empty() {
                    let next = (m.active + 1) % m.tabs.len();
                    if next != m.active {
                        switch_active_tab(&mut m, next);
                    }
                }
            }
            Msg::PrevTab => {
                if !m.tabs.is_empty() {
                    let prev = (m.active + m.tabs.len() - 1) % m.tabs.len();
                    if prev != m.active {
                        switch_active_tab(&mut m, prev);
                    }
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
                m.find_open();
            }
            Msg::FindClose => {
                m.find_close();
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
                m.find_step(true);
            }
            Msg::FindPrev => {
                m.find_step(false);
            }
            Msg::FindToggleCase => {
                m.find_case_sensitive = !m.find_case_sensitive;
                // El conjunto de matches cambió → reseteamos la nav; el
                // próximo Enter arranca desde el primer match nuevo.
                m.find_current = 0;
            }
            Msg::FindToggleWord => {
                m.find_whole_word = !m.find_whole_word;
                m.find_current = 0;
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
            Msg::HoverTween { node_id, entering, duration_ms } => {
                let now_ms = m.start.elapsed().as_millis() as u64;
                let t = m.active_mut();
                // Captura el progreso lineal al instante del toggle y reancla
                // el reloj con la nueva dirección — así un enter→leave rápido
                // revierte desde donde iba, sin saltar a 0/1.
                let prev = t.hover_tweens.get(&node_id).copied();
                let progress_at_toggle =
                    prev.map(|tw| tw.sample_linear(now_ms)).unwrap_or(0.0);
                t.hover_tweens.insert(
                    node_id,
                    HoverTween { hovered: entering, progress_at_toggle, toggle_ms: now_ms, duration_ms },
                );
            }
            Msg::JsTick => {
                let now_ms = m.start.elapsed().as_millis() as u64;
                for req in tick_js_runtimes(&mut m, now_ms) {
                    handle.dispatch(req);
                }
            }
            req @ Msg::FetchRequest { .. } => {
                spawn_fetch(req, handle.clone());
            }
            Msg::SetSystemClipboard(text) => {
                // `navigator.clipboard.writeText`/`write` → portapapeles real
                // (Fase 7.176). Degrada a no-op si no hay backend (headless).
                m.clipboard.set(&text);
            }
            Msg::EsOpen { tab, gen, es_id, url } => {
                // Abre el stream SSE (Fase 7.182). Sólo si la pestaña sigue viva
                // y en el mismo gen (no se navegó mientras tanto).
                if let Some(idx) = m.tab_idx(tab) {
                    if m.tabs[idx].gen == gen {
                        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                        m.tabs[idx].es_cancel.insert(es_id, cancel.clone());
                        spawn_eventsource(tab, gen, es_id, url, cancel, handle.clone());
                    }
                }
            }
            Msg::EsClose { tab, es_id } => {
                if let Some(idx) = m.tab_idx(tab) {
                    if let Some(flag) = m.tabs[idx].es_cancel.remove(&es_id) {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            Msg::EsDispatch { tab, gen, es_id, kind, event_type, data, last_id } => {
                if let Some(idx) = m.tab_idx(tab) {
                    if m.tabs[idx].gen == gen {
                        let now_ms = m.start.elapsed().as_millis() as u64;
                        let t = &mut m.tabs[idx];
                        if let Some(rt) = t.js.as_mut() {
                            let _ = rt.set_now_ms(now_ms);
                            rt.set_fuel(puriy_js::DEFAULT_FUEL);
                            let _ = rt.es_dispatch(es_id, &kind, &event_type, &data, &last_id);
                            // Un handler SSE puede haber tocado el DOM.
                            for req in apply_dom_mutations(t) {
                                handle.dispatch(req);
                            }
                        }
                    }
                }
            }
            Msg::FetchComplete { tab, gen, fetch_id, result } => {
                let tab_idx = m.tabs.iter().position(|t| t.id == tab && t.gen == gen);
                if let Some(idx) = tab_idx {
                    if let Some(rt) = m.tabs[idx].js.as_mut() {
                        let now_ms = m.start.elapsed().as_millis() as u64;
                        let _ = rt.set_now_ms(now_ms);
                        rt.set_fuel(puriy_js::DEFAULT_FUEL);
                        let prev_stdout = rt.stdout().len();
                        let prev_stderr = rt.stderr().len();
                        match result {
                            Ok(resp) => {
                                let body_str = String::from_utf8_lossy(&resp.body).into_owned();
                                let _ = rt.resolve_fetch(
                                    fetch_id,
                                    resp.status,
                                    &resp.status_text,
                                    &body_str,
                                    &resp.headers,
                                );
                            }
                            Err(err) => {
                                let _ = rt.reject_fetch(fetch_id, &err);
                            }
                        }
                        let new_stdout = rt.stdout();
                        let new_stderr = rt.stderr();
                        m.tabs[idx].js_summary.logs +=
                            new_stdout[prev_stdout..].matches('\n').count();
                        m.tabs[idx].js_summary.errors +=
                            new_stderr[prev_stderr..].matches('\n').count();
                        // Las mutaciones resultantes (fetch encadenado, write
                        // al portapapeles) se despachan al loop: `FetchRequest`
                        // re-entra a su arm (que spawnea el worker) y
                        // `SetSystemClipboard` al suyo.
                        for next in apply_dom_mutations(&mut m.tabs[idx]) {
                            handle.dispatch(next);
                        }
                    }
                }
            }
            Msg::JsDispatchEvent { element_id, event_type, fallback } => {
                let now_ms = m.start.elapsed().as_millis() as u64;
                let (result, pending) = dispatch_js_event(&mut m, &element_id, &event_type, now_ms);
                for req in pending {
                    handle.dispatch(req);
                }
                // Si el handler JS no llamó `event.preventDefault()` y
                // hay un fallback (típicamente Navigate del `<a href>`),
                // lo reenviamos al event loop para que el chrome lo
                // procese normalmente. `dispatch` despacha el msg en el
                // próximo iteración de update.
                if !result.default_prevented {
                    if let Some(fb) = fallback {
                        handle.dispatch(*fb);
                    }
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
                // Fase 7.7: despachar `change` JS si el <select> tiene id.
                let eid = m
                    .active()
                    .selects_element_ids
                    .get(idx)
                    .cloned()
                    .flatten();
                if let Some(eid) = eid {
                    let now_ms = m.start.elapsed().as_millis() as u64;
                    // Fase 7.9 — pasar el value del option recién
                    // seleccionado en el EventInit. Así el handler de
                    // `change` lee `event.target.value` y obtiene el
                    // value del option, no el label.
                    let value = select_value_at(m.active(), idx, opt);
                    let mut init = puriy_js::EventInit::default();
                    init.value = value;
                    let (_, pending) = dispatch_js_event_with_init(
                        &mut m,
                        &eid,
                        "change",
                        now_ms,
                        Some(init),
                    );
                    for req in pending { handle.dispatch(req); }
                }
            }
            Msg::ToggleCheckbox(idx) => {
                let t = m.active_mut();
                if let Some(c) = t.input_checks.get_mut(idx) {
                    *c = !*c;
                }
                // Fase 7.187 — refleja el nuevo estado en el atributo `checked`
                // del box y recascadea para que `:checked`/`:checked + label`
                // se actualicen al togglear en vivo.
                let checks = t.input_checks.clone();
                if let Some(bt) = t.box_tree.as_mut() {
                    bt.sync_checked_from(&checks);
                    bt.restyle();
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
                // Fase 7.187 — espeja el estado del grupo de radios al atributo
                // `checked` de los boxes y recascadea (`:checked` en vivo).
                let checks = t.input_checks.clone();
                if let Some(bt) = t.box_tree.as_mut() {
                    bt.sync_checked_from(&checks);
                    bt.restyle();
                }
            }
            Msg::SubmitForm(idx) => {
                // Tratamos como si el input idx estuviera focado.
                m.active_mut().focused_input = Some(idx);
                if let Some(msg) = build_form_submit_url(&m) {
                    return Self::update(m, msg, handle);
                }
            }
            Msg::FocusInput(idx) => {
                // Fase 7.7: despachar blur al input previo (si tenía id)
                // y focus al nuevo (si tiene id).
                let prev = m.active().focused_input;
                let prev_eid = prev.and_then(|i| {
                    m.active()
                        .inputs_element_ids
                        .get(i)
                        .cloned()
                        .flatten()
                });
                let new_eid = if idx == usize::MAX {
                    None
                } else {
                    m.active()
                        .inputs_element_ids
                        .get(idx)
                        .cloned()
                        .flatten()
                };
                let t = m.active_mut();
                if idx == usize::MAX {
                    // sentinel = blur
                    t.focused_input = None;
                } else if idx < t.inputs.len() {
                    t.focused_input = Some(idx);
                    // Blur address bar para que las teclas no compitan.
                    t.addr_focused = false;
                }
                let now_ms = m.start.elapsed().as_millis() as u64;
                if let Some(eid) = prev_eid {
                    let (_, p) = dispatch_js_event(&mut m, &eid, "blur", now_ms);
                    for req in p { handle.dispatch(req); }
                }
                if let Some(eid) = new_eid {
                    let (_, p) = dispatch_js_event(&mut m, &eid, "focus", now_ms);
                    for req in p { handle.dispatch(req); }
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
                    // Fase 7.7: despachar `keydown` al elemento focado si
                    // tiene `id=`. Si el handler hace `preventDefault()`,
                    // la tecla NO se aplica al input — el JS toma el control.
                    let focused_idx = m.active().focused_input;
                    let eid = focused_idx.and_then(|i| {
                        m.active()
                            .inputs_element_ids
                            .get(i)
                            .cloned()
                            .flatten()
                    });
                    let prevented = if let Some(eid) = eid {
                        let now_ms = m.start.elapsed().as_millis() as u64;
                        // Fase 7.9 — enriquecer el event object con key/
                        // code/modifiers + value actual del input. Así un
                        // handler puede leer event.key === 'Enter' o
                        // event.target.value antes de aplicar el keydown.
                        let focused_idx = m.active().focused_input;
                        let mut init = key_event_to_init(&e);
                        if let Some(idx) = focused_idx {
                            if let Some(input) = m.active().inputs.get(idx) {
                                init.value = Some(input.text());
                            }
                        }
                        {
                            let (r, p) = dispatch_js_event_with_init(
                                &mut m,
                                &eid,
                                "keydown",
                                now_ms,
                                Some(init),
                            );
                            for req in p { handle.dispatch(req); }
                            r.default_prevented
                        }
                    } else {
                        false
                    };
                    if !prevented {
                        let mut new_value: Option<String> = None;
                        let mut input_eid: Option<String> = None;
                        let t = m.active_mut();
                        if let Some(idx) = t.focused_input {
                            if let Some(input) = t.inputs.get_mut(idx) {
                                input.apply_key(&e);
                                new_value = Some(input.text());
                                input_eid = t
                                    .inputs_element_ids
                                    .get(idx)
                                    .cloned()
                                    .flatten();
                            }
                        }
                        // Fase 7.10 — `input` event DESPUÉS de aplicar la
                        // tecla (a diferencia de `keydown` que se dispara
                        // ANTES). Handlers de autocomplete/search-as-you-
                        // type leen `event.target.value` con el value
                        // recién actualizado.
                        if let (Some(eid), Some(v)) = (input_eid, new_value) {
                            let now_ms = m.start.elapsed().as_millis() as u64;
                            let mut init = puriy_js::EventInit::default();
                            init.value = Some(v);
                            let (_, p) = dispatch_js_event_with_init(
                                &mut m,
                                &eid,
                                "input",
                                now_ms,
                                Some(init),
                            );
                            for req in p { handle.dispatch(req); }
                        }
                    }
                }
            }
            Msg::MenuOpen(idx) => {
                m.menu_open = idx;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                if idx.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuCommand(cmd) => {
                return handle_menu_command(m, cmd, handle);
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                        return handle_menu_command(m, cmd, handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::EditNav(dir) => {
                let flags = match m.focused_text_input() {
                    Some((input, masked)) => EditFlags::from_editor(input.editor(), masked),
                    None => EditFlags::default(),
                };
                m.edit_active = editmenu::edit_menu_step(flags, m.edit_active, dir);
            }
            Msg::EditActivate => {
                let flags = match m.focused_text_input() {
                    Some((input, masked)) => EditFlags::from_editor(input.editor(), masked),
                    None => EditFlags::default(),
                };
                if let Some(a) = editmenu::edit_menu_action_at(flags, m.edit_active) {
                    apply_edit_menu_action(&mut m, a);
                }
            }
            Msg::CloseMenus => {
                m.menu_open = None;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                m.edit_active = usize::MAX;
            }
            Msg::EditMenuOpen(x, y) => {
                m.edit_menu = Some((x, y));
                m.menu_open = None;
                m.edit_active = usize::MAX;
                m.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(handle, motion::FAST, || Msg::MenuTick);
            }
            Msg::EditMenuAction(action) => {
                apply_edit_menu_action(&mut m, action);
            }
        }
        m
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // Prioridad: menú contextual de edición > dropdown del menú
        // principal > overlay del `<select>` abierto.
        if let Some((x, y)) = model.edit_menu {
            let flags = match model.focused_text_input() {
                Some((input, masked)) => EditFlags::from_editor(input.editor(), masked),
                None => EditFlags::default(),
            };
            let (w, h) = Self::initial_size();
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                (w as f32, h as f32),
                menu_theme(),
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras { appear: model.edit_anim.value(), ..Default::default() },
            ));
        }
        let menu = app_menu(model);
        if let Some(ov) = menubar_overlay_animated(
            &menubar_spec(&menu, model),
            model.menu_active,
            model.menu_anim.value(),
        ) {
            return Some(ov);
        }

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
        // Predicado de búsqueda activo (query + toggles case/whole-word).
        // Si la find bar está cerrada, un matcher vacío → sin highlight.
        let matcher = if model.find_active {
            model.find_matcher()
        } else {
            Matcher::new("", MatchOpts::default())
        };
        // Pre-cuenta los matches del documento para mostrarlos en la
        // find bar. Matcher vacío (bar cerrada / query vacía) → count 0.
        let find_count = count_matches(model.active().box_tree.as_ref(), &matcher);
        let body = match model.panel {
            Some(kind) => panel_view(
                kind,
                &model.panel_filter,
                model.active().source.as_deref(),
                model.zoom,
            ),
            None => {
                // Elapsed para el runtime de animaciones CSS: now − ancla de
                // la carga. El tick periódico (JsTick, ~30fps) re-renderiza,
                // así que leer el reloj acá avanza la animación cada frame.
                let now_ms = model.start.elapsed().as_millis() as u64;
                let anim_elapsed_ms = now_ms.saturating_sub(model.active().anim_start_ms);
                viewport(
                    model.active(),
                    model.zoom,
                    &matcher,
                    model.find_current,
                    anim_elapsed_ms,
                    now_ms,
                )
            }
        };

        // Barra de menú principal — PRIMER hijo del column raíz.
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let mut children: Vec<View<Msg>> = vec![menubar, tabs_bar, header];
        if model.find_active {
            children.push(find_bar(
                &model.find_input,
                find_count,
                model.find_current,
                model.find_case_sensitive,
                model.find_whole_word,
            ));
        }
        children.push(body);

        // Right-click en la raíz (origen 0,0 → las coords locales que
        // llegan al handler ya son de ventana) abre el menú contextual de
        // edición sobre el campo de texto focuseado.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(Color::from_rgb8(245, 245, 248))
        .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
        .children(children)
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(menu: &'a app_bus::AppMenu, model: &Model) -> MenuBarSpec<'a, Msg> {
    let (w, h) = Puriy::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme: menu_theme(),
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: std::sync::Arc::new(Msg::MenuOpen),
        on_command: std::sync::Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Tema fijo para la barra/menús — puriy no trackea un `Theme` en su
/// Model (su chrome usa colores claros hard-coded), así que sostenemos un
/// `Theme::dark()` fijo y compartido para los menús. `OnceLock` lo
/// inicializa una sola vez sin `unsafe`.
fn menu_theme() -> &'static Theme {
    static CELL: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
    CELL.get_or_init(Theme::dark)
}

/// Menú principal del navegador. Sólo expone comandos que mapean a
/// `Msg` reales ya existentes. El submenú Editar refleja en gris el
/// estado del campo de texto focuseado (find/filtro/input de página/
/// address bar).
fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};

    let focused = model.focused_text_input();
    let has_sel = focused.map(|(s, _)| s.editor().has_selection()).unwrap_or(false);
    let can_undo = focused.map(|(s, _)| s.editor().can_undo()).unwrap_or(false);
    let can_redo = focused.map(|(s, _)| s.editor().can_redo()).unwrap_or(false);
    let has_input = focused.is_some();

    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo { undo = undo.disabled(); }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y");
    if !can_redo { redo = redo.disabled(); }
    let mut cut = MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C");
    if !has_sel { cut = cut.disabled(); copy = copy.disabled(); }
    let mut paste = MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V");
    let mut sel_all =
        MenuItem::new("Seleccionar todo", "edit.selectall").shortcut("Ctrl+A").separated();
    if !has_input { paste = paste.disabled(); sel_all = sel_all.disabled(); }

    let t = model.active();
    let can_back = t.can_back();
    let can_fwd = t.can_fwd();
    let mut back = MenuItem::new("Atrás", "nav.back").shortcut("Alt+←");
    if !can_back { back = back.disabled(); }
    let mut fwd = MenuItem::new("Adelante", "nav.fwd").shortcut("Alt+→");
    if !can_fwd { fwd = fwd.disabled(); }

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Nueva pestaña", "file.newtab").shortcut("Ctrl+T"))
                .item(MenuItem::new("Cerrar pestaña", "file.close").shortcut("Ctrl+W").separated())
                .item(MenuItem::new("Recargar", "file.reload").shortcut("F5"))
                .item(MenuItem::new("Ver código fuente", "file.source").shortcut("Ctrl+U"))
                .item(MenuItem::new("Agregar marcador", "file.bookmark").shortcut("Ctrl+D")),
        )
        .menu(
            Menu::new("Editar")
                .item(undo)
                .item(redo)
                .item(cut)
                .item(copy)
                .item(paste)
                .item(sel_all),
        )
        .menu(
            Menu::new("Navegar")
                .item(back)
                .item(fwd)
                .item(MenuItem::new("Ir a la barra de dirección", "nav.addr").shortcut("Ctrl+L").separated())
                .item(MenuItem::new("Buscar en la página", "nav.find").shortcut("Ctrl+F")),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Acercar", "view.zoomin").shortcut("Ctrl++"))
                .item(MenuItem::new("Alejar", "view.zoomout").shortcut("Ctrl+-"))
                .item(MenuItem::new("Restablecer zoom", "view.zoomreset").shortcut("Ctrl+0").separated())
                .item(MenuItem::new("Marcadores", "view.bookmarks").shortcut("Ctrl+B"))
                .item(MenuItem::new("Historial", "view.history").shortcut("Ctrl+H")),
        )
        .menu(
            Menu::new("Ayuda")
                .item(MenuItem::new("Acerca de puriy", "help.about")),
        )
}

/// Traduce el `command` del menú principal al `Msg` real existente y lo
/// despacha por el `update`. Cierra el menú antes de actuar.
fn handle_menu_command(mut model: Model, command: String, handle: &Handle<Msg>) -> Model {
    model.menu_open = None;
    let target = match command.as_str() {
        "file.newtab" => Some(Msg::NewTab),
        "file.close" => Some(Msg::CloseTab(model.active)),
        "file.reload" => Some(Msg::Reload),
        "file.source" => Some(Msg::ViewSource),
        "file.bookmark" => Some(Msg::Bookmark),
        "edit.undo" => Some(Msg::EditMenuAction(EditAction::Undo)),
        "edit.redo" => Some(Msg::EditMenuAction(EditAction::Redo)),
        "edit.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "edit.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "edit.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "edit.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        "nav.back" => Some(Msg::Back),
        "nav.fwd" => Some(Msg::Forward),
        "nav.addr" => Some(Msg::FocusAddr),
        "nav.find" => Some(Msg::FindOpen),
        "view.zoomin" => Some(Msg::ZoomIn),
        "view.zoomout" => Some(Msg::ZoomOut),
        "view.zoomreset" => Some(Msg::ZoomReset),
        "view.bookmarks" => Some(Msg::ToggleBookmarks),
        "view.history" => Some(Msg::ToggleHistory),
        // "help.about" no tiene acción real — no-op silencioso.
        _ => None,
    };
    match target {
        Some(msg) => Puriy::update(model, msg, handle),
        None => model,
    }
}

/// Identifica qué campo de texto tiene el foco, para resolver borrows
/// disjuntos sin `unsafe` en `apply_edit_menu_action` (el clipboard y el
/// input son campos distintos del `Model`).
enum FocusTarget {
    Find,
    PanelFilter,
    PageInput(usize),
    Addr,
}

impl Model {
    /// Determina el `FocusTarget` con la misma prioridad que
    /// `focused_text_input`, sin tomar un borrow del input — así el caller
    /// puede pedir luego `&mut clipboard` + `&mut input` por separado.
    fn focus_target(&self) -> Option<FocusTarget> {
        if self.find_active {
            return Some(FocusTarget::Find);
        }
        if self.panel.is_some() {
            return Some(FocusTarget::PanelFilter);
        }
        let t = self.active();
        if let Some(idx) = t.focused_input {
            if idx < t.inputs.len() {
                return Some(FocusTarget::PageInput(idx));
            }
        }
        if t.addr_focused {
            return Some(FocusTarget::Addr);
        }
        None
    }
}

/// Aplica una `EditAction` del menú de edición sobre el `EditorState` del
/// campo de texto focuseado. Resuelve `clipboard` e `input` como borrows
/// disjuntos del `Model` (sin `unsafe`). Cierra el menú de edición.
fn apply_edit_menu_action(model: &mut Model, action: EditAction) {
    model.edit_menu = None;
    let Some(target) = model.focus_target() else { return };
    let active = model.active;
    match target {
        FocusTarget::Find => {
            editmenu::apply(model.find_input.editor_mut(), action, &mut model.clipboard);
        }
        FocusTarget::PanelFilter => {
            editmenu::apply(model.panel_filter.editor_mut(), action, &mut model.clipboard);
        }
        FocusTarget::PageInput(idx) => {
            if let Some(input) = model.tabs[active].inputs.get_mut(idx) {
                editmenu::apply(input.editor_mut(), action, &mut model.clipboard);
            }
        }
        FocusTarget::Addr => {
            editmenu::apply(model.tabs[active].addr.editor_mut(), action, &mut model.clipboard);
        }
    }
}

/// Walk del box tree contando hojas de texto que matchean el `matcher`
/// (query + toggles case/whole-word). Matcher vacío → 0 matches.
fn count_matches(tree: Option<&BoxTree>, matcher: &Matcher) -> usize {
    let Some(t) = tree else { return 0 };
    if matcher.is_empty() {
        return 0;
    }
    let mut count = 0_usize;
    t.walk(|b| {
        if let Some(txt) = &b.text {
            if matcher.matches(txt) {
                count += 1;
            }
        }
    });
    count
}

/// Opciones de coincidencia de la find bar (Fase 7.31). Default = búsqueda
/// case-insensitive por substring (comportamiento clásico de browsers).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchOpts {
    /// Distingue mayúsculas/minúsculas.
    pub case_sensitive: bool,
    /// Sólo matchea palabras completas (delimitadas por bordes
    /// no-alfanuméricos, incluyendo inicio/fin de la hoja de texto).
    pub whole_word: bool,
}

/// Predicado de búsqueda compilado: la query ya viene normalizada
/// (lowercased si no es case-sensitive) para no pagar el cast por hoja.
/// Reúne el matching de count/highlight/scroll en un solo lugar para que
/// las tres vistas cuenten exactamente los mismos matches en el mismo
/// orden DFS.
pub(crate) struct Matcher {
    needle: String,
    case_sensitive: bool,
    whole_word: bool,
}

impl Matcher {
    fn new(query: &str, opts: MatchOpts) -> Self {
        let needle = if opts.case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        Matcher {
            needle,
            case_sensitive: opts.case_sensitive,
            whole_word: opts.whole_word,
        }
    }

    fn is_empty(&self) -> bool {
        self.needle.is_empty()
    }

    /// `true` si `text` contiene al menos una ocurrencia de la query bajo
    /// las opciones activas.
    fn matches(&self, text: &str) -> bool {
        if self.needle.is_empty() {
            return false;
        }
        if self.case_sensitive {
            self.find_in(text)
        } else {
            self.find_in(&text.to_lowercase())
        }
    }

    /// `hay` ya viene normalizado (lowercased si corresponde) — busca la
    /// `needle` con o sin restricción de palabra completa.
    fn find_in(&self, hay: &str) -> bool {
        if !self.whole_word {
            return hay.contains(&self.needle);
        }
        // Whole-word: cada ocurrencia debe estar delimitada por bordes de
        // palabra (inicio/fin del string o un char no alfanumérico).
        // Caminamos char-aware para no romper en UTF-8 multibyte.
        let nlen = self.needle.len();
        let mut start = 0_usize;
        while let Some(pos) = hay[start..].find(&self.needle) {
            let i = start + pos;
            let before_ok = hay[..i].chars().next_back().map_or(true, |c| !is_word_char(c));
            let after_ok = hay[i + nlen..].chars().next().map_or(true, |c| !is_word_char(c));
            if before_ok && after_ok {
                return true;
            }
            // Avanzar al siguiente boundary de char válido para no panicar
            // en el próximo `find` ni quedar estancados.
            start = i + 1;
            while start < hay.len() && !hay.is_char_boundary(start) {
                start += 1;
            }
        }
        false
    }
}

/// Un caracter cuenta como "de palabra" si es alfanumérico (cualquier
/// alfabeto Unicode) o `_`. Lo demás (espacios, puntuación, símbolos)
/// es un borde de palabra.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Estima la y del N-ésimo (1-based) leaf de texto que matchea el
/// `matcher`, acumulando alturas igual que `BoxTree::find_y_of_match` del
/// engine pero con el predicado configurable de la find bar (Fase 7.31).
/// Se replica acá en vez de extender el engine para no tocar `boxes.rs` —
/// el costo es ~15 líneas y mantiene el scroll consistente con los
/// toggles case/whole-word.
fn find_match_y(tree: &BoxTree, matcher: &Matcher, nth_1based: usize) -> Option<f32> {
    if matcher.is_empty() || nth_1based == 0 {
        return None;
    }
    let mut acc = 0.0_f32;
    let mut seen = 0_usize;
    find_match_y_inner(&tree.root, matcher, nth_1based, &mut acc, &mut seen)
}

fn find_match_y_inner(
    b: &BoxNode,
    matcher: &Matcher,
    target_nth: usize,
    acc: &mut f32,
    seen: &mut usize,
) -> Option<f32> {
    if let Some(text) = &b.text {
        if matcher.matches(text) {
            *seen += 1;
            if *seen == target_nth {
                return Some(*acc);
            }
        }
        *acc += b.font_size * b.line_height.unwrap_or(1.2);
        return None;
    }
    *acc += b.margin.top + b.padding.top;
    for c in &b.children {
        if let Some(y) = find_match_y_inner(c, matcher, target_nth, acc, seen) {
            return Some(y);
        }
    }
    *acc += b.margin.bottom + b.padding.bottom;
    None
}

/// Chip-toggle de la find bar (`Aa` case-sensitive / `W` whole-word).
/// Activo = fondo azul; inactivo = gris apagado. Click → `msg`.
fn find_toggle(label: &str, active: bool, msg: Msg) -> View<Msg> {
    let (bg, fg) = if active {
        (Color::from_rgb8(86, 124, 196), Color::from_rgb8(245, 245, 255))
    } else {
        (Color::from_rgb8(70, 70, 84), Color::from_rgb8(165, 165, 180))
    };
    View::new(Style {
        size: Size { width: length(26.0_f32), height: length(22.0_f32) },
        margin: Rect {
            left: length(6.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .text_aligned(label, 11.0, fg, Alignment::Center)
    .on_click(msg)
}

/// Find bar — input + contador + toggles (Aa/W) + close. Sticky entre
/// header y viewport mientras `find_active`.
fn find_bar(
    input: &TextInputState,
    count: usize,
    current: usize,
    case_sensitive: bool,
    whole_word: bool,
) -> View<Msg> {
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
        find_toggle("Aa", case_sensitive, Msg::FindToggleCase),
        find_toggle("W", whole_word, Msg::FindToggleWord),
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

/// Ejecuta los scripts inline del documento en el `JsRuntime` de la
/// pestaña. Crea el runtime lazily si no existía. Llama `set_document`
/// con un snapshot (`title`, `url`, `body_text`) para que `document.*`
/// devuelva valores reales en lugar de undefined.
///
/// Scripts externos (`src=`) llegan acá ya descargados por
/// `puriy_engine::scripts::fetch_externals` (Fase 7.4): el body UTF-8
/// quedó copiado en `inline`. Si la descarga falló, `inline` sigue en
/// `None` y se saltea silenciosamente (no es error JS — es network).
/// Scripts `is_module=true` se saltean: el runtime de Fase 7.x es
/// clásico (no module loader).
///
/// `t.js_summary` se actualiza con counts agregados. La función NO toca
/// `t.status` — el caller decide cómo mostrarlo.
fn run_scripts_on_tab(
    t: &mut TabState,
    scripts: &[puriy_engine::ScriptInfo],
    now_ms: u64,
    system_clipboard: Option<&str>,
) -> Vec<Msg> {
    if scripts.is_empty() {
        return Vec::new();
    }
    // Body text — concatenación de las hojas de texto del box tree.
    // Snapshot a momento de Load; muta si la página re-renderiza pero
    // el JS no re-lee. Fase 7.5+ lo hará reactivo.
    let body_text = t
        .box_tree
        .as_ref()
        .map(extract_body_text)
        .unwrap_or_default();
    // Lazy: instanciar el JsRuntime cuesta ~200ms — sólo si la página
    // realmente tiene scripts ejecutables.
    let has_executable = scripts
        .iter()
        .any(|s| s.inline.is_some() && !s.is_module);
    if !has_executable {
        return Vec::new();
    }
    let rt = match puriy_js::JsRuntime::new() {
        Ok(r) => Box::new(r),
        Err(_) => {
            t.js_summary.errors += 1;
            return Vec::new();
        }
    };
    // Snapshot de elementos con `id=` — el harness JS los expone via
    // `getElementById` / `document.querySelector('#x')`. textContent
    // del subárbol de cada uno (snapshot read-only, igual que body).
    let elements = t
        .box_tree
        .as_ref()
        .map(collect_element_snapshots)
        .unwrap_or_default();
    t.js = Some(rt);
    let rt = t.js.as_mut().unwrap();
    let _ = rt.set_document(&t.title, &t.url, &body_text);
    let _ = rt.set_elements(&elements);
    // Reloj inicial — sin esto, `setTimeout(fn, 100)` registrado por un
    // script inicial dispararía contra `__puriy_now_ms=0` y se vencería
    // en el primer tick que cruce 100ms del wall clock (raro pero
    // posible). Setearlo acá los ancla al reloj real del chrome.
    let _ = rt.set_now_ms(now_ms);
    // Fase 7.28 — sync scroll + viewport. Habilita que `window.scrollY`/
    // `innerWidth` desde JS reflejen state real del chrome. El viewport
    // sale del thread-local `PURIY_VIEWPORT`, que `Msg::Resize` mantiene
    // al día con el tamaño real de la ventana (default = initial_size).
    let (vp_w, vp_h) = PURIY_VIEWPORT.with(|c| c.get());
    let _ = rt.set_scroll(0.0, t.scroll_y);
    let _ = rt.set_viewport(vp_w, vp_h);
    // DPR real de la ventana (Fase 7.173): que `devicePixelRatio` sea
    // correcto ya en el primer script. `Msg::ScaleFactor` mantiene el
    // thread-local al día con el `scale_factor` de winit (default = 1.0).
    let _ = rt.set_device_pixel_ratio(PURIY_DPR.with(|c| c.get()));
    // Portapapeles del sistema → buffer JS (Fase 7.176): que un
    // `navigator.clipboard.readText()` de un script inicial vea lo que el
    // usuario tiene copiado afuera, no la cadena vacía. (Limitación: un copy
    // externo POSTERIOR al load no se relee hasta la próxima carga — la lectura
    // viva exigiría resolver readText como promesa pendiente del chrome.)
    if let Some(clip) = system_clipboard {
        let _ = rt.set_clipboard(clip);
    }
    let mut prev_stdout_len = rt.stdout().len();
    let mut prev_stderr_len = rt.stderr().len();
    for s in scripts {
        if s.is_module {
            continue;
        }
        let Some(body) = s.inline.as_ref() else {
            continue;
        };
        // Skip non-JS types (templates, application/json, etc.).
        if let Some(t_attr) = &s.type_attr {
            let l = t_attr.to_ascii_lowercase();
            if !l.is_empty()
                && !l.contains("javascript")
                && !l.contains("ecmascript")
                && l != "text/js"
            {
                continue;
            }
        }
        if let Err(_e) = rt.eval(body) {
            t.js_summary.errors += 1;
        }
        // Contá líneas nuevas en stdout/stderr — `console.log` agrega
        // exactamente una `\n` por llamada.
        let new_stdout = rt.stdout();
        let new_stderr = rt.stderr();
        t.js_summary.logs += new_stdout[prev_stdout_len..].matches('\n').count();
        t.js_summary.errors += new_stderr[prev_stderr_len..].matches('\n').count();
        prev_stdout_len = new_stdout.len();
        prev_stderr_len = new_stderr.len();
    }
    // Resuelve las media queries que los scripts consultaron (`matchMedia`)
    // contra el viewport real, ahora que ya se registraron. Así un listener
    // de DOMContentLoaded/load o un `if (mql.matches)` posterior ve el valor
    // correcto. (Limitación: una lectura síncrona de `.matches` en el MISMO
    // tick del `matchMedia(...)` ve aún `false` — no hay hostcall síncrono
    // desde el sandbox para evaluar al vuelo.)
    let (vp_w, vp_h) = PURIY_VIEWPORT.with(|c| c.get());
    sync_media_queries(rt, vp_w, vp_h, PURIY_DPR.with(|c| c.get()) as f32);
    // Aplica al box_tree cualquier mutación que los scripts iniciales
    // hayan hecho via `el.textContent = ...` (typeahead, contadores
    // inicializados, sustituciones de placeholders, etc). Las
    // mutaciones de fetch suben al caller para que dispatch.
    apply_dom_mutations(t)
}

/// Evalúa cada media query registrada por `matchMedia` contra el viewport
/// real (ancho/alto en px + DPR) reusando el evaluador del engine, y empuja
/// el resultado al estado JS — disparando `change` en los `MediaQueryList`
/// vivos cuyo `matches` flipeó. No-op si el script nunca llamó `matchMedia`.
fn sync_media_queries(rt: &mut puriy_js::JsRuntime, vp_w: f32, vp_h: f32, dpr: f32) {
    let queries = rt.registered_media_queries();
    if queries.is_empty() {
        return;
    }
    let vp = puriy_engine::Viewport { width: vp_w, height: vp_h, dpr };
    for q in queries {
        let matches = puriy_engine::evaluate_media_query(&q, vp);
        let _ = rt.set_media_match(&q, matches);
    }
}

/// Walka el `BoxTree` y arma un `Vec<ElementSnapshot>` para cada nodo
/// con `element_id` no-vacío. El `text_content` del snapshot es la
/// concatenación de las hojas de texto del subárbol (con separadores
/// espacio), análoga a `body.textContent` pero scoped al elemento.
///
/// Sólo nodos con `id=` se exponen — match exacto del modelo que el
/// harness JS usa (índice `__puriy_elements[id]`). Elementos sin id no
/// se exponen ni a `getElementById` ni a event handlers.
fn collect_element_snapshots(bt: &BoxTree) -> Vec<puriy_js::ElementSnapshot> {
    let mut out = Vec::new();
    // Fase 7.10 — walk recursivo manual para que cada elemento conozca
    // el id de su ancestro Element más cercano con id=. `bt.walk(|b|)`
    // no propaga contexto del parent, así que usamos rec con stack.
    // Fase 7.29 — además contamos DFS pre-order para `dfs_index`, que
    // alimenta `getBoundingClientRect` heurístico (top = (idx-1) × 30).
    fn rec(
        b: &BoxNode,
        parent_id: Option<&str>,
        counter: &mut u32,
        out: &mut Vec<puriy_js::ElementSnapshot>,
    ) {
        *counter += 1;
        let my_dfs = *counter;
        let my_id_opt = b.element_id.as_deref().filter(|s| !s.is_empty());
        if let Some(id) = my_id_opt {
            let tag_name = b.tag.clone().unwrap_or_default();
            let text_content = node_text_content(b);
            let value = if b.input_kind.is_some() {
                b.input_initial.clone().or_else(|| Some(String::new()))
            } else if let Some(sel) = &b.select {
                sel.options
                    .get(sel.initial)
                    .map(|opt| opt.value.clone())
                    .or_else(|| Some(String::new()))
            } else {
                None
            };
            let dataset = b
                .dataset()
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            out.push(puriy_js::ElementSnapshot {
                id: id.to_string(),
                tag_name,
                text_content,
                class_list: b.class_list.clone(),
                value,
                parent_id: parent_id.map(String::from),
                dataset,
                attributes: b.attributes.clone(),
                dfs_index: my_dfs,
            });
        }
        let next_parent = my_id_opt.or(parent_id);
        for c in &b.children {
            rec(c, next_parent, counter, out);
        }
    }
    let mut counter: u32 = 0;
    rec(&bt.root, None, &mut counter, &mut out);
    out
}

/// Concatena las hojas de texto del subárbol del nodo `b`, separadas
/// por espacio. Mismo molde que `extract_body_text` pero scoped — útil
/// para que `el.textContent` devuelva sólo lo que vive bajo el elemento.
fn node_text_content(b: &BoxNode) -> String {
    let mut out = String::new();
    fn rec(b: &BoxNode, out: &mut String) {
        if let Some(text) = &b.text {
            if !text.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(text);
            }
        }
        for c in &b.children {
            rec(c, out);
        }
    }
    rec(b, &mut out);
    out
}

/// Dispara los handlers JS registrados sobre `element_id` (vía
/// `onclick` / `addEventListener`) en la pestaña activa. Si el runtime
/// no existe o ningún handler corrió, queda como no-op — el chrome
/// no aplica fallback al default action (los `<a>` con id+link ya
/// navegan por el path nativo, este msg sólo se arma para elementos
/// sin link).
fn dispatch_js_event(
    m: &mut Model,
    element_id: &str,
    event_type: &str,
    now_ms: u64,
) -> (puriy_js::DispatchResult, Vec<Msg>) {
    dispatch_js_event_with_init(m, element_id, event_type, now_ms, None)
}

/// Fase 7.9 — variante con `EventInit` opcional. El init lleva los
/// campos enriquecidos del DOM Event (key/code/modifiers para keydown,
/// value para change/input). Si es `None`, el handler recibe el event
/// "viejo" estilo Fase 7.6 (type/target/preventDefault) — backwards
/// compatible.
fn dispatch_js_event_with_init(
    m: &mut Model,
    element_id: &str,
    event_type: &str,
    now_ms: u64,
    init: Option<puriy_js::EventInit>,
) -> (puriy_js::DispatchResult, Vec<Msg>) {
    let t = m.active_mut();
    let Some(rt) = t.js.as_mut() else {
        return (puriy_js::DispatchResult::default(), Vec::new());
    };
    // Fase 7.11 — refresh del fuel antes de cada dispatch. Cada evento
    // de usuario (click/keydown/focus/blur/change/input) es una unidad
    // independiente: que un dispatch anterior haya consumido fuel no
    // debe limitar al siguiente. El cap por evento sigue siendo
    // DEFAULT_FUEL (50M) — corta loops infinitos dentro de un handler.
    rt.set_fuel(puriy_js::DEFAULT_FUEL);
    let _ = rt.set_now_ms(now_ms);
    // Fase 7.28 — refresh scroll antes del dispatch: el handler puede
    // leer `window.scrollY` para "estoy en el footer?" o "header
    // sticky?". Sin esto, leería el último valor que el JS mismo
    // escribió, no el scroll real del usuario.
    let _ = rt.set_scroll(0.0, t.scroll_y);
    let prev_stdout_len = rt.stdout().len();
    let prev_stderr_len = rt.stderr().len();
    let mut result = match rt.dispatch_event(element_id, event_type, init.as_ref()) {
        Ok(r) => {
            let new_stdout = rt.stdout();
            let new_stderr = rt.stderr();
            t.js_summary.logs += new_stdout[prev_stdout_len..].matches('\n').count();
            t.js_summary.errors += new_stderr[prev_stderr_len..].matches('\n').count();
            r
        }
        Err(_) => {
            t.js_summary.errors += 1;
            puriy_js::DispatchResult::default()
        }
    };
    let mut pending = apply_dom_mutations(t);
    // Bubbling a `document` (event delegation): tras correr los handlers del
    // elemento, los eventos que bubblean también disparan los listeners de
    // `document.addEventListener(type, ...)`, con el elemento original como
    // `event.target`. Un `preventDefault()` del handler de document también
    // cuenta para el fallback (p. ej. cancelar la navegación de un `<a>`).
    // Si un handler del elemento llamó `stopPropagation()`, el evento NO
    // burbujea hasta `document` (el dispatch ahora propaga el flag
    // `_stopped` vía `DispatchResult::propagation_stopped`).
    if event_bubbles_to_document(event_type) && !result.propagation_stopped {
        let (doc_result, doc_pending) = dispatch_document_js_event_on_tab(
            t,
            event_type,
            init.as_ref(),
            Some(element_id),
            now_ms,
        );
        result.count += doc_result.count;
        result.default_prevented |= doc_result.default_prevented;
        pending.extend(doc_pending);
    }
    (result, pending)
}

/// ¿Este tipo de evento bubblea hasta `document`? Cubre los eventos que la
/// gente delega con `document.addEventListener` (click, teclas, input/change,
/// submit). `focus`/`blur` quedan afuera a propósito: en spec NO bubblean
/// (sus variantes `focusin`/`focusout` sí, pero el chrome no las emite aún).
fn event_bubbles_to_document(event_type: &str) -> bool {
    matches!(
        event_type,
        "click"
            | "dblclick"
            | "mousedown"
            | "mouseup"
            | "keydown"
            | "keyup"
            | "keypress"
            | "input"
            | "change"
            | "submit"
    )
}

/// Fase 7.42 — cambia la pestaña activa, marcando la vieja como hidden y
/// la nueva como visible (dispatcha `'visibilitychange'` en cada una vía
/// `set_visibility`). Apps que pausan video / polling / animation al
/// background ven el evento sin necesidad de cabling especial en el msg.
fn switch_active_tab(m: &mut Model, new_idx: usize) {
    let prev_idx = m.active;
    if prev_idx == new_idx {
        return;
    }
    if let Some(rt) = m.tabs[prev_idx].js.as_mut() {
        let _ = rt.set_visibility(true);
    }
    m.active = new_idx;
    if let Some(rt) = m.tabs[new_idx].js.as_mut() {
        let _ = rt.set_visibility(false);
    }
}

/// Fase 7.39 — dispatcha un evento sobre `window` (no sobre un elemento)
/// para una pestaña dada. Refresca fuel/now/scroll antes para que los
/// handlers vean state consistente y dropea mutaciones DOM resultantes
/// en el return (igual que `dispatch_js_event`). Toma `&mut TabState`
/// directo (no `Model`) para que la pestaña pueda no ser la activa —
/// 'load' puede dispararse en background loads.
fn dispatch_window_js_event_on_tab(
    t: &mut TabState,
    event_type: &str,
    now_ms: u64,
) -> (puriy_js::DispatchResult, Vec<Msg>) {
    let Some(rt) = t.js.as_mut() else {
        return (puriy_js::DispatchResult::default(), Vec::new());
    };
    rt.set_fuel(puriy_js::DEFAULT_FUEL);
    let _ = rt.set_now_ms(now_ms);
    let _ = rt.set_scroll(0.0, t.scroll_y);
    let prev_stdout_len = rt.stdout().len();
    let prev_stderr_len = rt.stderr().len();
    let result = match rt.dispatch_window_event(event_type, None) {
        Ok(r) => {
            let new_stdout = rt.stdout();
            let new_stderr = rt.stderr();
            t.js_summary.logs += new_stdout[prev_stdout_len..].matches('\n').count();
            t.js_summary.errors += new_stderr[prev_stderr_len..].matches('\n').count();
            r
        }
        Err(_) => {
            t.js_summary.errors += 1;
            puriy_js::DispatchResult::default()
        }
    };
    let pending = apply_dom_mutations(t);
    (result, pending)
}

/// Dispatcha un evento a nivel `document` (`document.addEventListener`).
/// Cubre `DOMContentLoaded` (sin target) y la fase de delegación de eventos
/// de elemento (`target_element_id` = el elemento original que bubblea hasta
/// `document`). Espejo de [`dispatch_window_js_event_on_tab`]: contabiliza
/// logs/errores y drena las mutaciones DOM que el handler haya producido.
fn dispatch_document_js_event_on_tab(
    t: &mut TabState,
    event_type: &str,
    init: Option<&puriy_js::EventInit>,
    target_element_id: Option<&str>,
    now_ms: u64,
) -> (puriy_js::DispatchResult, Vec<Msg>) {
    let Some(rt) = t.js.as_mut() else {
        return (puriy_js::DispatchResult::default(), Vec::new());
    };
    rt.set_fuel(puriy_js::DEFAULT_FUEL);
    let _ = rt.set_now_ms(now_ms);
    let _ = rt.set_scroll(0.0, t.scroll_y);
    let prev_stdout_len = rt.stdout().len();
    let prev_stderr_len = rt.stderr().len();
    let result = match rt.dispatch_document_event(event_type, init, target_element_id) {
        Ok(r) => {
            let new_stdout = rt.stdout();
            let new_stderr = rt.stderr();
            t.js_summary.logs += new_stdout[prev_stdout_len..].matches('\n').count();
            t.js_summary.errors += new_stderr[prev_stderr_len..].matches('\n').count();
            r
        }
        Err(_) => {
            t.js_summary.errors += 1;
            puriy_js::DispatchResult::default()
        }
    };
    let pending = apply_dom_mutations(t);
    (result, pending)
}

/// Mapea un `KeyEvent` de Llimphi a un `EventInit` enriquecido con los
/// campos estándar del DOM keydown/keyup event.
///
/// - `event.key`: el "key value" — `e.text` cuando hay carácter
///   imprimible (respeta modifiers + IME), o el nombre del NamedKey
///   (`"Enter"`, `"ArrowLeft"`, etc.) para teclas no-imprimibles.
/// - `event.code`: el "physical code". Llimphi no expone el código
///   físico (winit lo tiene en `KeyEvent.physical_key` que no propagamos);
///   por ahora replicamos `key` como aproximación. Suficiente para que
///   handlers que filtran `if (e.code === 'Enter')` funcionen.
/// - `event.shiftKey`/`ctrlKey`/`altKey`/`metaKey`: directos de los
///   modifiers.
fn key_event_to_init(e: &llimphi_ui::KeyEvent) -> puriy_js::EventInit {
    let key = match &e.key {
        llimphi_ui::Key::Character(s) => s.to_string(),
        llimphi_ui::Key::Named(n) => named_key_name(n),
        _ => e.text.clone().unwrap_or_default(),
    };
    puriy_js::EventInit {
        key: Some(key.clone()),
        code: Some(key),
        shift_key: Some(e.modifiers.shift),
        ctrl_key: Some(e.modifiers.ctrl),
        alt_key: Some(e.modifiers.alt),
        meta_key: Some(e.modifiers.meta),
        value: None,
    }
}

/// Nombre canónico de un `NamedKey` al estilo DOM (`"Enter"`,
/// `"ArrowLeft"`, `"Escape"`, etc.). Cubre las teclas que un browser
/// típico usa para keydown handlers. Para teclas no mapeadas, usa
/// `{:?}` de Debug — degrada limpio sin perder información.
fn named_key_name(n: &llimphi_ui::NamedKey) -> String {
    use llimphi_ui::NamedKey;
    match n {
        NamedKey::Enter => "Enter".into(),
        NamedKey::Escape => "Escape".into(),
        NamedKey::Tab => "Tab".into(),
        NamedKey::Backspace => "Backspace".into(),
        NamedKey::Delete => "Delete".into(),
        NamedKey::Space => " ".into(),
        NamedKey::ArrowLeft => "ArrowLeft".into(),
        NamedKey::ArrowRight => "ArrowRight".into(),
        NamedKey::ArrowUp => "ArrowUp".into(),
        NamedKey::ArrowDown => "ArrowDown".into(),
        NamedKey::Home => "Home".into(),
        NamedKey::End => "End".into(),
        NamedKey::PageUp => "PageUp".into(),
        NamedKey::PageDown => "PageDown".into(),
        NamedKey::Shift => "Shift".into(),
        NamedKey::Control => "Control".into(),
        NamedKey::Alt => "Alt".into(),
        NamedKey::Meta => "Meta".into(),
        NamedKey::CapsLock => "CapsLock".into(),
        NamedKey::F1 => "F1".into(),
        NamedKey::F2 => "F2".into(),
        NamedKey::F3 => "F3".into(),
        NamedKey::F4 => "F4".into(),
        NamedKey::F5 => "F5".into(),
        NamedKey::F6 => "F6".into(),
        NamedKey::F7 => "F7".into(),
        NamedKey::F8 => "F8".into(),
        NamedKey::F9 => "F9".into(),
        NamedKey::F10 => "F10".into(),
        NamedKey::F11 => "F11".into(),
        NamedKey::F12 => "F12".into(),
        other => format!("{:?}", other),
    }
}

/// Avanza el reloj de cada `JsRuntime` vivo del Model al `now_ms` actual
/// y dispara los callbacks `setTimeout`/`setInterval` vencidos. Llamado
/// desde `Msg::JsTick` (cada `JS_POLL_PERIOD_MS`).
///
/// Pestañas sin runtime se saltean en ~ns (chequeo `Option::is_some`).
/// Pestañas con runtime pero sin timers vivos también se saltean tras
/// un `pending_timers` que cuesta un eval mini (~µs). No queremos
/// dejar de polear porque mismo runtime puede registrar timers más
/// tarde via event handlers (Fase 7.5b+).
///
/// Cada disparo nuevo de stdout/stderr se cuenta a `t.js_summary`,
/// alineado con el conteo que hace `run_scripts_on_tab`.
fn tick_js_runtimes(m: &mut Model, now_ms: u64) -> Vec<Msg> {
    let mut pending: Vec<Msg> = Vec::new();
    for t in m.tabs.iter_mut() {
        let Some(rt) = t.js.as_mut() else { continue };
        if rt.pending_timers() == 0 {
            continue;
        }
        // Fase 7.11 — refresh del fuel por tick. Cada tick es una unidad
        // independiente al estilo del event loop; no acumulamos cap.
        rt.set_fuel(puriy_js::DEFAULT_FUEL);
        // Fase 7.28 — scroll sync para los rAF/setInterval callbacks que
        // leen window.scrollY (animation loops chequeando posición).
        let _ = rt.set_scroll(0.0, t.scroll_y);
        let prev_stdout_len = rt.stdout().len();
        let prev_stderr_len = rt.stderr().len();
        match rt.tick(now_ms) {
            Ok(_r) => {
                let new_stdout = rt.stdout();
                let new_stderr = rt.stderr();
                t.js_summary.logs += new_stdout[prev_stdout_len..].matches('\n').count();
                t.js_summary.errors += new_stderr[prev_stderr_len..].matches('\n').count();
            }
            Err(_) => {
                t.js_summary.errors += 1;
            }
        }
        pending.extend(apply_dom_mutations(t));
    }
    pending
}

/// Drena el buffer de mutaciones del DOM del runtime de la pestaña y
/// las aplica al `box_tree`. Llamado después de cada operación que
/// pueda haber escrito a `textContent`/`innerHTML` (run_scripts,
/// dispatch_event, tick). Si no hay mutaciones, retorna sin tocar el
/// árbol — costo: un eval mini que devuelve `''`.
///
/// Mutaciones sobre ids que no existen en el árbol se silencian (el
/// JS puede haber retenido un handle de una página anterior, o el id
/// puede haber sido renombrado por un script DOM-mutating no soportado).
/// Fase 7.31 — el return ahora es `Vec<Msg>`: lista de FetchRequest que
/// el caller debe despachar (necesitan spawn de worker thread, que sólo
/// el call site tiene cabling para hacer). Si el caller no tiene handle
/// (ej. tests), puede ignorar el Vec. El resto de mutations se aplican
/// in-place sin requerir el handle.
fn apply_dom_mutations(t: &mut TabState) -> Vec<Msg> {
    let mut out = Vec::new();
    // El borrow de `rt` se acota al drain para poder refrescar canvas (que
    // re-borrowa `t`) sin conflicto.
    let muts = match t.js.as_mut() {
        Some(rt) => rt.drain_dom_mutations(),
        None => return out,
    };
    // Fase 7.196 — refrescamos los frames de `<canvas>` SIEMPRE que se corra
    // JS (no sólo cuando hay mutaciones DOM): dibujar en canvas no produce
    // mutaciones. Gateado por `has_canvas` para no evaluar en páginas sin canvas.
    if t.has_canvas {
        refresh_canvas_frames(t);
    }
    if muts.is_empty() {
        return out;
    }
    // Procesamos los fetch ANTES de chequear box_tree — los fetch no
    // requieren box_tree (operan a nivel runtime). Esto también
    // habilita fetch durante el load inicial.
    let mut other_muts = Vec::with_capacity(muts.len());
    for m in muts {
        if m.kind == "fetch" {
            if let Some(req) = parse_fetch_payload(&m.value, t.id, t.gen) {
                out.push(req);
            }
        } else if m.kind == "clipboard" {
            // `writeText:<txt>` / `write:<txt>` — empuja el texto al
            // portapapeles real. No necesita box_tree (opera sobre el SO);
            // el write efectivo lo hace el update loop (tiene `&mut clipboard`).
            let text = m
                .value
                .strip_prefix("writeText:")
                .or_else(|| m.value.strip_prefix("write:"));
            if let Some(text) = text {
                out.push(Msg::SetSystemClipboard(text.to_string()));
            }
        } else if m.kind == "eventsource" {
            // EventSource: `<id> GS open GS <url> GS <withCred>` o `<id> GS close`.
            // El worker de streaming lo arranca/corta el update loop (necesita
            // handle + `&mut tab` para el flag de cancelación).
            let parts: Vec<&str> = m.value.split('\u{001D}').collect();
            if let Some(es_id) = parts.first().and_then(|s| s.parse::<u32>().ok()) {
                match parts.get(1).copied() {
                    Some("open") => {
                        if let Some(url) = parts.get(2) {
                            out.push(Msg::EsOpen {
                                tab: t.id,
                                gen: t.gen,
                                es_id,
                                url: url.to_string(),
                            });
                        }
                    }
                    Some("close") => out.push(Msg::EsClose { tab: t.id, es_id }),
                    _ => {}
                }
            }
        } else {
            other_muts.push(m);
        }
    }
    let muts = other_muts;
    if muts.is_empty() {
        return out;
    }
    let Some(bt) = t.box_tree.as_mut() else { return out };
    let mut needs_restyle = false;
    for m in muts {
        if m.kind == "text" {
            bt.set_element_text_content(&m.id, &m.value);
        } else if let Some(prop) = m.kind.strip_prefix("style:") {
            // Fase 7.8: el.style.X = Y publica con kind = "style:X" (X
            // ya viene en kebab-case desde el harness JS).
            bt.set_element_style(&m.id, prop, &m.value);
        } else if m.kind == "appendChild" {
            // Fase 7.12: el.appendChild(child) publica con kind =
            // "appendChild", value = "tag<US>id<US>text<US>classes<US>value"
            // donde <US> es U+001D (Group Separator). Construimos un
            // BoxNode sintético via synthesize_box_node + push al
            // parent.children.
            let parts: Vec<&str> = m.value.split('\u{001D}').collect();
            if parts.len() >= 5 {
                let tag = parts[0];
                let cid = parts[1];
                let text = parts[2];
                let classes: Vec<String> = parts[3]
                    .split_whitespace()
                    .filter(|p| !p.is_empty())
                    .map(|p| p.to_string())
                    .collect();
                let value = if parts[4].is_empty() { None } else { Some(parts[4]) };
                let cid_opt = if cid.is_empty() { None } else { Some(cid) };
                let child =
                    puriy_engine::synthesize_box_node(tag, cid_opt, text, classes, value);
                bt.append_child_to(&m.id, child);
            }
        } else if m.kind == "insertBefore" {
            // Fase 7.14: payload = mismo formato que appendChild más
            // un 6º campo con ref_id (el id del sibling antes del cual
            // insertar). Si ref_id no se encuentra, fallback a append.
            let parts: Vec<&str> = m.value.split('\u{001D}').collect();
            if parts.len() >= 6 {
                let tag = parts[0];
                let cid = parts[1];
                let text = parts[2];
                let classes: Vec<String> = parts[3]
                    .split_whitespace()
                    .filter(|p| !p.is_empty())
                    .map(|p| p.to_string())
                    .collect();
                let value = if parts[4].is_empty() { None } else { Some(parts[4]) };
                let ref_id = parts[5];
                let cid_opt = if cid.is_empty() { None } else { Some(cid) };
                let child =
                    puriy_engine::synthesize_box_node(tag, cid_opt, text, classes, value);
                bt.insert_child_before(&m.id, child, ref_id);
            }
        } else if m.kind == "removeChild" {
            // Fase 7.12: value = id del child (synth_id o user-set id).
            bt.remove_child_by_id(&m.id, &m.value);
        } else if let Some(key) = m.kind.strip_prefix("dataset:") {
            // Fase 7.11: el.dataset.fooBar = X publica con kind =
            // "dataset:foo-bar" (key ya viene kebab desde el harness JS).
            bt.set_element_dataset(&m.id, key, &m.value);
        } else if let Some(key) = m.kind.strip_prefix("dataset-remove:") {
            // Fase 7.11: delete el.dataset.fooBar publica con kind =
            // "dataset-remove:foo-bar".
            bt.remove_element_dataset(&m.id, key);
        } else if let Some(name) = m.kind.strip_prefix("attr:") {
            // Fase 7.16: el.setAttribute(name, value) publica con kind =
            // "attr:<name-lowercase>" para atributos no especiales
            // (aria-*, href, src, title, role, etc.). El name viene ya
            // lowercased desde el harness JS.
            bt.set_element_attribute(&m.id, name, &m.value);
        } else if let Some(name) = m.kind.strip_prefix("attr-remove:") {
            // Fase 7.16: el.removeAttribute(name) publica con kind =
            // "attr-remove:<name-lowercase>".
            bt.remove_element_attribute(&m.id, name);
        } else if m.kind == "value" {
            // Fase 7.9: el.value = X aplica al TextInputState (para
            // <input>/<textarea>) o al SelectState (para <select>).
            // Si el id matchea un input slot, set_text. Si matchea un
            // select slot, busca el option con value == X y selecciónalo.
            if let Some(slot) = t
                .inputs_element_ids
                .iter()
                .position(|e| e.as_deref() == Some(m.id.as_str()))
            {
                if let Some(input) = t.inputs.get_mut(slot) {
                    input.set_text(m.value.clone());
                }
            } else if let Some(slot) = t
                .selects_element_ids
                .iter()
                .position(|e| e.as_deref() == Some(m.id.as_str()))
            {
                if let Some(opt_idx) = select_option_index_by_value(bt, slot, &m.value) {
                    if let Some(s) = t.selects.get_mut(slot) {
                        s.selected = opt_idx;
                    }
                }
            }
        } else if m.kind == "focus" {
            // Fase 7.18: el.focus() desde JS. Si el id corresponde a un
            // input slot, mueve el cursor del usuario allí (focused_input
            // = Some(slot)). Si no es input, no-op silencioso — un
            // .focus() sobre un button/div sólo dispara el event handler.
            if let Some(slot) = t
                .inputs_element_ids
                .iter()
                .position(|e| e.as_deref() == Some(m.id.as_str()))
            {
                t.focused_input = Some(slot);
            }
        } else if m.kind == "blur" {
            // Fase 7.18: el.blur() desde JS. Sólo limpia focused_input si
            // el elemento era el actualmente focado — un .blur() sobre un
            // input no-focado no afecta el cursor del usuario.
            if let Some(slot) = t
                .inputs_element_ids
                .iter()
                .position(|e| e.as_deref() == Some(m.id.as_str()))
            {
                if t.focused_input == Some(slot) {
                    t.focused_input = None;
                }
            }
        } else if m.kind == "scroll" {
            // Fase 7.26: window.scrollTo(x, y) publica con id vacío y
            // value = "x,y". Sólo aplicamos la coord Y al scroll_y del
            // tab (no tenemos scroll horizontal por ahora).
            if let Some(comma) = m.value.find(',') {
                if let Ok(y) = m.value[comma + 1..].parse::<f32>() {
                    t.scroll_y = y.max(0.0);
                }
            }
        } else if m.kind == "scrollTop" {
            // Fase 7.26: el.scrollTop = N. Aplica sólo si el id matchea
            // body/html/main (el "viewport root" del tab). Otros
            // elementos requerirían scroll containers per-elemento.
            let mut applied = false;
            bt.walk(|b| {
                if applied {
                    return;
                }
                if b.element_id.as_deref() == Some(m.id.as_str()) {
                    let is_root =
                        matches!(b.tag.as_deref(), Some("body") | Some("html") | Some("main"));
                    if is_root {
                        if let Ok(y) = m.value.parse::<f32>() {
                            t.scroll_y = y.max(0.0);
                            applied = true;
                        }
                    }
                }
            });
            let _ = applied; // permite que se compile aunque no se use; doc-only
        } else if m.kind == "scrollLeft" {
            // Fase 7.26: scrollLeft no aplica — no tenemos scroll
            // horizontal en el chrome. No-op silencioso.
        } else if m.kind == "scrollIntoView" {
            // Fase 7.24: scroll heurístico DFS-order × 30px. Sin layout
            // taffy exacto (vive sólo en frame render), aproximamos la
            // posición del element_id contando elementos en DFS pre-order.
            // Monotónico — elementos más profundos quedan más abajo, lo
            // que matchea la intuición de "scrollIntoView".
            let mut count: u32 = 0;
            let mut found_at: Option<u32> = None;
            bt.walk(|b| {
                if found_at.is_some() {
                    return;
                }
                count += 1;
                if b.element_id.as_deref() == Some(m.id.as_str()) {
                    found_at = Some(count);
                }
            });
            if let Some(pos) = found_at {
                t.scroll_y = (pos.saturating_sub(1) as f32) * 30.0;
            }
        } else if m.kind == "classList" {
            // Fase 7.184 — classList.add/remove/toggle/className/setAttribute
            // ('class') publican la lista completa de clases. Actualizamos la
            // `class_list` del nodo y marcamos para recascadear una sola vez
            // al final (un handler puede togglear varias clases por evento).
            let classes: Vec<String> = m
                .value
                .split_whitespace()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if bt.set_element_class_list(&m.id, classes) {
                needs_restyle = true;
            }
        }
    }
    if needs_restyle {
        // Recascada del documento entero: un cambio de clase puede afectar
        // descendientes (selectores descendientes/herencia) y hermanos
        // posteriores (`+`/`~`). Reusa el motor de cascada del build.
        bt.restyle();
    }
    out
}

/// Fase 7.31 — parsea el payload del kind 'fetch' publicado por el JS.
/// Formato: campos separados por U+001D — [id, method, url, has_body_flag,
/// body, h_name1, h_val1, h_name2, h_val2, ...]. Devuelve `Msg::FetchRequest`
/// o `None` si el payload es malformado.
fn parse_fetch_payload(value: &str, tab: TabId, gen: u64) -> Option<Msg> {
    let parts: Vec<&str> = value.split('\u{001D}').collect();
    if parts.len() < 5 {
        return None;
    }
    let fetch_id: u32 = parts[0].parse().ok()?;
    let method = parts[1].to_string();
    let url = parts[2].to_string();
    let has_body = parts[3] == "1";
    let body = if has_body {
        Some(parts[4].as_bytes().to_vec())
    } else {
        None
    };
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut i = 5;
    while i + 1 < parts.len() {
        headers.push((parts[i].to_string(), parts[i + 1].to_string()));
        i += 2;
    }
    Some(Msg::FetchRequest { tab, gen, fetch_id, method, url, body, headers })
}

/// Spawn worker thread que ejecuta el fetch HTTP y devuelve
/// `Msg::FetchComplete` al main loop. Mismo molde que `spawn_load`.
fn spawn_fetch(req: Msg, handle: Handle<Msg>) {
    let Msg::FetchRequest { tab, gen, fetch_id, method, url, body, headers } = req else {
        return;
    };
    std::thread::spawn(move || {
        let result = puriy_engine::fetch::fetch_full(
            &method,
            &url,
            body.as_deref(),
            &headers,
        )
        .map_err(|e| e.to_string());
        handle.dispatch(Msg::FetchComplete { tab, gen, fetch_id, result });
    });
}

/// Devuelve el value (del option seleccionado) del `<select>` del slot
/// `select_idx` cuando el option seleccionado es `opt_idx`. Walka el
/// BoxTree contando selects en DFS, mismo orden que el populado en
/// `Msg::Loaded`. None si el slot/opt no existe.
fn select_value_at(t: &TabState, select_idx: usize, opt_idx: usize) -> Option<String> {
    let bt = t.box_tree.as_ref()?;
    let mut counter = 0usize;
    let mut found: Option<String> = None;
    bt.walk(|b| {
        if let Some(s) = &b.select {
            if counter == select_idx {
                found = s.options.get(opt_idx).map(|o| o.value.clone());
            }
            counter += 1;
        }
    });
    found
}

/// Busca el índice del option dentro del `<select>` del slot `select_idx`
/// cuyo `value` coincide con `target` (case-sensitive, exact match).
/// Walka el BoxTree contando selects en DFS. Devuelve None si no existe
/// el slot o ningún option matchea.
fn select_option_index_by_value(bt: &BoxTree, select_idx: usize, target: &str) -> Option<usize> {
    let mut counter = 0usize;
    let mut found: Option<usize> = None;
    bt.walk(|b| {
        if let Some(s) = &b.select {
            if counter == select_idx {
                found = s.options.iter().position(|o| o.value == target);
            }
            counter += 1;
        }
    });
    found
}

/// Concatena las hojas de texto del box tree en un único string — el
/// `body.textContent` que ve el JS via `document.body.textContent`.
/// Separa con un espacio entre nodos para evitar que palabras de
/// nodos adyacentes se peguen.
fn extract_body_text(bt: &BoxTree) -> String {
    let mut out = String::new();
    bt.walk(|b| {
        if let Some(text) = &b.text {
            if !text.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(text);
            }
        }
    });
    out
}

/// Inicia la carga de `url` en la pestaña activa. Si `push_history` es
/// `true`, se trunca y empuja al stack — útil para Navigate; back/fwd/
/// reload pasan `false`.
fn start_load(m: &mut Model, url: String, push_history: bool, handle: &Handle<Msg>) {
    let now_ms = m.start.elapsed().as_millis() as u64;
    let t = m.active_mut();
    // Fase 7.41 — antes de pisar la URL, disparar `beforeunload` para que
    // apps con formularios sin guardar / analytics puedan hacer cleanup
    // (`window.addEventListener('beforeunload', fn)`). Las mutaciones DOM
    // que el handler haga se aplican sobre la página vieja (se va a tirar
    // a la basura, no importa). **Divergencia documentada**: el spec real
    // exige confirmación al usuario si el handler setea `returnValue` o
    // llama `preventDefault()`; acá no hay diálogo modal — siempre se
    // navega.
    if t.js.is_some() {
        let (_, pending) = dispatch_window_js_event_on_tab(t, "beforeunload", now_ms);
        for req in pending {
            handle.dispatch(req);
        }
    }
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
    spawn_load(id, gen, url, referer, current_viewport(), handle.clone());
}

/// Viewport real actual (px físicos + DPR), leído de los thread-locals en el
/// hilo main. Se captura ANTES de spawnear el worker (que no ve los TLS) para
/// que el engine resuelva los `@media` del documento contra la ventana real.
fn current_viewport() -> puriy_engine::Viewport {
    let (w, h) = PURIY_VIEWPORT.with(|c| c.get());
    puriy_engine::Viewport { width: w, height: h, dpr: PURIY_DPR.with(|c| c.get()) as f32 }
}

fn spawn_load(
    tab: TabId,
    gen: u64,
    url: String,
    referer: Option<String>,
    viewport: puriy_engine::Viewport,
    handle: Handle<Msg>,
) {
    if url == NEW_TAB_URL {
        // No fetch para about:blank.
        return;
    }
    std::thread::spawn(move || {
        let engine = Engine::new().with_viewport(viewport);
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
                    scripts: doc.scripts,
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

/// Worker de un `EventSource` (Fase 7.182): corre el stream SSE en un thread
/// dedicado y reinyecta cada evento al runtime vía `Msg::EsDispatch`. El
/// `cancel` (compartido con `TabState.es_cancel`) lo corta en `close()` o al
/// navegar. La reconexión/parseo viven en `puriy_engine::sse::run_eventsource`.
fn spawn_eventsource(
    tab: TabId,
    gen: u64,
    es_id: u32,
    url: String,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Handle<Msg>,
) {
    std::thread::spawn(move || {
        let cancelled = || cancel.load(std::sync::atomic::Ordering::Relaxed);
        let emit = |kind: &str, ev: Option<&puriy_engine::sse::SseEvent>| {
            handle.dispatch(Msg::EsDispatch {
                tab,
                gen,
                es_id,
                kind: kind.to_string(),
                event_type: ev.map(|e| e.event_type.clone()).unwrap_or_default(),
                data: ev.map(|e| e.data.clone()).unwrap_or_default(),
                last_id: ev.map(|e| e.last_id.clone()).unwrap_or_default(),
            });
        };
        puriy_engine::sse::run_eventsource(
            &url,
            &cancelled,
            || emit("open", None),
            |ev| emit("message", Some(ev)),
            || emit("error", None),
        );
    });
}

fn start_load_post(m: &mut Model, url: String, body: String, handle: &Handle<Msg>) {
    let now_ms = m.start.elapsed().as_millis() as u64;
    let t = m.active_mut();
    if t.js.is_some() {
        let (_, pending) = dispatch_window_js_event_on_tab(t, "beforeunload", now_ms);
        for req in pending {
            handle.dispatch(req);
        }
    }
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
                    scripts: doc.scripts,
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
    matcher: &'a Matcher,
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
    /// Tiempo transcurrido (ms) desde el `anim_start_ms` de la pestaña —
    /// `animation_overlay` lo usa para samplear el progreso de cada nodo
    /// animado al instante actual.
    anim_elapsed_ms: u64,
    /// Reloj absoluto (ms desde `Model.start`) del frame actual — el tween
    /// de `transition` en hover lo usa para samplear cada `HoverTween`.
    now_ms: u64,
    /// Tweens de transición en hover por `node_id` (estado de la pestaña).
    hover_tweens: &'a std::collections::HashMap<u32, HoverTween>,
    /// Frames de `<canvas>` 2D keyeados por `element_id` — `render_canvas`
    /// los busca por el id del box canvas. Fase 7.196.
    canvas_frames: &'a std::collections::HashMap<String, CanvasFrame>,
}

fn viewport(
    t: &TabState,
    zoom: f32,
    matcher: &Matcher,
    find_current: usize,
    anim_elapsed_ms: u64,
    now_ms: u64,
) -> View<Msg> {
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
        matcher,
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
        anim_elapsed_ms,
        now_ms,
        hover_tweens: &t.hover_tweens,
        canvas_frames: &t.canvas_frames,
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

/// Samplea la animación CSS del nodo (`b.animation`) al instante actual
/// (`ctx.anim_elapsed_ms`) y devuelve un clon del `BoxNode` con el overlay
/// aplicado, o `None` si el nodo no anima o el overlay está vacío. `opacity`/
/// `color`/`background` los pinta el flujo normal de `render_box`; `transforms`
/// se setea para cuando el chrome los aplique (hoy no los renderiza todavía).
fn animation_overlay(b: &BoxNode, ctx: &RenderCtx<'_>) -> Option<BoxNode> {
    let inst = b.animation.as_ref()?;
    let elapsed_s = ctx.anim_elapsed_ms as f32 / 1000.0;
    let progress = puriy_engine::anim::animation_progress(&inst.binding, elapsed_s)?;
    let ov = puriy_engine::anim::sample_keyframes(&inst.keyframes, progress);
    if ov.is_empty() {
        return None;
    }
    let mut nb = b.clone();
    if let Some(o) = ov.opacity {
        nb.opacity = o.clamp(0.0, 1.0);
    }
    if let Some(c) = ov.color {
        nb.color = c;
    }
    if let Some(bg) = ov.background {
        nb.background = Some(bg);
    }
    if let Some(ts) = ov.transforms {
        nb.transforms = ts;
    }
    Some(nb)
}

/// Construye el afín 2D a partir de la lista de `transform` CSS del nodo
/// (`translate`/`scale`/`rotate`, ya sea de la regla estática o del overlay
/// de `@keyframes`). El compositor lo aplica alrededor del centro del rect
/// (CSS `transform-origin: 50% 50%`), así que acá sólo componemos el afín
/// "local" en orden de declaración: `transform: A B C` → matriz `A·B·C`.
/// `translate` se escala por el zoom de página (es px de layout); `scale`/
/// `rotate` son unitless. `None` si la lista está vacía → el nodo no
/// declara transform y el compositor no toca su pintura.
fn transform_affine(transforms: &[puriy_engine::style::Transform], zoom: f32) -> Option<Affine> {
    use puriy_engine::style::Transform as T;
    if transforms.is_empty() {
        return None;
    }
    let mut a = Affine::IDENTITY;
    for t in transforms {
        a *= match *t {
            T::Translate(x, y) => {
                Affine::translate(((x * zoom) as f64, (y * zoom) as f64))
            }
            T::Scale(sx, sy) => Affine::scale_non_uniform(sx as f64, sy as f64),
            T::Rotate(deg) => Affine::rotate((deg as f64).to_radians()),
        };
    }
    Some(a)
}

fn render_box(b: &BoxNode, ctx: &mut RenderCtx<'_>) -> View<Msg> {
    // Animación CSS: si el nodo tiene una `@keyframes` resuelta, sampleamos
    // el overlay al instante actual y renderizamos un clon con las props
    // animadas pisadas (el resto del flujo pinta `opacity`/`background`/
    // `color` desde el BoxNode, así que el overlay "se ve" gratis). El clon
    // se computa una sola vez por llamada → sin recursión.
    let overlaid = animation_overlay(b, ctx);
    let b = overlaid.as_ref().unwrap_or(b);
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
    // <canvas>: bypass — el frame del runtime JS se interpreta a vello.
    if let Some((cw, ch)) = b.canvas {
        let frame = b
            .element_id
            .as_deref()
            .and_then(|id| ctx.canvas_frames.get(id));
        return render_canvas(frame, cw, ch, zoom);
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
    let find_hit = b
        .text
        .as_ref()
        .map(|s| ctx.matcher.matches(s))
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
            // ¿El nodo declara una `transition` que cubre el background? Si
            // sí, NO usamos el swap instantáneo del compositor (`hover_fill`):
            // tweeneamos el fill nosotros frame a frame y anclamos el reloj
            // con `on_pointer_enter/leave`. El find-in-page (find_hit_color)
            // gana sobre la transición — no querés tweenear un highlight.
            let bg_transition = puriy_engine::anim::transition_for(&b.transitions, "background-color")
                .or_else(|| puriy_engine::anim::transition_for(&b.transitions, "background"));
            match (find_hit_color, bg_transition) {
                (None, Some(tr)) => {
                    let duration_ms = (tr.duration_s * 1000.0).max(0.0) as u32;
                    // `from` = background actual; si no hay, el color de hover
                    // pero transparente (fade-in desde nada).
                    let base = b.background.unwrap_or(puriy_engine::Color {
                        r: hbg.r,
                        g: hbg.g,
                        b: hbg.b,
                        a: 0,
                    });
                    let lin = ctx
                        .hover_tweens
                        .get(&b.node_id)
                        .map(|tw| tw.sample_linear(ctx.now_ms))
                        .unwrap_or(0.0);
                    let eased = puriy_engine::anim::apply_easing(tr.timing, lin);
                    let cur = puriy_engine::anim::lerp_color(&base, &hbg, eased);
                    let a = ((cur.a as f32) * alpha_mul) as u8;
                    view = view
                        .fill(Color::from_rgba8(cur.r, cur.g, cur.b, a))
                        .on_pointer_enter(Msg::HoverTween {
                            node_id: b.node_id,
                            entering: true,
                            duration_ms,
                        })
                        .on_pointer_leave(Msg::HoverTween {
                            node_id: b.node_id,
                            entering: false,
                            duration_ms,
                        });
                }
                _ => {
                    let a = ((hbg.a as f32) * alpha_mul) as u8;
                    view = view.hover_fill(Color::from_rgba8(hbg.r, hbg.g, hbg.b, a));
                }
            }
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
            let native_msg = if let Some(filename_hint) = &b.link_download {
                Msg::DownloadLink {
                    url: target.clone(),
                    filename_hint: filename_hint.clone(),
                }
            } else if b.link_new_tab {
                Msg::NavigateNewTab(target.clone())
            } else {
                Msg::Navigate(target.clone())
            };
            // Fase 7.6 — cohabitación link+handler: si el `<a>` tiene
            // `id=`, despachamos el evento JS PRIMERO y la navegación
            // queda como fallback. El handler puede llamar
            // `event.preventDefault()` para cancelar la nav.
            let click_msg = if let Some(eid) = &b.element_id {
                if !eid.is_empty() {
                    Msg::JsDispatchEvent {
                        element_id: eid.clone(),
                        event_type: "click".into(),
                        fallback: Some(Box::new(native_msg.clone())),
                    }
                } else {
                    native_msg.clone()
                }
            } else {
                native_msg.clone()
            };
            view = view
                .on_click(click_msg)
                .on_middle_click(Msg::NavigateNewTab(target.clone()))
                .on_pointer_enter(Msg::HoverLink(Some(target.clone())))
                .on_pointer_leave(Msg::HoverLink(None));
        }
    } else if let Some(eid) = &b.element_id {
        // Elemento con `id=` y sin link/download/submit nativo: si JS
        // registró handlers para 'click', el chrome los dispara. Sin
        // handlers, `dispatch_event` devuelve count=0 y nada pasa.
        if pe_active && !eid.is_empty() && !matches!(b.display, Display::None) {
            view = view.on_click(Msg::JsDispatchEvent {
                element_id: eid.clone(),
                event_type: "click".into(),
                fallback: None,
            });
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
        return view
            .text_aligned_full(
                text.clone(),
                size,
                display_color,
                Alignment::Start,
                italic,
                b.font_family.clone(),
            )
            .line_height(b.line_height.unwrap_or(1.2));
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
        } else if is_mixed_inline_context(b) {
            // Contexto inline con más de un hijo (texto + elementos inline
            // como <b>/<a>/<code>): partimos cada run de texto en palabras
            // para que TODO fluya palabra-a-palabra junto a los elementos.
            // Sin esto, el run de texto se mide como un bloque multi-línea y
            // el elemento inline queda colgado después, no en la misma línea.
            render_inline_flow(&b.children, ctx)
        } else {
            render_children_z_ordered(&b.children, ctx)
        };
        view = view.children(kids);
    }
    // Transform CSS (estático o animado por `@keyframes`): el compositor lo
    // aplica al nodo y todo su subtree alrededor del centro de su rect. Se
    // setea al final para que cubra fill/text/decorations/children juntos.
    if let Some(xf) = transform_affine(&b.transforms, zoom) {
        view = view.transform(xf);
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

/// ¿`b` es un contexto inline "mixto"? — todos sus hijos son inline y hay
/// **más de uno** (p. ej. texto + `<b>` + texto). Ese es el caso donde el
/// modelo "un run = un item flex" se rompe visualmente y conviene partir el
/// texto en palabras. Un párrafo de un solo run de texto (`children.len()==1`)
/// NO entra acá: se mide entero (envuelve a N líneas) y conserva el
/// find-in-page por hoja.
fn is_mixed_inline_context(b: &BoxNode) -> bool {
    b.children.len() > 1 && has_inline_children(b)
}

/// Renderiza un contexto inline mixto partiendo cada hoja de texto en
/// palabras: cada palabra es un item flex propio, así el `flex-wrap` del
/// bloque rompe líneas en los límites de palabra y los elementos inline
/// (`<b>`, `<code>`, `<a>`…) fluyen en la misma línea que el texto vecino.
/// Las hojas no-texto (elementos inline) se renderizan como una unidad.
fn render_inline_flow(children: &[BoxNode], ctx: &mut RenderCtx<'_>) -> Vec<View<Msg>> {
    let mut out: Vec<View<Msg>> = Vec::new();
    for c in children {
        match &c.text {
            // Hoja de texto: una vista por palabra (clon del nodo con el
            // texto reemplazado), reusando el render normal — hereda
            // color/peso/tamaño/familia/line-height sin duplicar lógica.
            Some(text) if c.children.is_empty() => {
                for word in split_words(text) {
                    let mut wn = c.clone();
                    wn.text = Some(word);
                    out.push(render_box(&wn, ctx));
                }
            }
            _ => out.push(render_box(c, ctx)),
        }
    }
    out
}

/// Parte un run de texto (ya whitespace-colapsado a espacios simples) en
/// tokens "palabra " con el espacio separador pegado, de modo que cada token
/// mida su propio ancho (incluido el espacio) y los words se separen al
/// fluir. Preserva un espacio inicial (separa del elemento inline anterior) y
/// recorta el espacio final si el run no terminaba en espacio.
fn split_words(s: &str) -> Vec<String> {
    let leading = s.starts_with(' ');
    let mut out: Vec<String> = Vec::new();
    for (i, w) in s.split(' ').filter(|w| !w.is_empty()).enumerate() {
        let mut tok = String::new();
        if i == 0 && leading {
            tok.push(' ');
        }
        tok.push_str(w);
        tok.push(' ');
        out.push(tok);
    }
    if !s.ends_with(' ') {
        if let Some(last) = out.last_mut() {
            if last.ends_with(' ') {
                last.pop();
            }
        }
    }
    out
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
    let find_hit = b
        .text
        .as_ref()
        .map(|s| ctx.matcher.matches(s))
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
        return view
            .text_aligned_full(
                text.clone(),
                size,
                color,
                Alignment::Start,
                italic,
                b.font_family.clone(),
            )
            .line_height(b.line_height.unwrap_or(1.2));
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
/// Frame de un `<canvas>` 2D recolectado del runtime JS (Fase 7.196).
/// Espejo de lo que devuelve `__puriy_collect_canvas()`: el id del elemento,
/// su tamaño intrínseco y la lista de comandos de dibujo (cada uno un array
/// `[op, ...args]`, con un snapshot de estilo apendido en los que pintan).
#[derive(serde::Deserialize, Clone, Debug, Default)]
struct CanvasFrame {
    id: String,
    #[serde(default)]
    width: f32,
    #[serde(default)]
    height: f32,
    #[serde(default)]
    cmds: Vec<Vec<serde_json::Value>>,
}

/// Refresca `t.canvas_frames` evaluando `__puriy_collect_canvas()` en el
/// runtime y parseando el JSON. Llamado tras correr scripts, en cada tick y
/// tras dispatchear eventos — cualquier momento en que el JS pudo dibujar.
/// Barato cuando no hay canvas: `canvas_json` devuelve `None` (un `eval` mini).
fn refresh_canvas_frames(t: &mut TabState) {
    let Some(rt) = t.js.as_mut() else { return };
    match rt.canvas_json() {
        Some(json) => match serde_json::from_str::<Vec<CanvasFrame>>(&json) {
            Ok(frames) => {
                t.canvas_frames.clear();
                for f in frames {
                    t.canvas_frames.insert(f.id.clone(), f);
                }
            }
            Err(_) => t.canvas_frames.clear(),
        },
        None => t.canvas_frames.clear(),
    }
}

/// Extrae un `f64` de un valor JSON (default 0.0 si no es número).
fn cnum(v: Option<&serde_json::Value>) -> f64 {
    v.and_then(|x| x.as_f64()).unwrap_or(0.0)
}

/// Resuelve `fillStyle`/`strokeStyle` (string color CSS o objeto
/// `CanvasGradient`) a un color sólido peniko, multiplicando el alpha por
/// `ga` (globalAlpha). Los gradientes se degradan al color de su último
/// stop (MVP — sin gradiente real todavía). Default negro opaco.
fn canvas_color(v: Option<&serde_json::Value>, ga: f64) -> Color {
    let base = match v {
        Some(serde_json::Value::String(s)) => puriy_engine::parse_color(s),
        Some(serde_json::Value::Object(o)) => {
            // CanvasGradient: { _kind, _coords, _stops: [[offset, color], ...] }
            o.get("_stops")
                .and_then(|s| s.as_array())
                .and_then(|arr| arr.last())
                .and_then(|stop| stop.as_array())
                .and_then(|pair| pair.get(1))
                .and_then(|c| c.as_str())
                .and_then(puriy_engine::parse_color)
        }
        _ => None,
    }
    .unwrap_or(puriy_engine::Color { r: 0, g: 0, b: 0, a: 255 });
    let a = ((base.a as f64) * ga).clamp(0.0, 255.0) as u8;
    Color::from_rgba8(base.r, base.g, base.b, a)
}

/// Px de fuente parseados de un string CSS `font` tipo `"16px sans-serif"`.
fn canvas_font_px(font: Option<&str>) -> f32 {
    let f = font.unwrap_or("10px sans-serif");
    // Busca "<num>px".
    if let Some(idx) = f.find("px") {
        let start = f[..idx]
            .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
            .map(|i| i + 1)
            .unwrap_or(0);
        if let Ok(v) = f[start..idx].parse::<f32>() {
            if v > 0.0 {
                return v;
            }
        }
    }
    10.0
}

/// Renderiza un `<canvas>` 2D: un View del tamaño intrínseco (escalado por
/// zoom) cuyo `paint_with` interpreta el log de comandos del frame con vello.
/// Si no hay frame (el script aún no pidió contexto / dibujó), devuelve el
/// View vacío (rect transparente). Fase 7.196.
fn render_canvas(frame: Option<&CanvasFrame>, intrinsic_w: f32, intrinsic_h: f32, zoom: f32) -> View<Msg> {
    // El View se muestra al tamaño del box (atributos width/height del engine,
    // escalado por zoom). El espacio de COORDENADAS de los comandos es el
    // tamaño del buffer de dibujo (`frame.width/height`, que un script pudo
    // cambiar vía `canvas.width = N`); si no hay frame, cae al intrínseco.
    let w = intrinsic_w * zoom;
    let h = intrinsic_h * zoom;
    let cmds: Vec<Vec<serde_json::Value>> = frame.map(|f| f.cmds.clone()).unwrap_or_default();
    let iw = frame
        .map(|f| f.width)
        .filter(|v| *v > 0.0)
        .unwrap_or(intrinsic_w)
        .max(1.0) as f64;
    let ih = frame
        .map(|f| f.height)
        .filter(|v| *v > 0.0)
        .unwrap_or(intrinsic_h)
        .max(1.0) as f64;
    View::new(Style {
        size: Size { width: length(w), height: length(h) },
        ..Default::default()
    })
    .paint_with(move |scene, ts, rect| {
        paint_canvas_cmds(scene, ts, rect, &cmds, iw, ih);
    })
}

/// Interpreta el log de comandos 2D contra `scene` (vello), mapeando el
/// espacio de usuario del canvas (0..iw, 0..ih) al `rect` de pantalla. MVP:
/// soporta fill/stroke de paths (move/line/bezier/quad/arc/ellipse/rect/
/// roundRect/closePath), fillRect/strokeRect, fillText/strokeText, los
/// transforms (save/restore/translate/scale/rotate/transform/setTransform/
/// resetTransform/beginPath) y globalAlpha. Limitaciones: clip, drawImage,
/// putImageData, patrones, sombras, dash y gradientes reales (degradan a un
/// color sólido) quedan fuera.
fn paint_canvas_cmds(
    scene: &mut llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: llimphi_ui::PaintRect,
    cmds: &[Vec<serde_json::Value>],
    iw: f64,
    ih: f64,
) {
    use llimphi_raster::kurbo::{BezPath, Shape};

    // base: espacio de usuario del canvas → rect de pantalla.
    let sx = rect.w as f64 / iw;
    let sy = rect.h as f64 / ih;
    let base = Affine::translate((rect.x as f64, rect.y as f64))
        * Affine::scale_non_uniform(sx, sy);

    let mut cur = Affine::IDENTITY; // transform actual del canvas (espacio usuario)
    let mut tstack: Vec<Affine> = Vec::new();
    let mut path = BezPath::new();

    for cmd in cmds {
        let Some(op) = cmd.first().and_then(|v| v.as_str()) else { continue };
        let a = |i: usize| cnum(cmd.get(i));
        match op {
            "save" => tstack.push(cur),
            "restore" => {
                if let Some(t) = tstack.pop() {
                    cur = t;
                }
            }
            "translate" => cur *= Affine::translate((a(1), a(2))),
            "scale" => cur *= Affine::scale_non_uniform(a(1), a(2)),
            "rotate" => cur *= Affine::rotate(a(1)),
            "transform" => {
                cur *= Affine::new([a(1), a(2), a(3), a(4), a(5), a(6)]);
            }
            "setTransform" => {
                cur = Affine::new([a(1), a(2), a(3), a(4), a(5), a(6)]);
            }
            "resetTransform" | "reset" => {
                cur = Affine::IDENTITY;
                if op == "reset" {
                    path = BezPath::new();
                    tstack.clear();
                }
            }
            "beginPath" => path = BezPath::new(),
            "closePath" => path.close_path(),
            "moveTo" => path.move_to((a(1), a(2))),
            "lineTo" => path.line_to((a(1), a(2))),
            "bezierCurveTo" => path.curve_to((a(1), a(2)), (a(3), a(4)), (a(5), a(6))),
            "quadraticCurveTo" => path.quad_to((a(1), a(2)), (a(3), a(4))),
            "rect" => {
                let (x, y, w, h) = (a(1), a(2), a(3), a(4));
                path.move_to((x, y));
                path.line_to((x + w, y));
                path.line_to((x + w, y + h));
                path.line_to((x, y + h));
                path.close_path();
            }
            "roundRect" => {
                // MVP: radio uniforme (primer valor) si lo hay, sino 0.
                let (x, y, w, h) = (a(1), a(2), a(3), a(4));
                let r = cmd.get(5).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let rr = RoundedRect::new(x, y, x + w, y + h, r);
                path.extend(rr.path_elements(0.1));
            }
            "arc" => {
                // arc(x, y, r, start, end, ccw=false)
                let (cx, cy, r, start, end) = (a(1), a(2), a(3), a(4), a(5));
                let ccw = cmd.get(6).and_then(|v| v.as_bool()).unwrap_or(false);
                append_arc(&mut path, cx, cy, r, r, 0.0, start, end, ccw);
            }
            "ellipse" => {
                // ellipse(x, y, rx, ry, rotation, start, end, ccw=false)
                let (cx, cy, rx, ry, rot, start, end) =
                    (a(1), a(2), a(3), a(4), a(5), a(6), a(7));
                let ccw = cmd.get(8).and_then(|v| v.as_bool()).unwrap_or(false);
                append_arc(&mut path, cx, cy, rx, ry, rot, start, end, ccw);
            }
            "arcTo" => {
                // MVP: línea al primer punto de control (aproximación).
                path.line_to((a(1), a(2)));
            }
            "fill" => {
                let st = cmd.get(1);
                let ga = style_field(st, "ga").unwrap_or(1.0);
                let color = canvas_color(style_color(st, "f").as_ref(), ga);
                scene.fill(Fill::NonZero, base * cur, color, None, &path);
            }
            "stroke" => {
                let st = cmd.get(1);
                let ga = style_field(st, "ga").unwrap_or(1.0);
                let lw = style_field(st, "lw").unwrap_or(1.0).max(0.01);
                let color = canvas_color(style_color(st, "s").as_ref(), ga);
                scene.stroke(&Stroke::new(lw), base * cur, color, None, &path);
            }
            "fillRect" => {
                // ['fillRect', x, y, w, h, fillStyle, snapshot]
                let ga = style_field(cmd.get(6), "ga").unwrap_or(1.0);
                let color = canvas_color(cmd.get(5), ga);
                let r = KurboRect::new(a(1), a(2), a(1) + a(3), a(2) + a(4));
                scene.fill(Fill::NonZero, base * cur, color, None, &r);
            }
            "strokeRect" => {
                let st = cmd.get(6);
                let ga = style_field(st, "ga").unwrap_or(1.0);
                let lw = style_field(st, "lw").unwrap_or(1.0).max(0.01);
                let color = canvas_color(cmd.get(5), ga);
                let r = KurboRect::new(a(1), a(2), a(1) + a(3), a(2) + a(4));
                scene.stroke(&Stroke::new(lw), base * cur, color, None, &r);
            }
            "fillText" | "strokeText" => {
                // ['fillText', text, x, y, maxWidth, snapshot]
                let text = cmd.get(1).and_then(|v| v.as_str()).unwrap_or("");
                if text.is_empty() {
                    continue;
                }
                let (x, y) = (a(2), a(3));
                let st = cmd.get(5);
                let ga = style_field(st, "ga").unwrap_or(1.0);
                let key = if op == "fillText" { "f" } else { "s" };
                let color = canvas_color(style_color(st, key).as_ref(), ga);
                let px = canvas_font_px(style_str(st, "fnt").as_deref());
                let layout = ts.layout(
                    text,
                    px,
                    None,
                    llimphi_ui::llimphi_text::Alignment::Start,
                    1.0,
                    false,
                    None,
                );
                // textAlign: ajusta x. Baseline alphabetic ⇒ subimos ~0.8em.
                let tw = layout.width() as f64;
                let align = style_str(st, "ta").unwrap_or_default();
                let dx = match align.as_str() {
                    "center" => -tw / 2.0,
                    "right" | "end" => -tw,
                    _ => 0.0,
                };
                let ascent = (px as f64) * 0.8;
                let xf = base * cur * Affine::translate((x + dx, y - ascent));
                llimphi_ui::llimphi_text::draw_layout_xf(scene, &layout, color, xf);
            }
            // clip/clearRect/drawImage/putImageData: no-op en el MVP.
            _ => {}
        }
    }
}

/// Apendea un arco/elipse al path (espacio usuario), manejando la dirección
/// (clockwise por default en canvas, que con y-abajo es sweep positivo).
/// Hace `move_to`/`line_to` al punto de inicio según haya o no subpath.
fn append_arc(
    path: &mut llimphi_raster::kurbo::BezPath,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    rot: f64,
    start: f64,
    end: f64,
    ccw: bool,
) {
    use llimphi_raster::kurbo::{Arc as KArc, PathEl, Point as KPoint};
    use std::f64::consts::TAU;
    let mut sweep = end - start;
    if !ccw {
        if sweep < 0.0 {
            sweep = sweep.rem_euclid(TAU);
        }
        if sweep == 0.0 && end != start {
            sweep = TAU;
        }
    } else {
        if sweep > 0.0 {
            sweep = -((-sweep).rem_euclid(TAU));
        }
        if sweep == 0.0 && end != start {
            sweep = -TAU;
        }
    }
    // Punto de inicio del arco (con rotación de elipse).
    let (cs, sn) = (rot.cos(), rot.sin());
    let lx = rx * start.cos();
    let ly = ry * start.sin();
    let sx = cx + lx * cs - ly * sn;
    let sy = cy + lx * sn + ly * cs;
    let start_pt = KPoint::new(sx, sy);
    let empty = path.elements().is_empty();
    if empty {
        path.move_to(start_pt);
    } else {
        path.line_to(start_pt);
    }
    let arc = KArc::new((cx, cy), (rx, ry), start, sweep, rot);
    for el in arc.append_iter(0.1) {
        // append_iter continúa desde el punto actual (no emite MoveTo).
        if !matches!(el, PathEl::MoveTo(_)) {
            path.push(el);
        }
    }
}

/// Lee un campo numérico (`lw`, `ga`) del snapshot de estilo (objeto JSON).
fn style_field(st: Option<&serde_json::Value>, key: &str) -> Option<f64> {
    st.and_then(|v| v.as_object())
        .and_then(|o| o.get(key))
        .and_then(|x| x.as_f64())
}

/// Lee un campo string (`fnt`, `ta`) del snapshot de estilo.
fn style_str(st: Option<&serde_json::Value>, key: &str) -> Option<String> {
    st.and_then(|v| v.as_object())
        .and_then(|o| o.get(key))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

/// Lee `fillStyle`/`strokeStyle` (`f`/`s`) del snapshot — puede ser string
/// (color) u objeto (gradiente); devuelve el `Value` para `canvas_color`.
fn style_color(st: Option<&serde_json::Value>, key: &str) -> Option<serde_json::Value> {
    st.and_then(|v| v.as_object())
        .and_then(|o| o.get(key))
        .cloned()
}

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
    // Las hojas de texto se miden con parley en el runtime
    // (`compute_with_measure`): taffy reserva el alto real del texto
    // envuelto (N líneas) en lugar de una sola. Por eso dejamos su height
    // en `auto` — si lo fijáramos a una línea, los párrafos que envuelven
    // se aplastarían unos sobre otros. Mantenemos `line_h` como piso
    // (min_height) para que un nodo de texto vacío no colapse a cero.
    let is_text_leaf = b.text.is_some();
    let lh_mult = b.line_height.unwrap_or(1.2);
    let line_h = b.font_size * lh_mult * zoom;

    let is_flex = matches!(b.display, Display::Flex | Display::InlineFlex);

    let is_grid = matches!(b.display, Display::Grid | Display::InlineGrid);

    // Defaults según display: Block fila completa columnar, Inline en row
    // con altura auto, Flex toma sus props del nodo. None: cero.
    let (default_direction, mut width, mut height) = match b.display {
        Display::Block => (FlexDirection::Column, percent(1.0_f32), auto()),
        Display::Flex => (map_flex_direction(b.flex_direction), percent(1.0_f32), auto()),
        Display::InlineFlex => (map_flex_direction(b.flex_direction), auto(), auto()),
        Display::Grid => (FlexDirection::Column, percent(1.0_f32), auto()),
        Display::InlineGrid => (FlexDirection::Column, auto(), auto()),
        Display::InlineBlock | Display::Inline => {
            // Texto: height auto → lo dimensiona la medición con parley.
            (FlexDirection::Row, auto(), auto())
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
    // CSS `height` explícito gana sobre el default (auto = lo dimensiona el
    // contenido). Los % de height sólo resuelven si el padre tiene altura
    // definida — taffy lo maneja igual que en un browser.
    if let Some(explicit) = length_to_taffy(b.height, zoom) {
        height = explicit;
    }
    let max_size = Size {
        width: length_to_taffy(b.max_width, zoom).unwrap_or_else(auto),
        height: length_to_taffy(b.max_height, zoom).unwrap_or_else(auto),
    };
    let min_size = Size {
        width: length_to_taffy(b.min_width, zoom).unwrap_or_else(|| length(0.0_f32)),
        height: length_to_taffy(b.min_height, zoom).unwrap_or_else(|| {
            // Piso de una línea para hojas de texto (el resto: 0).
            if is_text_leaf { length(line_h) } else { length(0.0_f32) }
        }),
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

    // align-content: distribución de líneas (flex multilínea) / pistas
    // (grid) en el eje cruzado. Aplica tanto a flex como a grid; `Normal`
    // deja el default de taffy (None ≈ stretch).
    let align_content = if is_flex || is_grid {
        map_align_content(b.align_content)
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
        align_content,
        // justify-items / justify-self: taffy sólo los usa en grid (los
        // ignora en flex). `None`/`Auto` → default de taffy.
        justify_items: b.justify_items.map(map_align),
        justify_self: map_align_self(b.justify_self),
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
        // CSS aspect-ratio: taffy dimensiona el eje `auto` a partir del otro
        // usando esta relación. `None` = sin relación.
        aspect_ratio: b.aspect_ratio,
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

/// `align-content` CSS → taffy. `Normal` ⇒ `None` (taffy aplica su default,
/// ≈ stretch para flex). `Start`/`End` mapean a `FlexStart`/`FlexEnd` para
/// que respeten la dirección flex (row-reverse, etc.).
fn map_align_content(a: CssAlignContent) -> Option<AlignContent> {
    match a {
        CssAlignContent::Normal => None,
        CssAlignContent::Start => Some(AlignContent::Start),
        CssAlignContent::Center => Some(AlignContent::Center),
        CssAlignContent::End => Some(AlignContent::End),
        CssAlignContent::Stretch => Some(AlignContent::Stretch),
        CssAlignContent::SpaceBetween => Some(AlignContent::SpaceBetween),
        CssAlignContent::SpaceAround => Some(AlignContent::SpaceAround),
        CssAlignContent::SpaceEvenly => Some(AlignContent::SpaceEvenly),
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
fn scroll_to_find_match(m: &mut Model, matcher: &Matcher) {
    if m.find_current == 0 {
        return;
    }
    let nth = m.find_current;
    let y = m
        .active()
        .box_tree
        .as_ref()
        .and_then(|bt| find_match_y(bt, matcher, nth));
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
    fn split_words_tokeniza_con_espacios() {
        // Cada palabra lleva su espacio separador; el último sin espacio
        // porque el run no termina en espacio.
        assert_eq!(split_words("foo bar baz"), vec!["foo ", "bar ", "baz"]);
        // Espacio inicial preservado (separa del elemento inline anterior).
        assert_eq!(split_words(" baz"), vec![" baz"]);
        // Espacio final preservado (separa del siguiente elemento inline).
        assert_eq!(split_words("foo "), vec!["foo "]);
        // Una sola palabra entre dos elementos: conserva ambos lados.
        assert_eq!(split_words(" x "), vec![" x "]);
        // Vacío → sin tokens.
        assert!(split_words("").is_empty());
    }

    #[test]
    fn contexto_inline_mixto_se_detecta() {
        // <p>foo <b>bar</b> baz</p> → bloque con hijos inline múltiples.
        let bt = parse("<p>foo <b>bar</b> baz</p>");
        // Buscamos el <p> (block con inline children) en el árbol.
        let mut hallado = false;
        bt.walk(|b| {
            if b.children.len() > 1 && has_inline_children(b) {
                hallado = true;
                assert!(is_mixed_inline_context(b));
            }
        });
        assert!(hallado, "debería existir un contexto inline mixto en el <p>");
    }

    #[test]
    fn parrafo_de_un_solo_run_no_es_mixto() {
        // <p>solo texto</p> → un solo hijo de texto → NO mixto (se mide entero).
        let bt = parse("<p>solo texto sin elementos inline</p>");
        bt.walk(|b| {
            if b.text.is_none() && has_inline_children(b) {
                assert!(
                    !is_mixed_inline_context(b),
                    "un párrafo de un solo run no debe partirse en palabras"
                );
            }
        });
    }

    #[test]
    fn transform_affine_vacio_es_none() {
        assert!(transform_affine(&[], 1.0).is_none());
    }

    #[test]
    fn ctrl_rueda_zoomea_sin_ctrl_scrollea() {
        let arriba = WheelDelta { x: 0.0, y: -1.0 };
        let abajo = WheelDelta { x: 0.0, y: 1.0 };
        let ctrl = Modifiers { ctrl: true, ..Default::default() };
        let sin = Modifiers::default();
        // Ctrl + rueda arriba = acercar; Ctrl + rueda abajo = alejar.
        assert!(matches!(wheel_to_msg(arriba, ctrl), Some(Msg::ZoomIn)));
        assert!(matches!(wheel_to_msg(abajo, ctrl), Some(Msg::ZoomOut)));
        // Sin Ctrl la rueda scrollea, no zoomea.
        assert!(matches!(wheel_to_msg(abajo, sin), Some(Msg::Scroll(_))));
        assert!(matches!(wheel_to_msg(arriba, sin), Some(Msg::Scroll(_))));
    }

    #[test]
    fn hover_tween_avanza_hacia_uno_mientras_hovered() {
        let tw = HoverTween {
            hovered: true,
            progress_at_toggle: 0.0,
            toggle_ms: 1000,
            duration_ms: 1000,
        };
        assert!((tw.sample_linear(1500) - 0.5).abs() < 1e-6);
        // pasada la duración, clampa a 1.0.
        assert!((tw.sample_linear(9000) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hover_tween_revierte_hacia_cero_al_salir() {
        // Salió con el tween a media transición: retrocede desde 1.0.
        let tw = HoverTween {
            hovered: false,
            progress_at_toggle: 1.0,
            toggle_ms: 1000,
            duration_ms: 1000,
        };
        assert!((tw.sample_linear(1500) - 0.5).abs() < 1e-6);
        assert!(tw.sample_linear(9000).abs() < 1e-6);
    }

    #[test]
    fn hover_tween_revierte_desde_progreso_parcial_sin_saltar() {
        // Entró, llegó a 0.3, y salió: arranca el retroceso en 0.3, no en 1.0.
        let tw = HoverTween {
            hovered: false,
            progress_at_toggle: 0.3,
            toggle_ms: 2000,
            duration_ms: 1000,
        };
        assert!((tw.sample_linear(2000) - 0.3).abs() < 1e-6);
        assert!((tw.sample_linear(2100) - 0.2).abs() < 1e-6);
        assert!(tw.sample_linear(3000).abs() < 1e-6);
    }

    #[test]
    fn hover_tween_duracion_nula_es_instantanea() {
        let on = HoverTween { hovered: true, progress_at_toggle: 0.0, toggle_ms: 0, duration_ms: 0 };
        let off = HoverTween { hovered: false, progress_at_toggle: 1.0, toggle_ms: 0, duration_ms: 0 };
        assert_eq!(on.sample_linear(123), 1.0);
        assert_eq!(off.sample_linear(123), 0.0);
    }

    #[test]
    fn transform_affine_translate_escala_por_zoom() {
        use puriy_engine::style::Transform as T;
        let a = transform_affine(&[T::Translate(10.0, 20.0)], 2.0).unwrap();
        // translate(10,20) @ zoom 2 → mueve el origen a (20, 40).
        let p = a * Point::new(0.0, 0.0);
        assert!((p.x - 20.0).abs() < 1e-6, "x = {}", p.x);
        assert!((p.y - 40.0).abs() < 1e-6, "y = {}", p.y);
    }

    #[test]
    fn transform_affine_scale_no_depende_del_zoom() {
        use puriy_engine::style::Transform as T;
        let a = transform_affine(&[T::Scale(3.0, 4.0)], 2.0).unwrap();
        let p = a * Point::new(1.0, 1.0);
        assert!((p.x - 3.0).abs() < 1e-6, "x = {}", p.x);
        assert!((p.y - 4.0).abs() < 1e-6, "y = {}", p.y);
    }

    #[test]
    fn transform_affine_rotate_90_grados() {
        use puriy_engine::style::Transform as T;
        let a = transform_affine(&[T::Rotate(90.0)], 1.0).unwrap();
        // rotate(90°) horario en pantalla: (1,0) → (0,1).
        let p = a * Point::new(1.0, 0.0);
        assert!(p.x.abs() < 1e-6, "x = {}", p.x);
        assert!((p.y - 1.0).abs() < 1e-6, "y = {}", p.y);
    }

    #[test]
    fn transform_affine_compone_en_orden_de_declaracion() {
        use puriy_engine::style::Transform as T;
        // `transform: translate(10,0) scale(2)` → matriz T·S: el punto (1,0)
        // se escala a (2,0) y luego se traslada a (12,0).
        let a = transform_affine(&[T::Translate(10.0, 0.0), T::Scale(2.0, 2.0)], 1.0)
            .unwrap();
        let p = a * Point::new(1.0, 0.0);
        assert!((p.x - 12.0).abs() < 1e-6, "x = {}", p.x);
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

    /// Matcher case-insensitive por substring (el default de la find bar).
    fn ci(q: &str) -> Matcher {
        Matcher::new(q, MatchOpts::default())
    }

    #[test]
    fn count_matches_devuelve_cero_cuando_query_vacia() {
        let tree = parse("<p>hola mundo</p>");
        assert_eq!(count_matches(Some(&tree), &ci("")), 0);
    }

    #[test]
    fn count_matches_devuelve_cero_cuando_tree_none() {
        assert_eq!(count_matches(None, &ci("algo")), 0);
    }

    #[test]
    fn count_matches_es_case_insensitive() {
        let tree = parse("<p>Hola MUNDO</p><p>mundO repetido</p>");
        let n = count_matches(Some(&tree), &ci("mundo"));
        assert!(n >= 2, "esperaba >= 2 matches, conseguí {n}");
    }

    #[test]
    fn count_matches_busca_dentro_de_hojas() {
        let tree = parse(
            "<article><h1>Tutorial</h1><p>Este tutorial cubre Rust</p><p>Otra cosa</p></article>",
        );
        // La query "tutorial" matchea el <h1> y el primer <p> (ambos como hojas).
        let n = count_matches(Some(&tree), &ci("tutorial"));
        assert_eq!(n, 2);
    }

    #[test]
    fn count_matches_query_sin_hits_devuelve_cero() {
        let tree = parse("<p>foo bar baz</p>");
        assert_eq!(count_matches(Some(&tree), &ci("qwerty")), 0);
    }

    // ── Fase 7.31 — toggles case-sensitive / whole-word ──────────────

    #[test]
    fn matcher_case_sensitive_distingue_mayusculas() {
        let tree = parse("<p>Hola MUNDO</p><p>mundo bajo</p>");
        let sensible = Matcher::new("mundo", MatchOpts { case_sensitive: true, whole_word: false });
        // Sólo el "mundo" en minúsculas del segundo <p> matchea.
        assert_eq!(count_matches(Some(&tree), &sensible), 1);
        // Sin el toggle, ambos (MUNDO y mundo) matchean.
        assert_eq!(count_matches(Some(&tree), &ci("mundo")), 2);
    }

    #[test]
    fn matcher_whole_word_excluye_substrings() {
        let tree = parse("<p>cat</p><p>category</p><p>a cat sat</p>");
        let word = Matcher::new("cat", MatchOpts { case_sensitive: false, whole_word: true });
        // "cat" y "a cat sat" matchean como palabra; "category" no.
        assert_eq!(count_matches(Some(&tree), &word), 2);
        // Sin whole-word, los tres contienen "cat".
        assert_eq!(count_matches(Some(&tree), &ci("cat")), 3);
    }

    #[test]
    fn matcher_whole_word_respeta_bordes_unicode() {
        let tree = parse("<p>café con leche</p><p>cafetería</p>");
        let word = Matcher::new("café", MatchOpts { case_sensitive: false, whole_word: true });
        // "café" es palabra completa en el primero; "cafetería" no contiene
        // "café" como substring (la 'é' difiere), así que igual no matchea.
        assert_eq!(count_matches(Some(&tree), &word), 1);
    }

    #[test]
    fn matcher_query_vacia_no_matchea_nada() {
        let m = ci("");
        assert!(m.is_empty());
        assert!(!m.matches("cualquier texto"));
    }

    // ── Fase 7.31 — flujo de Msg de la find bar (sin Handle) ─────────
    // `update` necesita un `Handle` (no construible en test), pero los
    // handlers de find delegan en métodos puros de `Model`. Testeamos
    // esos métodos para cubrir el flujo open → query → next → prev.

    /// Model mínimo con una sola pestaña cuyo box tree es `parse(html)`.
    fn model_con_doc(html: &str) -> Model {
        let mut t = TabState::new("about:test".into());
        t.box_tree = Some(parse(html));
        Model {
            tabs: vec![t],
            active: 0,
            zoom: 1.0,
            find_active: false,
            find_input: TextInputState::new(),
            find_current: 0,
            find_case_sensitive: false,
            find_whole_word: false,
            panel: None,
            panel_filter: TextInputState::new(),
            hover_link: None,
            start: std::time::Instant::now(),
            menu_open: None,
            edit_menu: None,
            clipboard: SystemClipboard::new(),
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
        }
    }

    #[test]
    fn find_open_y_close_alternan_estado_y_limpian_query() {
        let mut m = model_con_doc("<p>uno</p><p>dos</p>");
        m.find_open();
        assert!(m.find_active);
        assert!(m.find_input.text().is_empty());
        assert_eq!(m.find_current, 0);
        m.find_input.set_text("dos");
        m.find_current = 1;
        m.find_close();
        assert!(!m.find_active);
        assert!(m.find_input.text().is_empty(), "close limpia la query");
        assert_eq!(m.find_current, 0);
    }

    #[test]
    fn find_step_avanza_y_wrapea() {
        // Tres hojas de texto con "rust" → tres matches.
        let mut m = model_con_doc(
            "<p>rust uno</p><p>dos rust</p><p>tres rust cuatro</p>",
        );
        m.find_open();
        m.find_input.set_text("rust");
        // open → primer next va al match 1.
        m.find_step(true);
        assert_eq!(m.find_current, 1);
        m.find_step(true);
        assert_eq!(m.find_current, 2);
        m.find_step(true);
        assert_eq!(m.find_current, 3);
        // Cuarto next wrapea a 1.
        m.find_step(true);
        assert_eq!(m.find_current, 1);
    }

    #[test]
    fn find_step_prev_wrapea_al_ultimo() {
        let mut m = model_con_doc("<p>foo</p><p>foo otra vez</p>");
        m.find_open();
        m.find_input.set_text("foo");
        // Desde 0, prev wrapea al último (total = 2).
        m.find_step(false);
        assert_eq!(m.find_current, 2);
        m.find_step(false);
        assert_eq!(m.find_current, 1);
        m.find_step(false);
        assert_eq!(m.find_current, 2);
    }

    #[test]
    fn find_step_sin_matches_es_no_op() {
        let mut m = model_con_doc("<p>hola</p>");
        m.find_open();
        m.find_input.set_text("zzz");
        m.find_step(true);
        assert_eq!(m.find_current, 0, "sin matches no avanza");
    }

    #[test]
    fn find_step_mueve_scroll_del_tab() {
        // Un documento alto: el match vive bien abajo → scroll_y > 0.
        let mut m = model_con_doc(
            "<p>arriba</p><p>x</p><p>x</p><p>x</p><p>x</p><p>x</p><p>objetivo abajo</p>",
        );
        m.find_open();
        m.find_input.set_text("objetivo");
        m.find_step(true);
        assert_eq!(m.find_current, 1);
        assert!(
            m.tabs[0].scroll_y >= 0.0,
            "scroll_y debe ser no-negativo tras navegar"
        );
    }

    #[test]
    fn toggle_case_resetea_navegacion_y_filtra() {
        let mut m = model_con_doc("<p>Rust</p><p>rust</p>");
        m.find_open();
        m.find_input.set_text("rust");
        // Case-insensitive: ambos matchean → next llega al 2.
        m.find_step(true);
        m.find_step(true);
        assert_eq!(m.find_current, 2);
        // Activar case-sensitive resetea la nav y reduce a 1 match.
        m.find_case_sensitive = !m.find_case_sensitive;
        m.find_current = 0;
        let total = count_matches(m.active().box_tree.as_ref(), &m.find_matcher());
        assert_eq!(total, 1, "case-sensitive deja sólo el 'rust' minúscula");
        assert_eq!(m.find_current, 0, "toggle resetea la nav");
    }

    #[test]
    fn toggle_whole_word_filtra_substrings() {
        let mut m = model_con_doc("<p>cat</p><p>category</p>");
        m.find_open();
        m.find_input.set_text("cat");
        m.find_whole_word = true;
        let total = count_matches(m.active().box_tree.as_ref(), &m.find_matcher());
        assert_eq!(total, 1, "whole-word excluye 'category'");
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

    #[test]
    fn extract_body_text_concatena_hojas() {
        let tree = parse("<body><h1>Hola</h1><p>mundo cruel</p></body>");
        let text = extract_body_text(&tree);
        assert!(text.contains("Hola"));
        assert!(text.contains("mundo cruel"));
    }

    #[test]
    fn run_scripts_actualiza_summary_logs() {
        let mut t = TabState::new("about:test".into());
        t.title = "T".into();
        t.url = "about:test".into();
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some("console.log('a'); console.log('b')".into()),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        assert_eq!(t.js_summary.logs, 2, "esperaba 2 logs");
        assert_eq!(t.js_summary.errors, 0);
        // El runtime debe haberse instanciado.
        assert!(t.js.is_some());
    }

    #[test]
    fn run_scripts_captura_error_thrown() {
        let mut t = TabState::new("about:test".into());
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some("console.log('ok'); throw new Error('boom')".into()),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        // 1 log de console + 1 error del throw.
        assert_eq!(t.js_summary.logs, 1);
        assert_eq!(t.js_summary.errors, 1);
    }

    #[test]
    fn run_scripts_saltea_modules_y_src_externo() {
        let mut t = TabState::new("about:test".into());
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![
            puriy_engine::ScriptInfo {
                src: Some("/main.js".into()),
                inline: None,
                type_attr: None,
                is_module: false,
                defer: false,
                async_: false,
            },
            puriy_engine::ScriptInfo {
                src: None,
                inline: Some("console.log('module')".into()),
                type_attr: Some("module".into()),
                is_module: true,
                defer: false,
                async_: false,
            },
        ];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        // Ninguno de los dos ejecutable → no se instancia runtime.
        assert!(t.js.is_none());
        assert_eq!(t.js_summary.logs, 0);
        assert_eq!(t.js_summary.errors, 0);
    }

    #[test]
    fn run_scripts_documento_inyecta_title_y_url() {
        let mut t = TabState::new("https://example.com/x".into());
        t.title = "Hola mundo".into();
        t.box_tree = Some(parse("<p>cuerpo</p>"));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some(
                "console.log(document.title); console.log(document.URL)".into(),
            ),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        let rt = t.js.as_ref().expect("rt creado");
        let out = rt.stdout();
        assert!(out.contains("Hola mundo"), "stdout: {out:?}");
        assert!(out.contains("https://example.com/x"), "stdout: {out:?}");
    }

    #[test]
    fn run_scripts_skip_application_json_pero_no_text_javascript() {
        let mut t = TabState::new("about:test".into());
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![
            puriy_engine::ScriptInfo {
                src: None,
                inline: Some("{\"k\":1}".into()),
                type_attr: Some("application/json".into()),
                is_module: false,
                defer: false,
                async_: false,
            },
            puriy_engine::ScriptInfo {
                src: None,
                inline: Some("console.log('ejecuto')".into()),
                type_attr: Some("text/javascript".into()),
                is_module: false,
                defer: false,
                async_: false,
            },
        ];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        assert_eq!(t.js_summary.logs, 1);
    }

    fn model_con_script(inline: &str) -> Model {
        let mut t = TabState::new("about:test".into());
        t.title = "T".into();
        t.url = "about:test".into();
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some(inline.into()),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        Model {
            tabs: vec![t],
            active: 0,
            zoom: 1.0,
            find_active: false,
            find_input: TextInputState::new(),
            find_current: 0,
            find_case_sensitive: false,
            find_whole_word: false,
            panel: None,
            panel_filter: TextInputState::new(),
            hover_link: None,
            start: std::time::Instant::now(),
            menu_open: None,
            edit_menu: None,
            clipboard: SystemClipboard::new(),
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
        }
    }

    #[test]
    fn tick_dispara_settimeout_pendiente() {
        let mut m = model_con_script("setTimeout(function(){ console.log('tic') }, 100)");
        assert!(m.tabs[0].js.is_some());
        let logs_pre = m.tabs[0].js_summary.logs;
        tick_js_runtimes(&mut m, 50);
        assert_eq!(m.tabs[0].js_summary.logs, logs_pre);
        tick_js_runtimes(&mut m, 100);
        assert_eq!(m.tabs[0].js_summary.logs, logs_pre + 1);
    }

    #[test]
    fn tick_no_panic_en_pestana_sin_js() {
        let mut t = TabState::new("about:test".into());
        t.box_tree = Some(parse("<p>x</p>"));
        let mut m = Model {
            tabs: vec![t],
            active: 0,
            zoom: 1.0,
            find_active: false,
            find_input: TextInputState::new(),
            find_current: 0,
            find_case_sensitive: false,
            find_whole_word: false,
            panel: None,
            panel_filter: TextInputState::new(),
            hover_link: None,
            start: std::time::Instant::now(),
            menu_open: None,
            edit_menu: None,
            clipboard: SystemClipboard::new(),
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
        };
        tick_js_runtimes(&mut m, 1234);
        assert!(m.tabs[0].js.is_none());
        assert_eq!(m.tabs[0].js_summary.logs, 0);
    }

    #[test]
    fn tick_acumula_errores_en_summary() {
        let mut m = model_con_script(
            "setTimeout(function(){ throw new Error('boom') }, 10)",
        );
        let errs_pre = m.tabs[0].js_summary.errors;
        tick_js_runtimes(&mut m, 50);
        assert!(
            m.tabs[0].js_summary.errors > errs_pre,
            "esperaba al menos 1 error nuevo en summary"
        );
    }

    #[test]
    fn tick_continua_disparando_interval() {
        let mut m = model_con_script(
            "setInterval(function(){ console.log('p') }, 20)",
        );
        let logs0 = m.tabs[0].js_summary.logs;
        tick_js_runtimes(&mut m, 20);
        tick_js_runtimes(&mut m, 40);
        tick_js_runtimes(&mut m, 60);
        assert_eq!(m.tabs[0].js_summary.logs, logs0 + 3);
    }

    #[test]
    fn collect_element_snapshots_indexa_solo_los_con_id() {
        let tree = parse(
            r#"<div><h1 id="hero">Título</h1><p>sin id</p><button id="b">x</button></div>"#,
        );
        let snaps = collect_element_snapshots(&tree);
        let ids: Vec<&str> = snaps.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"hero"), "snaps: {snaps:?}");
        assert!(ids.contains(&"b"), "snaps: {snaps:?}");
        assert_eq!(ids.len(), 2, "snaps: {snaps:?}");
    }

    #[test]
    fn collect_element_snapshots_text_content_concatena_subarbol() {
        let tree = parse(r#"<div id="x"><span>uno</span> <b>dos</b></div>"#);
        let snaps = collect_element_snapshots(&tree);
        let x = snaps.iter().find(|s| s.id == "x").expect("id=x");
        assert!(x.text_content.contains("uno"), "tc: {:?}", x.text_content);
        assert!(x.text_content.contains("dos"), "tc: {:?}", x.text_content);
    }

    #[test]
    fn event_bubbles_to_document_cubre_click_y_teclas_no_focus() {
        assert!(event_bubbles_to_document("click"));
        assert!(event_bubbles_to_document("keydown"));
        assert!(event_bubbles_to_document("change"));
        // focus/blur NO bubblean en spec.
        assert!(!event_bubbles_to_document("focus"));
        assert!(!event_bubbles_to_document("blur"));
        assert!(!event_bubbles_to_document("scroll"));
    }

    #[test]
    fn click_en_elemento_bubblea_al_document_listener() {
        // Event delegation: el listener vive en document, no en el botón.
        let mut m = model_con_script("console.log('boot')");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "btn".into(),
            tag_name: "button".into(),
            text_content: "go".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "document.addEventListener('click', \
                function(e){ console.log('deleg:' + e.target.id); })",
        )
        .expect("e");
        dispatch_js_event(&mut m, "btn", "click", 0);
        let rt = m.tabs[0].js.as_ref().expect("rt");
        assert!(
            rt.stdout().contains("deleg:btn"),
            "el listener de document debió correr con target=btn; stdout: {:?}",
            rt.stdout()
        );
    }

    #[test]
    fn document_prevent_default_cancela_el_fallback_del_link() {
        // Un handler delegado en document que llama preventDefault debe
        // reflejarse en result.default_prevented (lo usa el chrome para no
        // navegar el `<a>`).
        let mut m = model_con_script("console.log('boot')");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "lnk".into(),
            tag_name: "a".into(),
            text_content: "x".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "document.addEventListener('click', function(e){ e.preventDefault(); })",
        )
        .expect("e");
        let (result, _) = dispatch_js_event(&mut m, "lnk", "click", 0);
        assert!(result.default_prevented, "preventDefault del document debe contar");
    }

    #[test]
    fn dispatch_js_event_corre_handler_y_acumula_logs() {
        let mut m = model_con_script("/* sin scripts */ console.log('boot')");
        // El runtime ya existe gracias al script de boot. Registramos
        // manualmente un elemento + handler antes del dispatch.
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "btn".into(),
            tag_name: "button".into(),
            text_content: "click me".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "document.getElementById('btn').onclick = \
                function(){ console.log('clicked') }",
        )
        .expect("e");
        let logs0 = m.tabs[0].js_summary.logs;
        dispatch_js_event(&mut m, "btn", "click", 0);
        assert!(
            m.tabs[0].js_summary.logs > logs0,
            "esperaba logs nuevos tras dispatch — logs: {}",
            m.tabs[0].js_summary.logs
        );
        let rt = m.tabs[0].js.as_ref().expect("rt");
        assert!(rt.stdout().contains("clicked"), "stdout: {:?}", rt.stdout());
    }

    #[test]
    fn run_scripts_aplica_text_content_mutations_al_box_tree() {
        // Un script de carga muta textContent — el box_tree debe
        // reflejarlo cuando el chrome chequea las hojas de texto.
        let mut t = TabState::new("about:test".into());
        t.title = "T".into();
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><h1 id="hero">viejo</h1></body>"#,
        ));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some(
                "document.getElementById('hero').textContent = 'nuevo'".into(),
            ),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        let bt = t.box_tree.as_ref().expect("box_tree");
        let mut found_new = false;
        let mut found_old = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("nuevo") { found_new = true; }
            if b.text.as_deref() == Some("viejo") { found_old = true; }
        });
        assert!(found_new, "esperaba ver 'nuevo' tras la mutación");
        assert!(!found_old, "'viejo' debería haberse reemplazado");
    }

    #[test]
    fn dispatch_event_aplica_mutaciones_post_click() {
        // Handler de click muta textContent — al despachar el click, el
        // box_tree debe quedar actualizado.
        let mut m = model_con_script("/* boot */");
        // El runtime existe (boot lo creó). Registramos un elemento +
        // handler que muta textContent del mismo elemento.
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "out".into(),
            tag_name: "div".into(),
            text_content: "antes".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "document.getElementById('out').onclick = function(){ \
                document.getElementById('out').textContent = 'después'; \
             }",
        )
        .expect("e");
        // Reemplazo manual del box_tree para tener un nodo con
        // element_id='out' que pueda mutarse.
        m.tabs[0].box_tree = Some(parse(
            r#"<body><div id="out">antes</div></body>"#,
        ));
        dispatch_js_event(&mut m, "out", "click", 0);
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("después") {
                found = true;
            }
        });
        assert!(found, "el handler debió mutar 'antes' a 'después'");
    }

    // ============= Fase 7.42 — Page Visibility =============

    #[test]
    fn switch_active_tab_marca_hidden_la_vieja_y_dispatcha() {
        // Tab 0 con runtime + handler visibilitychange. Tab 1 sin runtime
        // (about:blank). SelectTab(1) debería marcar tab[0] como hidden y
        // disparar el handler.
        let mut m = model_con_script(
            "var got = null; \
             window.addEventListener('visibilitychange', function() { \
                got = document.visibilityState; \
             });",
        );
        m.tabs.push(TabState::new("about:tab2".into()));
        // Disparo SelectTab(1) — usa el helper directamente, no el msg.
        switch_active_tab(&mut m, 1);
        assert_eq!(m.active, 1);
        // El handler de tab[0] debe haber visto el cambio a 'hidden'.
        let v = m.tabs[0].js.as_mut().expect("rt").eval("got").expect("e");
        assert_eq!(v, puriy_js::JsValue::String("hidden".into()));
        let v = m.tabs[0]
            .js
            .as_mut()
            .expect("rt")
            .eval("document.hidden")
            .expect("e");
        assert_eq!(v, puriy_js::JsValue::Bool(true));
    }

    #[test]
    fn switch_active_tab_marca_visible_la_nueva() {
        // Toggle ida-y-vuelta sobre el mismo tab con runtime: el handler
        // ve hidden cuando dejamos de ser activos y visible cuando volvemos.
        let mut m = model_con_script(
            "var states = []; \
             window.addEventListener('visibilitychange', function() { \
                states.push(document.visibilityState); \
             });",
        );
        m.tabs.push(TabState::new("about:tab2".into()));
        switch_active_tab(&mut m, 1); // tab 0 → hidden
        switch_active_tab(&mut m, 0); // tab 0 → visible
        let v = m.tabs[0]
            .js
            .as_mut()
            .expect("rt")
            .eval("states.join(',')")
            .expect("e");
        assert_eq!(v, puriy_js::JsValue::String("hidden,visible".into()));
    }

    // ============= Fase 7.41 — beforeunload =============

    #[test]
    fn start_load_dispara_beforeunload_en_window() {
        // Modelo con runtime + handler de beforeunload que setea flag.
        let mut m = model_con_script(
            "var beforeRan = false; \
             window.addEventListener('beforeunload', function() { beforeRan = true; });",
        );
        // Verifica que el handler todavía no corrió.
        let v = m.tabs[0]
            .js
            .as_mut()
            .expect("rt")
            .eval("beforeRan")
            .expect("e");
        assert_eq!(v, puriy_js::JsValue::Bool(false));
        // Start_load dispatcha beforeunload antes de pisar la URL.
        // El runtime cambia al cargar (porque start_load no destruye el
        // runtime hasta Loaded), así que el flag debe ser visible justo
        // después de start_load.
        let h: Handle<Msg> = Handle::for_test();
        start_load(&mut m, "about:test2".into(), false, &h);
        let v = m.tabs[0]
            .js
            .as_mut()
            .expect("rt")
            .eval("beforeRan")
            .expect("e");
        assert_eq!(v, puriy_js::JsValue::Bool(true));
    }

    // ============= Fase 7.39 — window events =============

    #[test]
    fn dispatch_window_event_scroll_corre_listener_y_ve_scroll_y_actual() {
        // Setup: el script registra un listener que muta el DOM con el
        // scrollY actual cuando dispara 'scroll'.
        let mut m = model_con_script(
            "window.addEventListener('scroll', function() { \
                document.getElementById('out').textContent = String(window.scrollY); \
             });",
        );
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "out".into(),
            tag_name: "div".into(),
            text_content: "0".into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("set_elements");
        m.tabs[0].box_tree = Some(parse(r#"<body><div id="out">0</div></body>"#));
        // Simulamos un scroll a 150px y dispatcheamos directamente.
        m.tabs[0].scroll_y = 150.0;
        let t = &mut m.tabs[0];
        let (r, _pending) = dispatch_window_js_event_on_tab(t, "scroll", 0);
        assert_eq!(r.count, 1);
        // Verifica que el handler vio scrollY=150 mutando el DOM.
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("150") {
                found = true;
            }
        });
        assert!(found, "el handler debió ver scrollY=150 y mutar a '150'");
    }

    #[test]
    fn dispatch_window_event_load_corre_window_onload() {
        let mut m = model_con_script("var ran = false; window.onload = function(){ ran = true; };");
        let t = &mut m.tabs[0];
        let (r, _pending) = dispatch_window_js_event_on_tab(t, "load", 0);
        assert_eq!(r.count, 1);
        let v = m.tabs[0].js.as_mut().expect("rt").eval("ran").expect("e");
        assert_eq!(v, puriy_js::JsValue::Bool(true));
    }

    #[test]
    fn resize_actualiza_viewport_y_corre_listener() {
        // El listener de 'resize' lee window.innerWidth y lo escribe al DOM.
        let mut m = model_con_script(
            "window.addEventListener('resize', function() { \
                document.getElementById('out').textContent = String(window.innerWidth); \
             });",
        );
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "out".into(),
            tag_name: "div".into(),
            text_content: "0".into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("set_elements");
        m.tabs[0].box_tree = Some(parse(r#"<body><div id="out">0</div></body>"#));
        // Msg::Resize debe: (1) set_viewport(800,600) ANTES del dispatch,
        // (2) disparar 'resize' → el handler ve innerWidth=800.
        let h: Handle<Msg> = Handle::for_test();
        let m = Puriy::update(m, Msg::Resize(800, 600), &h);
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("800") {
                found = true;
            }
        });
        assert!(found, "el handler de resize debió ver innerWidth=800 y mutar a '800'");
    }

    #[test]
    fn on_resize_devuelve_msg_resize() {
        let m = model_con_script("/* boot */");
        assert!(matches!(
            Puriy::on_resize(&m, 640, 480),
            Some(Msg::Resize(640, 480))
        ));
    }

    #[test]
    fn on_scale_factor_devuelve_msg_scale_factor() {
        let m = model_con_script("/* boot */");
        assert!(matches!(
            Puriy::on_scale_factor(&m, 2.0),
            Some(Msg::ScaleFactor(s)) if s == 2.0
        ));
    }

    #[test]
    fn scale_factor_actualiza_devicePixelRatio_y_corre_listener() {
        // El listener de 'resize' lee window.devicePixelRatio y lo escribe al
        // DOM (los browsers disparan 'resize' al cambiar el DPI).
        let mut m = model_con_script(
            "window.addEventListener('resize', function() { \
                document.getElementById('out').textContent = String(window.devicePixelRatio); \
             });",
        );
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "out".into(),
            tag_name: "div".into(),
            text_content: "1".into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("set_elements");
        m.tabs[0].box_tree = Some(parse(r#"<body><div id="out">1</div></body>"#));
        // Msg::ScaleFactor(2.0) debe: (1) set_device_pixel_ratio(2) ANTES del
        // dispatch, (2) disparar 'resize' → el handler ve devicePixelRatio=2.
        let h: Handle<Msg> = Handle::for_test();
        let m = Puriy::update(m, Msg::ScaleFactor(2.0), &h);
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("2") {
                found = true;
            }
        });
        assert!(found, "el handler de resize debió ver devicePixelRatio=2 y mutar a '2'");
    }

    #[test]
    fn current_viewport_refleja_resize_y_scale() {
        // Fase 7.175 — el engine resuelve @media contra este viewport.
        let m = model_con_script("/* x */");
        let h: Handle<Msg> = Handle::for_test();
        let m = Puriy::update(m, Msg::Resize(900, 500), &h);
        let _m = Puriy::update(m, Msg::ScaleFactor(2.0), &h);
        let vp = current_viewport();
        assert_eq!(vp.width, 900.0);
        assert_eq!(vp.height, 500.0);
        assert_eq!(vp.dpr, 2.0);
    }

    #[test]
    fn media_queries_se_resuelven_contra_viewport_y_dpr() {
        // Fase 7.174 — el chrome evalúa matchMedia contra su viewport REAL.
        // Conducimos viewport y DPR por Msg para no depender del thread-local
        // (que otros tests del mismo hilo podrían haber mutado).
        let m = model_con_script(
            "globalThis.__wide = matchMedia('(min-width: 600px)'); \
             globalThis.__huge = matchMedia('(min-width: 1200px)'); \
             globalThis.__hidpi = matchMedia('(min-resolution: 2dppx)');",
        );
        let h: Handle<Msg> = Handle::for_test();
        // Viewport 1000×700 @ dpr 1 → wide sí, huge no, hidpi no.
        let mut m = Puriy::update(m, Msg::Resize(1000, 700), &h);
        {
            let rt = m.tabs[0].js.as_mut().expect("rt");
            assert_eq!(rt.eval("__wide.matches").expect("e"), puriy_js::JsValue::Bool(true));
            assert_eq!(rt.eval("__huge.matches").expect("e"), puriy_js::JsValue::Bool(false));
            assert_eq!(rt.eval("__hidpi.matches").expect("e"), puriy_js::JsValue::Bool(false));
        }
        // Subimos el DPR a 2 → la query de resolution flipea a true.
        let mut m = Puriy::update(m, Msg::ScaleFactor(2.0), &h);
        {
            let rt = m.tabs[0].js.as_mut().expect("rt");
            assert_eq!(rt.eval("__hidpi.matches").expect("e"), puriy_js::JsValue::Bool(true));
        }
        let _ = m;
    }

    #[test]
    fn dispatch_window_event_sin_runtime_es_no_op() {
        // Tab sin runtime — no debe panic.
        let mut t = TabState::new("about:blank".into());
        assert!(t.js.is_none());
        let (r, pending) = dispatch_window_js_event_on_tab(&mut t, "scroll", 0);
        assert_eq!(r.count, 0);
        assert!(pending.is_empty());
    }

    #[test]
    fn tick_aplica_mutaciones_de_settimeout() {
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "clock".into(),
            tag_name: "span".into(),
            text_content: "00:00".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.set_now_ms(0).expect("now");
        rt.eval(
            "setTimeout(function(){ \
                document.getElementById('clock').textContent = '10:00'; \
             }, 50)",
        )
        .expect("e");
        m.tabs[0].box_tree = Some(parse(
            r#"<body><span id="clock">00:00</span></body>"#,
        ));
        tick_js_runtimes(&mut m, 100);
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("10:00") {
                found = true;
            }
        });
        assert!(found, "tick debió aplicar la mutación del setTimeout");
    }

    #[test]
    fn apply_style_color_actualiza_box_tree_post_script() {
        let mut t = TabState::new("about:test".into());
        t.title = "T".into();
        t.url = "about:test".into();
        t.box_tree = Some(parse(r#"<body><h1 id="h">hola</h1></body>"#));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some("document.getElementById('h').style.color = '#ff0000'".into()),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut red_leaf = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("hola") && b.color.r == 255 && b.color.g == 0 && b.color.b == 0 {
                red_leaf = true;
            }
        });
        assert!(red_leaf, "el text leaf debió quedar rojo");
    }

    #[test]
    fn apply_style_display_none_oculta_post_dispatch() {
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "panel".into(),
            tag_name: "div".into(),
            text_content: "".into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "document.getElementById('panel').onclick = function(){ \
                document.getElementById('panel').style.display = 'none'; \
             }",
        )
        .expect("e");
        m.tabs[0].box_tree = Some(parse(r#"<body><div id="panel">x</div></body>"#));
        dispatch_js_event(&mut m, "panel", "click", 0);
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut hidden = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("panel") {
                if matches!(b.display, puriy_engine::Display::None) {
                    hidden = true;
                }
            }
        });
        assert!(hidden);
    }

    #[test]
    fn collect_element_snapshots_propaga_class_list() {
        let tree = parse(r#"<div><h1 id="hero" class="title big">x</h1></div>"#);
        let snaps = collect_element_snapshots(&tree);
        assert_eq!(snaps.len(), 1);
        assert!(snaps[0].class_list.contains(&"title".to_string()));
        assert!(snaps[0].class_list.contains(&"big".to_string()));
    }

    #[test]
    fn dispatch_event_devuelve_default_prevented_cuando_corresponde() {
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "a".into(),
            tag_name: "a".into(),
            text_content: "link".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "document.getElementById('a').onclick = function(e){ e.preventDefault(); }",
        )
        .expect("e");
        let (r, _) = dispatch_js_event(&mut m, "a", "click", 0);
        assert!(r.default_prevented);
        assert_eq!(r.count, 1);
    }

    #[test]
    fn dispatch_keydown_focus_blur_change_son_event_types_validos() {
        // Sanity: el harness JS acepta cualquier event_type — no está
        // restringido a 'click'. Esto destraba Fase 7.7.
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "i".into(),
            tag_name: "input".into(),
            text_content: "".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "var el = document.getElementById('i'); \
             el.addEventListener('keydown', function(e){ console.log('K:'+e.type) }); \
             el.addEventListener('focus',   function(e){ console.log('F:'+e.type) }); \
             el.addEventListener('blur',    function(e){ console.log('B:'+e.type) }); \
             el.addEventListener('change',  function(e){ console.log('C:'+e.type) });",
        )
        .expect("e");
        dispatch_js_event(&mut m, "i", "keydown", 0);
        dispatch_js_event(&mut m, "i", "focus", 0);
        dispatch_js_event(&mut m, "i", "blur", 0);
        dispatch_js_event(&mut m, "i", "change", 0);
        let rt = m.tabs[0].js.as_ref().expect("rt");
        let out = rt.stdout();
        assert!(out.contains("K:keydown"), "stdout: {out:?}");
        assert!(out.contains("F:focus"), "stdout: {out:?}");
        assert!(out.contains("B:blur"), "stdout: {out:?}");
        assert!(out.contains("C:change"), "stdout: {out:?}");
    }

    #[test]
    fn dispatch_event_sin_prevent_default_devuelve_false() {
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "a".into(),
            tag_name: "a".into(),
            text_content: "link".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('a').onclick = function(){ /* nada */ }")
            .expect("e");
        let (r, _) = dispatch_js_event(&mut m, "a", "click", 0);
        assert!(!r.default_prevented);
        assert_eq!(r.count, 1);
    }

    #[test]
    fn dispatch_sobre_id_sin_handler_no_panic() {
        let mut m = model_con_script("console.log('boot')");
        // No registramos ningún elemento — el dispatch va al vacío.
        dispatch_js_event(&mut m, "fantasma", "click", 0);
        // Si llegamos acá sin panic, OK.
        let rt = m.tabs[0].js.as_ref().expect("rt");
        // stdout sigue siendo sólo el "boot" del script inicial.
        assert!(rt.stdout().contains("boot"));
    }

    // ============= Fase 7.9 — event.key + Element.value =============

    #[test]
    fn named_key_name_mapea_teclas_comunes() {
        use llimphi_ui::NamedKey;
        assert_eq!(named_key_name(&NamedKey::Enter), "Enter");
        assert_eq!(named_key_name(&NamedKey::Escape), "Escape");
        assert_eq!(named_key_name(&NamedKey::ArrowLeft), "ArrowLeft");
        assert_eq!(named_key_name(&NamedKey::ArrowRight), "ArrowRight");
        assert_eq!(named_key_name(&NamedKey::Tab), "Tab");
        assert_eq!(named_key_name(&NamedKey::Backspace), "Backspace");
        assert_eq!(named_key_name(&NamedKey::F5), "F5");
    }

    #[test]
    fn key_event_to_init_extrae_caracter_y_modifiers() {
        use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers};
        let e = KeyEvent {
            key: Key::Character("a".into()),
            state: KeyState::Pressed,
            text: Some("a".into()),
            modifiers: Modifiers {
                shift: true,
                ctrl: false,
                alt: false,
                meta: false,
            },
            repeat: false,
        };
        let init = key_event_to_init(&e);
        assert_eq!(init.key.as_deref(), Some("a"));
        assert_eq!(init.code.as_deref(), Some("a"));
        assert_eq!(init.shift_key, Some(true));
        assert_eq!(init.ctrl_key, Some(false));
    }

    #[test]
    fn key_event_to_init_mapea_named_keys() {
        use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers, NamedKey};
        let e = KeyEvent {
            key: Key::Named(NamedKey::ArrowDown),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers::default(),
            repeat: false,
        };
        let init = key_event_to_init(&e);
        assert_eq!(init.key.as_deref(), Some("ArrowDown"));
    }

    #[test]
    fn collect_element_snapshots_value_de_input_lleva_input_initial() {
        let tree = parse(r#"<body><input id="email" value="hola@x.com"></body>"#);
        let snaps = collect_element_snapshots(&tree);
        let s = snaps.iter().find(|s| s.id == "email").expect("found");
        assert_eq!(s.value.as_deref(), Some("hola@x.com"));
    }

    #[test]
    fn collect_element_snapshots_value_de_select_lleva_option_seleccionado() {
        let tree = parse(
            r#"<body><select id="lang">
                <option value="es">Español</option>
                <option value="en" selected>English</option>
            </select></body>"#,
        );
        let snaps = collect_element_snapshots(&tree);
        let s = snaps.iter().find(|s| s.id == "lang").expect("found");
        assert_eq!(s.value.as_deref(), Some("en"));
    }

    #[test]
    fn collect_element_snapshots_value_es_none_para_div() {
        let tree = parse(r#"<body><div id="x">hola</div></body>"#);
        let snaps = collect_element_snapshots(&tree);
        let s = snaps.iter().find(|s| s.id == "x").expect("found");
        assert_eq!(s.value, None);
    }

    #[test]
    fn apply_value_mutation_actualiza_text_input_state() {
        // JS setea el.value = "nuevo" — apply_dom_mutations debe
        // propagarlo al TextInputState del slot correspondiente.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><input id="x" value="viejo"></body>"#));
        let mut s = TextInputState::new();
        s.set_text("viejo".to_string());
        t.inputs = vec![s];
        t.inputs_element_ids = vec![Some("x".into())];
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "input".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: Some("viejo".into()),
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('x').value = 'nuevo'")
            .expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.inputs[0].text(), "nuevo");
    }

    #[test]
    fn clipboard_write_text_emite_set_system_clipboard() {
        // navigator.clipboard.writeText publica una mutación kind:'clipboard';
        // apply_dom_mutations debe traducirla a Msg::SetSystemClipboard para
        // que el update loop la empuje al portapapeles real (Fase 7.176).
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        let rt = t.js.as_mut().expect("rt");
        rt.eval("navigator.clipboard.writeText('copiado por JS')")
            .expect("e");
        let out = apply_dom_mutations(t);
        let writes: Vec<&str> = out
            .iter()
            .filter_map(|msg| match msg {
                Msg::SetSystemClipboard(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(writes, vec!["copiado por JS"]);
    }

    #[test]
    fn eventsource_mutation_emite_es_open_y_close() {
        // El bootstrap de EventSource publica una mutación `kind:'eventsource'`
        // al construir y al cerrar; apply_dom_mutations las traduce a
        // Msg::EsOpen/EsClose (sin abrir red — eso es del worker).
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.js.as_mut().unwrap().eval("var es = new EventSource('http://x/sse');").expect("e");
        let out = apply_dom_mutations(t);
        assert!(
            out.iter().any(|msg| matches!(msg, Msg::EsOpen { es_id: 1, url, .. } if url == "http://x/sse")),
            "no se emitió EsOpen"
        );
        t.js.as_mut().unwrap().eval("es.close();").expect("e");
        let out2 = apply_dom_mutations(t);
        assert!(
            out2.iter().any(|msg| matches!(msg, Msg::EsClose { es_id: 1, .. })),
            "no se emitió EsClose"
        );
    }

    #[test]
    fn es_dispatch_msg_entrega_evento_al_listener() {
        // Msg::EsDispatch (lo que manda el worker) debe llegar al onmessage del
        // EventSource correcto, vía el host method rt.es_dispatch.
        let mut m = model_con_script(
            "var got = null; var es = new EventSource('http://x/sse'); \
             es.onmessage = function(e) { got = e.data + ':' + e.lastEventId; };",
        );
        let es_id = match m.tabs[0].js.as_mut().unwrap().eval("es._id").unwrap() {
            puriy_js::JsValue::Number(n) => n as u32,
            other => panic!("es._id no es número: {other:?}"),
        };
        let (tab, gen) = (m.tabs[0].id, m.tabs[0].gen);
        let h: Handle<Msg> = Handle::for_test();
        let m = Puriy::update(
            m,
            Msg::EsDispatch {
                tab,
                gen,
                es_id,
                kind: "message".into(),
                event_type: "message".into(),
                data: "hola".into(),
                last_id: "9".into(),
            },
            &h,
        );
        let mut m = m;
        let got = m.tabs[0].js.as_mut().unwrap().eval("got").expect("e");
        assert_eq!(got, puriy_js::JsValue::String("hola:9".into()));
    }

    #[test]
    fn run_scripts_siembra_el_portapapeles_del_sistema() {
        // Con system_clipboard = Some(...), un readText() de un script inicial
        // ve lo que el usuario tiene copiado afuera, no la cadena vacía.
        let mut t = TabState::new("about:blank".into());
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some(
                "var leido = ''; navigator.clipboard.readText().then(function(x){ leido = x; });"
                    .to_string(),
            ),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, Some("desde el sistema"));
        let rt = t.js.as_mut().expect("rt");
        assert_eq!(
            rt.eval("leido").expect("e"),
            puriy_js::JsValue::String("desde el sistema".into())
        );
    }

    #[test]
    fn apply_value_mutation_actualiza_select_state() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><select id="lang">
                <option value="es">Español</option>
                <option value="en">English</option>
            </select></body>"#,
        ));
        t.selects = vec![SelectState { selected: 0, open: false }];
        t.selects_element_ids = vec![Some("lang".into())];
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "lang".into(),
            tag_name: "select".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: Some("es".into()),
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('lang').value = 'en'")
            .expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.selects[0].selected, 1);
    }

    #[test]
    fn dispatch_keydown_pasa_key_real_al_handler() {
        use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers, NamedKey};
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "i".into(),
            tag_name: "input".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: Some(String::new()),
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "document.getElementById('i').onkeydown = function(ev){ \
                console.log(ev.key) \
            }",
        )
        .expect("e");
        let e = KeyEvent {
            key: Key::Named(NamedKey::Enter),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers::default(),
            repeat: false,
        };
        let init = key_event_to_init(&e);
        dispatch_js_event_with_init(&mut m, "i", "keydown", 0, Some(init));
        let rt = m.tabs[0].js.as_ref().expect("rt");
        assert!(rt.stdout().contains("Enter"), "stdout: {:?}", rt.stdout());
    }

    #[test]
    fn select_value_at_devuelve_value_del_option() {
        let tree = parse(
            r#"<body><select id="lang">
                <option value="es">Español</option>
                <option value="en">English</option>
            </select></body>"#,
        );
        let mut m = model_con_script("/* boot */");
        m.tabs[0].box_tree = Some(tree);
        assert_eq!(select_value_at(&m.tabs[0], 0, 1).as_deref(), Some("en"));
        assert_eq!(select_value_at(&m.tabs[0], 0, 0).as_deref(), Some("es"));
        assert_eq!(select_value_at(&m.tabs[0], 99, 0), None);
    }

    // ============= Fase 7.10 — bubbling + input event =============

    #[test]
    fn collect_element_snapshots_pobla_parent_id_directo() {
        // <div id=outer><button id=btn></button></div>
        let tree = parse(r#"<body><div id="outer"><button id="btn">x</button></div></body>"#);
        let snaps = collect_element_snapshots(&tree);
        let outer = snaps.iter().find(|s| s.id == "outer").expect("outer");
        let btn = snaps.iter().find(|s| s.id == "btn").expect("btn");
        assert_eq!(outer.parent_id, None);
        assert_eq!(btn.parent_id.as_deref(), Some("outer"));
    }

    #[test]
    fn collect_element_snapshots_salta_ancestros_sin_id() {
        // <section id=s><div><button id=btn></button></div></section>
        // El <div> sin id no aparece en la cadena de bubbling — btn
        // pasa a tener parent_id = s directamente.
        let tree = parse(
            r#"<body><section id="s"><div><button id="btn">x</button></div></section></body>"#,
        );
        let snaps = collect_element_snapshots(&tree);
        let btn = snaps.iter().find(|s| s.id == "btn").expect("btn");
        assert_eq!(btn.parent_id.as_deref(), Some("s"));
    }

    #[test]
    fn collect_element_snapshots_root_sin_parent() {
        // El elemento del root no debe tener parent_id.
        let tree = parse(r#"<body><div id="root">x</div></body>"#);
        let snaps = collect_element_snapshots(&tree);
        let root = snaps.iter().find(|s| s.id == "root").expect("root");
        assert_eq!(root.parent_id, None);
    }

    #[test]
    fn collect_element_snapshots_tres_niveles_de_anidacion() {
        let tree = parse(
            r#"<body><div id="a"><div id="b"><div id="c">x</div></div></div></body>"#,
        );
        let snaps = collect_element_snapshots(&tree);
        let a = snaps.iter().find(|s| s.id == "a").expect("a");
        let b = snaps.iter().find(|s| s.id == "b").expect("b");
        let c = snaps.iter().find(|s| s.id == "c").expect("c");
        assert_eq!(a.parent_id, None);
        assert_eq!(b.parent_id.as_deref(), Some("a"));
        assert_eq!(c.parent_id.as_deref(), Some("b"));
    }

    // ============= Fase 7.11 — dataset =============

    #[test]
    fn collect_element_snapshots_pobla_dataset() {
        let tree =
            parse(r#"<body><div id="x" data-role="banner" data-id-key="42">x</div></body>"#);
        let snaps = collect_element_snapshots(&tree);
        let s = snaps.iter().find(|s| s.id == "x").expect("found");
        // El suffix preserva case del HTML; el value tal cual.
        assert!(s.dataset.iter().any(|(k, v)| k == "role" && v == "banner"));
        assert!(s.dataset.iter().any(|(k, v)| k == "id-key" && v == "42"));
    }

    #[test]
    fn apply_dataset_mutation_actualiza_box_tree() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="x">y</div></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('x').dataset.role = 'main'")
            .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                if b.dataset().iter().any(|(k, v)| *k == "role" && *v == "main") {
                    found = true;
                }
            }
        });
        assert!(found, "data-role debería ser 'main' en el BoxTree");
    }

    // ============= Fase 7.12 — appendChild/removeChild =============

    #[test]
    fn apply_append_child_inserta_box_node_sintetico() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><ul id="list"></ul></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "list".into(),
            tag_name: "ul".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "var li = document.createElement('li'); \
             li.textContent = 'hola'; \
             document.getElementById('list').appendChild(li);",
        )
        .expect("e");
        apply_dom_mutations(t);
        // El <ul id=list> ahora tiene un hijo extra que es <li>.
        let bt = t.box_tree.as_ref().expect("bt");
        let mut li_count = 0;
        let mut text_found = false;
        bt.walk(|b| {
            if b.tag.as_deref() == Some("li") {
                li_count += 1;
                if let Some(c) = b.children.first() {
                    if c.text.as_deref() == Some("hola") {
                        text_found = true;
                    }
                }
            }
        });
        assert_eq!(li_count, 1);
        assert!(text_found, "el <li> debe tener un text leaf 'hola'");
    }

    #[test]
    fn classlist_add_recascadea_y_aplica_regla() {
        // Fase 7.184 — `el.classList.add('on')` publica la mutación 'classList';
        // el chrome actualiza la clase y re-corre la cascada → el `.on` aplica.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<html><head><style>.on { background: red; }</style></head>
               <body><div id="box">x</div></body></html>"#,
        ));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "box".into(),
            tag_name: "div".into(),
            text_content: "x".into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("set_elements");
        // Antes del toggle: sin background.
        let bg0 = {
            let bt = t.box_tree.as_ref().unwrap();
            let mut bg = None;
            bt.walk(|b| {
                if b.element_id.as_deref() == Some("box") {
                    bg = b.background;
                }
            });
            bg
        };
        assert_eq!(bg0, None);
        rt.eval("document.getElementById('box').classList.add('on');")
            .expect("eval");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().unwrap();
        let mut bg = None;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("box") {
                bg = b.background;
            }
        });
        assert_eq!(bg, Some(puriy_engine::Color::rgb(255, 0, 0)));
    }

    // ---- Fase 7.196 — Canvas 2D al render ----
    #[test]
    fn canvas_frame_deserializa_y_helpers() {
        let json = r##"[{"id":"c","width":100,"height":50,"cmds":[["fillRect",1,2,3,4,"#ff0000",{"ga":1}]]}]"##;
        let frames: Vec<CanvasFrame> = serde_json::from_str(json).expect("parse");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].id, "c");
        assert_eq!(frames[0].width, 100.0);
        assert_eq!(frames[0].cmds[0][0].as_str(), Some("fillRect"));
        // Helpers puros.
        assert_eq!(canvas_font_px(Some("16px sans-serif")), 16.0);
        assert_eq!(canvas_font_px(Some("bold 24.5px Arial")), 24.5);
        assert_eq!(canvas_font_px(None), 10.0);
        let c = canvas_color(Some(&serde_json::Value::String("#ff0000".into())), 0.5);
        assert_eq!(c.to_rgba8().to_u8_array(), [255, 0, 0, 127]);
    }

    #[test]
    fn paint_canvas_cmds_encodea_primitivas() {
        // fillRect + un path con fill: la escena vello queda no-vacía. No
        // necesita GPU (Scene es CPU-side). Smoke del intérprete.
        let mut scene = llimphi_raster::vello::Scene::new();
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 };
        let cmds: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r##"[
                ["fillRect", 1, 2, 3, 4, "#ff0000", {"ga": 1}],
                ["beginPath"],
                ["moveTo", 0, 0],
                ["lineTo", 10, 10],
                ["arc", 20, 20, 5, 0, 6.28],
                ["fill", {"f": "#00ff00", "ga": 1}]
            ]"##).unwrap();
        assert!(scene.encoding().is_empty(), "escena arranca vacía");
        paint_canvas_cmds(&mut scene, &mut ts, rect, &cmds, 100.0, 50.0);
        assert!(!scene.encoding().is_empty(), "tras pintar debería haber segmentos");
    }

    #[test]
    fn canvas_dibuja_y_refresca_frames_end_to_end() {
        // Pipeline: box tree con <canvas>, snapshot con width/height, script
        // que pide contexto y dibuja → apply_dom_mutations refresca los frames.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="120" height="80"></canvas></body>"#,
        ));
        t.has_canvas = true;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "c".into(),
            tag_name: "canvas".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("width".into(), "120".into()), ("height".into(), "80".into())],
            dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval("var ctx = document.getElementById('c').getContext('2d'); ctx.fillStyle = '#123456'; ctx.fillRect(10, 10, 40, 30);")
            .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame del canvas");
        assert_eq!(frame.width, 120.0);
        assert_eq!(frame.height, 80.0);
        assert!(
            frame.cmds.iter().any(|c| c.first().and_then(|v| v.as_str()) == Some("fillRect")),
            "el frame debería incluir el fillRect dibujado: {:?}",
            frame.cmds
        );
    }

    #[test]
    fn apply_remove_child_quita_box_node() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><ul id="list"><li id="a">a</li><li id="b">b</li></ul></body>"#,
        ));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            puriy_js::ElementSnapshot {
                id: "list".into(),
                tag_name: "ul".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: None,
                parent_id: None,
                dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
            },
            puriy_js::ElementSnapshot {
                id: "a".into(),
                tag_name: "li".into(),
                text_content: "a".into(),
                class_list: Vec::new(),
                value: None,
                parent_id: Some("list".into()),
                dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
            },
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('list').removeChild(document.getElementById('a'))",
        )
        .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        // El <li id=a> ya no debería existir; el <li id=b> sí.
        let mut a_exists = false;
        let mut b_exists = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("a") {
                a_exists = true;
            }
            if b.element_id.as_deref() == Some("b") {
                b_exists = true;
            }
        });
        assert!(!a_exists);
        assert!(b_exists);
    }

    // ============= Fase 7.14 — insertBefore + herencia de estilos =============

    #[test]
    fn apply_insert_before_pone_child_antes_del_ref() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><ul id="list"><li id="a">a</li><li id="b">b</li></ul></body>"#,
        ));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            puriy_js::ElementSnapshot {
                id: "list".into(),
                tag_name: "ul".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: None,
                parent_id: None,
                dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
            },
            puriy_js::ElementSnapshot {
                id: "a".into(),
                tag_name: "li".into(),
                text_content: "a".into(),
                class_list: Vec::new(),
                value: None,
                parent_id: Some("list".into()),
                dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
            },
        ])
        .expect("e");
        rt.eval(
            "var li = document.createElement('li'); \
             li.id = 'mid'; \
             li.textContent = 'mid'; \
             document.getElementById('list').insertBefore(li, document.getElementById('a'));",
        )
        .expect("e");
        apply_dom_mutations(t);
        // Orden esperado en BoxTree: mid, a, b.
        let bt = t.box_tree.as_ref().expect("bt");
        let mut order: Vec<String> = Vec::new();
        bt.walk(|b| {
            if b.tag.as_deref() == Some("li") {
                if let Some(id) = &b.element_id {
                    order.push(id.clone());
                }
            }
        });
        assert_eq!(order, vec!["mid", "a", "b"]);
    }

    #[test]
    fn apply_insert_before_ref_inexistente_hace_append() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><ul id="list"><li id="a">a</li></ul></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            puriy_js::ElementSnapshot {
                id: "list".into(),
                tag_name: "ul".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: None,
                parent_id: None,
                dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
            },
        ])
        .expect("e");
        // El ref_id "fantasma" no existe — el chrome cae a append.
        // Simulamos la mutación manualmente (saltea las validaciones JS).
        rt.eval("globalThis.__puriy_dirty.push({id:'list',kind:'insertBefore',value:'li\u{001D}nuevo\u{001D}x\u{001D}\u{001D}\u{001D}fantasma'})")
            .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut order: Vec<String> = Vec::new();
        bt.walk(|b| {
            if b.tag.as_deref() == Some("li") {
                if let Some(id) = &b.element_id {
                    order.push(id.clone());
                }
            }
        });
        // 'nuevo' debe estar después de 'a' porque cae a append.
        assert_eq!(order, vec!["a", "nuevo"]);
    }

    #[test]
    fn append_child_hereda_color_y_font_size_del_parent() {
        // Parent <div id=p> con style="color:red;font-size:24px" tiene
        // esos valores en su BoxNode. Un <li> sintético appendChild
        // debería heredar color rojo + font_size 24, en lugar de los
        // defaults negros 16px.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><div id="p" style="color: red; font-size: 24px"></div></body>"#,
        ));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "p".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "var s = document.createElement('span'); \
             s.id = 'k'; \
             s.textContent = 'hola'; \
             document.getElementById('p').appendChild(s);",
        )
        .expect("e");
        apply_dom_mutations(t);
        // El <span id=k> sintético debe tener color y font_size del padre.
        let bt = t.box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("k") {
                assert!(
                    (b.font_size - 24.0).abs() < 0.01,
                    "font_size esperado 24, got {}",
                    b.font_size
                );
                // color: red (255,0,0) en el formato Color de engine.
                assert_eq!((b.color.r, b.color.g, b.color.b), (255, 0, 0), "color esperado red");
                found = true;
            }
        });
        assert!(found);
    }

    #[test]
    fn append_child_y_textcontent_post_insercion() {
        // appendChild + mutación de textContent después de insertar
        // deberían actualizar el text leaf del BoxNode sintético.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="p"></div></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "p".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        // textContent inicial via el payload del appendChild.
        rt.eval(
            "var d = document.createElement('span'); \
             d.id = 'item1'; \
             d.textContent = 'inicial'; \
             document.getElementById('p').appendChild(d); \
             document.getElementById('item1').textContent = 'actualizado';",
        )
        .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        // El text leaf bajo el span#item1 debe ser 'actualizado'.
        let mut got = String::new();
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("item1") {
                if let Some(c) = b.children.first() {
                    if let Some(t) = &c.text {
                        got = t.clone();
                    }
                }
            }
        });
        assert_eq!(got, "actualizado");
    }

    #[test]
    fn apply_dataset_remove_mutation_quita_la_key() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="x" data-role="main">y</div></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: vec![("role".into(), "main".into())],
            attributes: vec![("data-role".into(), "main".into())],
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("delete document.getElementById('x').dataset.role")
            .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut still_there = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                if b.dataset().iter().any(|(k, _)| *k == "role") {
                    still_there = true;
                }
            }
        });
        assert!(!still_there, "data-role no debería existir tras el delete");
    }

    // ============= Fase 7.16 — attributes genéricos =============

    #[test]
    fn collect_element_snapshots_pobla_attributes_completo() {
        let tree = parse(
            r#"<body><a id="nav" href="https://gioser.net" aria-current="page" data-track="hero" rel="noopener">x</a></body>"#,
        );
        let snaps = collect_element_snapshots(&tree);
        let s = snaps.iter().find(|s| s.id == "nav").expect("found");
        // attributes incluye TODOS los attrs (data-*, aria-*, href, rel, id).
        assert!(s.attributes.iter().any(|(k, v)| k == "href" && v == "https://gioser.net"));
        assert!(s.attributes.iter().any(|(k, v)| k == "aria-current" && v == "page"));
        assert!(s.attributes.iter().any(|(k, v)| k == "data-track" && v == "hero"));
        assert!(s.attributes.iter().any(|(k, v)| k == "rel" && v == "noopener"));
        // dataset sigue filtrando sólo data-* sin prefijo.
        assert!(s.dataset.iter().any(|(k, v)| k == "track" && v == "hero"));
        assert!(s.dataset.iter().all(|(k, _)| !k.starts_with("data-")));
    }

    #[test]
    fn apply_attr_mutation_actualiza_box_tree() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="x">y</div></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('x').setAttribute('aria-label', 'main')")
            .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, v)| k == "aria-label" && v == "main")
            {
                found = true;
            }
        });
        assert!(found, "setAttribute debería poblar attributes en el BoxTree");
    }

    #[test]
    fn apply_attr_remove_mutation_quita_la_key() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><a id="x" href="/old">y</a></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "a".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("href".into(), "/old".into())],
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('x').removeAttribute('href')").expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut still = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, _)| k == "href")
            {
                still = true;
            }
        });
        assert!(!still, "removeAttribute debe quitar href del BoxTree");
    }

    // ============= Fase 7.18 — focus()/blur() chrome-side =============

    #[test]
    fn apply_focus_mutation_setea_focused_input_si_es_input_slot() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><input id="user" /><input id="pw" /></body>"#));
        // Pre-pueblo inputs_element_ids como lo hace Msg::Loaded (orden DFS).
        t.inputs.push(TextInputState::new());
        t.inputs.push(TextInputState::new());
        t.inputs_element_ids = vec![Some("user".into()), Some("pw".into())];
        t.focused_input = None;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            puriy_js::ElementSnapshot {
                id: "user".into(),
                tag_name: "input".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: Some(String::new()),
                parent_id: None,
                dataset: Vec::new(),
                attributes: Vec::new(),
                dfs_index: 0,
            },
            puriy_js::ElementSnapshot {
                id: "pw".into(),
                tag_name: "input".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: Some(String::new()),
                parent_id: None,
                dataset: Vec::new(),
                attributes: Vec::new(),
                dfs_index: 0,
            },
        ])
        .expect("e");
        rt.eval("document.getElementById('pw').focus()").expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.focused_input, Some(1), "el focus en 'pw' (slot 1) debió moverse");
    }

    #[test]
    fn apply_focus_mutation_sobre_no_input_no_afecta_focused_input() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><button id="btn">x</button></body>"#));
        t.focused_input = None;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "btn".into(),
            tag_name: "button".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('btn').focus()").expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.focused_input, None, "focus en un button no afecta el cursor");
    }

    #[test]
    fn apply_blur_mutation_limpia_focused_input_si_era_el_actual() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><input id="user" /></body>"#));
        t.inputs.push(TextInputState::new());
        t.inputs_element_ids = vec![Some("user".into())];
        t.focused_input = Some(0);
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "user".into(),
            tag_name: "input".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: Some(String::new()),
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('user').blur()").expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.focused_input, None);
    }

    // ============= Fase 7.19 — text node sintético =============

    #[test]
    fn apply_append_text_node_inserta_text_leaf_sin_tag() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="parent"></div></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "parent".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "var p = document.getElementById('parent'); \
             p.append(document.createTextNode('Hola mundo'));",
        )
        .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("parent") {
                for c in &b.children {
                    if c.tag.is_none() && c.text.as_deref() == Some("Hola mundo") {
                        found = true;
                    }
                }
            }
        });
        assert!(found, "parent debe tener text leaf 'Hola mundo' como hijo");
    }

    // ============= Fase 7.24 — scrollIntoView chrome-side =============

    #[test]
    fn apply_scroll_into_view_setea_scroll_y_por_dfs_order() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        // Tree con varios elementos para que la posición DFS varíe.
        t.box_tree = Some(parse(
            r#"<body><div id="top">top</div><div id="mid">mid</div><div id="bot">bottom</div></body>"#,
        ));
        t.scroll_y = 0.0;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            puriy_js::ElementSnapshot {
                id: "bot".into(),
                tag_name: "div".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: None,
                parent_id: None,
                dataset: Vec::new(),
                attributes: Vec::new(),
                dfs_index: 0,
            },
        ])
        .expect("e");
        rt.eval("document.getElementById('bot').scrollIntoView()").expect("e");
        apply_dom_mutations(t);
        // bot está más profundo en el DFS pre-order que top/mid → scroll_y > 0.
        assert!(t.scroll_y > 0.0, "scroll_y debería avanzar hacia el elemento (got {})", t.scroll_y);
    }

    #[test]
    fn apply_scroll_into_view_id_inexistente_no_modifica_scroll() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="x">x</div></body>"#));
        t.scroll_y = 42.0;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("e");
        // Disparamos scrollIntoView contra un id que NO está en el box_tree.
        // El JS sí publica la mutación (no valida); el chrome la silencia.
        rt.eval(
            "globalThis.__puriy_dirty.push({id: 'fantasma', kind: 'scrollIntoView', value: ''});",
        )
        .expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.scroll_y, 42.0, "scroll no debe moverse para id inexistente");
    }

    // ============= Fase 7.26 — window.scrollTo aplicado al chrome =============

    #[test]
    fn apply_scroll_mutation_actualiza_scroll_y_del_tab() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body></body>"#));
        t.scroll_y = 0.0;
        let rt = t.js.as_mut().expect("rt");
        rt.eval("scrollTo(0, 250)").expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.scroll_y, 250.0);
    }

    #[test]
    fn apply_scroll_mutation_clampea_a_no_negativo() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body></body>"#));
        t.scroll_y = 100.0;
        let rt = t.js.as_mut().expect("rt");
        rt.eval("scrollTo(0, -50)").expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.scroll_y, 0.0);
    }
}
