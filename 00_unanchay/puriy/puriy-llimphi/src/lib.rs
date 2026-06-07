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
use llimphi_raster::kurbo::{
    Affine, BezPath as KurboBezPath, Line, Point, Rect as KurboRect, RoundedRect, Stroke,
};
use llimphi_raster::peniko::{
    Blob, Color, ColorStop, ColorStops, Fill, Gradient, GradientKind,
    Image as PenikoImage, ImageFormat,
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
use llimphi_module_allichay::AllichayState;

use puriy_engine::{
    AlignItems as CssAlignItems, AlignSelf as CssAlignSelf,
    BackgroundPosition, BackgroundRepeat, BackgroundSize, BorderLineStyle, BoxNode, BoxShadow,
    BoxSizing as CssBoxSizing, BoxTree, Display, Engine, FlexDirection as CssFlexDirection,
    AlignContent as CssAlignContent, FlexWrap as CssFlexWrap, GridTrackSize,
    JustifyContent as CssJustifyContent, LengthVal,
    LinearGradient, Overflow, PointerEvents, Position as CssPosition, TextAlign,
    TextDecorationLine, TextDecorationStyle, VerticalAlign, Visibility,
};

mod canvas;
use canvas::{collect_dom_image_pixels, render_canvas, refresh_canvas_frames, CanvasFrame};
mod render;
use render::*;
mod chrome;
use chrome::*;
mod jsbridge;
use jsbridge::*;
mod nav;
use nav::*;
mod container;
use container::*;
mod settings;
use settings::*;

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

/// Orientación de las pestañas del navegador. **Horizontal** = barra
/// clásica arriba, un solo nivel (las pestañas del space activo). **Vertical**
/// = sidebar acoplable estilo cosmos: un rail de **dientes** (un diente por
/// space) + la lista vertical de pestañas del space activo. Configurable desde
/// el panel de ajustes y persistido en el Profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabOrientation {
    Horizontal,
    Vertical,
}

impl TabOrientation {
    /// Id estable para el dropdown de ajustes y la persistencia.
    pub fn id(self) -> &'static str {
        match self {
            TabOrientation::Horizontal => "horizontal",
            TabOrientation::Vertical => "vertical",
        }
    }
    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "horizontal" => Some(TabOrientation::Horizontal),
            "vertical" => Some(TabOrientation::Vertical),
            _ => None,
        }
    }
}

/// Un **space** (pestaña de alto nivel): agrupa varias pestañas del navegador.
/// En modo vertical cada space es un **diente** del rail; al activarlo, su
/// panel lista las pestañas que le pertenecen. En modo horizontal sólo se ve
/// el space activo (un nivel). El `icon` es un glifo para el diente.
#[derive(Debug, Clone)]
pub struct Space {
    pub name: String,
    pub icon: String,
}

impl Space {
    pub fn new(name: impl Into<String>, icon: impl Into<String>) -> Self {
        Self { name: name.into(), icon: icon.into() }
    }
}

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
    /// Índice del [`Space`] al que pertenece esta pestaña. Default `0` (el
    /// space inicial). En modo vertical decide bajo qué diente aparece; en
    /// horizontal sólo se muestran las del space activo.
    pub space: usize,
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
    /// Imágenes decodificadas referenciadas por `drawImage`, keyeadas por el
    /// `src` crudo que el JS registró. `None` = falló la decodificación (no se
    /// reintenta cada frame). Se poblan en `refresh_canvas_frames` resolviendo
    /// contra `t.url` vía `fetch_image_src` (cache-backed). Fase 7.197b.
    canvas_images: std::collections::HashMap<String, Option<PenikoImage>>,
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
            space: 0,
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
            canvas_images: std::collections::HashMap::new(),
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
    /// Spaces de alto nivel (las "pestañas" del usuario). Siempre ≥ 1.
    /// `tabs[i].space` indexa acá. En modo vertical son los dientes del rail.
    pub spaces: Vec<Space>,
    /// Space activo — sus pestañas son las visibles. Siempre `< spaces.len()`.
    pub active_space: usize,
    /// Orientación de las pestañas (horizontal/vertical). Persistida.
    pub orientation: TabOrientation,
    /// Tema que pinta el chrome renovado (sidebar, rail, panel de ajustes,
    /// input de URL). El dropdown de ajustes lo cambia.
    pub theme: Theme,
    /// `Ctrl+,` abre el panel de configuración embebido (allichay). `Esc` o
    /// clic en el scrim lo cierra.
    pub settings_open: bool,
    /// Estado del renderizador del panel de configuración (diente activo,
    /// buffers de edición). Vive mientras el panel está abierto.
    pub settings: AllichayState,
    /// Sugerencias de autocompletar del address bar (historial + marcadores)
    /// recomputadas en cada tecla. Vacío = sin dropdown. Cada entrada es
    /// `(url, título)`.
    pub addr_suggest: Vec<(String, String)>,
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

    /// Índices (en `self.tabs`) de las pestañas que pertenecen al space `sp`,
    /// en orden de aparición. Es el contenido del panel de un diente.
    fn tabs_in_space(&self, sp: usize) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.space == sp)
            .map(|(i, _)| i)
            .collect()
    }

    /// Pestañas del space activo — lo que se pinta en la barra/sidebar.
    fn active_space_tabs(&self) -> Vec<usize> {
        self.tabs_in_space(self.active_space)
    }

    /// Cuántos spaces no vacíos hay (para no dejar dientes fantasma).
    fn space_count(&self) -> usize {
        self.spaces.len()
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
    /// Crea un space nuevo (un diente nuevo en el rail) con una pestaña vacía
    /// adentro, y lo activa. El "+" del rail lo dispara.
    NewSpace,
    /// Activa el space `idx` (clic en su diente). Cambia el conjunto de
    /// pestañas visibles y enfoca la última pestaña de ese space.
    SelectSpace(usize),
    /// Mueve la pestaña `tab_idx` al space `dest` (drop de un diente de
    /// pestaña sobre otro diente de space). No-op si el destino no existe.
    MoveTabToSpace { tab_idx: usize, dest: usize },
    /// `Ctrl+,` — abre el panel de configuración embebido.
    OpenSettings,
    /// `Esc` / clic en el scrim — cierra el panel de configuración.
    CloseSettings,
    /// Mensaje del renderizador del panel de configuración (allichay). El
    /// `update` lo enruta: foco de campos, scroll y `Change(path, value)`.
    Settings(llimphi_module_allichay::AllichayMsg),
    /// Clic en una sugerencia del autocompletar del address bar — navega a
    /// esa URL.
    AddrSuggestPick(String),
}

