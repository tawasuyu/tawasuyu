use super::*;

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
pub(crate) struct HoverTween {
    /// `true` mientras el cursor está sobre el nodo (avanza hacia 1.0).
    pub(crate) hovered: bool,
    /// Progreso lineal `∈ [0,1]` capturado en el último cambio de estado.
    pub(crate) progress_at_toggle: f32,
    /// Reloj del `Model` (ms) del último toggle.
    pub(crate) toggle_ms: u64,
    /// Duración de la transición de background, en ms (de la binding CSS).
    pub(crate) duration_ms: u32,
}

impl HoverTween {
    /// Progreso lineal `∈ [0,1]` al instante `now_ms`: avanza desde
    /// `progress_at_toggle` hacia 1.0 si `hovered`, o hacia 0.0 si no,
    /// a razón de `1/duration`. Duración nula = salto inmediato.
    pub(crate) fn sample_linear(&self, now_ms: u64) -> f32 {
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
    pub(crate) hover_tweens: std::collections::HashMap<u32, HoverTween>,
    /// `EventSource` vivos: id JS → flag de cancelación compartido con su
    /// worker de streaming. `close()` o una navegación lo prenden y el worker
    /// corta el stream y termina (Fase 7.182).
    pub(crate) es_cancel: std::collections::HashMap<u32, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Frames de `<canvas>` 2D del runtime JS, keyeados por `element_id`
    /// del canvas. Se refrescan (`refresh_canvas_frames`) tras correr
    /// scripts, en cada tick y tras dispatchear eventos — el render del box
    /// canvas (`render_canvas`) los interpreta y pinta con vello. Fase 7.196.
    pub(crate) canvas_frames: std::collections::HashMap<String, CanvasFrame>,
    /// Imágenes decodificadas referenciadas por `drawImage`, keyeadas por el
    /// `src` crudo que el JS registró. `None` = falló la decodificación (no se
    /// reintenta cada frame). Se poblan en `refresh_canvas_frames` resolviendo
    /// contra `t.url` vía `fetch_image_src` (cache-backed). Fase 7.197b.
    pub(crate) canvas_images: std::collections::HashMap<String, Option<PenikoImage>>,
    /// `true` si el box tree de esta pestaña tiene al menos un `<canvas>`.
    /// Gatea el refresh por tick (evita un `eval` por frame en páginas sin
    /// canvas, que son la mayoría).
    pub(crate) has_canvas: bool,
}

/// Resultado agregado de ejecutar todos los `<script>` de un load.
#[derive(Default, Debug, Clone)]
pub struct JsSummary {
    pub logs: usize,
    pub errors: usize,
}

impl TabState {
    pub(crate) fn new(url: String) -> Self {
        let mut addr = TextInputState::new();
        addr.set_text(url.clone());
        Self {
            id: fresh_tab_id(),
            space: 0,
            url: url.clone(),
            title: String::new(),
            status: rimay_localize::t("puriy-status-loading"),
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
    pub(crate) fn cancel_all_eventsources(&mut self) {
        for flag in self.es_cancel.values() {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.es_cancel.clear();
    }

    pub(crate) fn can_back(&self) -> bool {
        self.cursor > 0
    }
    pub(crate) fn can_fwd(&self) -> bool {
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
pub(crate) const JS_POLL_PERIOD_MS: u64 = 33;

/// Tipo de panel auxiliar que reemplaza el viewport cuando está abierto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    Bookmarks,
    History,
    /// Vista de "Page Source" — HTML crudo de la pestaña activa.
    Source,
}

pub(crate) const ZOOM_MIN: f32 = 0.5;
pub(crate) const ZOOM_MAX: f32 = 3.0;
pub(crate) const ZOOM_STEP: f32 = 1.1;

impl Model {
    pub(crate) fn active(&self) -> &TabState {
        &self.tabs[self.active]
    }
    pub(crate) fn active_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active]
    }
    pub(crate) fn tab_idx(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Índices (en `self.tabs`) de las pestañas que pertenecen al space `sp`,
    /// en orden de aparición. Es el contenido del panel de un diente.
    pub(crate) fn tabs_in_space(&self, sp: usize) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.space == sp)
            .map(|(i, _)| i)
            .collect()
    }

    /// Pestañas del space activo — lo que se pinta en la barra/sidebar.
    pub(crate) fn active_space_tabs(&self) -> Vec<usize> {
        self.tabs_in_space(self.active_space)
    }

    /// Cuántos spaces no vacíos hay (para no dejar dientes fantasma).
    pub(crate) fn space_count(&self) -> usize {
        self.spaces.len()
    }
    /// Compila el predicado de búsqueda activo (query + toggles) en un
    /// `Matcher` reutilizable por count/highlight/scroll. La query se
    /// normaliza una sola vez acá para no pagar el cast por hoja.
    pub(crate) fn find_matcher(&self) -> Matcher {
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
    pub(crate) fn focused_text_input(&self) -> Option<(&TextInputState, bool)> {
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
    pub(crate) fn find_open(&mut self) {
        self.find_active = true;
        // Re-abrir limpia query previa para que el usuario arranque fresh.
        self.find_input.clear();
        self.find_current = 0;
    }

    /// `Esc` / ✕ — cierra la find bar y limpia la query (el highlight
    /// desaparece porque el matcher queda vacío).
    pub(crate) fn find_close(&mut self) {
        self.find_active = false;
        self.find_input.clear();
        self.find_current = 0;
    }

    /// Navega al match siguiente (`forward=true`) o anterior con
    /// wrap-around 1↔N. No-op si la query no tiene matches. Mueve el
    /// `scroll_y` del tab activo al match resultante.
    pub(crate) fn find_step(&mut self, forward: bool) {
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
        mk(0, "https://tawasuyu.net", "tawasuyu · suite soberana"),
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
pub(crate) fn wheel_to_msg(delta: WheelDelta, mods: Modifiers) -> Option<Msg> {
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

pub(crate) fn truncate(s: &str, max: usize) -> String {
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
pub(crate) fn scroll_to_find_match(m: &mut Model, matcher: &Matcher) {
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
pub(crate) fn same_doc_with_fragment(current: &str, target: &str) -> Option<String> {
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