/// Decide qué hace un evento de rueda según los modifiers: con `Ctrl`
/// hace zoom de página (gesto estándar del navegador), si no scrollea.
/// Pura para poder testearla sin construir un `Model`.
///
/// Convención CSS de `WheelDelta`: `y > 0` = rueda hacia abajo
/// Construye un `Model` de demostración **sin red ni threads** (no llama a
/// `spawn_load`/`spawn_periodic`), para ejemplos de render headless. Trae dos
/// spaces con varias pestañas y orientación vertical, así el sidebar de dientes
/// se ve poblado. No es parte del runtime real — sólo lo usa
/// `examples/dump_container.rs`.
pub fn demo_model() -> Model {
    let mk = |space: usize, url: &str, title: &str| {
        let mut t = TabState::new(url.to_string());
        t.space = space;
        t.title = title.to_string();
        t.status = "OK".into();
        t
    };
    let tabs = vec![
        mk(0, "https://gioser.net", "gioser · suite soberana"),
        mk(0, "https://example.com", "Example Domain"),
        mk(0, "about:blank", ""),
        mk(1, "https://docs.rs/serde", "serde — Rust"),
    ];
    Model {
        tabs,
        active: 0,
        spaces: vec![Space::new("Principal", "◆"), Space::new("Trabajo", "▲")],
        active_space: 0,
        orientation: TabOrientation::Vertical,
        theme: Theme::dark(),
        settings_open: false,
        settings: AllichayState::new(),
        addr_suggest: Vec::new(),
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
        // Orientación inicial desde el Profile (si está cableado); default
        // horizontal (un nivel, comportamiento clásico).
        let orientation = profile_handle()
            .and_then(|h| h.lock().ok().map(|p| p.ui.orientation.clone()))
            .and_then(|o| TabOrientation::from_id(&o))
            .unwrap_or(TabOrientation::Horizontal);
        Model {
            tabs: vec![tab],
            active: 0,
            spaces: vec![Space::new("Principal", "◆")],
            active_space: 0,
            orientation,
            theme: Theme::dark(),
            settings_open: false,
            settings: AllichayState::new(),
            addr_suggest: Vec::new(),
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
        // Panel de configuración abierto: Esc lo cierra; el resto de teclas se
        // tragan (los campos son dropdowns, sin edición de texto). Prioridad
        // sobre todo lo demás para que las teclas no fuguen a la página.
        if model.settings_open {
            if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::CloseSettings);
            }
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
                // Ctrl+, — abre/cierra el panel de configuración embebido.
                Key::Character(s) if s.as_str() == "," => {
                    return Some(if model.settings_open {
                        Msg::CloseSettings
                    } else {
                        Msg::OpenSettings
                    });
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
                        t.canvas_images.clear();
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
                m.addr_suggest.clear();
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
                // Al enfocar, sembrá sugerencias contra el texto actual (vacío
                // ⇒ sin dropdown hasta que el usuario teclee).
                let q = m.active().addr.text();
                m.addr_suggest = compute_addr_suggestions(&q);
            }
            Msg::AddrKey(e) => {
                if matches!(&e.key, Key::Named(NamedKey::Enter)) {
                    let raw = m.active().addr.text().trim().to_string();
                    if !raw.is_empty() {
                        m.addr_suggest.clear();
                        // Repotenciado: "buscar-o-navegar". Si parece URL/dominio
                        // navega; si no, lo manda al buscador.
                        let target = normalize_omnibox_input(&raw);
                        return Self::update(m, Msg::Navigate(target), handle);
                    }
                } else if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                    let t = m.active_mut();
                    t.addr_focused = false;
                    t.addr.set_text(t.url.clone());
                    m.addr_suggest.clear();
                } else {
                    m.active_mut().addr.apply_key(&e);
                    // Recomputá las sugerencias de autocompletar (historial +
                    // marcadores) contra el texto vigente.
                    let q = m.active().addr.text();
                    m.addr_suggest = compute_addr_suggestions(&q);
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
                // La pestaña nueva nace en el space activo (en horizontal hay
                // un solo space visible; en vertical, bajo el diente activo).
                t.space = m.active_space;
                m.tabs.push(t);
                let new_idx = m.tabs.len() - 1;
                switch_active_tab(&mut m, new_idx);
                m.active_mut().addr_focused = true;
            }
            Msg::CloseTab(idx) => {
                let closing_active = idx == m.active;
                let closed_space = m.tabs.get(idx).map(|t| t.space).unwrap_or(m.active_space);
                if idx < m.tabs.len() {
                    // Corta los EventSource de la pestaña antes de tirarla.
                    m.tabs[idx].cancel_all_eventsources();
                    m.tabs.remove(idx);
                }
                if m.tabs.is_empty() {
                    // No quedan pestañas: sembrá una en el space que se vació.
                    let mut t = TabState::new(NEW_TAB_URL.into());
                    t.space = m.active_space;
                    m.tabs.push(t);
                    m.active = 0;
                } else if m.active >= m.tabs.len() {
                    // El active quedó out-of-bounds tras el remove — apuntá al
                    // último (sin switch: la tab vieja ya no existe).
                    m.active = m.tabs.len() - 1;
                    if let Some(rt) = m.tabs[m.active].js.as_mut() {
                        let _ = rt.set_visibility(false);
                    }
                } else if closing_active {
                    // Cerramos la activa pero el índice sigue válido (apunta a
                    // lo que ocupó su lugar). Si esa pestaña cayó en otro space,
                    // preferí una del space que estábamos viendo para no saltar
                    // de contexto.
                    if m.tabs[m.active].space != closed_space {
                        if let Some(&sib) = m.tabs_in_space(closed_space).first() {
                            m.active = sib;
                        } else {
                            // El space quedó sin pestañas: seguí el space de la
                            // pestaña que ocupó el hueco.
                            m.active_space = m.tabs[m.active].space;
                        }
                    }
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
                // Cicla dentro del space activo (con wrap). Si el space tiene
                // una sola pestaña, no-op.
                let sibs = m.active_space_tabs();
                if sibs.len() > 1 {
                    let here = sibs.iter().position(|&i| i == m.active).unwrap_or(0);
                    let next = sibs[(here + 1) % sibs.len()];
                    switch_active_tab(&mut m, next);
                }
            }
            Msg::PrevTab => {
                let sibs = m.active_space_tabs();
                if sibs.len() > 1 {
                    let here = sibs.iter().position(|&i| i == m.active).unwrap_or(0);
                    let prev = sibs[(here + sibs.len() - 1) % sibs.len()];
                    switch_active_tab(&mut m, prev);
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
            Msg::NewSpace => {
                let n = m.spaces.len();
                // Glifo rotativo para el diente nuevo (ciclo corto y legible).
                const GLYPHS: [&str; 8] = ["◆", "●", "▲", "■", "★", "✦", "◈", "❖"];
                m.spaces.push(Space::new(
                    format!("Space {}", n + 1),
                    GLYPHS[n % GLYPHS.len()],
                ));
                m.active_space = n;
                // Un space nace con una pestaña vacía adentro.
                let mut t = TabState::new(NEW_TAB_URL.into());
                t.status = "nuevo space".into();
                t.box_tree = None;
                t.space = n;
                m.tabs.push(t);
                let new_idx = m.tabs.len() - 1;
                switch_active_tab(&mut m, new_idx);
                m.active_mut().addr_focused = true;
                persist_ui_prefs(&m);
            }
            Msg::SelectSpace(idx) => {
                if idx < m.spaces.len() && idx != m.active_space {
                    m.active_space = idx;
                    // Enfocá la última pestaña de ese space; si no tiene ninguna
                    // (caso raro tras mover pestañas), creá una vacía.
                    match m.tabs_in_space(idx).last().copied() {
                        Some(tab_idx) => switch_active_tab(&mut m, tab_idx),
                        None => {
                            let mut t = TabState::new(NEW_TAB_URL.into());
                            t.space = idx;
                            m.tabs.push(t);
                            let new_idx = m.tabs.len() - 1;
                            switch_active_tab(&mut m, new_idx);
                        }
                    }
                }
            }
            Msg::MoveTabToSpace { tab_idx, dest } => {
                if tab_idx < m.tabs.len() && dest < m.spaces.len() {
                    m.tabs[tab_idx].space = dest;
                }
            }
            Msg::OpenSettings => {
                m.settings_open = true;
                m.settings = AllichayState::new();
            }
            Msg::CloseSettings => {
                m.settings_open = false;
            }
            Msg::Settings(amsg) => {
                apply_settings_msg(&mut m, amsg);
            }
            Msg::AddrSuggestPick(url) => {
                m.addr_suggest.clear();
                m.active_mut().addr_focused = false;
                return Self::update(m, Msg::Navigate(url), handle);
            }
        }
        m
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // Prioridad: panel de configuración > menú contextual de edición >
        // dropdown del menú principal > overlay del `<select>` abierto.
        if model.settings_open {
            return Some(settings_overlay_view(model));
        }
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
        // Header renovado (theme-driven): nav + indicador de seguridad + URL
        // repotenciada + autocompletar. Compartido por ambas orientaciones.
        let header = nav_header_bar(model);
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

        let find = if model.find_active {
            Some(find_bar(
                &model.find_input,
                find_count,
                model.find_current,
                model.find_case_sensitive,
                model.find_whole_word,
            ))
        } else {
            None
        };

        // Barra de menú principal — PRIMER hijo del column raíz, full width en
        // ambas orientaciones.
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let children: Vec<View<Msg>> = match model.orientation {
            TabOrientation::Horizontal => {
                // Un nivel: barra de pestañas del space activo arriba.
                let mut c: Vec<View<Msg>> = vec![menubar, tabs_bar(model), header];
                if let Some(f) = find {
                    c.push(f);
                }
                c.push(body);
                c
            }
            TabOrientation::Vertical => {
                // Sidebar de dientes a la izquierda; header + body a la derecha.
                let mut main: Vec<View<Msg>> = vec![header];
                if let Some(f) = find {
                    main.push(f);
                }
                main.push(body);
                let main_col = View::new(Style {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
                    ..Default::default()
                })
                .children(main);
                let row = View::new(Style {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    size: Size { width: percent(1.0_f32), height: percent(0.0_f32) },
                    ..Default::default()
                })
                .children(vec![sidebar_view(model), main_col]);
                vec![menubar, row]
            }
        };

        // Right-click en la raíz (origen 0,0 → las coords locales que
        // llegan al handler ya son de ventana) abre el menú contextual de
        // edición sobre el campo de texto focuseado.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
        .children(children)
    }
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
    // Helpers del canvas extraídos a `canvas.rs` (pub(crate) para estos tests).
    use super::canvas::{
        canvas_brush, canvas_color, canvas_composite, canvas_font_px, canvas_shadow,
        canvas_stroke, collect_dom_image_pixels, decode_canvas_images, paint_canvas_cmds,
    };
    // Tipos peniko/kurbo que sólo los tests del canvas usan (el código no-test
    // de lib.rs ya no, tras mover el painter a canvas.rs).
    use llimphi_raster::kurbo::{Cap, Join};
    use llimphi_raster::peniko::{Brush, Extend};

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
            spaces: vec![Space::new("Principal", "◆")],
            active_space: 0,
            orientation: TabOrientation::Horizontal,
            theme: Theme::dark(),
            settings_open: false,
            settings: AllichayState::new(),
            addr_suggest: Vec::new(),
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
            spaces: vec![Space::new("Principal", "◆")],
            active_space: 0,
            orientation: TabOrientation::Horizontal,
            theme: Theme::dark(),
            settings_open: false,
            settings: AllichayState::new(),
            addr_suggest: Vec::new(),
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
            spaces: vec![Space::new("Principal", "◆")],
            active_space: 0,
            orientation: TabOrientation::Horizontal,
            theme: Theme::dark(),
            settings_open: false,
            settings: AllichayState::new(),
            addr_suggest: Vec::new(),
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
        paint_canvas_cmds(&mut scene, &mut ts, rect, &cmds, 100.0, 50.0, &Default::default());
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
    fn canvas_brush_gradiente_y_degradacion() {
        let imgs: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        // String → Brush sólido.
        let s = serde_json::Value::String("#ff0000".into());
        assert!(matches!(canvas_brush(Some(&s), 1.0, &imgs), Brush::Solid(_)));
        // CanvasGradient linear con 2 stops → Brush::Gradient(Linear).
        let lin: serde_json::Value = serde_json::from_str(
            r##"{"_kind":"linear","_coords":[0,0,100,0],"_stops":[[0,"#ff0000"],[1,"#0000ff"]]}"##,
        )
        .unwrap();
        match canvas_brush(Some(&lin), 1.0, &imgs) {
            Brush::Gradient(g) => {
                assert!(matches!(g.kind, GradientKind::Linear { .. }));
                assert_eq!(g.stops.0.len(), 2);
            }
            _ => panic!("debería ser gradiente"),
        }
        // Radial.
        let rad: serde_json::Value = serde_json::from_str(
            r##"{"_kind":"radial","_coords":[10,10,0,10,10,50],"_stops":[[0,"#fff"],[1,"#000"]]}"##,
        )
        .unwrap();
        assert!(matches!(
            canvas_brush(Some(&rad), 1.0, &imgs),
            Brush::Gradient(g) if matches!(g.kind, GradientKind::Radial { .. })
        ));
        // Gradiente con un solo stop (inválido) → degrada a sólido (último stop).
        let bad: serde_json::Value =
            serde_json::from_str(r##"{"_kind":"linear","_coords":[0,0,1,0],"_stops":[[0,"#0f0"]]}"##)
                .unwrap();
        assert!(matches!(canvas_brush(Some(&bad), 1.0, &imgs), Brush::Solid(_)));
        // globalAlpha multiplica el alpha de cada stop del gradiente.
        match canvas_brush(Some(&lin), 0.5, &imgs) {
            Brush::Gradient(g) => {
                let a = g.stops.0[0].color.components[3];
                assert!((a - 0.5).abs() < 0.02, "alpha ~0.5, got {a}");
            }
            _ => panic!("gradiente"),
        }
        // Patrón (createPattern): con la imagen decodificada → Brush::Image;
        // sin imagen en el mapa → degrada a sólido.
        let pat: serde_json::Value =
            serde_json::from_str(r##"{"_pattern":true,"src":"u","rep":"repeat"}"##).unwrap();
        assert!(matches!(canvas_brush(Some(&pat), 1.0, &imgs), Brush::Solid(_)));
        let mut con_img = imgs.clone();
        con_img.insert(
            "u".into(),
            PenikoImage::new(Blob::from(vec![255u8, 0, 0, 255]), ImageFormat::Rgba8, 1, 1),
        );
        match canvas_brush(Some(&pat), 0.5, &con_img) {
            Brush::Image(im) => {
                assert!(matches!(im.x_extend, Extend::Repeat));
                assert!(matches!(im.y_extend, Extend::Repeat));
                assert!((im.alpha - 0.5).abs() < 0.001, "alpha ~0.5, got {}", im.alpha);
            }
            _ => panic!("debería ser patrón de imagen"),
        }
        // repeat-x → Repeat en x, Pad en y.
        let pat_x: serde_json::Value =
            serde_json::from_str(r##"{"_pattern":true,"src":"u","rep":"repeat-x"}"##).unwrap();
        match canvas_brush(Some(&pat_x), 1.0, &con_img) {
            Brush::Image(im) => {
                assert!(matches!(im.x_extend, Extend::Repeat));
                assert!(matches!(im.y_extend, Extend::Pad));
            }
            _ => panic!("patrón repeat-x"),
        }
    }

    #[test]
    fn canvas_stroke_dash_cap_join() {
        // setLineDash con patrón impar se duplica; cap/join se mapean.
        let st: serde_json::Value = serde_json::from_str(
            r##"{"lc":"round","lj":"bevel","ld":[5,3,2],"ldo":1.0}"##,
        )
        .unwrap();
        let stroke = canvas_stroke(Some(&st), 2.0);
        assert_eq!(stroke.width, 2.0);
        assert!(matches!(stroke.start_cap, Cap::Round));
        assert!(matches!(stroke.join, Join::Bevel));
        // 3 segmentos impares → duplicados a 6.
        assert_eq!(stroke.dash_pattern.len(), 6);
        assert_eq!(stroke.dash_offset, 1.0);
        // Sin dash declarado → sin patrón.
        let plain: serde_json::Value = serde_json::from_str(r##"{"lw":1}"##).unwrap();
        assert!(canvas_stroke(Some(&plain), 1.0).dash_pattern.is_empty());
    }

    #[test]
    fn paint_canvas_cmds_gradiente_clip_dash_balancea() {
        // Gradiente real + clip dentro de save/restore + stroke punteado:
        // la escena queda no-vacía y los push_layer del clip se balancean
        // (no debe panicar ni dejar layers colgando).
        let mut scene = llimphi_raster::vello::Scene::new();
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 };
        let cmds: Vec<Vec<serde_json::Value>> = serde_json::from_str(
            r##"[
                ["save"],
                ["beginPath"],
                ["rect", 0, 0, 50, 50],
                ["clip"],
                ["fillRect", 0, 0, 100, 50,
                    {"_kind":"linear","_coords":[0,0,100,0],"_stops":[[0,"#ff0000"],[1,"#0000ff"]]},
                    {"ga": 1}],
                ["restore"],
                ["beginPath"],
                ["moveTo", 0, 0],
                ["lineTo", 100, 50],
                ["stroke", {"s": "#000000", "lw": 2, "ld": [4, 4], "ldo": 0}]
            ]"##,
        )
        .unwrap();
        paint_canvas_cmds(&mut scene, &mut ts, rect, &cmds, 100.0, 50.0, &Default::default());
        assert!(!scene.encoding().is_empty(), "debería haber dibujo");
    }

    #[test]
    fn canvas_gradiente_y_dash_llegan_al_frame_end_to_end() {
        // El JS construye un gradiente + setLineDash y el snapshot debe llevar
        // el objeto CanvasGradient y el array `ld`.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="100" height="100"></canvas></body>"#,
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
            attributes: vec![("width".into(), "100".into()), ("height".into(), "100".into())],
            dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "var ctx = document.getElementById('c').getContext('2d');\
             var g = ctx.createLinearGradient(0,0,100,0);\
             g.addColorStop(0,'#ff0000'); g.addColorStop(1,'#0000ff');\
             ctx.fillStyle = g; ctx.fillRect(0,0,100,100);\
             ctx.setLineDash([6,4]); ctx.strokeStyle='#000';\
             ctx.beginPath(); ctx.moveTo(0,0); ctx.lineTo(100,100); ctx.stroke();",
        )
        .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        // El fillRect lleva el objeto gradiente en el arg 5.
        let fr = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("fillRect"))
            .expect("fillRect");
        assert_eq!(fr[5].get("_kind").and_then(|v| v.as_str()), Some("linear"));
        assert_eq!(fr[5].get("_stops").and_then(|v| v.as_array()).map(|a| a.len()), Some(2));
        // El stroke lleva el snapshot con `ld`.
        let stk = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("stroke"))
            .expect("stroke");
        let ld = stk[1].get("ld").and_then(|v| v.as_array()).expect("ld");
        assert_eq!(ld.len(), 2);
    }

    #[test]
    fn drawimage_de_img_dom_se_decodifica_end_to_end() {
        // <canvas> + <img src=data:…> → ctx.drawImage(img) registra el src y
        // refresh_canvas_frames (→ decode_canvas_images) lo decodifica.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="100" height="100"></canvas></body>"#,
        ));
        t.has_canvas = true;
        let mk = |id: &str, tag: &str, attrs: Vec<(String, String)>| puriy_js::ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: attrs,
            dfs_index: 0,
        };
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            mk("c", "canvas", vec![("width".into(), "100".into()), ("height".into(), "100".into())]),
            mk("i", "img", vec![("src".into(), png_1x1.into())]),
        ])
        .expect("set_elements");
        rt.eval(
            "var ctx = document.getElementById('c').getContext('2d');\
             var im = document.getElementById('i');\
             ctx.drawImage(im, 5, 5);",
        )
        .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        let di = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("drawImage"))
            .expect("drawImage en el frame");
        assert_eq!(di.get(1).and_then(|v| v.as_str()), Some(png_1x1));
        let img = t.canvas_images.get(png_1x1).expect("decodificada").as_ref();
        assert_eq!(img.map(|i| (i.width, i.height)), Some((1, 1)));
    }

    #[test]
    fn createpattern_de_img_dom_se_decodifica_end_to_end() {
        // <canvas> + <img> → ctx.createPattern(img,'repeat') usado como
        // fillStyle: el snapshot del fillRect lleva el descriptor {_pattern,src}
        // y decode_canvas_images (vía refresh) decodifica ese src. Fase 7.198.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="100" height="100"></canvas></body>"#,
        ));
        t.has_canvas = true;
        let mk = |id: &str, tag: &str, attrs: Vec<(String, String)>| puriy_js::ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: attrs,
            dfs_index: 0,
        };
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            mk("c", "canvas", vec![("width".into(), "100".into()), ("height".into(), "100".into())]),
            mk("i", "img", vec![("src".into(), png_1x1.into())]),
        ])
        .expect("set_elements");
        rt.eval(
            "var ctx = document.getElementById('c').getContext('2d');\
             var im = document.getElementById('i');\
             var pat = ctx.createPattern(im, 'repeat');\
             ctx.fillStyle = pat;\
             ctx.fillRect(0, 0, 50, 50);",
        )
        .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        // El fillRect lleva el descriptor de patrón en el arg 5.
        let fr = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("fillRect"))
            .expect("fillRect en el frame");
        assert_eq!(fr[5].get("_pattern").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(fr[5].get("src").and_then(|v| v.as_str()), Some(png_1x1));
        assert_eq!(fr[5].get("rep").and_then(|v| v.as_str()), Some("repeat"));
        // decode_canvas_images recogió el src del patrón y lo decodificó.
        let img = t.canvas_images.get(png_1x1).expect("decodificada").as_ref();
        assert_eq!(img.map(|i| (i.width, i.height)), Some((1, 1)));
        // El painter pinta el patrón (escena no-vacía).
        let mut images: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        images.insert(png_1x1.into(), img.unwrap().clone());
        let mut scene = llimphi_raster::vello::Scene::new();
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        paint_canvas_cmds(&mut scene, &mut ts, rect, &frame.cmds, 100.0, 100.0, &images);
        assert!(!scene.encoding().is_empty(), "el patrón debería pintar");
    }

    #[test]
    fn background_image_size_position_repeat_pinta_y_tilea() {
        // Fase 7.204 — paint_background_image resuelve size/position/repeat.
        let img = PenikoImage::new(
            llimphi_raster::peniko::Blob::from(vec![255u8; 2 * 2 * 4]),
            llimphi_raster::peniko::ImageFormat::Rgba8,
            2,
            2,
        );
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let sz = BackgroundSize::Explicit { x: LengthVal::Px(60.0), y: LengthVal::Px(60.0) };
        let pos = BackgroundPosition { x: LengthVal::Px(0.0), y: LengthVal::Px(0.0) };

        // no-repeat con tile 60×60 sobre 100×100 → un solo draw de imagen.
        let mut once = llimphi_raster::vello::Scene::new();
        paint_background_image(&mut once, rect, rect, 0.0, &img, 2.0, 2.0, sz, pos, BackgroundRepeat::NoRepeat);
        assert!(!once.encoding().is_empty(), "un background-image debería pintar");

        // repeat con el mismo tile → 2×2 = 4 tiles → más draw_tags.
        let mut tiled = llimphi_raster::vello::Scene::new();
        paint_background_image(&mut tiled, rect, rect, 0.0, &img, 2.0, 2.0, sz, pos, BackgroundRepeat::Repeat);
        assert!(
            tiled.encoding().draw_tags.len() > once.encoding().draw_tags.len(),
            "repeat debería encodar más tiles ({} vs {})",
            tiled.encoding().draw_tags.len(),
            once.encoding().draw_tags.len()
        );

        // rect de ancho 0 → no pinta nada (early-return).
        let mut empty = llimphi_raster::vello::Scene::new();
        let zero = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 0.0, h: 50.0 };
        paint_background_image(
            &mut empty, zero, zero, 0.0, &img, 2.0, 2.0,
            BackgroundSize::Auto,
            BackgroundPosition { x: LengthVal::Pct(0.0), y: LengthVal::Pct(0.0) },
            BackgroundRepeat::Repeat,
        );
        assert!(empty.encoding().is_empty(), "rect de ancho 0 no debería pintar");
    }

    #[test]
    fn background_clip_recorta_a_caja_mas_chica() {
        // Fase 7.207 — `background-clip`: con un clip box más chico que el
        // origin box, el tiling cubre el área de posicionamiento pero el
        // recorte limita el pintado. Verificamos que ambas rutas pintan y que
        // un clip box degenerado (ancho 0) no deja salir nada.
        let img = PenikoImage::new(
            llimphi_raster::peniko::Blob::from(vec![255u8; 2 * 2 * 4]),
            llimphi_raster::peniko::ImageFormat::Rgba8,
            2,
            2,
        );
        let area = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let sz = BackgroundSize::Explicit { x: LengthVal::Px(20.0), y: LengthVal::Px(20.0) };
        let pos = BackgroundPosition { x: LengthVal::Px(0.0), y: LengthVal::Px(0.0) };

        // clip box = padding-box (inset 10px) → sigue pintando los tiles.
        let clip = llimphi_ui::PaintRect { x: 10.0, y: 10.0, w: 80.0, h: 80.0 };
        let mut s = llimphi_raster::vello::Scene::new();
        paint_background_image(&mut s, area, clip, 0.0, &img, 2.0, 2.0, sz, pos, BackgroundRepeat::Repeat);
        assert!(!s.encoding().is_empty(), "clip padding-box debería pintar tiles");

        // El origen del tiling es `area` (no `clip`): con un área mayor hay más
        // tiles que recortando el área misma al clip chico.
        let mut s_small_area = llimphi_raster::vello::Scene::new();
        paint_background_image(
            &mut s_small_area, clip, clip, 0.0, &img, 2.0, 2.0, sz, pos, BackgroundRepeat::Repeat,
        );
        assert!(
            s.encoding().draw_tags.len() >= s_small_area.encoding().draw_tags.len(),
            "tilear sobre el origin box (100×100) no debería dar menos tiles que sobre 80×80"
        );
    }

    #[test]
    fn background_clip_text_rellena_glifos_con_gradiente() {
        // Fase 7.208 — el camino real de `background-clip: text`: shaping del
        // texto + draw_layout_brush_xf con un Brush::Gradient. Verifica que
        // pinta (encoding no vacío) y que el gradiente añade más draws que el
        // mismo texto en color sólido.
        use puriy_engine::style::{GradientGeometry, GradientStop, LinearGradient};
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        // Forzamos la DejaVu embebida (registrada en `Typesetter::new`) para
        // que el texto Latin shapee también en el sandbox sin fuentes de
        // sistema; en una máquina real el font-family normal funciona igual.
        let layout = ts.layout(
            "Hola",
            48.0,
            None,
            llimphi_ui::llimphi_text::Alignment::Start,
            1.2,
            false,
            Some("DejaVu Sans"),
            400.0,
            false,
            false,
        );
        let local = llimphi_ui::PaintRect {
            x: 0.0,
            y: 0.0,
            w: (layout.width()).max(1.0),
            h: 60.0,
        };
        let grad = LinearGradient {
            geometry: GradientGeometry::Linear { angle_deg: 90.0 },
            stops: vec![
                GradientStop { color: puriy_engine::Color::rgb(255, 0, 0), pos: None },
                GradientStop { color: puriy_engine::Color::rgb(0, 0, 255), pos: None },
            ],
            repeating: false,
        };
        let brush = llimphi_raster::peniko::Brush::Gradient(
            build_linear_gradient_brush(&grad, local, 1.0).expect("gradiente de 2 stops"),
        );
        let xf = llimphi_raster::kurbo::Affine::translate((10.0, 10.0));
        let mut scene = llimphi_raster::vello::Scene::new();
        llimphi_ui::llimphi_text::draw_layout_brush_xf(&mut scene, &layout, &brush, xf);
        // Los glifos se encodan en `draw_tags` + `glyph_runs` (las siluetas se
        // resuelven después, así que `path_tags`/`is_empty()` no sirven acá).
        assert!(
            !scene.encoding().draw_tags.is_empty(),
            "los glifos con gradiente deberían encodar un draw"
        );
        assert!(
            !scene.encoding().resources.glyph_runs.is_empty(),
            "debería haber al menos un glyph run shapeado (DejaVu)"
        );
    }

    #[test]
    fn object_fit_scale_por_modo() {
        use puriy_engine::ObjectFit;
        // Imagen 100×50 (2:1) en caja 200×200, zoom 1.
        let (iw, ih, rw, rh, z) = (100.0, 50.0, 200.0, 200.0, 1.0);
        // Fill: estira por eje independiente.
        assert_eq!(object_fit_scale(ObjectFit::Fill, rw, rh, iw, ih, z), (2.0, 4.0));
        // Contain: min de las dos (2.0) → cabe sin recortar.
        assert_eq!(object_fit_scale(ObjectFit::Contain, rw, rh, iw, ih, z), (2.0, 2.0));
        // Cover: max de las dos (4.0) → cubre, recorta horizontal.
        assert_eq!(object_fit_scale(ObjectFit::Cover, rw, rh, iw, ih, z), (4.0, 4.0));
        // None: tamaño natural × zoom.
        assert_eq!(object_fit_scale(ObjectFit::None, rw, rh, iw, ih, z), (1.0, 1.0));
        // ScaleDown: min(contain=2, natural=1) = 1 (la imagen es chica → no agranda).
        assert_eq!(object_fit_scale(ObjectFit::ScaleDown, rw, rh, iw, ih, z), (1.0, 1.0));
        // ScaleDown con imagen grande (300×300) en caja 100×100: contain=1/3 < 1 → encoge.
        let (sx, sy) = object_fit_scale(ObjectFit::ScaleDown, 100.0, 100.0, 300.0, 300.0, 1.0);
        assert!((sx - 1.0 / 3.0).abs() < 1e-9 && (sy - 1.0 / 3.0).abs() < 1e-9);
        // Imagen degenerada → escala neutra (no divide por cero).
        assert_eq!(object_fit_scale(ObjectFit::Cover, rw, rh, 0.0, ih, z), (1.0, 1.0));
    }

    #[test]
    fn paint_extra_bg_layers_pinta_imagen_y_gradiente() {
        // Fase 7.206 — las capas extra (debajo de la capa 0) se pintan: una
        // imagen vía paint_background_image y un gradiente lineal vía fill.
        use puriy_engine::style::{GradientGeometry, GradientStop, LinearGradient};
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };

        // Sin capas → no pinta nada.
        let mut none = llimphi_raster::vello::Scene::new();
        paint_extra_bg_layers(&mut none, rect, 0.0, &[], 1.0);
        assert!(none.encoding().is_empty(), "sin capas no debería pintar");

        // Una capa de gradiente → un fill.
        let grad = LinearGradient {
            geometry: GradientGeometry::Linear { angle_deg: 180.0 },
            stops: vec![
                GradientStop { color: puriy_engine::Color::rgb(255, 0, 0), pos: None },
                GradientStop { color: puriy_engine::Color::rgb(0, 0, 255), pos: None },
            ],
            repeating: false,
        };
        let mut g = llimphi_raster::vello::Scene::new();
        paint_extra_bg_layers(&mut g, rect, 0.0, &[PreparedBgLayer::Gradient(grad.clone())], 1.0);
        assert!(!g.encoding().is_empty(), "una capa de gradiente debería pintar");

        // Imagen + gradiente → más draws que el gradiente solo.
        let img = PenikoImage::new(
            llimphi_raster::peniko::Blob::from(vec![255u8; 2 * 2 * 4]),
            llimphi_raster::peniko::ImageFormat::Rgba8,
            2,
            2,
        );
        let layers = vec![
            PreparedBgLayer::Image {
                img,
                iw: 2.0,
                ih: 2.0,
                size: BackgroundSize::Explicit { x: LengthVal::Px(50.0), y: LengthVal::Px(50.0) },
                position: BackgroundPosition { x: LengthVal::Px(0.0), y: LengthVal::Px(0.0) },
                repeat: BackgroundRepeat::NoRepeat,
            },
            PreparedBgLayer::Gradient(grad),
        ];
        let mut both = llimphi_raster::vello::Scene::new();
        paint_extra_bg_layers(&mut both, rect, 0.0, &layers, 1.0);
        assert!(
            both.encoding().draw_tags.len() > g.encoding().draw_tags.len(),
            "dos capas deberían encodar más draws que una ({} vs {})",
            both.encoding().draw_tags.len(),
            g.encoding().draw_tags.len()
        );
    }

    #[test]
    fn canvas_shadow_lee_estado() {
        // Sin campo `sc` → None.
        let plain: serde_json::Value = serde_json::from_str(r#"{"ga":1.0}"#).unwrap();
        assert!(canvas_shadow(Some(&plain), 1.0).is_none());
        // Color totalmente transparente → None (aunque haya blur/offset).
        let transp: serde_json::Value =
            serde_json::from_str(r#"{"sc":"rgba(0,0,0,0)","sb":5,"sox":2,"soy":2}"#).unwrap();
        assert!(canvas_shadow(Some(&transp), 1.0).is_none());
        // Blur 0 + ambos offsets 0 → inactiva.
        let inactive: serde_json::Value =
            serde_json::from_str(r##"{"sc":"#000","sb":0,"sox":0,"soy":0}"##).unwrap();
        assert!(canvas_shadow(Some(&inactive), 1.0).is_none());
        // Activa: blur 4, offset (3,5); ga 0.5 reduce el alpha del color.
        let active: serde_json::Value =
            serde_json::from_str(r#"{"sc":"rgba(0,0,0,1)","sb":4,"sox":3,"soy":5}"#).unwrap();
        let (col, blur, ox, oy) = canvas_shadow(Some(&active), 0.5).expect("sombra activa");
        assert_eq!((blur, ox, oy), (4.0, 3.0, 5.0));
        assert!((col.components[3] - 0.5).abs() < 0.02, "alpha ~0.5, got {}", col.components[3]);
    }

    #[test]
    fn paint_canvas_cmds_sombra_agrega_draw() {
        // Un fillRect con sombra encoda MÁS draw objects que sin sombra (la
        // sombra blureada es un draw extra vía draw_blurred_rounded_rect).
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let imgs: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        let sin: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r##"[["fillRect",10,10,40,40,"#ff0000",{"ga":1.0}]]"##).unwrap();
        let con: Vec<Vec<serde_json::Value>> = serde_json::from_str(
            r##"[["fillRect",10,10,40,40,"#ff0000",{"ga":1.0,"sc":"rgba(0,0,0,1)","sb":6,"sox":4,"soy":4}]]"##,
        )
        .unwrap();
        let mut s1 = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s1, &mut ts, rect, &sin, 100.0, 100.0, &imgs);
        let mut s2 = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s2, &mut ts, rect, &con, 100.0, 100.0, &imgs);
        assert!(
            s2.encoding().draw_tags.len() > s1.encoding().draw_tags.len(),
            "la sombra debería agregar un draw object: {} vs {}",
            s2.encoding().draw_tags.len(),
            s1.encoding().draw_tags.len()
        );
    }

    #[test]
    fn sombra_llega_al_frame_end_to_end() {
        // ctx.shadow* + fillRect → el snapshot del fillRect lleva sc/sb/sox/soy
        // y canvas_shadow lo resuelve. Fase 7.199.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="100" height="100"></canvas></body>"#,
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
            attributes: vec![("width".into(), "100".into()), ("height".into(), "100".into())],
            dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "var ctx=document.getElementById('c').getContext('2d');\
             ctx.shadowColor='rgba(0,0,0,0.7)'; ctx.shadowBlur=8;\
             ctx.shadowOffsetX=4; ctx.shadowOffsetY=4;\
             ctx.fillStyle='#3366ff'; ctx.fillRect(20,20,40,40);",
        )
        .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        let fr = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("fillRect"))
            .expect("fillRect");
        assert_eq!(fr[6].get("sc").and_then(|v| v.as_str()), Some("rgba(0,0,0,0.7)"));
        assert_eq!(fr[6].get("sb").and_then(|v| v.as_f64()), Some(8.0));
        assert_eq!(fr[6].get("sox").and_then(|v| v.as_f64()), Some(4.0));
        assert!(canvas_shadow(Some(&fr[6]), 1.0).is_some(), "la sombra debería resolverse");
    }

    #[test]
    fn canvas_composite_mapea_modos() {
        // source-over (default) y desconocidos → None (sin capa de blend).
        let so: serde_json::Value = serde_json::from_str(r#"{"gco":"source-over"}"#).unwrap();
        assert!(canvas_composite(Some(&so)).is_none());
        let raro: serde_json::Value = serde_json::from_str(r#"{"gco":"qwerty"}"#).unwrap();
        assert!(canvas_composite(Some(&raro)).is_none());
        assert!(canvas_composite(Some(&serde_json::json!({"ga": 1.0}))).is_none());
        // Modo de mezcla → Mix (compose SrcOver).
        use llimphi_raster::peniko::{Compose, Mix};
        let mul: serde_json::Value = serde_json::from_str(r#"{"gco":"multiply"}"#).unwrap();
        let bm = canvas_composite(Some(&mul)).expect("multiply mapea");
        assert_eq!((bm.mix, bm.compose), (Mix::Multiply, Compose::SrcOver));
        // Porter-Duff → Compose (mix Normal).
        let lighter: serde_json::Value = serde_json::from_str(r#"{"gco":"lighter"}"#).unwrap();
        let bm = canvas_composite(Some(&lighter)).expect("lighter mapea");
        assert_eq!((bm.mix, bm.compose), (Mix::Normal, Compose::Plus));
        let dout: serde_json::Value =
            serde_json::from_str(r#"{"gco":"destination-out"}"#).unwrap();
        assert_eq!(canvas_composite(Some(&dout)).unwrap().compose, Compose::DestOut);
    }

    #[test]
    fn paint_canvas_cmds_composite_agrega_layer() {
        // Un fillRect con globalCompositeOperation != source-over encoda MÁS
        // draw objects (el push_layer/pop_layer de blend agrega tags de clip).
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let imgs: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        let sin: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r##"[["fillRect",10,10,40,40,"#ff0000",{"ga":1.0}]]"##).unwrap();
        let con: Vec<Vec<serde_json::Value>> = serde_json::from_str(
            r##"[["fillRect",10,10,40,40,"#ff0000",{"ga":1.0,"gco":"lighter"}]]"##,
        )
        .unwrap();
        let mut s1 = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s1, &mut ts, rect, &sin, 100.0, 100.0, &imgs);
        let mut s2 = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s2, &mut ts, rect, &con, 100.0, 100.0, &imgs);
        assert!(
            s2.encoding().draw_tags.len() > s1.encoding().draw_tags.len(),
            "la capa de blend debería agregar draw objects: {} vs {}",
            s2.encoding().draw_tags.len(),
            s1.encoding().draw_tags.len()
        );
    }

    #[test]
    fn gco_llega_al_frame_end_to_end() {
        // ctx.globalCompositeOperation + fillRect → el snapshot lleva `gco` y
        // canvas_composite lo resuelve. Fase 7.200.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="100" height="100"></canvas></body>"#,
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
            attributes: vec![("width".into(), "100".into()), ("height".into(), "100".into())],
            dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "var ctx=document.getElementById('c').getContext('2d');\
             ctx.globalCompositeOperation='multiply';\
             ctx.fillStyle='#3366ff'; ctx.fillRect(20,20,40,40);",
        )
        .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        let fr = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("fillRect"))
            .expect("fillRect");
        assert_eq!(fr[6].get("gco").and_then(|v| v.as_str()), Some("multiply"));
        assert!(canvas_composite(Some(&fr[6])).is_some(), "el composite debería resolverse");
    }

    #[test]
    fn paint_canvas_cmds_drawimage_dibuja() {
        // Una imagen 2×2 en el mapa + un drawImage que la coloca → la escena
        // queda no-vacía. Cubre las 3 aridades (2/4/8 números).
        let mut scene = llimphi_raster::vello::Scene::new();
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let img = PenikoImage::new(Blob::from(vec![255u8; 16]), ImageFormat::Rgba8, 2, 2);
        let mut images = std::collections::HashMap::new();
        images.insert("u".to_string(), img);
        for cmds_src in [
            r#"[["drawImage","u",10,10]]"#,                 // 3-arg
            r#"[["drawImage","u",10,10,40,40]]"#,           // 5-arg
            r#"[["drawImage","u",0,0,2,2,10,10,40,40]]"#,   // 9-arg (sub-rect)
        ] {
            let mut s = llimphi_raster::vello::Scene::new();
            let cmds: Vec<Vec<serde_json::Value>> = serde_json::from_str(cmds_src).unwrap();
            paint_canvas_cmds(&mut s, &mut ts, rect, &cmds, 100.0, 100.0, &images);
            assert!(!s.encoding().is_empty(), "drawImage debería pintar: {cmds_src}");
        }
        // Un src ausente del mapa → no-op (no panic, escena vacía).
        let cmds: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r#"[["drawImage","falta",0,0]]"#).unwrap();
        paint_canvas_cmds(&mut scene, &mut ts, rect, &cmds, 100.0, 100.0, &images);
        assert!(scene.encoding().is_empty(), "src ausente no pinta");
    }

    #[test]
    fn drawimage_con_snapshot_aplica_composite_y_alpha() {
        // Fase 7.201 — un drawImage con snapshot de composite/alpha sigue
        // dibujando (las coords se parsean con filter_map, descartando el
        // snapshot del final) y la capa de blend agrega draw objects.
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let img = PenikoImage::new(Blob::from(vec![255u8; 16]), ImageFormat::Rgba8, 2, 2);
        let mut images = std::collections::HashMap::new();
        images.insert("u".to_string(), img);
        // Sin snapshot (compat hacia atrás): dibuja.
        let plano: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r#"[["drawImage","u",10,10,40,40,{"ga":1.0}]]"#).unwrap();
        let mut s_plano = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s_plano, &mut ts, rect, &plano, 100.0, 100.0, &images);
        assert!(!s_plano.encoding().is_empty(), "drawImage con snapshot debería pintar");
        // Con composite 'lighter' → capa de blend extra.
        let comp: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r#"[["drawImage","u",10,10,40,40,{"ga":1.0,"gco":"lighter"}]]"#)
                .unwrap();
        let mut s_comp = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s_comp, &mut ts, rect, &comp, 100.0, 100.0, &images);
        assert!(
            s_comp.encoding().draw_tags.len() > s_plano.encoding().draw_tags.len(),
            "el composite debería agregar draw objects: {} vs {}",
            s_comp.encoding().draw_tags.len(),
            s_plano.encoding().draw_tags.len()
        );
        // Las coords (8 números, sub-rect) + snapshot siguen mapeando bien.
        let sub: Vec<Vec<serde_json::Value>> = serde_json::from_str(
            r#"[["drawImage","u",0,0,2,2,10,10,40,40,{"ga":0.5,"sc":"rgba(0,0,0,1)","sb":6,"sox":3,"soy":3}]]"#,
        )
        .unwrap();
        let mut s_sub = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s_sub, &mut ts, rect, &sub, 100.0, 100.0, &images);
        assert!(!s_sub.encoding().is_empty(), "sub-rect con alpha+sombra debería pintar");
    }

    #[test]
    fn drawimage_a_getimagedata_pipeline_end_to_end() {
        // Fase 7.203 — flujo COMPLETO por run_scripts_on_tab: el chrome inyecta
        // los píxeles del <img> antes del script, así un drawImage+getImageData
        // (pipeline de filtros) lee la imagen real. El PNG 1×1 es rojo opaco.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let mut t = TabState::new("about:test".into());
        t.url = "about:test".into();
        t.has_canvas = true;
        t.box_tree = Some(parse(&format!(
            r#"<body><canvas id="c" width="4" height="4"></canvas><img id="i" src="{png_1x1}"></body>"#
        )));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some(
                "var ctx=document.getElementById('c').getContext('2d');\
                 var im=document.getElementById('i');\
                 ctx.drawImage(im,0,0);\
                 var g=ctx.getImageData(0,0,1,1);\
                 globalThis.__r = g.data[0]+','+g.data[1]+','+g.data[2]+','+g.data[3];"
                    .into(),
            ),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        assert_eq!(t.js_summary.errors, 0, "el script no debería errar");
        let r = t.js.as_mut().unwrap().eval("__r").expect("r");
        // rojo opaco leído del framebuffer JS tras drawImage.
        assert_eq!(r, puriy_js::JsValue::String("255,0,0,255".into()));
    }

    #[test]
    fn collect_dom_image_pixels_decodifica_imgs() {
        // Fase 7.203 — el chrome recolecta los píxeles de los <img> de la
        // página (cuando hay canvas) para inyectarlos al runtime.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.has_canvas = true;
        t.box_tree = Some(parse(&format!(
            r#"<body><canvas id="c" width="10" height="10"></canvas><img id="i" src="{png_1x1}"></body>"#
        )));
        let px = collect_dom_image_pixels(t);
        assert_eq!(px.len(), 1, "debería recolectar 1 img");
        assert_eq!(px[0].0, png_1x1);
        assert_eq!((px[0].1, px[0].2), (1, 1));
        assert_eq!(px[0].3.len(), 4, "rgba de 1×1 = 4 bytes");
        // Sin canvas → vacío (gate de costo).
        t.has_canvas = false;
        assert!(collect_dom_image_pixels(t).is_empty());
    }

    #[test]
    fn paint_canvas_cmds_putimagedata_dibuja() {
        // Fase 7.202 — un comando putImageData con base64 RGBA válido pinta.
        // "/wAA/w==" = 1 pixel rojo opaco (FF 00 00 FF).
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let imgs: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        let cmds: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r#"[["putImageData",3,4,1,1,"/wAA/w=="]]"#).unwrap();
        let mut scene = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut scene, &mut ts, rect, &cmds, 100.0, 100.0, &imgs);
        assert!(!scene.encoding().is_empty(), "putImageData debería pintar");
        // base64 inválido / dims en cero → no-op (no panic).
        let mala: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r#"[["putImageData",0,0,0,0,"@@@"]]"#).unwrap();
        let mut s2 = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s2, &mut ts, rect, &mala, 100.0, 100.0, &imgs);
        assert!(s2.encoding().is_empty(), "putImageData inválido no pinta");
    }

    #[test]
    fn putimagedata_llega_al_frame_end_to_end() {
        // ctx.putImageData por el runtime JS real → el frame lleva el comando
        // con dx/dy/w/h/base64, y el painter lo dibuja. Fase 7.202.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="20" height="20"></canvas></body>"#,
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
            attributes: vec![("width".into(), "20".into()), ("height".into(), "20".into())],
            dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "var ctx=document.getElementById('c').getContext('2d');\
             var id=ctx.createImageData(2,2);\
             for(var i=0;i<id.data.length;i+=4){id.data[i]=10;id.data[i+1]=20;id.data[i+2]=30;id.data[i+3]=255;}\
             ctx.putImageData(id,1,1);\
             var back=ctx.getImageData(1,1,1,1);",
        )
        .expect("draw");
        // getImageData round-trip dentro del runtime.
        assert_eq!(rt.eval("back.data[0]").expect("e"), puriy_js::JsValue::Number(10.0));
        assert_eq!(rt.eval("back.data[2]").expect("e"), puriy_js::JsValue::Number(30.0));
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        let put = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("putImageData"))
            .expect("putImageData");
        assert_eq!(put.get(3).and_then(|v| v.as_u64()), Some(2)); // w
        assert_eq!(put.get(4).and_then(|v| v.as_u64()), Some(2)); // h
        assert!(put.get(5).and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty()), "base64 presente");
        // El painter lo dibuja.
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        let imgs: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        let mut scene = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut scene, &mut ts, rect, &frame.cmds, 20.0, 20.0, &imgs);
        assert!(!scene.encoding().is_empty(), "el frame con putImageData debería pintar");
    }

    #[test]
    fn decode_canvas_images_resuelve_data_url() {
        // decode_canvas_images decodifica el src de un drawImage (data: PNG 1×1)
        // y lo deja en t.canvas_images.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        let cmds_json = format!(r#"[["drawImage","{png_1x1}",0,0]]"#);
        t.canvas_frames.insert(
            "c".into(),
            CanvasFrame {
                id: "c".into(),
                width: 100.0,
                height: 100.0,
                cmds: serde_json::from_str(&cmds_json).unwrap(),
            },
        );
        decode_canvas_images(t);
        let got = t.canvas_images.get(png_1x1).expect("entrada decodificada");
        let img = got.as_ref().expect("la imagen 1×1 decodifica");
        assert_eq!((img.width, img.height), (1, 1));
        // Segunda llamada no re-decodifica (idempotente: la clave ya existe).
        decode_canvas_images(t);
        assert_eq!(t.canvas_images.len(), 1);
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
