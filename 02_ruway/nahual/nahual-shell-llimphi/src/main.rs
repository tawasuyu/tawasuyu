//! `nahual-shell-llimphi` — MVP del shell nahual sobre Llimphi.
//!
//! Composición mínima: barra superior con la ruta + split draggable
//! con `nahual-file-explorer-llimphi` a la izquierda y
//! `nahual-text-viewer-llimphi` a la derecha. Foco en validar la
//! composición Llimphi y consumir crates reusables; no en paridad con
//! el shell GPUI.
//!
//! Lo que **sí** hace este MVP:
//! - Navegación con teclado: ↑/↓ y rueda mueven la selección/scroll;
//!   Enter entra a un directorio o abre un archivo; Backspace sube al
//!   padre.
//! - Click en una fila: selecciona; si es archivo, lo previsualiza.
//! - Preview de archivos texto pequeños (delegado al crate
//!   `nahual-text-viewer-llimphi`, ≤ 256 KB, UTF-8 sin null bytes).
//! - Splitter draggable.
//!
//! El viewer se elige por **contenido**, no por extensión:
//! `viewer_registry::pick` despacha el `Discernment` de `shuma-discern`
//! (magic-bytes, JSON/TOML/Card probe, UTF-8) al visor que sabe pintar
//! esa naturaleza de dato. Es el germen del "open-with universal":
//! cuando lleguen más visores y un AppBus con `EntityType`, el registro
//! crece por tabla sin tocar el resto del shell.
//!
//! Hoy embebe once visores in-process — texto (fallback universal),
//! imagen, video (AV1 nativo), audio (WAV/MP3/FLAC/Opus/Vorbis por cpal,
//! con espectro en vivo), card (`shared/card` presentada por campos),
//! tree (árbol JSON/TOML indentado), hex (dump de binarios), table
//! (CSV/TSV alineado), markdown (`.md` renderizado con encabezados,
//! listas, código y citas), archive (listado de ZIP/tar/tar.gz; ZIP
//! cubre .jar/.apk/.epub/OOXML) y font (TTF/OTF: metadatos + muestra
//! dibujada con los contornos de la propia fuente) — todos ruteados por
//! `viewer_registry::pick` sobre el `lens`/`mime` discernido. `Space`
//! hace play/pausa del video o audio.
//!
//! Lo que **todavía** no:
//! - `layout.json` / `Persister` / hot-reload.
//! - Otros containers (Tabs, Tiled) y un reader PDF nativo.
//! - AppBus: el viewer recibe el path directo desde el modelo. Cuando
//!   tengamos un bus, el shell publica `EntitySelected` y los viewers
//!   se suscriben.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod ops;
mod state;
mod viewer_registry;
use ops::{OpKind, OpQueue, OpStatus};
use state::{Label, ShellState};
use viewer_registry::ViewerKind;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    style::Position,
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use nahual_thumb_core::{generar_thumb_de_archivo, ThumbRgba};
use llimphi_theme::Theme;
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_grid::{
    grid_view, ventana_visible, GridCell, GridMetrics, GridPalette, GridSpec,
};
use llimphi_widget_breadcrumb::{breadcrumb_view, BreadcrumbPalette};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};
use llimphi_widget_text_editor::{
    text_editor_view_full, EditorMetrics, EditorPalette, EditorState, Language, PointerEvent,
};
use llimphi_icons::{icon_view, Icon};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use app_bus::{AppMenu, AppRegistry, Menu, MenuItem};
use nahual_source_core::{
    ArchiveSource, MingaSource, Navigator, Node, NodeKind, NouserSource, Opened, PosixSource,
    WawaImgSource,
};
use nahual_image_viewer_llimphi::{
    image_viewer_view, image_viewer_view_zoom, load_image, ImagePreviewState, ImageViewerPalette,
    ImageViewport, DEFAULT_IMAGE_BYTES_MAX,
};
use nahual_text_viewer_llimphi::{
    load_preview, text_viewer_view, PreviewState, TextViewerPalette,
    DEFAULT_PREVIEW_BYTES_MAX,
};
use nahual_video_viewer_llimphi::{
    video_viewer_view, VideoViewerPalette, VideoViewerState,
};
use nahual_card_viewer_llimphi::{
    card_viewer_view, load_card, CardPreview, CardViewerPalette,
};
use nahual_audio_viewer_llimphi::{
    audio_viewer_view, AudioViewerPalette, AudioViewerState,
};
use nahual_tree_viewer_llimphi::{
    load_tree, tree_viewer_view, TreePreview, TreeViewerPalette, DEFAULT_TREE_BYTES_MAX,
};
use nahual_hex_viewer_llimphi::{
    hex_viewer_view, load_hex, HexPreview, HexViewerPalette, DEFAULT_HEX_BYTES_MAX,
};
use nahual_table_viewer_llimphi::{
    load_table, table_viewer_view, TablePreview, TableViewerPalette, DEFAULT_TABLE_BYTES_MAX,
};
use nahual_markdown_viewer_llimphi::{
    load_markdown, markdown_viewer_view, MarkdownPreview, MarkdownViewerPalette,
    DEFAULT_MARKDOWN_BYTES_MAX,
};
use nahual_archive_viewer_llimphi::{
    archive_viewer_view, load_archive, ArchivePreview, ArchiveViewerPalette,
};
use nahual_font_viewer_llimphi::{
    font_viewer_view, load_font, FontPreview, FontViewerPalette, DEFAULT_FONT_BYTES_MAX,
};
use nahual_map_viewer_llimphi::{
    load_map, map_viewer_view, Basemap, MapPreview, MapView, MapViewerPalette, DEFAULT_MAP_BYTES_MAX,
};
use wawa_config_llimphi::theme_from_wawa;

fn main() {
    llimphi_ui::run::<Shell>();
}

/// Qué viewer pinta el panel derecho. Lo decide [`viewer_registry::pick`]
/// sobre el `Discernment` del **contenido** (no la extensión); los
/// archivos sin match caen como `Text` y el text viewer los muestra como
/// binarios si no son UTF-8 — fallback que pasa por la guard de
/// `load_preview`.
enum PreviewPane {
    Empty,
    Text(PreviewState),
    Image(ImagePreviewState),
    Video(VideoViewerState),
    Audio(AudioViewerState),
    Card(CardPreview),
    Tree(TreePreview),
    Hex(HexPreview),
    Table(TablePreview),
    Markdown(MarkdownPreview),
    Archive(ArchivePreview),
    Font(FontPreview),
    Map(MapPreview),
    /// Página HTML. El panel muestra el fuente (mismo visor de texto); el
    /// render real es asunto de **puriy**, que se lanza al abrir el archivo
    /// (Enter) sobre `file://<path>`. Costura nahual↔puriy.
    Web(PreviewState),
}

/// Cadencia del avance de los visores con reloj (video, audio) ~30 Hz.
/// `spawn_periodic` la dispara siempre; el `update` sólo tickea el panel
/// derecho cuando es de los que avanzan.
const FRAME_TICK: Duration = Duration::from_millis(33);

/// Alto de cada fila del árbol lateral (px) — debe coincidir con el
/// `row_height` que se le pasa a `tree_view`, para que el ventaneo cuadre.
const TREE_ROW_H: f32 = 22.0;

/// Ancho del rail de dientes (sesiones de trabajo), px.
const SESSION_RAIL_W: f32 = 40.0;

/// Intervalo mínimo entre re-streams del basemap PMTiles (debounce): los
/// pans/zooms se acumulan y se recalcula el viewport a lo sumo cada tanto,
/// para no rehacer la fusión de tiles en cada evento de arrastre.
const RESTREAM_THROTTLE: Duration = Duration::from_millis(90);

/// Un panel del file manager: su propia pila de navegación (mount stack). En
/// modo simple sólo el panel 0 se ve (panel 1 = visor); en modo dual ambos son
/// listas de archivos lado a lado (Fase 4.2c).
struct Pane {
    /// Pila de navegación: `[0]` = base POSIX (anclada en `/`, arrancada en el
    /// cwd con miga completa); montar una fuente no-POSIX empuja, desmontar
    /// saca. El navegador activo del panel es el tope. Nunca vacía.
    nav_stack: Vec<Navigator>,
    /// Selección **múltiple** marcada (Insert): ids de nodos sobre los que
    /// actúan las operaciones por lote (borrar/copiar/mover). Vacía = la
    /// operación recae sobre el cursor (`selected`). Se limpia al cambiar de
    /// directorio o tras ejecutar una operación.
    marked: BTreeSet<nahual_source_core::NodeId>,
    /// Historial de navegación estilo browser (sólo carpetas POSIX): la
    /// posición `hist_pos` es el presente; back/forward se mueven por acá
    /// sin truncar, y navegar a un lugar nuevo poda la cola forward.
    hist: Vec<PathBuf>,
    /// Posición actual dentro de `hist`.
    hist_pos: usize,
}

impl Pane {
    /// Panel vacío transitorio: sólo se usa como placeholder durante el swap
    /// de sesiones (`std::mem::replace`). Su `nav_stack` está vacío, así que
    /// **nunca** se le debe llamar `nav()`/`nav_mut()` mientras es placeholder.
    fn empty() -> Self {
        Pane { nav_stack: Vec::new(), marked: BTreeSet::new(), hist: Vec::new(), hist_pos: 0 }
    }

    fn nav(&self) -> &Navigator {
        self.nav_stack.last().expect("nav_stack nunca vacía")
    }
    fn nav_mut(&mut self) -> &mut Navigator {
        self.nav_stack.last_mut().expect("nav_stack nunca vacía")
    }
    fn is_foreign(&self) -> bool {
        self.nav_stack.len() > 1
    }
    /// Los ids objetivo de una operación por lote: la marca si hay, si no el
    /// nodo bajo el cursor. Cada uno con su nombre, para el rótulo del job.
    fn op_targets(&self) -> Vec<(nahual_source_core::NodeId, String)> {
        if !self.marked.is_empty() {
            let nav = self.nav();
            self.marked
                .iter()
                .filter_map(|id| {
                    nav.children()
                        .iter()
                        .find(|n| &n.id == id)
                        .map(|n| (id.clone(), n.name.clone()))
                })
                .collect()
        } else if let Some(n) = self.nav().selected_node() {
            vec![(n.id.clone(), n.name.clone())]
        } else {
            Vec::new()
        }
    }
}

/// Pedido de un nombre antes de ejecutar una operación (nueva carpeta, nuevo
/// archivo, renombrar). Captura el teclado mientras está activo.
struct Prompt {
    kind: PromptKind,
    text: String,
}

/// Qué operación dispara el [`Prompt`] al confirmarse.
enum PromptKind {
    /// Crear un directorio dentro del id contenedor.
    NewDir { parent: nahual_source_core::NodeId },
    /// Crear un archivo vacío dentro del id contenedor.
    NewFile { parent: nahual_source_core::NodeId },
    /// Renombrar el nodo `id` (el texto arranca con su nombre actual).
    Rename { id: nahual_source_core::NodeId },
}

impl Prompt {
    /// Título humano del overlay.
    fn title(&self) -> &'static str {
        match self.kind {
            PromptKind::NewDir { .. } => "Nueva carpeta",
            PromptKind::NewFile { .. } => "Nuevo archivo",
            PromptKind::Rename { .. } => "Renombrar",
        }
    }
}

/// Estado del **renombrado por lote** (Fase 4.5): un patrón en edición + los
/// nodos objetivo (la marca del panel). El patrón soporta tokens `{name}`
/// (nombre sin extensión), `{ext}` (extensión sin punto) y `{n}` (contador
/// 1-based, en el orden de los objetivos). El overlay pinta la previsualización
/// `viejo → nuevo` antes de aplicar.
struct BatchRename {
    /// Patrón en edición (p. ej. `foto_{n}.{ext}`).
    pattern: String,
    /// `(id, nombre_original)` de cada nodo a renombrar, en orden estable.
    targets: Vec<(nahual_source_core::NodeId, String)>,
}

impl BatchRename {
    /// Calcula el nuevo nombre del objetivo `idx` aplicando el patrón al nombre
    /// original. Si el resultado queda vacío, conserva el original (no se
    /// renombra a "nada").
    fn nuevo_nombre(&self, idx: usize) -> String {
        let original = &self.targets[idx].1;
        let out = aplicar_patron(&self.pattern, original, idx + 1);
        if out.trim().is_empty() {
            original.clone()
        } else {
            out
        }
    }
}

/// Sustituye los tokens del patrón de batch-rename sobre `original` (el nombre
/// completo, con extensión) usando el contador `n` (1-based). Tokens:
/// `{name}` = stem, `{ext}` = extensión sin punto, `{n}` = contador. El texto
/// fuera de los tokens es literal. Un `{ext}` sobre un archivo sin extensión
/// rinde vacío (y el `.` que lo preceda queda — responsabilidad del patrón).
fn aplicar_patron(pattern: &str, original: &str, n: usize) -> String {
    let (stem, ext) = match original.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), e.to_string()),
        _ => (original.to_string(), String::new()),
    };
    pattern
        .replace("{name}", &stem)
        .replace("{ext}", &ext)
        .replace("{n}", &n.to_string())
}

/// App integrada abierta **en el canvas** (doble click / Enter sobre un
/// archivo): editor de texto potente, visor de imágenes con zoom, o player
/// de media (video/audio sobre `media-source-*`). Esc/⌫ vuelve a la carpeta.
enum CanvasApp {
    Texto { path: PathBuf, editor: Box<EditorState>, dirty: bool, saved: bool },
    Imagen { path: PathBuf, state: ImagePreviewState, viewport: ImageViewport },
    Video(VideoViewerState),
    Audio(AudioViewerState),
}

/// Clipboard del sistema para el editor del canvas (mismo backend que nada).
struct ShellClipboard {
    inner: Option<arboard::Clipboard>,
}

impl ShellClipboard {
    fn new() -> Self {
        Self { inner: arboard::Clipboard::new().ok() }
    }
}

impl llimphi_widget_text_editor::Clipboard for ShellClipboard {
    fn get(&mut self) -> Option<String> {
        self.inner.as_mut()?.get_text().ok()
    }
    fn set(&mut self, s: &str) {
        if let Some(c) = self.inner.as_mut() {
            let _ = c.set_text(s.to_owned());
        }
    }
}

/// Estado *movible* de una sesión de trabajo (diente del rail): todo lo que
/// cambia al saltar de una sesión a otra. La sesión activa **no** guarda su
/// snap aquí — sus campos viven directamente en `Model` (los `panes`,
/// `preview`, el árbol expandido, etc.). Al cambiar de diente se hace swap:
/// los campos vivos del `Model` se vuelcan a un `SessionSnap` y los de la
/// sesión destino se restauran.
struct SessionSnap {
    panes: [Pane; 2],
    focus: usize,
    dual: bool,
    list_width: f32,
    nav_filtering: bool,
    preview: PreviewPane,
    preview_of: Option<PathBuf>,
    preview_temp: Option<tempfile::TempDir>,
    map_view: MapView,
    basemap: Option<Basemap>,
    basemap_dirty: bool,
    last_restream: Option<Instant>,
    /// Carpetas descolapsadas del árbol lateral — **por sesión**.
    tree_expanded: BTreeSet<PathBuf>,
    /// Offset de scroll del árbol lateral (en filas) — por sesión.
    tree_scroll: usize,
    /// `true` = el sidebar derecho de preview está abierto.
    viewer_open: bool,
    /// App integrada abierta en el canvas (editor/imagen/media), si hay.
    canvas: Option<CanvasApp>,
}

/// Una sesión de trabajo, representada por un **diente** del rail. `snap` es
/// `None` para la sesión **activa** (sus campos están vivos en `Model`) y
/// `Some(_)` para las inactivas.
struct Session {
    name: String,
    snap: Option<SessionSnap>,
}

struct Model {
    /// Sesiones de trabajo abiertas (los dientes del rail). `sessions[active]`
    /// es la viva (sus campos de navegación/preview/árbol están en los campos
    /// sueltos de abajo).
    sessions: Vec<Session>,
    /// Índice de la sesión activa.
    active: usize,
    /// Carpetas descolapsadas del árbol lateral de la sesión activa.
    tree_expanded: BTreeSet<PathBuf>,
    /// Offset de scroll del árbol lateral (en filas) de la sesión activa.
    tree_scroll: usize,
    /// Cache **global** de subcarpetas por carpeta (sólo directorios, ya
    /// ordenados). No es por-sesión: el contenido de un dir es el mismo para
    /// todas; sólo el set de expandidas cambia por sesión.
    tree_children: HashMap<PathBuf, Vec<PathBuf>>,
    /// Ancho del sidebar (árbol de carpetas) en px. Lo muta su splitter.
    tree_w: f32,
    /// Ancho del sidebar derecho del visor (preview), px. Lo muta su splitter.
    preview_w: f32,
    /// `true` = el sidebar derecho de preview está abierto (lo togglea su
    /// diente; Esc/⌫ también lo cierra). Por sesión (va al snap).
    viewer_open: bool,
    /// App integrada abierta en el canvas (editor/imagen/media), si hay.
    /// Por sesión (va al snap). Esc/⌫ la cierra.
    canvas: Option<CanvasApp>,
    /// Clipboard del sistema para el editor del canvas. Transitorio.
    clipboard: ShellClipboard,
    /// Último click en una fila (pane, idx, instante) — para detectar el
    /// doble click que abre carpeta/archivo. Transitorio, no va al snap.
    last_click: Option<(usize, usize, Instant)>,
    /// Acumulador del drag del editor del canvas (como `drag_accum` de nada).
    canvas_drag: (f32, f32),
    /// Los dos paneles (Fase 4.2c). `panes[focus]` es el activo (recibe
    /// teclado). En modo simple sólo se ve el 0; en dual, ambos.
    panes: [Pane; 2],
    /// Panel activo: 0 o 1.
    focus: usize,
    /// `true` = dos paneles de archivos lado a lado; `false` = panel + visor.
    dual: bool,
    /// Ancho del panel izquierdo en px. Lo muta el drag del splitter.
    list_width: f32,
    /// `true` mientras se teclea el filtro vivo sobre la fuente montada
    /// (entra con `/`, sale con Esc/Enter). El teclado se captura al filtro.
    nav_filtering: bool,
    preview: PreviewPane,
    /// Path del archivo previsualizado (header del panel derecho).
    preview_of: Option<PathBuf>,
    /// Materialización temporal de una hoja no-POSIX: los visores son
    /// path-based (`load_image(path)`), así que los bytes de un objeto wawa
    /// se vuelcan a un tempfile y se previsualizan por ahí. Vive mientras el
    /// visor lo lea (audio/video streamean del path); se reemplaza al cambiar
    /// de preview.
    preview_temp: Option<tempfile::TempDir>,
    theme: Theme,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    menu_open: Option<usize>,
    /// Fila activa dentro del dropdown abierto (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición del dropdown.
    menu_anim: Tween<f32>,
    /// Menú contextual sobre el nodo/archivo seleccionado: ancla `(x, y)`
    /// en coords de ventana. `None` cerrado. No hay edición de texto en el
    /// shell, así que el contextual lista acciones de navegación/montaje.
    context_menu: Option<(f32, f32)>,
    /// Cámara del visor de mapas (zoom/pan). Se resetea al cambiar de
    /// preview; la mutan el arrastre y la rueda sobre el panel del mapa.
    map_view: MapView,
    /// Basemap PMTiles vivo, si el archivo abierto es un `.pmtiles`. Mantiene
    /// el contenedor + caché de tiles para el streaming por viewport.
    basemap: Option<Basemap>,
    /// La cámara cambió y el basemap necesita re-streamear. El Tick lo procesa
    /// con throttle (debounce): coalesce muchos pans en pocos recálculos.
    basemap_dirty: bool,
    /// Último instante en que se re-streameó (para el throttle).
    last_restream: Option<Instant>,
    /// Suscripción al bus de configuración del SO.
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,
    /// Catálogo de apps de la suite (AppBus): qué app abre qué mime. Se
    /// consulta al abrir el menú contextual sobre un archivo (open-with).
    registry: AppRegistry,
    /// Opciones "Abrir con <app>" precomputadas al abrir el contextual:
    /// `(app_id, label)`. El render del menú las pinta sin tocar el registro.
    ctx_open_with: Vec<(String, String)>,
    /// El archivo que el contextual abriría: ruta POSIX real, o un tempfile
    /// materializado de una hoja no-POSIX (Mónada/wawa). `None` si la
    /// selección no es un archivo abrible.
    ctx_target: Option<PathBuf>,
    /// Tempfile de la hoja no-POSIX materializada (lo mantiene vivo mientras
    /// la app externa lo lee). Se reemplaza al recomputar el contextual.
    ctx_temp: Option<tempfile::TempDir>,
    /// Cola de operaciones de archivo en vuelo / historial (Fase 4.3). El panel
    /// inferior colapsable la lista.
    queue: OpQueue,
    /// Pedido de nombre activo (nueva carpeta/archivo, renombrar). `None` =
    /// sin overlay; mientras esté `Some`, el teclado va al texto.
    prompt: Option<Prompt>,
    /// Confirmación de borrado pendiente: los `(id, nombre)` a borrar. `None` =
    /// sin diálogo. Borrar es destructivo, así que pasa por este sí/no.
    confirm_delete: Option<Vec<(nahual_source_core::NodeId, String)>>,
    /// Renombrado por lote en curso (Fase 4.5): patrón + objetivos + preview.
    /// `None` = sin overlay; mientras esté `Some`, el teclado va al patrón.
    batch: Option<BatchRename>,
    /// Preferencias persistidas (Fase 4.5): labels de color por archivo,
    /// favoritos, recientes, folder formats. Se relee al arrancar y se reescribe
    /// tras cada cambio.
    state: ShellState,
    /// Cache RAM de miniaturas listas para pintar (vista iconos, Fase 4.8).
    /// Clave = ruta POSIX del archivo. Se llena async vía `Handle::spawn`.
    thumbs: HashMap<PathBuf, Image>,
    /// Miniaturas pedidas y aún en vuelo (dedup: no relanzar el mismo path).
    thumbs_pending: HashSet<PathBuf>,
    /// Miniaturas que fallaron al generarse (se pinta un ⚠, no se reintenta).
    thumbs_failed: HashSet<PathBuf>,
}

#[derive(Clone)]
enum Msg {
    Up,
    Down,
    OpenSelected,
    Parent,
    /// Click en una fila del panel `pane`: lo enfoca y selecciona la fila.
    SelectIn(usize, usize),
    /// Alterna panel doble ↔ panel + visor (`d`).
    ToggleDual,
    /// Cambia el foco al otro panel (Tab), sólo en modo dual.
    SwitchFocus,
    /// Scroll en filas — positivo abajo, negativo arriba.
    Scroll(i32),
    /// Arrastre sobre el mapa: panea la cámara `(dx, dy)` en px físicos.
    MapPan(f32, f32),
    /// Rueda sobre el mapa: zoom anclado al cursor `(delta, cx, cy)`.
    MapZoom(f32, f32, f32),
    /// Reencuadra el mapa (zoom 1, sin pan).
    MapReset,
    /// Alterna el mapa-base mundial de fondo.
    MapToggleBase,
    /// Clic sobre el mapa: `(fx, fy)` fracción del rect → selecciona feature.
    MapClick(f32, f32),
    /// Cicla el campo numérico de coloreo (choropleth) del mapa.
    MapCycleColor,
    /// Entra en modo búsqueda de features (`/`).
    MapSearchStart,
    /// Agrega texto a la consulta de búsqueda.
    MapSearchInput(String),
    /// Borra el último carácter de la consulta.
    MapSearchBackspace,
    /// Confirma la búsqueda: vuela al mejor resultado.
    MapSearchSubmit,
    /// Cancela la búsqueda.
    MapSearchCancel,
    /// Alterna el modo ruteo (clics = origen/destino).
    MapRouteToggle,
    /// Drag del divisor — positivo = lista crece.
    ResizeList(f32),
    /// El bus `wawa-config` publicó una versión nueva.
    WawaConfigChanged(Box<wawa_config::WawaConfig>),
    /// Pulso de reloj de los visores con transporte (video/audio, ~30 Hz).
    Tick,
    /// Espacio: play/pausa del panel derecho si es video o audio.
    TogglePlay,
    /// `m`: montar el directorio objetivo como Mónadas semánticas (nouser).
    MountNouser,
    /// `g`: montar el directorio objetivo como grafo CAS de minga, si parece
    /// un repo `.minga` (guard anti-creación de sled en dirs ajenos).
    MountMinga,
    /// Desmonta la fuente no-POSIX activa y vuelve al filesystem.
    Unmount,
    /// Cicla el tema claro/oscuro (preset siguiente).
    CycleTheme,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Navega la fila activa del dropdown (+1/-1).
    MenuNav(i32),
    /// Ejecuta el comando de la fila activa (Enter).
    MenuActivate,
    /// No-op: sólo fuerza re-render durante la animación del dropdown.
    MenuTick,
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click en la raíz → abre el menú contextual anclado en `(x, y)`
    /// de ventana sobre la entrada seleccionada.
    ContextMenuOpen(f32, f32),
    /// Click en un encabezado de la vista detalle del panel `pane` → ordena por
    /// esa columna (0 nombre · 1 tamaño · 2 fecha · 3 tipo). Toggle si repite.
    SortByIn(usize, usize),
    /// Alterna lista ↔ detalle sobre la fuente montada (`v`).
    NavToggleView,
    /// Entra en modo filtro vivo de la fuente montada (`/`).
    NavFilterStart,
    /// Agrega texto al filtro vivo.
    NavFilterInput(String),
    /// Borra el último carácter del filtro.
    NavFilterBackspace,
    /// Sale del modo filtro (conserva el filtro aplicado).
    NavFilterEnd,
    /// Click en un segmento del breadcrumb del panel `pane` → sube a ese nivel.
    BreadcrumbIn(usize, usize),
    /// Abre el archivo seleccionado con la app `id` de la suite (AppBus).
    OpenWith(String),
    /// Edita el archivo seleccionado en `nada`.
    EditSelected,
    /// Abre una terminal `shuma` en el directorio actual.
    TerminalHere,

    // ---- Fase 4.3: operaciones de archivo + cola ----
    /// Marca/desmarca el nodo bajo el cursor (Insert) y baja una fila.
    ToggleMark,
    /// Abre el prompt de nombre para crear una carpeta en el dir actual.
    NewDirPrompt,
    /// Abre el prompt de nombre para crear un archivo en el dir actual.
    NewFilePrompt,
    /// Abre el prompt de renombrar sobre el nodo seleccionado (texto = nombre).
    RenamePrompt,
    /// Agrega texto al prompt activo.
    PromptInput(String),
    /// Borra el último carácter del prompt.
    PromptBackspace,
    /// Confirma el prompt → encola la operación.
    PromptSubmit,
    /// Cancela el prompt sin operar.
    PromptCancel,
    /// Pide confirmación para borrar la selección (marca o cursor).
    DeleteSelection,
    /// Confirma el borrado pendiente → encola un job por nodo.
    ConfirmDelete,
    /// Cancela el diálogo de borrado.
    CancelConfirm,
    /// Copia la selección (marca o cursor) al directorio del otro panel.
    CopyToOther,
    /// Mueve la selección (marca o cursor) al directorio del otro panel.
    MoveToOther,
    /// Encola una operación de archivo y lanza su worker.
    RunOp(OpKind),
    /// El worker terminó la operación `id` con este resultado.
    OpFinished {
        id: u64,
        result: Result<Option<nahual_source_core::NodeId>, String>,
    },
    /// Despliega/colapsa el panel de la cola.
    ToggleQueue,
    /// Olvida los jobs ya terminados de la cola.
    ClearQueue,

    // ---- Fase 4.5: renombrado por lote ----
    /// Abre el overlay de renombrado por lote sobre la marca del panel.
    BatchRenameStart,
    /// Agrega texto al patrón del batch.
    BatchPatternInput(String),
    /// Borra el último carácter del patrón.
    BatchPatternBackspace,
    /// Aplica el patrón: encola un Rename por objetivo cuyo nombre cambie.
    BatchApply,
    /// Cierra el overlay sin renombrar.
    BatchCancel,
    /// Asigna un label de color a la selección (marca o cursor).
    SetLabel(Label),
    /// Quita el label de la selección.
    ClearLabel,
    /// Agrega a favoritos la carpeta seleccionada (o la actual si no es dir).
    AddPlace,

    // ---- Sesiones de trabajo (dientes del rail) ----
    /// Abre una sesión nueva (posada en la carpeta actual).
    SessionNew,
    /// Activa la sesión (diente) en el índice dado.
    SessionActivate(usize),

    // ---- Árbol de carpetas del sidebar ----
    /// Expande/colapsa una carpeta del árbol lateral.
    TreeToggle(PathBuf),
    /// Navega el panel activo a una carpeta del árbol lateral (y la expande).
    TreeSelect(PathBuf),
    /// Rueda sobre el árbol lateral: scrollea sus filas (delta en líneas).
    TreeScroll(f32),
    /// Drag del splitter del sidebar: ajusta el ancho del árbol.
    ResizeTree(f32),
    /// Drag del splitter del visor derecho: ajusta su ancho.
    ResizePreview(f32),
    /// Fija el modo de vista del panel activo (toolbar; `v` sigue ciclando).
    SetViewMode(nahual_source_core::ViewMode),
    /// Expande inline la carpeta seleccionada (→ en lista/detalle).
    ExpandSelected,
    /// Colapsa la carpeta seleccionada; si ya está colapsada, salta al padre.
    CollapseSelected,

    // ---- Historia de navegación (estilo browser) ----
    /// Vuelve a la carpeta anterior del historial del panel activo.
    NavBack,
    /// Avanza a la carpeta siguiente del historial.
    NavForward,

    // ---- Canvas apps + panel de preview ----
    /// Diente derecho: abre/cierra el sidebar de preview.
    TogglePreviewPanel,
    /// Cierra la app integrada del canvas (editor/imagen/media).
    CanvasClose,
    /// Tecla rumbo al editor de texto del canvas.
    CanvasEditKey(KeyEvent),
    /// Click/drag sobre el editor del canvas (posicionar caret / seleccionar).
    CanvasEditPointer(PointerEvent),
    /// Guarda el archivo del editor del canvas (Ctrl+S).
    CanvasSave,
    /// Zoom del visor de imagen del canvas (Ctrl+rueda / pinch).
    CanvasImgZoom(f32),
    /// Pan (arrastre) del visor de imagen del canvas.
    CanvasImgPan(f32, f32),
    /// Doble tap: resetea zoom/pan de la imagen del canvas.
    CanvasImgReset,

    // ---- Fase 4.8: vista iconos con miniaturas ----
    /// Una miniatura terminó de generarse (llega del worker).
    ThumbReady(PathBuf, ThumbRgba),
    /// La miniatura de este path falló (formato no soportado / I/O).
    ThumbFailed(PathBuf),
}

impl Model {
    /// El navegador activo: el tope de la pila del panel enfocado.
    fn cur(&self) -> &Navigator {
        self.panes[self.focus].nav()
    }

    /// El navegador activo, mutable.
    fn cur_mut(&mut self) -> &mut Navigator {
        self.panes[self.focus].nav_mut()
    }

    /// `true` si el panel enfocado tiene una fuente no-POSIX montada (pila > 1).
    /// Gatea el montaje (no se anidan fuentes) y el desmontaje.
    fn is_foreign(&self) -> bool {
        self.panes[self.focus].is_foreign()
    }

    /// El panel enfocado, mutable (para empujar/sacar de su pila de montaje).
    fn cur_pane_mut(&mut self) -> &mut Pane {
        let f = self.focus;
        &mut self.panes[f]
    }

    /// El panel enfocado (lectura).
    fn cur_pane(&self) -> &Pane {
        &self.panes[self.focus]
    }

    /// `true` si la fuente activa admite operaciones de archivo (POSIX). Las
    /// fuentes montadas (wawa/minga/nouser) son read-only → sin `SourceMut`.
    fn can_edit(&self) -> bool {
        self.cur().writable().is_some()
    }

    /// Vuelca los campos vivos de navegación/preview a un `SessionSnap`
    /// (deja los campos del `Model` en su estado por defecto, listos para
    /// recibir los de otra sesión).
    fn snapshot_active(&mut self) -> SessionSnap {
        SessionSnap {
            panes: [
                std::mem::replace(&mut self.panes[0], Pane::empty()),
                std::mem::replace(&mut self.panes[1], Pane::empty()),
            ],
            focus: self.focus,
            dual: self.dual,
            list_width: self.list_width,
            nav_filtering: self.nav_filtering,
            preview: std::mem::replace(&mut self.preview, PreviewPane::Empty),
            preview_of: self.preview_of.take(),
            preview_temp: self.preview_temp.take(),
            map_view: std::mem::take(&mut self.map_view),
            basemap: self.basemap.take(),
            basemap_dirty: self.basemap_dirty,
            last_restream: self.last_restream.take(),
            tree_expanded: std::mem::take(&mut self.tree_expanded),
            tree_scroll: self.tree_scroll,
            viewer_open: self.viewer_open,
            canvas: self.canvas.take(),
        }
    }

    /// Restaura los campos vivos desde un `SessionSnap` (al activar su diente).
    fn restore(&mut self, snap: SessionSnap) {
        let [p0, p1] = snap.panes;
        self.panes = [p0, p1];
        self.focus = snap.focus;
        self.dual = snap.dual;
        self.list_width = snap.list_width;
        self.nav_filtering = snap.nav_filtering;
        self.preview = snap.preview;
        self.preview_of = snap.preview_of;
        self.preview_temp = snap.preview_temp;
        self.map_view = snap.map_view;
        self.basemap = snap.basemap;
        self.basemap_dirty = snap.basemap_dirty;
        self.last_restream = snap.last_restream;
        self.tree_expanded = snap.tree_expanded;
        self.tree_scroll = snap.tree_scroll;
        self.viewer_open = snap.viewer_open;
        self.canvas = snap.canvas;
        // Asegurá el cache de subcarpetas de lo que esta sesión tiene abierto.
        ensure_children_for_expanded(&mut self.tree_children, &self.tree_expanded);
    }

    /// Activa la sesión `i`: guarda la sesión viva en su slot y restaura la
    /// destino. No hace nada si `i` ya es la activa o está fuera de rango.
    fn switch_to(&mut self, i: usize) {
        if i == self.active || i >= self.sessions.len() {
            return;
        }
        let snap = self.snapshot_active();
        self.sessions[self.active].snap = Some(snap);
        self.active = i;
        if let Some(snap) = self.sessions[i].snap.take() {
            self.restore(snap);
        }
    }
}

/// La carpeta actual del panel enfocado de la sesión viva, para sembrar una
/// sesión nueva y nombrarla.
fn cur_dir(m: &Model) -> PathBuf {
    PathBuf::from(m.cur().current_id().as_str())
}

/// Nombre corto de una sesión a partir de su carpeta de arranque.
fn session_name(cwd: &Path) -> String {
    cwd.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/".to_string())
}

/// Snapshot fresco para una sesión nueva, posada en `cwd`. El árbol arranca
/// descolapsado a lo largo de la cadena de ancestros de `cwd`.
fn fresh_snap(cwd: &Path) -> SessionSnap {
    SessionSnap {
        panes: [
            Pane {
                nav_stack: vec![posix_nav(cwd)],
                marked: BTreeSet::new(),
                hist: vec![cwd.to_path_buf()],
                hist_pos: 0,
            },
            Pane {
                nav_stack: vec![posix_nav(cwd)],
                marked: BTreeSet::new(),
                hist: vec![cwd.to_path_buf()],
                hist_pos: 0,
            },
        ],
        focus: 0,
        dual: false,
        list_width: 400.0,
        nav_filtering: false,
        preview: PreviewPane::Empty,
        preview_of: None,
        preview_temp: None,
        map_view: MapView::default(),
        basemap: None,
        basemap_dirty: false,
        last_restream: None,
        tree_expanded: ancestors_set(cwd),
        tree_scroll: 0,
        viewer_open: false,
        canvas: None,
    }
}

/// Construye el navegador **POSIX base**: ancla la fuente en `/` (para poder
/// subir hasta la raíz del filesystem) y arranca parado en `cwd`, sembrando la
/// pila de ancestros para que el breadcrumb tenga la ruta completa. Si algo
/// falla, cae a la raíz `/`.
fn posix_nav(cwd: &Path) -> Navigator {
    use std::path::Component;
    let mut stack = vec![Node::new("/", "/", true).with_kind(NodeKind::Dir)];
    let mut acc = PathBuf::from("/");
    for comp in cwd.components() {
        if let Component::Normal(c) = comp {
            acc.push(c);
            stack.push(
                Node::new(acc.to_string_lossy().into_owned(), c.to_string_lossy().into_owned(), true)
                    .with_kind(NodeKind::Dir),
            );
        }
    }
    Navigator::open_at(Box::new(PosixSource::new("/")), stack)
        .or_else(|_| Navigator::open(Box::new(PosixSource::new("/"))))
        .expect("la raíz / siempre se puede listar")
}

struct Shell;

impl App for Shell {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "nahual · shell"
    }

    fn initial_size() -> (u32, u32) {
        (1200, 800)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // El primer argumento, si es un directorio, fija el cwd de arranque
        // (lo usa `app_bus::reveal` para "Reveal in nahual <dir>").
        let cwd = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/"));
        let cfg = wawa_config::WawaConfig::load();
        let theme = theme_from_wawa(&cfg, &Theme::dark());
        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("nahual-shell · wawa-config watcher: {e}"))
        .ok();
        // Los visores con transporte (video, audio) necesitan un reloj
        // externo: cada pulso avanza un frame / refresca el espectro. Es
        // barato cuando el panel no avanza (el update sale temprano).
        handle.spawn_periodic(FRAME_TICK, || Msg::Tick);
        // El sidebar arranca con el árbol descolapsado hasta el cwd, para que la
        // carpeta actual se vea de entrada.
        let tree_expanded = ancestors_set(&cwd);
        let mut tree_children = HashMap::new();
        ensure_children_for_expanded(&mut tree_children, &tree_expanded);
        Model {
            // Una sola sesión al arrancar; la activa lleva `snap: None`.
            sessions: vec![Session { name: session_name(&cwd), snap: None }],
            active: 0,
            tree_expanded,
            tree_scroll: 0,
            tree_children,
            tree_w: 230.0,
            preview_w: 420.0,
            viewer_open: false,
            canvas: None,
            clipboard: ShellClipboard::new(),
            last_click: None,
            canvas_drag: (0.0, 0.0),
            // Ambos paneles arrancan en el cwd POSIX; el 1 se ve sólo en dual.
            panes: [
                Pane {
                    nav_stack: vec![posix_nav(&cwd)],
                    marked: BTreeSet::new(),
                    hist: vec![cwd.clone()],
                    hist_pos: 0,
                },
                Pane {
                    nav_stack: vec![posix_nav(&cwd)],
                    marked: BTreeSet::new(),
                    hist: vec![cwd.clone()],
                    hist_pos: 0,
                },
            ],
            focus: 0,
            dual: false,
            list_width: 400.0,
            nav_filtering: false,
            preview: PreviewPane::Empty,
            preview_of: None,
            preview_temp: None,
            theme,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
            map_view: MapView::default(),
            basemap: None,
            basemap_dirty: false,
            last_restream: None,
            _wawa_watcher: watcher,
            registry: AppRegistry::with_defaults(),
            ctx_open_with: Vec::new(),
            ctx_target: None,
            ctx_temp: None,
            queue: OpQueue::default(),
            prompt: None,
            confirm_delete: None,
            batch: None,
            state: ShellState::load(),
            thumbs: HashMap::new(),
            thumbs_pending: HashSet::new(),
            thumbs_failed: HashSet::new(),
        }
    }

    fn on_key(_model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Prompt de nombre (nueva carpeta/archivo, renombrar): captura todo el
        // teclado. Máxima prioridad — es un modal.
        if _model.prompt.is_some() {
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::PromptCancel),
                Key::Named(NamedKey::Enter) => Some(Msg::PromptSubmit),
                Key::Named(NamedKey::Backspace) => Some(Msg::PromptBackspace),
                Key::Named(NamedKey::Space) => Some(Msg::PromptInput(" ".to_string())),
                Key::Character(c) => Some(Msg::PromptInput(c.to_string())),
                _ => None,
            };
        }
        // Renombrado por lote: el teclado edita el patrón. Enter aplica.
        if _model.batch.is_some() {
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::BatchCancel),
                Key::Named(NamedKey::Enter) => Some(Msg::BatchApply),
                Key::Named(NamedKey::Backspace) => Some(Msg::BatchPatternBackspace),
                Key::Named(NamedKey::Space) => Some(Msg::BatchPatternInput(" ".to_string())),
                Key::Character(c) => Some(Msg::BatchPatternInput(c.to_string())),
                _ => None,
            };
        }
        // Diálogo de confirmación de borrado: Enter/y confirma, Esc/n cancela.
        if _model.confirm_delete.is_some() {
            return match &e.key {
                Key::Named(NamedKey::Enter) => Some(Msg::ConfirmDelete),
                Key::Character(c) if c == "y" => Some(Msg::ConfirmDelete),
                Key::Named(NamedKey::Escape) => Some(Msg::CancelConfirm),
                Key::Character(c) if c == "n" => Some(Msg::CancelConfirm),
                _ => None,
            };
        }
        // Menú principal abierto: las flechas navegan, Enter ejecuta, Esc
        // cierra. Tiene prioridad sobre la navegación del explorer.
        if let Some(mi) = _model.menu_open {
            let n = app_menu(_model).menus.len().max(1);
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        // Editor de texto del canvas abierto: el teclado es del editor.
        // Ctrl+S guarda; Esc sin selección/multicursor cierra; el resto va
        // al buffer (incluidos Ctrl+C/X/V/Z/Y, que resuelve el editor).
        if let Some(CanvasApp::Texto { editor, .. }) = &_model.canvas {
            if e.modifiers.ctrl {
                if let Key::Character(c) = &e.key {
                    if c == "s" {
                        return Some(Msg::CanvasSave);
                    }
                }
            }
            if matches!(e.key, Key::Named(NamedKey::Escape))
                && !editor.has_selection()
                && editor.extra_cursors.is_empty()
            {
                return Some(Msg::CanvasClose);
            }
            return Some(Msg::CanvasEditKey(e.clone()));
        }
        // Canvas con imagen/media: Esc o ⌫ cierran y vuelven a la carpeta.
        if _model.canvas.is_some() {
            if matches!(
                e.key,
                Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Backspace)
            ) {
                return Some(Msg::CanvasClose);
            }
        }
        // Modo búsqueda del mapa: captura todo el teclado para la consulta.
        if matches!(_model.preview, PreviewPane::Map(_)) && _model.map_view.searching {
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::MapSearchCancel),
                Key::Named(NamedKey::Enter) => Some(Msg::MapSearchSubmit),
                Key::Named(NamedKey::Backspace) => Some(Msg::MapSearchBackspace),
                Key::Named(NamedKey::Space) => Some(Msg::MapSearchInput(" ".to_string())),
                Key::Character(c) => Some(Msg::MapSearchInput(c.to_string())),
                _ => None,
            };
        }
        // Modo filtro vivo: captura el teclado para el filtro por nombre.
        if _model.nav_filtering {
            return match &e.key {
                Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Enter) => Some(Msg::NavFilterEnd),
                Key::Named(NamedKey::Backspace) => Some(Msg::NavFilterBackspace),
                Key::Named(NamedKey::Space) => Some(Msg::NavFilterInput(" ".to_string())),
                Key::Character(c) => Some(Msg::NavFilterInput(c.to_string())),
                _ => None,
            };
        }
        match &e.key {
            Key::Named(NamedKey::ArrowUp) => Some(Msg::Up),
            Key::Named(NamedKey::ArrowDown) => Some(Msg::Down),
            Key::Named(NamedKey::Enter) => Some(Msg::OpenSelected),
            Key::Named(NamedKey::Backspace) => Some(Msg::Parent),
            // Esc con el visor abierto vuelve a la vista de carpeta (el
            // handler de Parent cierra el visor antes de subir de dir).
            Key::Named(NamedKey::Escape) if _model.viewer_open => Some(Msg::Parent),
            // → expande inline la carpeta seleccionada; ← colapsa (o salta
            // al padre si ya está colapsada) — lista/detalle.
            Key::Named(NamedKey::ArrowRight) => Some(Msg::ExpandSelected),
            Key::Named(NamedKey::ArrowLeft) => Some(Msg::CollapseSelected),
            Key::Named(NamedKey::Space) => Some(Msg::TogglePlay),
            // `v` alterna lista/detalle, `/` filtra (salvo que un mapa quiera
            // `/` para su propia búsqueda, que tiene su arm más abajo).
            Key::Character(c) if c == "v" => Some(Msg::NavToggleView),
            Key::Character(c) if c == "/" && !matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::NavFilterStart)
            }
            // `d` alterna panel doble; Tab cambia el foco entre los dos.
            Key::Character(c) if c == "d" => Some(Msg::ToggleDual),
            Key::Named(NamedKey::Tab) => Some(Msg::SwitchFocus),
            // ---- Fase 4.3: operaciones de archivo (sólo sobre POSIX). ----
            // Marcar/desmarcar (selección múltiple) bajo el cursor.
            Key::Named(NamedKey::Insert) if _model.can_edit() => Some(Msg::ToggleMark),
            // F7 nueva carpeta · F2 renombrar · Delete borrar.
            Key::Named(NamedKey::F7) if _model.can_edit() => Some(Msg::NewDirPrompt),
            Key::Named(NamedKey::F2) if _model.can_edit() => Some(Msg::RenamePrompt),
            Key::Named(NamedKey::Delete) if _model.can_edit() => Some(Msg::DeleteSelection),
            // F5 copiar / F6 mover al otro panel (sólo en dual).
            Key::Named(NamedKey::F5) if _model.can_edit() && _model.dual => Some(Msg::CopyToOther),
            Key::Named(NamedKey::F6) if _model.can_edit() && _model.dual => Some(Msg::MoveToOther),
            // Puntos de entrada del front universal: montar el directorio
            // objetivo (el subdir seleccionado, o el cwd) como otra `Source`.
            // Sólo desde POSIX — dentro de una fuente montada no aplican.
            Key::Character(c) if c == "m" => Some(Msg::MountNouser),
            Key::Character(c) if c == "g" => Some(Msg::MountMinga),
            // Sobre un mapa: `f` reencuadra, `b` alterna el mapa-base.
            Key::Character(c) if c == "f" && matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::MapReset)
            }
            Key::Character(c) if c == "b" && matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::MapToggleBase)
            }
            Key::Character(c) if c == "c" && matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::MapCycleColor)
            }
            Key::Character(c) if c == "/" && matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::MapSearchStart)
            }
            Key::Character(c) if c == "r" && matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::MapRouteToggle)
            }
            _ => None,
        }
    }

    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Self::Msg> {
        // La rueda sobre el sidebar del árbol va SIEMPRE al árbol — ruteo por
        // región: el hit-test del `on_scroll` local se pierde entre updates
        // rápidos (el cache de render se invalida en cada update), y el
        // sobrante caía acá moviendo el canvas.
        if cursor.0 < model.tree_w {
            return Some(Msg::TreeScroll(delta.y));
        }
        // Si la rueda cae sobre el panel del mapa, hace zoom de la cámara en
        // vez de scrollear la lista (gateo por el rect que el canvas registra).
        if matches!(model.preview, PreviewPane::Map(_)) && model.map_view.contains(cursor.0, cursor.1)
        {
            return Some(Msg::MapZoom(delta.y, cursor.0, cursor.1));
        }
        // El delta del touchpad se acumula en `FileExplorerState`; acá
        // sólo aproximamos los pasos para evitar un round-trip por
        // sub-fila. El update llamará a `apply_wheel(delta.y)` para que
        // el acumulador real viva en el explorer, no en el shell.
        let steps = delta.y.trunc() as i32;
        Some(Msg::Scroll(steps))
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Up => {
                if m.cur_mut().up() {
                    refresh_preview(&mut m);
                }
            }
            Msg::Down => {
                if m.cur_mut().down() {
                    refresh_preview(&mut m);
                }
            }
            Msg::SelectIn(pane, idx) => {
                m.focus = pane;
                // Doble click (mismo pane+fila, < 400 ms) = abrir: carpeta →
                // desciende al canvas + revela en el árbol; archivo → visor.
                let ahora = Instant::now();
                let doble = m.last_click.take().is_some_and(|(p, i, t)| {
                    p == pane && i == idx && ahora.duration_since(t) < Duration::from_millis(400)
                });
                if m.cur_mut().select(idx) {
                    if doble {
                        do_open_selected(&mut m, handle);
                    } else {
                        m.last_click = Some((pane, idx, ahora));
                        // Click simple en una carpeta (lista/detalle) la
                        // expande/colapsa inline; el doble click la abre.
                        let es_dir =
                            m.cur().selected_node().is_some_and(|n| n.is_container);
                        if es_dir && !m.cur().view.is_grid() {
                            let i = m.cur().selected;
                            let _ = m.cur_mut().toggle_expand(i);
                        }
                        refresh_preview(&mut m);
                    }
                }
            }
            Msg::ToggleDual => {
                m.dual = !m.dual;
                if !m.dual {
                    m.focus = 0; // al volver a simple, el visor vuelve a la derecha
                }
            }
            Msg::SwitchFocus => {
                if m.dual {
                    m.focus = 1 - m.focus;
                    refresh_preview(&mut m);
                }
            }
            Msg::OpenSelected => {
                do_open_selected(&mut m, handle);
            }
            Msg::Parent => {
                // ⌫/Esc pelan por capas: primero la app del canvas, después
                // el panel de preview, recién entonces sube de directorio.
                if m.canvas.is_some() {
                    m.canvas = None;
                    return m;
                }
                if m.viewer_open {
                    m.viewer_open = false;
                    return m;
                }
                m.cur_pane_mut().marked.clear();
                match m.cur_mut().parent() {
                    Ok(true) => {
                        record_history(&mut m);
                        apply_format(&mut m);
                        refresh_preview(&mut m);
                        if m.cur().view.is_grid() {
                            request_thumbs(&mut m, &handle);
                        }
                    }
                    // Subir desde la raíz de una fuente montada la desmonta
                    // (vuelve al nivel de abajo de la pila). En POSIX, la raíz
                    // es `/` y no hay a dónde subir.
                    Ok(false) => {
                        if m.is_foreign() {
                            m.cur_pane_mut().nav_stack.pop();
                            clear_preview(&mut m);
                            record_history(&mut m);
                        }
                    }
                    Err(_) => {}
                }
            }
            Msg::SortByIn(pane, col) => {
                m.focus = pane;
                m.cur_mut().set_sort(col_to_sortkey(col as u8));
                // Recordá el orden elegido para esta carpeta (folder format).
                save_format(&mut m);
            }
            Msg::NavToggleView => {
                // Cicla lista → detalle → iconos → lista.
                let nav = m.cur_mut();
                nav.view = nav.view.next();
                // En vista iconos, pedí miniaturas de lo que entró en pantalla.
                if m.cur().view.is_grid() {
                    request_thumbs(&mut m, &handle);
                }
                // Recordá la vista elegida para esta carpeta (folder format).
                save_format(&mut m);
            }
            Msg::NavFilterStart => {
                m.nav_filtering = true;
            }
            Msg::NavFilterInput(s) => {
                let mut f = m.cur().filter().to_string();
                f.push_str(&s);
                m.cur_mut().set_filter(f);
                refresh_preview(&mut m);
            }
            Msg::NavFilterBackspace => {
                let mut f = m.cur().filter().to_string();
                f.pop();
                m.cur_mut().set_filter(f);
                refresh_preview(&mut m);
            }
            Msg::NavFilterEnd => {
                m.nav_filtering = false;
            }
            Msg::BreadcrumbIn(pane, depth) => {
                m.focus = pane;
                if matches!(m.cur_mut().ascend_to(depth), Ok(true)) {
                    m.cur_pane_mut().marked.clear();
                    m.canvas = None;
                    record_history(&mut m);
                    apply_format(&mut m);
                    refresh_preview(&mut m);
                }
            }
            Msg::ResizeList(dx) => {
                m.list_width = (m.list_width + dx).clamp(220.0, 900.0);
            }
            Msg::ResizeTree(dx) => {
                m.tree_w = (m.tree_w + dx).clamp(170.0, 420.0);
            }
            Msg::ResizePreview(dx) => {
                // El divisor está a la izquierda del visor: moverlo a la
                // derecha achica el panel.
                m.preview_w = (m.preview_w - dx).clamp(280.0, 860.0);
            }
            Msg::SetViewMode(v) => {
                m.cur_mut().view = v;
                if v.is_grid() {
                    request_thumbs(&mut m, &handle);
                }
                save_format(&mut m);
            }
            Msg::ExpandSelected => {
                let i = m.cur().selected;
                let ya = m
                    .cur()
                    .selected_node()
                    .is_some_and(|n| m.cur().is_expanded(&n.id));
                if !ya {
                    let _ = m.cur_mut().toggle_expand(i);
                }
            }
            Msg::CollapseSelected => {
                let i = m.cur().selected;
                let expandida = m
                    .cur()
                    .selected_node()
                    .is_some_and(|n| n.is_container && m.cur().is_expanded(&n.id));
                if expandida {
                    let _ = m.cur_mut().toggle_expand(i);
                } else if let Some(p) = m.cur().parent_of(i) {
                    // Colapsada (o archivo): saltá a la fila padre.
                    m.cur_mut().select(p);
                    refresh_preview(&mut m);
                }
            }
            Msg::NavBack => nav_history_go(&mut m, &handle, -1),
            Msg::NavForward => nav_history_go(&mut m, &handle, 1),
            Msg::TogglePreviewPanel => {
                if m.viewer_open {
                    m.viewer_open = false;
                } else {
                    m.viewer_open = true;
                    refresh_preview(&mut m);
                }
            }
            Msg::CanvasClose => {
                m.canvas = None;
            }
            Msg::CanvasSave => {
                if let Some(CanvasApp::Texto { path, editor, dirty, saved }) = &mut m.canvas {
                    match std::fs::write(&*path, editor.text()) {
                        Ok(()) => {
                            *dirty = false;
                            *saved = true;
                        }
                        Err(e) => eprintln!("[nahual] guardar {}: {e}", path.display()),
                    }
                }
            }
            Msg::CanvasEditKey(ev) => {
                let lines = canvas_editor_lines(&m);
                if let Some(CanvasApp::Texto { editor, dirty, saved, .. }) = &mut m.canvas {
                    let r = editor.apply_key_with_clipboard(&ev, &mut m.clipboard);
                    if r.changed() {
                        *dirty = true;
                        *saved = false;
                    }
                    if r.touched() {
                        editor.ensure_caret_visible(lines);
                    }
                }
            }
            Msg::CanvasEditPointer(ev) => {
                let metrics = EditorMetrics::for_font_size(13.0);
                if let Some(CanvasApp::Texto { editor, .. }) = &mut m.canvas {
                    let scroll = editor.scroll_offset;
                    match ev {
                        PointerEvent::Click { x, y } => {
                            m.canvas_drag = (0.0, 0.0);
                            let (line, col) = metrics.screen_to_pos(x, y, scroll);
                            editor.set_caret_at(line, col);
                        }
                        PointerEvent::Drag { initial_x, initial_y, dx, dy } => {
                            m.canvas_drag.0 += dx;
                            m.canvas_drag.1 += dy;
                            let cx = initial_x + m.canvas_drag.0;
                            let cy = initial_y + m.canvas_drag.1;
                            let (line, col) = metrics.screen_to_pos(cx, cy, scroll);
                            editor.extend_selection_to(line, col);
                        }
                    }
                }
            }
            Msg::CanvasImgZoom(factor) => {
                if let Some(CanvasApp::Imagen { viewport, .. }) = &mut m.canvas {
                    viewport.zoom_by(factor);
                }
            }
            Msg::CanvasImgPan(dx, dy) => {
                if let Some(CanvasApp::Imagen { viewport, .. }) = &mut m.canvas {
                    viewport.pan_by(dx, dy);
                }
            }
            Msg::CanvasImgReset => {
                if let Some(CanvasApp::Imagen { viewport, .. }) = &mut m.canvas {
                    viewport.reset();
                }
            }
            Msg::Scroll(steps) => {
                // Con una app de canvas abierta, la rueda es suya (el editor
                // scrollea; imagen/media la ignoran — el zoom va por
                // Ctrl+rueda).
                if let Some(canvas) = &mut m.canvas {
                    if let CanvasApp::Texto { editor, .. } = canvas {
                        editor.scroll_by(steps);
                    }
                    return m;
                }
                if m.cur().view.is_grid() {
                    // En grilla la unidad de scroll es la FILA entera (cols
                    // items), no el item — si no, las celdas se van "halando"
                    // de a una y bailan de columna. El offset queda alineado
                    // a múltiplo de cols.
                    let cols = grid_cols(&m).max(1);
                    let nav = m.cur_mut();
                    nav.apply_wheel(steps as f32 * cols as f32);
                    nav.visible_offset -= nav.visible_offset % cols;
                    // Lo que entró en pantalla al scrollear pide su miniatura.
                    request_thumbs(&mut m, &handle);
                } else {
                    // El navegador activo tiene su propio acumulador para
                    // touchpads — le pasamos el delta crudo (en líneas).
                    m.cur_mut().apply_wheel(steps as f32);
                }
            }
            Msg::MapPan(dx, dy) => {
                m.map_view.pan_by(dx as f64, dy as f64);
                m.basemap_dirty = true;
            }
            Msg::MapZoom(dy, cx, cy) => {
                // Cada "línea" de rueda → ±12% de zoom, anclado al cursor.
                m.map_view.zoom_at(1.12_f64.powf(dy as f64), cx, cy);
                m.basemap_dirty = true;
            }
            Msg::MapReset => {
                m.map_view.reset();
                m.basemap_dirty = true;
            }
            Msg::MapToggleBase => m.map_view.toggle_base(),
            Msg::MapClick(fx, fy) => {
                if let PreviewPane::Map(MapPreview::Map { data, .. }) = &m.preview {
                    if m.map_view.routing {
                        // Ruteo: cada clic fija un punto; con dos, calcula la ruta.
                        if let Some(c) =
                            nahual_map_viewer_llimphi::unproject(data, &m.map_view, fx as f64, fy as f64)
                        {
                            if m.map_view.route_pins.len() >= 2 {
                                m.map_view.clear_route();
                            }
                            m.map_view.route_pins.push(c);
                            if m.map_view.route_pins.len() == 2 {
                                let (a, b) = (m.map_view.route_pins[0], m.map_view.route_pins[1]);
                                match nahual_map_viewer_llimphi::route(data, a, b) {
                                    Some(res) => {
                                        m.map_view.route_path = res.path;
                                        m.map_view.route_meters = res.meters;
                                    }
                                    None => {
                                        m.map_view.route_path.clear();
                                        m.map_view.route_meters = 0.0;
                                    }
                                }
                            }
                        }
                    } else {
                        m.map_view.selected = nahual_map_viewer_llimphi::hit_test(
                            data,
                            &m.map_view,
                            fx as f64,
                            fy as f64,
                        );
                    }
                }
            }
            Msg::MapRouteToggle => {
                m.map_view.routing = !m.map_view.routing;
                m.map_view.clear_route();
            }
            Msg::MapCycleColor => {
                if let PreviewPane::Map(MapPreview::Map { data, .. }) = &m.preview {
                    let fields = nahual_map_viewer_llimphi::numeric_fields(data);
                    m.map_view.color_field = next_in_cycle(&fields, &m.map_view.color_field);
                }
            }
            Msg::MapSearchStart => {
                m.map_view.searching = true;
                m.map_view.query.clear();
            }
            Msg::MapSearchInput(s) => {
                if m.map_view.searching {
                    m.map_view.query.push_str(&s);
                }
            }
            Msg::MapSearchBackspace => {
                m.map_view.query.pop();
            }
            Msg::MapSearchCancel => {
                m.map_view.searching = false;
                m.map_view.query.clear();
            }
            Msg::MapSearchSubmit => {
                if let PreviewPane::Map(MapPreview::Map { data, .. }) = &m.preview {
                    let hits = nahual_map_viewer_llimphi::search(data, &m.map_view.query, 1);
                    if let Some(&fi) = hits.first() {
                        nahual_map_viewer_llimphi::focus_on(data, &mut m.map_view, fi);
                    }
                }
                m.map_view.searching = false;
                m.basemap_dirty = true;
            }
            Msg::WawaConfigChanged(cfg) => {
                m.theme = theme_from_wawa(&cfg, &m.theme);
                // nahual-shell no usa rimay_localize hoy; si en el
                // futuro lo hace, agregar el set_locale acá.
            }
            Msg::Tick => {
                match &mut m.preview {
                    PreviewPane::Video(state) => {
                        state.tick(FRAME_TICK);
                    }
                    PreviewPane::Audio(state) => state.tick(FRAME_TICK),
                    _ => {}
                }
                // El player del canvas también corre con el reloj.
                match &mut m.canvas {
                    Some(CanvasApp::Video(state)) => {
                        state.tick(FRAME_TICK);
                    }
                    Some(CanvasApp::Audio(state)) => state.tick(FRAME_TICK),
                    _ => {}
                }
                // Debounce del streaming del basemap: coalesce los pans/zooms
                // y re-streamea a lo sumo cada `RESTREAM_THROTTLE`.
                if m.basemap_dirty && m.basemap.is_some() {
                    let now = Instant::now();
                    let ready = m
                        .last_restream
                        .map_or(true, |t| now.duration_since(t) >= RESTREAM_THROTTLE);
                    if ready && restream_basemap(&mut m) {
                        m.last_restream = Some(now);
                        m.basemap_dirty = false;
                    }
                }
            }
            Msg::TogglePlay => match &mut m.canvas {
                // El player del canvas tiene prioridad sobre el preview.
                Some(CanvasApp::Video(state)) => state.toggle_play(),
                Some(CanvasApp::Audio(state)) => state.toggle_play(),
                _ => match &mut m.preview {
                    PreviewPane::Video(state) => state.toggle_play(),
                    PreviewPane::Audio(state) => state.toggle_play(),
                    _ => {}
                },
            },
            Msg::MountNouser => {
                // Sólo montamos desde POSIX (no anidamos fuentes). nouser sólo
                // LEE el dir, así que no hay riesgo de efecto secundario.
                if !m.is_foreign() {
                    let dir = target_dir(&m);
                    if let Some(nav) = NouserSource::escanear(&dir, 1)
                        .ok()
                        .and_then(|src| Navigator::open(Box::new(src)).ok())
                    {
                        m.cur_pane_mut().nav_stack.push(nav);
                        clear_preview(&mut m);
                    }
                }
            }
            Msg::MountMinga => {
                // Guard: `PersistentRepo::open` (sled) CREA archivos si el dir
                // no es un repo — sólo montamos si ya parece uno, para no
                // ensuciar directorios ajenos.
                if !m.is_foreign() {
                    let dir = target_dir(&m);
                    if parece_repo_minga(&dir) {
                        if let Some(nav) = MingaSource::abrir(&dir)
                            .ok()
                            .and_then(|src| Navigator::open(Box::new(src)).ok())
                        {
                            m.cur_pane_mut().nav_stack.push(nav);
                            clear_preview(&mut m);
                        }
                    }
                }
            }
            Msg::Unmount => {
                if m.is_foreign() {
                    m.cur_pane_mut().nav_stack.pop();
                    clear_preview(&mut m);
                }
            }
            Msg::CycleTheme => {
                m.theme = Theme::next_after(m.theme.name);
            }
            Msg::MenuOpen(which) => {
                m.menu_open = which;
                // Abrir un menú raíz cierra cualquier contextual.
                m.context_menu = None;
                m.menu_active = usize::MAX;
                if which.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
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
                        m.menu_open = None;
                        return handle_menu_command(m, &cmd, handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenus => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
                m.context_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
                return handle_menu_command(m, &cmd, handle);
            }
            Msg::ContextMenuOpen(x, y) => {
                // Sólo si hay algo seleccionado (POSIX o fuente montada).
                if hay_seleccion(&m) {
                    m.menu_open = None;
                    // Precomputa las opciones "Abrir con…" del archivo
                    // seleccionado (discernir → handlers_for) para que el
                    // render no toque el registro ni el disco.
                    compute_open_with(&mut m);
                    m.context_menu = Some((x, y));
                }
            }
            Msg::OpenWith(id) => {
                if let (Some(app), Some(target)) =
                    (m.registry.get(&id), m.ctx_target.as_ref().and_then(|p| p.to_str()))
                {
                    if let Err(e) = app.open(target) {
                        eprintln!("[nahual] abrir con {id}: {e}");
                    }
                }
                m.context_menu = None;
            }
            Msg::EditSelected => {
                if let Some(target) = m.ctx_target.as_ref().and_then(|p| p.to_str()) {
                    let bin = std::env::var("NADA_BIN").unwrap_or_else(|_| "nada".into());
                    if let Err(e) = std::process::Command::new(bin).arg(target).spawn() {
                        eprintln!("[nahual] editar en nada: {e}");
                    }
                }
                m.context_menu = None;
            }
            Msg::TerminalHere => {
                // El dir POSIX base (la fuente del fondo de la pila), aunque
                // haya una fuente montada encima.
                let dir = PathBuf::from(m.panes[m.focus].nav_stack[0].current_id());
                let bin = std::env::var("SHUMA_BIN").unwrap_or_else(|_| "shuma-shell-llimphi".into());
                if let Err(e) = std::process::Command::new(bin).current_dir(&dir).spawn() {
                    eprintln!("[nahual] terminal shuma: {e}");
                }
                m.context_menu = None;
            }

            // ---- Fase 4.3: operaciones de archivo + cola ----
            Msg::ToggleMark => {
                if let Some(n) = m.cur().selected_node() {
                    let id = n.id.clone();
                    let pane = m.cur_pane_mut();
                    // `insert` devuelve `false` si ya estaba → entonces se quita.
                    if !pane.marked.insert(id.clone()) {
                        pane.marked.remove(&id);
                    }
                }
                m.cur_mut().down();
                refresh_preview(&mut m);
            }
            Msg::NewDirPrompt => {
                if m.can_edit() {
                    let parent = m.cur().current_id().clone();
                    m.prompt = Some(Prompt { kind: PromptKind::NewDir { parent }, text: String::new() });
                    m.context_menu = None;
                }
            }
            Msg::NewFilePrompt => {
                if m.can_edit() {
                    let parent = m.cur().current_id().clone();
                    m.prompt = Some(Prompt { kind: PromptKind::NewFile { parent }, text: String::new() });
                    m.context_menu = None;
                }
            }
            Msg::RenamePrompt => {
                if m.can_edit() {
                    // Con marca múltiple, "Renombrar" abre el batch; si no, el
                    // renombrado simple del nodo bajo el cursor.
                    if !m.cur_pane().marked.is_empty() {
                        return Shell::update(m, Msg::BatchRenameStart, handle);
                    }
                    if let Some(n) = m.cur().selected_node() {
                        let (id, name) = (n.id.clone(), n.name.clone());
                        m.prompt = Some(Prompt { kind: PromptKind::Rename { id }, text: name });
                        m.context_menu = None;
                    }
                }
            }
            Msg::PromptInput(s) => {
                if let Some(p) = m.prompt.as_mut() {
                    p.text.push_str(&s);
                }
            }
            Msg::PromptBackspace => {
                if let Some(p) = m.prompt.as_mut() {
                    p.text.pop();
                }
            }
            Msg::PromptSubmit => {
                if let Some(p) = m.prompt.take() {
                    let name = p.text.trim().to_string();
                    if !name.is_empty() {
                        let kind = match p.kind {
                            PromptKind::NewDir { parent } => OpKind::NewDir { parent, name },
                            PromptKind::NewFile { parent } => OpKind::NewFile { parent, name },
                            PromptKind::Rename { id } => OpKind::Rename { id, new_name: name },
                        };
                        enqueue(&mut m, handle, kind);
                    }
                }
            }
            Msg::PromptCancel => {
                m.prompt = None;
            }
            Msg::DeleteSelection => {
                let targets = m.cur_pane().op_targets();
                if !targets.is_empty() {
                    m.confirm_delete = Some(targets);
                    m.context_menu = None;
                }
            }
            Msg::ConfirmDelete => {
                if let Some(targets) = m.confirm_delete.take() {
                    for (id, name) in targets {
                        enqueue(&mut m, handle, OpKind::Delete { id, name });
                    }
                    m.cur_pane_mut().marked.clear();
                }
            }
            Msg::CancelConfirm => {
                m.confirm_delete = None;
            }
            Msg::CopyToOther => copy_or_move(&mut m, handle, false),
            Msg::MoveToOther => copy_or_move(&mut m, handle, true),
            Msg::RunOp(kind) => {
                m.context_menu = None;
                enqueue(&mut m, handle, kind);
            }
            Msg::OpFinished { id, result } => {
                let status = match &result {
                    Ok(r) => OpStatus::Done(r.clone()),
                    Err(e) => OpStatus::Failed(e.clone()),
                };
                m.queue.finish(id, status);
                reload_panes(&mut m);
                // Dejá el cursor sobre el resultado (carpeta/archivo nuevo,
                // renombrado) en el panel enfocado.
                if let Ok(Some(new_id)) = &result {
                    m.cur_pane_mut().nav_mut().select_id(new_id);
                }
                refresh_preview(&mut m);
            }
            Msg::ToggleQueue => {
                m.queue.open = !m.queue.open;
            }
            Msg::ClearQueue => {
                m.queue.clear_finished();
            }

            // ---- Fase 4.5: renombrado por lote ----
            Msg::BatchRenameStart => {
                if m.can_edit() {
                    // Objetivos: la marca, o el cursor si no hay marca.
                    let targets = m.cur_pane().op_targets();
                    if !targets.is_empty() {
                        m.batch = Some(BatchRename { pattern: "{name}".to_string(), targets });
                        m.context_menu = None;
                    }
                }
            }
            Msg::BatchPatternInput(s) => {
                if let Some(b) = m.batch.as_mut() {
                    b.pattern.push_str(&s);
                }
            }
            Msg::BatchPatternBackspace => {
                if let Some(b) = m.batch.as_mut() {
                    b.pattern.pop();
                }
            }
            Msg::BatchApply => {
                if let Some(b) = m.batch.take() {
                    for idx in 0..b.targets.len() {
                        let nuevo = b.nuevo_nombre(idx);
                        let (id, original) = &b.targets[idx];
                        // Sólo encolá los que efectivamente cambian de nombre.
                        if &nuevo != original {
                            enqueue(
                                &mut m,
                                handle,
                                OpKind::Rename { id: id.clone(), new_name: nuevo },
                            );
                        }
                    }
                    m.cur_pane_mut().marked.clear();
                }
            }
            Msg::BatchCancel => {
                m.batch = None;
            }
            Msg::SetLabel(label) => {
                for (id, _) in m.cur_pane().op_targets() {
                    m.state.set_label(&id, label);
                }
                m.state.save();
                m.context_menu = None;
            }
            Msg::ClearLabel => {
                for (id, _) in m.cur_pane().op_targets() {
                    m.state.clear_label(&id);
                }
                m.state.save();
                m.context_menu = None;
            }
            Msg::AddPlace => {
                // La carpeta seleccionada si es un dir; si no, la carpeta actual.
                let target = match m.cur().selected_node() {
                    Some(n) if n.is_container => n.id.clone(),
                    _ => m.cur().current_id().clone(),
                };
                if !m.is_foreign() {
                    m.state.add_place(&target);
                    m.state.save();
                }
                m.context_menu = None;
            }
            Msg::SessionNew => {
                // Guarda la sesión viva, abre una sesión (diente) nueva en el
                // cwd actual y la activa.
                let cwd = cur_dir(&m);
                let snap = m.snapshot_active();
                m.sessions[m.active].snap = Some(snap);
                m.sessions.push(Session {
                    name: session_name(&cwd),
                    snap: Some(fresh_snap(&cwd)),
                });
                let nuevo = m.sessions.len() - 1;
                m.active = nuevo;
                if let Some(snap) = m.sessions[nuevo].snap.take() {
                    m.restore(snap);
                }
            }
            Msg::SessionActivate(i) => {
                m.switch_to(i);
            }
            Msg::TreeToggle(path) => {
                if m.tree_expanded.contains(&path) {
                    m.tree_expanded.remove(&path);
                } else {
                    ensure_tree_children(&mut m.tree_children, &path);
                    m.tree_expanded.insert(path);
                }
            }
            Msg::TreeSelect(path) => {
                if path.is_dir() {
                    ensure_tree_children(&mut m.tree_children, &path);
                    m.tree_expanded.insert(path.clone());
                    m.cur_pane_mut().nav_stack = vec![posix_nav(&path)];
                    m.cur_pane_mut().marked.clear();
                    // Seleccionar una carpeta abre su vista en el canvas.
                    m.canvas = None;
                    record_history(&mut m);
                    apply_format(&mut m);
                    record_recent(&mut m);
                    refresh_preview(&mut m);
                    // Si la carpeta hereda vista iconos/galería, pedí thumbs.
                    if m.cur().view.is_grid() {
                        request_thumbs(&mut m, &handle);
                    }
                    // Mantené el nombre de la sesión en sync con la carpeta.
                    let nombre = session_name(&path);
                    let activa = m.active;
                    m.sessions[activa].name = nombre;
                }
            }
            Msg::TreeScroll(dy) => {
                // Rueda hacia abajo baja el árbol; ~3 filas por muesca.
                let total = count_tree_rows(&m);
                let max = total.saturating_sub(tree_visible_rows(&m));
                let delta = (dy * 3.0).round() as i32;
                let nuevo = (m.tree_scroll as i32 + delta).clamp(0, max as i32);
                m.tree_scroll = nuevo as usize;
            }
            Msg::ThumbReady(path, thumb) => {
                m.thumbs_pending.remove(&path);
                let img = Image::new(ImageData {
                    data: Blob::from(thumb.rgba),
                    format: ImageFormat::Rgba8,
                    alpha_type: ImageAlphaType::Alpha,
                    width: thumb.w,
                    height: thumb.h,
                });
                m.thumbs.insert(path, img);
            }
            Msg::ThumbFailed(path) => {
                m.thumbs_pending.remove(&path);
                m.thumbs_failed.insert(path);
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = model.theme;
        let splitter_palette = SplitterPalette::from_theme(&theme);
        let text_palette = TextViewerPalette::from_theme(&theme);
        let image_palette = ImageViewerPalette::from_theme(&theme);
        let video_palette = VideoViewerPalette::from_theme(&theme);
        let audio_palette = AudioViewerPalette::from_theme(&theme);
        let card_palette = CardViewerPalette::from_theme(&theme);
        let tree_palette = TreeViewerPalette::from_theme(&theme);
        let hex_palette = HexViewerPalette::from_theme(&theme);
        let table_palette = TableViewerPalette::from_theme(&theme);
        let markdown_palette = MarkdownViewerPalette::from_theme(&theme);
        let archive_palette = ArchiveViewerPalette::from_theme(&theme);
        let font_palette = FontViewerPalette::from_theme(&theme);
        let map_palette = MapViewerPalette::from_theme(&theme);
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let viewer_pane = match &model.preview {
            PreviewPane::Empty => text_viewer_view::<Msg>(
                &PreviewState::Empty,
                None,
                &text_palette,
            ),
            PreviewPane::Text(state) => text_viewer_view::<Msg>(
                state,
                model.preview_of.as_deref(),
                &text_palette,
            ),
            PreviewPane::Image(state) => image_viewer_view::<Msg>(
                state,
                model.preview_of.as_deref(),
                &image_palette,
            ),
            PreviewPane::Video(state) => video_viewer_view::<Msg>(state, &video_palette),
            PreviewPane::Audio(state) => audio_viewer_view::<Msg>(state, &audio_palette),
            PreviewPane::Card(state) => {
                card_viewer_view::<Msg>(state, model.preview_of.as_deref(), &card_palette)
            }
            PreviewPane::Tree(state) => {
                tree_viewer_view::<Msg>(state, model.preview_of.as_deref(), &tree_palette)
            }
            PreviewPane::Hex(state) => {
                hex_viewer_view::<Msg>(state, model.preview_of.as_deref(), &hex_palette)
            }
            PreviewPane::Table(state) => {
                table_viewer_view::<Msg>(state, model.preview_of.as_deref(), &table_palette)
            }
            PreviewPane::Markdown(state) => {
                markdown_viewer_view::<Msg>(state, model.preview_of.as_deref(), &markdown_palette)
            }
            PreviewPane::Archive(state) => {
                archive_viewer_view::<Msg>(state, model.preview_of.as_deref(), &archive_palette)
            }
            PreviewPane::Font(state) => {
                font_viewer_view::<Msg>(state, model.preview_of.as_deref(), &font_palette)
            }
            PreviewPane::Map(state) => {
                map_viewer_view::<Msg, _>(
                    state,
                    model.preview_of.as_deref(),
                    &map_palette,
                    &model.map_view,
                    // Clic → fracción del rect (el update resuelve con hit_test).
                    |lx, ly, w, h| {
                        (w > 0.0 && h > 0.0).then(|| Msg::MapClick(lx / w, ly / h))
                    },
                )
                // Arrastrar el panel panea la cámara del mapa.
                .draggable(|phase, dx, dy| match phase {
                    DragPhase::Move => Some(Msg::MapPan(dx, dy)),
                    DragPhase::End => None,
                })
            }
            // El visor de texto muestra el fuente HTML; abrir (Enter) lanza puriy.
            PreviewPane::Web(state) => text_viewer_view::<Msg>(
                state,
                model.preview_of.as_deref(),
                &text_palette,
            ),
        };

        // El CANVAS es la vista de la carpeta (lista/detalle/iconos/galería a
        // ancho completo); en dual, dos columnas de archivos. El visor del
        // archivo abierto vive en un **sidebar derecho** resizable (Esc/⌫ lo
        // cierra) — nunca tapa la vista de carpeta.
        let folder_view = if model.dual {
            splitter_two(
                Direction::Row,
                pane_column(model, 0, model.focus == 0, &theme),
                PaneSize::Fixed(model.list_width),
                pane_column(model, 1, model.focus == 1, &theme),
                PaneSize::Flex,
                |phase, dx| match phase {
                    DragPhase::Move => Some(Msg::ResizeList(dx)),
                    DragPhase::End => None,
                },
                &splitter_palette,
            )
        } else {
            pane_column(model, model.focus, true, &theme)
        };
        // Centro: la app integrada del canvas si hay una abierta (editor /
        // imagen / media); si no, la vista de carpeta.
        let centro = match &model.canvas {
            Some(canvas) => canvas_app_view(canvas, model, &theme),
            None => folder_view,
        };
        let canvas_core = if model.viewer_open {
            splitter_two(
                Direction::Row,
                centro,
                PaneSize::Flex,
                viewer_pane,
                PaneSize::Fixed(model.preview_w),
                |phase, dx| match phase {
                    DragPhase::Move => Some(Msg::ResizePreview(dx)),
                    DragPhase::End => None,
                },
                &splitter_palette,
            )
        } else {
            centro
        };
        // Canal interno: el contenido arranca después del ancho del rail para
        // que los dientes (overlay) no tapen las primeras columnas.
        let canvas_padded = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            min_size: Size { width: length(0.0), height: length(0.0) },
            padding: Rect {
                left: length(SESSION_RAIL_W),
                right: length(0.0),
                top: length(0.0),
                bottom: length(0.0),
            },
            ..Default::default()
        })
        .children(vec![canvas_core]);
        // Dientes de sesión: overlay absoluto pegado al borde interno del
        // canvas, sobresaliendo del sidebar (patrón canónico de cosmos).
        let canvas_area = View::new(Style {
            flex_grow: 1.0,
            min_size: Size { width: length(0.0), height: length(0.0) },
            size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![
            canvas_padded,
            session_teeth_overlay(model, &theme),
            preview_tooth_overlay(model, &theme),
        ]);

        // Sidebar único (árbol de carpetas) | canvas, con splitter.
        let body = splitter_two(
            Direction::Row,
            sidebar_view(model, &theme),
            PaneSize::Fixed(model.tree_w),
            canvas_area,
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeTree(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        );
        let body_wrap = View::new(Style {
            flex_grow: 1.0,
            min_size: Size { width: length(0.0), height: length(0.0) },
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![body]);

        let mut col: Vec<View<Msg>> = vec![menubar, shell_toolbar(model, &theme), body_wrap];
        if let Some(panel) = queue_panel(model, &theme) {
            col.push(panel);
        }

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        // Right-click en la raíz (origen 0,0 ⇒ local == coords de ventana)
        // abre el menú contextual sobre la entrada seleccionada.
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(col)
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // Los modales de operación (prompt de nombre, confirmación de borrado)
        // van por encima de todo.
        if let Some(p) = &model.prompt {
            return Some(prompt_overlay(p, &model.theme));
        }
        if let Some(targets) = &model.confirm_delete {
            return Some(confirm_overlay(targets, &model.theme));
        }
        if let Some(b) = &model.batch {
            return Some(batch_overlay(b, &model.theme));
        }
        // El menú contextual del nodo seleccionado tiene prioridad.
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu_view(context_menu_spec(model, x, y)));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &model.theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

/// Viewport para clampear overlays. El shell no trackea el tamaño de
/// ventana, así que usamos `initial_size()` (constante).
fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = Shell::initial_size();
    (w as f32, h as f32)
}

/// ¿Hay una entrada seleccionada sobre la que tenga sentido el menú
/// contextual? En POSIX, cualquier entry del explorer; en una fuente
/// montada, el nodo seleccionado.
fn hay_seleccion(m: &Model) -> bool {
    m.cur().selected_node().is_some()
}

/// Discierne el **mime** del contenido de `path` con el pipeline real de shuma
/// (los mismos primeros KB que usa `load_for`). `None` si no se puede leer o
/// shuma no le asigna mime.
fn discern_mime(path: &Path) -> Option<String> {
    let sample = read_header_sample(path, DISCERN_SAMPLE_BYTES)?;
    let pipeline = shuma_discern::DiscernPipeline::default();
    let hint = shuma_discern::Hint {
        path: path.to_str(),
        size_total: std::fs::metadata(path).ok().map(|m| m.len()),
    };
    pipeline.discern(&sample, &hint)?.mime
}

/// Precomputa las opciones de open-with del archivo seleccionado: resuelve el
/// target (ruta POSIX real, o tempfile de una hoja no-POSIX preservando el
/// nombre/extensión), discierne su mime y consulta el `AppRegistry`. Llena
/// `ctx_target`/`ctx_temp`/`ctx_open_with`. Si la selección no es un archivo
/// abrible, deja todo vacío (el contextual sólo muestra navegación/montaje).
fn compute_open_with(m: &mut Model) {
    m.ctx_open_with.clear();
    m.ctx_target = None;
    m.ctx_temp = None;

    let nav = m.cur();
    let (path, temp): (Option<PathBuf>, Option<tempfile::TempDir>) = match nav.selected_node() {
        Some(n) if !n.is_container => {
            let id_path = Path::new(&n.id);
            if id_path.is_file() {
                // Hoja POSIX: su id ES la ruta real.
                (Some(id_path.to_path_buf()), None)
            } else {
                // Hoja no-POSIX (wawa/nouser/minga): materializarla a un
                // tempfile con su nombre (preserva extensión para discernir).
                match nav.read(&n.id) {
                    Ok(bytes) => match tempfile::tempdir() {
                        Ok(dir) => {
                            let p = dir.path().join(&n.name);
                            if std::fs::write(&p, &bytes).is_ok() {
                                (Some(p), Some(dir))
                            } else {
                                (None, None)
                            }
                        }
                        Err(_) => (None, None),
                    },
                    Err(_) => (None, None),
                }
            }
        }
        _ => (None, None),
    };

    let Some(path) = path else {
        return;
    };
    if let Some(mime) = discern_mime(&path) {
        for app in m.registry.handlers_for(&mime) {
            m.ctx_open_with.push((app.id.clone(), app.label.clone()));
        }
    }
    m.ctx_target = Some(path);
    m.ctx_temp = temp;
}

/// Etiqueta de la entrada seleccionada para el header del contextual.
fn etiqueta_seleccion(m: &Model) -> String {
    m.cur()
        .selected_node()
        .map(|n| n.name.clone())
        .unwrap_or_else(|| "entrada".to_string())
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(menu: &'a AppMenu, model: &Model, theme: &'a Theme) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// El menú principal del shell. Sólo comandos que mapean a `Msg` reales:
/// navegación (abrir/subir), montaje de fuentes no-POSIX (nouser/minga),
/// desmontar, tema. Sin "Editar": el shell no tiene campos de texto
/// editables — el panel derecho son visores de sólo lectura.
fn app_menu(model: &Model) -> AppMenu {
    let montado = model.is_foreign();
    // Montar sólo aplica desde POSIX (no anidamos fuentes); desmontar sólo
    // cuando hay una fuente activa. Reflejamos eso en gris.
    let mut mount_nouser = MenuItem::new("Montar Mónadas (nouser)", "file.mount_nouser")
        .shortcut("m")
        .separated();
    let mut mount_minga = MenuItem::new("Montar grafo minga", "file.mount_minga").shortcut("g");
    let mut unmount = MenuItem::new("Desmontar fuente", "file.unmount").separated();
    if montado {
        mount_nouser = mount_nouser.disabled();
        mount_minga = mount_minga.disabled();
    } else {
        unmount = unmount.disabled();
    }
    // Operaciones de archivo (Fase 4.3): sólo sobre POSIX escribible. Sobre una
    // fuente montada read-only salen en gris.
    let editable = model.can_edit();
    let mut newdir = MenuItem::new("Nueva carpeta", "file.newdir").shortcut("F7").separated();
    let mut newfile = MenuItem::new("Nuevo archivo", "file.newfile");
    let mut rename = MenuItem::new("Renombrar", "file.rename").shortcut("F2");
    let mut delete = MenuItem::new("Borrar", "file.delete").shortcut("Supr");
    if !editable {
        newdir = newdir.disabled();
        newfile = newfile.disabled();
        rename = rename.disabled();
        delete = delete.disabled();
    }
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Abrir", "file.open").shortcut("Enter"))
                .item(MenuItem::new("Subir al padre", "file.parent").shortcut("Backspace"))
                .item(newdir)
                .item(newfile)
                .item(rename)
                .item(delete)
                .item(mount_nouser)
                .item(mount_minga)
                .item(unmount)
                .item(MenuItem::new("Salir", "file.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(etiqueta_menu(editable))
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// El menú "Etiqueta": los siete colores + "Sin etiqueta". Aplica a la marca
/// múltiple o, si no hay, al nodo bajo el cursor. Gris si la fuente no es POSIX.
fn etiqueta_menu(editable: bool) -> Menu {
    let mut menu = Menu::new("Etiqueta");
    for label in Label::ALL {
        // Un punto del color como prefijo del nombre (el menubar pinta texto).
        let mut it = MenuItem::new(format!("● {}", label.name()), label_cmd(label));
        if !editable {
            it = it.disabled();
        }
        menu = menu.item(it);
    }
    let mut sin = MenuItem::new("Sin etiqueta", "label.none").separated();
    if !editable {
        sin = sin.disabled();
    }
    menu.item(sin)
}

/// El command id del menú para cada color.
fn label_cmd(label: Label) -> &'static str {
    match label {
        Label::Red => "label.red",
        Label::Orange => "label.orange",
        Label::Yellow => "label.yellow",
        Label::Green => "label.green",
        Label::Blue => "label.blue",
        Label::Purple => "label.purple",
        Label::Gray => "label.gray",
    }
}

/// Inversa de [`label_cmd`]: el `Label` (o `None` para "Sin etiqueta") que un
/// command id de etiqueta denota.
fn label_from_cmd(cmd: &str) -> Option<Option<Label>> {
    match cmd {
        "label.red" => Some(Some(Label::Red)),
        "label.orange" => Some(Some(Label::Orange)),
        "label.yellow" => Some(Some(Label::Yellow)),
        "label.green" => Some(Some(Label::Green)),
        "label.blue" => Some(Some(Label::Blue)),
        "label.purple" => Some(Some(Label::Purple)),
        "label.gray" => Some(Some(Label::Gray)),
        "label.none" => Some(None),
        _ => None,
    }
}

/// Traduce un command id del menú principal al `Msg`/efecto real.
fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    match cmd {
        "file.open" => handle.dispatch(Msg::OpenSelected),
        "file.parent" => handle.dispatch(Msg::Parent),
        "file.newdir" => handle.dispatch(Msg::NewDirPrompt),
        "file.newfile" => handle.dispatch(Msg::NewFilePrompt),
        "file.rename" => handle.dispatch(Msg::RenamePrompt),
        "file.delete" => handle.dispatch(Msg::DeleteSelection),
        "file.mount_nouser" => handle.dispatch(Msg::MountNouser),
        "file.mount_minga" => handle.dispatch(Msg::MountMinga),
        "file.unmount" => handle.dispatch(Msg::Unmount),
        "file.quit" => std::process::exit(0),
        "view.theme" => handle.dispatch(Msg::CycleTheme),
        // Etiquetas: cada color (o "Sin etiqueta") despacha su Msg.
        _ if label_from_cmd(cmd).is_some() => match label_from_cmd(cmd).unwrap() {
            Some(label) => handle.dispatch(Msg::SetLabel(label)),
            None => handle.dispatch(Msg::ClearLabel),
        },
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => {}
    }
    model
}

/// Arma el `ContextMenuSpec` del menú contextual sobre la entrada
/// seleccionada. Las acciones son las navegaciones/montajes que ya existen
/// como `Msg` — no inventamos edición (no hay campos de texto).
fn context_menu_spec(model: &Model, x: f32, y: f32) -> ContextMenuSpec<Msg> {
    let montado = model.is_foreign();
    // Construimos la lista de (item, msg) según el contexto, para que el
    // índice del `on_pick` y el item visible siempre coincidan.
    let mut acciones: Vec<(ContextMenuItem, Msg)> = vec![
        (ContextMenuItem::action("Abrir"), Msg::OpenSelected),
        (ContextMenuItem::action("Subir al padre"), Msg::Parent),
    ];
    // Operaciones de archivo (Fase 4.3): sólo sobre POSIX escribible.
    if model.can_edit() {
        acciones.push((ContextMenuItem::action("Nueva carpeta"), Msg::NewDirPrompt));
        acciones.push((ContextMenuItem::action("Nuevo archivo"), Msg::NewFilePrompt));
        if model.cur().selected_node().is_some() {
            acciones.push((ContextMenuItem::action("Renombrar"), Msg::RenamePrompt));
            acciones.push((ContextMenuItem::action("Borrar"), Msg::DeleteSelection));
        }
        if !model.cur_pane().marked.is_empty() {
            acciones.push((
                ContextMenuItem::action("Renombrar por lote…"),
                Msg::BatchRenameStart,
            ));
        }
        acciones.push((ContextMenuItem::action("★ Añadir a favoritos"), Msg::AddPlace));
        if model.dual {
            acciones.push((ContextMenuItem::action("Copiar al otro panel"), Msg::CopyToOther));
            acciones.push((ContextMenuItem::action("Mover al otro panel"), Msg::MoveToOther));
        }
    }
    if montado {
        acciones.push((ContextMenuItem::action("Desmontar fuente"), Msg::Unmount));
    } else {
        acciones.push((
            ContextMenuItem::action("Montar Mónadas (nouser)"),
            Msg::MountNouser,
        ));
        acciones.push((
            ContextMenuItem::action("Montar grafo minga"),
            Msg::MountMinga,
        ));
    }
    // Open-with (AppBus): si la selección es un archivo, ofrecé abrirlo con
    // cada app de la suite que declara su mime, más "editar" y "terminal".
    if model.ctx_target.is_some() {
        for (id, label) in &model.ctx_open_with {
            acciones.push((
                ContextMenuItem::action(format!("Abrir con {label}")),
                Msg::OpenWith(id.clone()),
            ));
        }
        acciones.push((ContextMenuItem::action("Editar en Nada"), Msg::EditSelected));
        acciones.push((
            ContextMenuItem::action("Abrir terminal aquí"),
            Msg::TerminalHere,
        ));
    }
    let msgs: Vec<Msg> = acciones.iter().map(|(_, m)| m.clone()).collect();
    let items: Vec<ContextMenuItem> = acciones.into_iter().map(|(it, _)| it).collect();
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> =
        Arc::new(move |i: usize| msgs.get(i).cloned().unwrap_or(Msg::CloseMenus));
    ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some(etiqueta_seleccion(model)),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    }
}

/// Rect de padding uniforme — atajo para los modales/panel de la cola.
fn pad(v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(v), bottom: length(v) }
}

/// Rect de padding sólo horizontal (top/bottom 0).
fn pad_h(v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
}

/// Una fila de alto fijo, ancho total, contenido centrado verticalmente.
fn fila(h: f32) -> Style {
    Style {
        size: Size { width: percent(1.0_f32), height: length(h) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    }
}

/// Envuelve una `card` en un scrim full-screen centrado; un click fuera
/// dispatcha `dismiss`.
fn modal_scrim(card: View<Msg>, dismiss: Msg) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 130))
    .on_click(dismiss)
    .children(vec![card])
}

/// Overlay del prompt de nombre (nueva carpeta/archivo, renombrar): card
/// centrada con el título, el texto en edición y los atajos.
fn prompt_overlay(p: &Prompt, theme: &Theme) -> View<Msg> {
    let input = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0) },
        padding: pad(8.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .border(1.0, theme.fg_muted)
    .text(format!("{}_", p.text), 15.0, theme.fg_text);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(440.0_f32), height: length(160.0_f32) },
        padding: pad(18.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .border(1.0, theme.accent)
    .children(vec![
        View::new(fila(30.0)).text(p.title(), 16.0, theme.fg_text),
        input,
        View::new(fila(26.0)).text("Enter confirma · Esc cancela", 12.0, theme.fg_muted),
    ]);
    modal_scrim(card, Msg::PromptCancel)
}

/// Overlay de confirmación de borrado: lista los nombres a borrar y botones
/// Borrar / Cancelar. El click en el scrim cancela.
fn confirm_overlay(targets: &[(nahual_source_core::NodeId, String)], theme: &Theme) -> View<Msg> {
    let nombres: Vec<&str> = targets.iter().map(|(_, n)| n.as_str()).collect();
    let resumen = if nombres.len() == 1 {
        format!("¿Borrar «{}»?", nombres[0])
    } else {
        format!("¿Borrar {} elementos?", nombres.len())
    };
    let detalle = {
        let muestra: Vec<&str> = nombres.iter().take(4).copied().collect();
        let mut s = muestra.join(", ");
        if nombres.len() > 4 {
            s.push_str(&format!(", … (+{})", nombres.len() - 4));
        }
        s
    };

    let boton_borrar = View::new(Style {
        size: Size { width: length(120.0_f32), height: length(34.0_f32) },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        margin: Rect { left: length(0.0), right: length(10.0), top: length(0.0), bottom: length(0.0) },
        ..Default::default()
    })
    .fill(theme.fg_destructive)
    .radius(6.0)
    .on_click(Msg::ConfirmDelete)
    .text("Borrar (Enter)", 14.0, theme.bg_app);

    let boton_cancelar = View::new(Style {
        size: Size { width: length(120.0_f32), height: length(34.0_f32) },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .border(1.0, theme.fg_muted)
    .on_click(Msg::CancelConfirm)
    .text("Cancelar (Esc)", 14.0, theme.fg_text);

    let botones = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![boton_borrar, boton_cancelar]);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(460.0_f32), height: length(180.0_f32) },
        padding: pad(18.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .border(1.0, theme.fg_destructive)
    .children(vec![
        View::new(fila(32.0)).text(resumen, 16.0, theme.fg_text),
        View::new(fila(40.0)).text(detalle, 12.0, theme.fg_muted),
        botones,
    ]);
    modal_scrim(card, Msg::CancelConfirm)
}

/// Overlay del **renombrado por lote** (Fase 4.5): patrón en edición + tabla de
/// previsualización `viejo → nuevo`. Las colisiones (dos objetivos al mismo
/// nombre nuevo) se tiñen en rojo para avisar antes de aplicar.
fn batch_overlay(b: &BatchRename, theme: &Theme) -> View<Msg> {
    let total = b.targets.len();
    // Pre-calcula los nuevos nombres y cuenta colisiones entre ellos.
    let nuevos: Vec<String> = (0..total).map(|i| b.nuevo_nombre(i)).collect();
    let mut conteo: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for nn in &nuevos {
        *conteo.entry(nn.as_str()).or_insert(0) += 1;
    }

    let input = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        padding: pad(8.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .border(1.0, theme.accent)
    .text(format!("{}_", b.pattern), 15.0, theme.fg_text);

    // Filas de preview (hasta 12 visibles).
    let filas: Vec<View<Msg>> = (0..total)
        .take(12)
        .map(|i| {
            let original = &b.targets[i].1;
            let nuevo = &nuevos[i];
            let colision = conteo.get(nuevo.as_str()).copied().unwrap_or(0) > 1;
            let color = if colision {
                theme.fg_destructive
            } else if nuevo == original {
                theme.fg_muted
            } else {
                theme.fg_text
            };
            let marca = if colision { "⚠ " } else { "" };
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
                padding: pad_h(4.0),
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(format!("{marca}{original}  →  {nuevo}"), 13.0, color)
        })
        .collect();
    let oculto = total.saturating_sub(12);
    let mut hijos_lista = filas;
    if oculto > 0 {
        hijos_lista.push(
            View::new(fila(20.0)).text(format!("… y {oculto} más"), 12.0, theme.fg_muted),
        );
    }
    let lista = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(300.0_f32) },
        padding: pad(8.0),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .children(hijos_lista);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(640.0_f32), height: length(470.0_f32) },
        padding: pad(18.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .border(1.0, theme.accent)
    .children(vec![
        View::new(fila(30.0)).text(format!("Renombrar por lote · {total} elementos"), 16.0, theme.fg_text),
        View::new(fila(22.0)).text(
            "Patrón — tokens: {name} · {ext} · {n} (contador)",
            12.0,
            theme.fg_muted,
        ),
        input,
        View::new(fila(24.0)).text("Previsualización", 13.0, theme.fg_muted),
        lista,
        View::new(fila(26.0)).text("Enter aplica · Esc cancela", 12.0, theme.fg_muted),
    ]);
    modal_scrim(card, Msg::BatchCancel)
}

/// Directorio home del usuario (si existe y es un dir).
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Carpetas raíz del árbol lateral, con su ícono real: home, la raíz del
/// filesystem y los favoritos del usuario, sin duplicar.
fn tree_roots(state: &ShellState) -> Vec<(PathBuf, Icon)> {
    let mut roots: Vec<(PathBuf, Icon)> = Vec::new();
    if let Some(home) = home_dir() {
        roots.push((home, Icon::Home));
    }
    roots.push((PathBuf::from("/"), Icon::Folder));
    for p in &state.places {
        let pb = PathBuf::from(p);
        if pb.is_dir() && !roots.iter().any(|(r, _)| r == &pb) {
            roots.push((pb, Icon::Open));
        }
    }
    roots
}

/// Lista las subcarpetas (sólo directorios) de `dir`, ordenadas por nombre
/// (case-insensitive). Vacío si no se puede leer.
fn list_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort_by_key(|p| {
        p.file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    v
}

/// Carga (si falta) el cache **global** de subcarpetas de `dir`.
fn ensure_tree_children(children: &mut HashMap<PathBuf, Vec<PathBuf>>, dir: &Path) {
    if !children.contains_key(dir) {
        children.insert(dir.to_path_buf(), list_dirs(dir));
    }
}

/// El set de ancestros de `target` (incluido él): `/`, `/a`, `/a/b`, … Sirve
/// para arrancar el árbol descolapsado a lo largo del camino al cwd.
fn ancestors_set(target: &Path) -> BTreeSet<PathBuf> {
    let mut set = BTreeSet::new();
    let mut acc = PathBuf::new();
    for comp in target.components() {
        acc.push(comp);
        set.insert(acc.clone());
    }
    set
}

/// Asegura el cache de subcarpetas para cada carpeta descolapsada.
fn ensure_children_for_expanded(
    children: &mut HashMap<PathBuf, Vec<PathBuf>>,
    expanded: &BTreeSet<PathBuf>,
) {
    for dir in expanded {
        ensure_tree_children(children, dir);
    }
}

/// Rótulo de un nodo del árbol: el nombre de la carpeta, o la ruta entera para
/// la raíz `/`.
fn node_label(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Cuenta las filas visibles del árbol (según el set de descolapsadas) — para
/// el clamp del scroll, sin construir las `View`s.
fn count_tree_rows(model: &Model) -> usize {
    fn rec(model: &Model, path: &Path) -> usize {
        let mut n = 1;
        if model.tree_expanded.contains(path) {
            if let Some(ch) = model.tree_children.get(path) {
                for c in ch {
                    n += rec(model, c);
                }
            }
        }
        n
    }
    tree_roots(&model.state).iter().map(|(r, _)| rec(model, r)).sum()
}

/// Alto disponible del árbol (aprox: ventana menos menubar + cabecera).
fn tree_viewport_h(model: &Model) -> f32 {
    let (_, vh) = viewport_of(model);
    (vh - 60.0).max(120.0)
}

/// Cuántas filas del árbol entran en el viewport.
fn tree_visible_rows(model: &Model) -> usize {
    (tree_viewport_h(model) / TREE_ROW_H).floor().max(1.0) as usize
}

/// Ícono vectorial (real, no glifo unicode) para una fila del árbol.
fn tree_icon(icon: Icon, selected: bool, theme: &Theme) -> View<Msg> {
    let color = if selected { theme.fg_text } else { theme.fg_muted };
    View::new(Style {
        size: Size { width: length(16.0_f32), height: length(16.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![icon_view(icon, color, 1.7)])
}

/// Acumula recursivamente las filas visibles del árbol bajo `path`.
fn push_tree_node(
    model: &Model,
    path: &Path,
    depth: usize,
    cur: &Path,
    icon: Icon,
    theme: &Theme,
    rows: &mut Vec<TreeRow<Msg>>,
) {
    let expanded = model.tree_expanded.contains(path);
    let selected = path == cur;
    rows.push(
        TreeRow::new(
            node_label(path),
            depth,
            true,
            expanded,
            selected,
            Msg::TreeToggle(path.to_path_buf()),
            Msg::TreeSelect(path.to_path_buf()),
        )
        .with_icon(tree_icon(icon, selected, theme)),
    );
    if expanded {
        if let Some(children) = model.tree_children.get(path) {
            for child in children {
                // Carpeta cerrada/abierta según su propio estado.
                let ic = if model.tree_expanded.contains(child) {
                    Icon::FolderOpen
                } else {
                    Icon::Folder
                };
                push_tree_node(model, child, depth + 1, cur, ic, theme, rows);
            }
        }
    }
}

/// Filas aplanadas del árbol lateral, partiendo de las raíces.
fn build_tree_rows(model: &Model, theme: &Theme) -> Vec<TreeRow<Msg>> {
    let cur = cur_dir(model);
    let mut rows: Vec<TreeRow<Msg>> = Vec::new();
    for (root, icon) in tree_roots(&model.state) {
        push_tree_node(model, &root, 0, &cur, icon, theme, &mut rows);
    }
    rows
}

/// Sidebar **único**: el árbol de carpetas navegable (home · raíz · favoritos),
/// con íconos reales. Click en el chevron expande/colapsa (`TreeToggle`); click
/// en la fila navega el panel activo (`TreeSelect`). La rueda lo scrollea por
/// filas (`TreeScroll`) — sin esto el wheel caía al canvas. Ancho fijo. El set
/// de descolapsadas y el scroll se recuerdan **por sesión**.
fn sidebar_view(model: &Model, theme: &Theme) -> View<Msg> {
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text("CARPETAS", 12.0, theme.fg_muted);

    // Ventaneo: sólo las filas que entran (offset recordado por sesión).
    let all = build_tree_rows(model, theme);
    let vis = tree_visible_rows(model);
    let off = model.tree_scroll.min(all.len().saturating_sub(vis));
    let rows: Vec<TreeRow<Msg>> = all.into_iter().skip(off).take(vis).collect();

    let tree = tree_view(TreeSpec {
        rows,
        row_height: TREE_ROW_H,
        indent_px: 14.0,
        palette: TreePalette::from_theme(theme),
        guides: true,
    });
    let tree_wrap = View::new(Style {
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![tree]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        // El ancho lo dicta el splitter del sidebar (pane Fixed(tree_w)).
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    // La rueda sobre el sidebar la rutea `on_wheel` por región (cursor.x <
    // tree_w) — el handler local se perdía entre updates rápidos.
    .children(vec![header, tree_wrap])
}

/// La app integrada abierta en el canvas: editor de texto potente (con
/// header de estado y Ctrl+S), visor de imagen con zoom/pan, o player de
/// media. Esc/⌫ vuelve a la vista de carpeta.
fn canvas_app_view(canvas: &CanvasApp, model: &Model, theme: &Theme) -> View<Msg> {
    match canvas {
        CanvasApp::Texto { path, editor, dirty, saved } => {
            let nombre = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let estado = if *dirty {
                "● sin guardar"
            } else if *saved {
                "✓ guardado"
            } else {
                ""
            };
            let titulo = View::new(Style {
                flex_grow: 1.0,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(format!("{nombre}  {estado}"), 13.0, theme.fg_text);
            let hint = View::new(Style {
                size: Size { width: auto(), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text("Ctrl+S guarda · Esc cierra", 11.5, theme.fg_muted);
            let header = View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
                padding: pad_h(12.0),
                align_items: Some(AlignItems::Center),
                flex_shrink: 0.0,
                ..Default::default()
            })
            .fill(theme.bg_panel)
            .children(vec![titulo, hint]);

            let cuerpo = text_editor_view_full(
                editor,
                &EditorPalette::from_theme(theme),
                EditorMetrics::for_font_size(13.0),
                canvas_editor_lines(model),
                language_for_path(path),
                &[],
                |ev| Some(Msg::CanvasEditPointer(ev)),
            );
            let cuerpo_wrap = View::new(Style {
                flex_grow: 1.0,
                min_size: Size { width: length(0.0), height: length(0.0) },
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .children(vec![cuerpo]);

            View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .children(vec![header, cuerpo_wrap])
        }
        CanvasApp::Imagen { path, state, viewport } => image_viewer_view_zoom(
            state,
            Some(path),
            &ImageViewerPalette::from_theme(theme),
            *viewport,
            |factor, _fx, _fy| Msg::CanvasImgZoom(factor),
            |dx, dy| Msg::CanvasImgPan(dx, dy),
            Msg::CanvasImgReset,
        ),
        CanvasApp::Video(state) => {
            video_viewer_view(state, &VideoViewerPalette::from_theme(theme))
        }
        CanvasApp::Audio(state) => {
            audio_viewer_view(state, &AudioViewerPalette::from_theme(theme))
        }
    }
}

/// Diente del **panel derecho de preview**: overlay absoluto pegado al borde
/// interno derecho (espejo del rail de sesiones). Click abre/cierra el panel.
fn preview_tooth_overlay(model: &Model, theme: &Theme) -> View<Msg> {
    let items = [DockRailItem { id: 0, active: model.viewer_open }];
    let rail = dock_rail_view(
        &items,
        SESSION_RAIL_W,
        &DockRailPalette::from_theme(theme),
        |_id, size, color| {
            View::new(Style {
                size: Size { width: length(size), height: length(size) },
                ..Default::default()
            })
            .children(vec![icon_view(Icon::Search, color, 1.7)])
        },
        |_id| Msg::TogglePreviewPanel,
        |_payload| -> Option<Msg> { None },
    );
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0_f32),
            right: length(0.0_f32),
            left: auto(),
            bottom: auto(),
        },
        size: Size { width: length(SESSION_RAIL_W), height: auto() },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![rail])
}

/// **Dientes** de sesión como overlay absoluto pegado al borde interno del
/// canvas (el patrón canónico de cosmos: `dock_rail_overlay`): cada diente
/// (`llimphi-widget-dock-rail`) es una sesión de trabajo y sobresale del
/// sidebar sobre el canvas. Click activa esa sesión (su árbol + su vista de
/// carpeta vuelven); debajo, un `+` abre una sesión nueva.
fn session_teeth_overlay(model: &Model, theme: &Theme) -> View<Msg> {
    let items: Vec<DockRailItem> = (0..model.sessions.len())
        .map(|i| DockRailItem { id: i as u64, active: i == model.active })
        .collect();
    let rail = dock_rail_view(
        &items,
        SESSION_RAIL_W,
        &DockRailPalette::from_theme(theme),
        |_id, size, color| {
            View::new(Style {
                size: Size { width: length(size), height: length(size) },
                ..Default::default()
            })
            .children(vec![icon_view(Icon::Folder, color, 1.7)])
        },
        |id| Msg::SessionActivate(id as usize),
        |_payload| -> Option<Msg> { None },
    );
    // "+" nueva sesión, colgado debajo de los dientes.
    let plus = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::SessionNew)
    .children(vec![View::new(Style {
        size: Size { width: length(16.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .children(vec![icon_view(Icon::Plus, theme.fg_muted, 1.8)])]);

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0_f32),
            left: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(SESSION_RAIL_W), height: auto() },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![rail, plus])
}

/// Panel inferior colapsable de la **cola de operaciones**. `None` si no hay
/// jobs. La barra de cabecera (siempre visible) resume y alterna el detalle;
/// cuando está abierto, lista cada job con su estado.
fn queue_panel(model: &Model, theme: &Theme) -> Option<View<Msg>> {
    let q = &model.queue;
    if q.ops.is_empty() {
        return None;
    }
    let corriendo = q.running_count();
    let total = q.ops.len();
    let resumen = if corriendo > 0 {
        format!("⚙ Operaciones · {corriendo} en curso / {total}")
    } else {
        format!("✓ Operaciones · {total} terminadas")
    };
    let flecha = if q.open { "▾" } else { "▸" };

    // Cabecera: resumen (toggle) + botón limpiar.
    let titulo = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .on_click(Msg::ToggleQueue)
    .text(format!("{flecha} {resumen}"), 13.0, theme.fg_text);

    let limpiar = View::new(Style {
        size: Size { width: length(96.0_f32), height: length(24.0_f32) },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(5.0)
    .on_click(Msg::ClearQueue)
    .text("Limpiar", 12.0, theme.fg_muted);

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        padding: Rect { left: length(12.0), right: length(12.0), top: length(0.0), bottom: length(0.0) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![titulo, limpiar]);

    let mut hijos = vec![header];

    if q.open {
        // Hasta 6 filas de jobs (las más recientes arriba).
        let filas: Vec<View<Msg>> = q
            .ops
            .iter()
            .rev()
            .take(6)
            .map(|op| {
                let (glyph, color) = match &op.status {
                    OpStatus::Running => ("⋯", theme.accent),
                    OpStatus::Done(_) => ("✓", theme.fg_muted),
                    OpStatus::Failed(_) => ("✗", theme.fg_destructive),
                };
                let texto = match &op.status {
                    OpStatus::Failed(e) => format!("{glyph} {} — {e}", op.label),
                    _ => format!("{glyph} {}", op.label),
                };
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
                    padding: Rect { left: length(16.0), right: length(12.0), top: length(0.0), bottom: length(0.0) },
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                })
                .text(texto, 12.0, color)
            })
            .collect();
        let lista = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: length(140.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .children(filas);
        hijos.push(lista);
    }

    let alto = if q.open { 172.0 } else { 30.0 };
    Some(
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: length(alto) },
            ..Default::default()
        })
        .children(hijos),
    )
}

/// Barra de **breadcrumb clicable** de un panel (Fase 4.2): cada segmento sube
/// a ese nivel (`BreadcrumbIn(pane, depth)`). Sobre una fuente no-POSIX, el
/// primer segmento lleva el prefijo `⊟ <fuente>`. `focused` tiñe la barra
/// cuando el panel está enfocado (sólo se nota en modo dual).
fn pane_breadcrumb(pane_obj: &Pane, pane: usize, focused: bool, theme: &Theme) -> View<Msg> {
    let nav = pane_obj.nav();
    let mut segs: Vec<String> = nav.ancestors().iter().map(|n| n.name.clone()).collect();
    if pane_obj.is_foreign() && !segs.is_empty() {
        segs[0] = format!("⊟ {}", nav.label());
    }
    let seg_refs: Vec<&str> = segs.iter().map(String::as_str).collect();
    let crumbs = breadcrumb_view(
        &seg_refs,
        move |depth| Msg::BreadcrumbIn(pane, depth),
        &BreadcrumbPalette::from_theme(theme),
    );
    let bg = if focused { theme.bg_selected } else { theme.bg_panel };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .children(vec![crumbs])
}

/// Una columna de panel: su breadcrumb arriba + su lista/grilla. `focused`
/// resalta el panel activo (relevante en modo dual). Las filas emiten `Msg`s
/// que llevan `pane`, así que el click actúa sobre el panel correcto.
fn pane_column(model: &Model, pane: usize, focused: bool, theme: &Theme) -> View<Msg> {
    let crumb = pane_breadcrumb(&model.panes[pane], pane, focused, theme);
    // El filtro vivo sólo aplica al panel enfocado.
    let filtering = focused && model.nav_filtering;
    // La vista iconos necesita el cache de miniaturas y las dimensiones del
    // panel: se arma aparte (las otras dos sólo dependen del navegador).
    let content = if model.panes[pane].nav().view.is_grid() {
        navigator_icons_view(model, pane, theme)
    } else {
        nav_pane_view(
            model.panes[pane].nav(),
            &model.panes[pane].marked,
            &model.state,
            theme,
            filtering,
            pane,
        )
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![crumb, content])
}

/// Índice de columna (0 nombre · 1 tamaño · 2 fecha · 3 tipo) → `SortKey`.
fn col_to_sortkey(col: u8) -> nahual_source_core::SortKey {
    use nahual_source_core::SortKey::*;
    match col {
        1 => Size,
        2 => Mtime,
        3 => Kind,
        _ => Name,
    }
}

/// `SortKey` → índice de columna.
fn sortkey_to_col(key: nahual_source_core::SortKey) -> u8 {
    use nahual_source_core::SortKey::*;
    match key {
        Name => 0,
        Size => 1,
        Mtime => 2,
        Kind => 3,
    }
}

/// El `FolderFormat` (vista + orden) actual del navegador.
fn current_format(nav: &Navigator) -> state::FolderFormat {
    let (key, dir) = nav.sort();
    state::FolderFormat {
        view: view_to_u8(nav.view),
        sort_col: sortkey_to_col(key),
        sort_asc: matches!(dir, nahual_source_core::SortDir::Asc),
    }
}

/// `ViewMode` → primitivo persistible (0 lista · 1 detalle · 2 iconos).
fn view_to_u8(v: nahual_source_core::ViewMode) -> u8 {
    match v {
        nahual_source_core::ViewMode::List => 0,
        nahual_source_core::ViewMode::Details => 1,
        nahual_source_core::ViewMode::Icons => 2,
        nahual_source_core::ViewMode::Gallery => 3,
    }
}

/// Primitivo persistido → `ViewMode` (cualquier valor desconocido = lista).
fn u8_to_view(n: u8) -> nahual_source_core::ViewMode {
    match n {
        1 => nahual_source_core::ViewMode::Details,
        2 => nahual_source_core::ViewMode::Icons,
        3 => nahual_source_core::ViewMode::Gallery,
        _ => nahual_source_core::ViewMode::List,
    }
}

/// Recuerda el formato (vista/orden) de la carpeta actual del panel enfocado.
/// No-op sobre fuentes montadas (sus ids no son rutas estables).
fn save_format(m: &mut Model) {
    if m.is_foreign() {
        return;
    }
    let id = m.cur().current_id().clone();
    let fmt = current_format(m.cur());
    m.state.set_format(&id, fmt);
    m.state.save();
}

/// Aplica el formato guardado de la carpeta actual (si hay), tras entrar a ella.
fn apply_format(m: &mut Model) {
    if m.is_foreign() {
        return;
    }
    let id = m.cur().current_id().clone();
    if let Some(fmt) = m.state.format_of(&id) {
        let nav = m.cur_mut();
        nav.view = u8_to_view(fmt.view);
        nav.set_sort_to(col_to_sortkey(fmt.sort_col), fmt.sort_asc);
    }
}

/// Lado máximo (px) de las miniaturas de la vista iconos.
const THUMB_LADO: u32 = 128;
/// Tope de miniaturas pedidas por pasada — acota los `Handle::spawn` para que
/// una carpeta con miles de imágenes no dispare un thread por archivo.
const MAX_ICON_TILES: usize = 160;

/// ¿La extensión sugiere una imagen rasterizable? Filtro barato antes de
/// gastar un worker en decodificar (los no-imagen muestran su glifo de tipo).
fn es_imagen(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some(
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "tif" | "ico" | "avif"
                | "qoi" | "tga"
        )
    )
}

/// Pide (async) las miniaturas de las imágenes visibles del panel enfocado en
/// vista iconos. Sólo POSIX (los ids son rutas reales); dedup por
/// `thumbs_pending`; tope `MAX_ICON_TILES`. Cada worker reentra con
/// `Msg::ThumbReady`/`ThumbFailed`.
fn request_thumbs(m: &mut Model, handle: &Handle<Msg>) {
    if m.is_foreign() {
        return; // fuentes montadas (wawa/minga/archivo) no tienen path en disco
    }
    let pedir: Vec<PathBuf> = {
        let nav = m.cur();
        let visibles = nav.visible();
        let start = nav.visible_offset.min(visibles.len());
        let end = (start + MAX_ICON_TILES).min(visibles.len());
        visibles[start..end]
            .iter()
            .filter(|(_, n)| !n.is_container)
            .map(|(_, n)| PathBuf::from(&n.id))
            .filter(|p| {
                es_imagen(p)
                    && !m.thumbs.contains_key(p)
                    && !m.thumbs_pending.contains(p)
                    && !m.thumbs_failed.contains(p)
            })
            .collect()
    };
    for path in pedir {
        m.thumbs_pending.insert(path.clone());
        handle.spawn(move || match generar_thumb_de_archivo(&path, THUMB_LADO) {
            Ok(t) => Msg::ThumbReady(path, t),
            Err(_) => Msg::ThumbFailed(path),
        });
    }
}

/// Registra la carpeta actual del panel enfocado como reciente (MRU).
fn record_recent(m: &mut Model) {
    if m.is_foreign() {
        return;
    }
    let id = m.cur().current_id().clone();
    m.state.push_recent(&id);
    m.state.save();
}

/// Encola una operación y lanza su worker (`Handle::spawn`): el job corre en un
/// hilo aparte y, al terminar, reentra al `update` con `Msg::OpFinished`. La UI
/// no se bloquea ni para una copia de un árbol grande.
fn enqueue(m: &mut Model, handle: &Handle<Msg>, kind: OpKind) {
    let id = m.queue.push(kind.clone());
    handle.spawn(move || {
        let result = kind.run().map_err(|e| e.to_string());
        Msg::OpFinished { id, result }
    });
}

/// Recarga los hijos de ambos paneles desde el disco tras una operación, y
/// poda las marcas que ya no apuntan a un nodo existente (borrado/movido).
fn reload_panes(m: &mut Model) {
    for p in m.panes.iter_mut() {
        let _ = p.nav_mut().reload();
        let ids: BTreeSet<nahual_source_core::NodeId> =
            p.nav().children().iter().map(|n| n.id.clone()).collect();
        p.marked.retain(|id| ids.contains(id));
    }
}

/// Copia (o mueve, si `is_move`) la selección del panel enfocado al directorio
/// del **otro** panel. Sólo si el destino es POSIX escribible (no se escribe
/// sobre una fuente montada read-only). Encola un job por nodo objetivo.
fn copy_or_move(m: &mut Model, handle: &Handle<Msg>, is_move: bool) {
    let other = 1 - m.focus;
    if m.panes[other].nav().writable().is_none() {
        return;
    }
    let dest = m.panes[other].nav().current_id().clone();
    for (id, name) in m.cur_pane().op_targets() {
        let kind = if is_move {
            OpKind::Move { id, name, dest_parent: dest.clone() }
        } else {
            OpKind::Copy { id, name, dest_parent: dest.clone() }
        };
        enqueue(m, handle, kind);
    }
    m.cur_pane_mut().marked.clear();
}

/// Limpia el panel derecho y suelta cualquier tempfile de hoja no-POSIX.
fn clear_preview(m: &mut Model) {
    m.preview = PreviewPane::Empty;
    m.preview_of = None;
    m.preview_temp = None;
    m.basemap = None;
}

/// Abre `path` como basemap PMTiles vivo si su magic lo delata.
fn open_basemap_if_pmtiles(path: &Path) -> Option<Basemap> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.starts_with(b"PMTiles") {
        Basemap::open(bytes).ok()
    } else {
        None
    }
}

/// Si hay un basemap PMTiles abierto, recalcula el viewport (tiles visibles a
/// la cámara actual) y lo deja como preview. Se llama tras cada cambio de
/// cámara para el streaming.
/// Devuelve `true` si re-streameó (había basemap y el canvas ya registró su
/// rect). `false` deja el pedido pendiente para reintentar (p. ej. en el
/// primer tick tras abrir, antes del primer paint).
fn restream_basemap(m: &mut Model) -> bool {
    let Some(bm) = m.basemap.as_mut() else {
        return false;
    };
    // Sin rect aún (no se pintó): conservamos el overview y reintentamos.
    if m.map_view.rect().is_none() {
        return false;
    }
    let md = bm.viewport(&m.map_view);
    m.preview = PreviewPane::Map(MapPreview::Map { data: md, truncated: false });
    true
}

/// Intenta montar `path` como una fuente no-POSIX. Hoy sólo prueba imagen
/// wawa: `WawaImgSource::abrir` hace un chequeo de magic barato y sólo carga
/// el grafo si el archivo realmente es una imagen wawa — para todo lo demás
/// falla rápido y devolvemos `None` (se previsualiza normal).
fn try_mount(path: &Path) -> Option<Navigator> {
    // Imagen wawa `.img` → su DAG content-addressed.
    if let Ok(src) = WawaImgSource::abrir(path) {
        return Navigator::open(Box::new(src)).ok();
    }
    // Archivo contenedor (.zip/.tar/.tar.gz) → su árbol interno como carpeta.
    if ArchiveSource::es_archivo(path) {
        if let Ok(src) = ArchiveSource::abrir(path) {
            return Navigator::open(Box::new(src)).ok();
        }
    }
    None
}

/// El directorio que un montaje explícito (`m`/`g`) toma como objetivo: el
/// subdirectorio seleccionado si lo hay, o el `cwd` del explorador POSIX.
fn target_dir(m: &Model) -> PathBuf {
    let nav = m.cur();
    match nav.selected_node() {
        // Subdir seleccionado (en POSIX su id ES la ruta absoluta).
        Some(n) if n.is_container => PathBuf::from(&n.id),
        // Si no, el dir actual.
        _ => PathBuf::from(nav.current_id()),
    }
}

/// Heurística no destructiva: ¿este directorio ya parece un repo minga
/// (sled)? Chequea los artefactos que `sled::open` deja (`conf`/`db`) sin
/// abrirlo — abrir crearía esos archivos en un dir cualquiera, justo lo que
/// queremos evitar.
fn parece_repo_minga(dir: &Path) -> bool {
    dir.is_dir() && (dir.join("conf").exists() || dir.join("db").exists())
}

/// Materializa los bytes de una hoja no-POSIX en un tempfile y la
/// previsualiza con [`load_for`]. El tempdir se guarda en el modelo para que
/// el path siga válido mientras el visor lo lea (audio/video streamean).
fn preview_from_bytes(m: &mut Model, bytes: Vec<u8>, nombre: &str) {
    let Ok(dir) = tempfile::tempdir() else {
        clear_preview(m);
        return;
    };
    let path = dir.path().join(sanitizar_nombre(nombre));
    if std::fs::write(&path, &bytes).is_ok() {
        m.preview = load_for(&path);
        m.preview_of = Some(path);
        m.preview_temp = Some(dir); // mantener vivo el tempdir
    } else {
        clear_preview(m);
    }
}

/// Vuelve un nombre de nodo apto para un filename de tempfile (los objetos
/// wawa son hashes sin separadores, pero por las dudas sacamos `/` y `\`).
fn sanitizar_nombre(nombre: &str) -> String {
    let limpio: String = nombre
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    if limpio.is_empty() {
        "objeto".to_string()
    } else {
        limpio
    }
}

/// Pinta el contenido de un panel según su `ViewMode` (lista o detalle). `pane`
/// es el índice del panel (0/1): las filas y encabezados emiten `Msg`s que lo
/// llevan, para que el click actúe sobre el panel correcto en modo dual.
fn nav_pane_view(
    nav: &Navigator,
    marked: &BTreeSet<nahual_source_core::NodeId>,
    state: &ShellState,
    theme: &Theme,
    filtering: bool,
    pane: usize,
) -> View<Msg> {
    match nav.view {
        nahual_source_core::ViewMode::List => {
            navigator_list_view(nav, marked, state, ListPalette::from_theme(theme), filtering, pane)
        }
        nahual_source_core::ViewMode::Details => {
            navigator_detail_view(nav, marked, state, theme, filtering, pane)
        }
        // Icons y Gallery se interceptan en `pane_column` (necesitan el cache
        // de thumbs y las dimensiones del panel); estos brazos no se alcanzan,
        // pero mantienen el match exhaustivo. Fallback honesto: detalle.
        nahual_source_core::ViewMode::Icons | nahual_source_core::ViewMode::Gallery => {
            navigator_detail_view(nav, marked, state, theme, filtering, pane)
        }
    }
}

/// Pinta los hijos visibles como **grilla de iconos/miniaturas** (Fase 4.8).
/// Las imágenes muestran su thumbnail (cache `model.thumbs`, llenado async);
/// el resto, un glifo por `NodeKind`. Reusa `llimphi-widget-grid` (la misma
/// grilla virtualizada de `nahual-gallery`).
/// Métricas de la grilla según el modo: galería = tiles grandes (carpetas de
/// imágenes); iconos = la grilla compacta por defecto.
fn grid_metrics_for(view: nahual_source_core::ViewMode) -> GridMetrics {
    if matches!(view, nahual_source_core::ViewMode::Gallery) {
        GridMetrics { tile_w: 220.0, tile_h: 248.0, gap: 14.0, pad: 14.0 }
    } else {
        GridMetrics::default()
    }
}

/// Ancho útil del panel de la grilla: la ventana menos el sidebar (árbol),
/// el canal de los dientes y, si está abierto, el visor derecho; en dual,
/// la mitad de eso.
fn grid_pane_w(model: &Model) -> f32 {
    let (vw, _) = viewport_of(model);
    let mut canvas_w = (vw - model.tree_w - SESSION_RAIL_W - 8.0).max(240.0);
    if model.viewer_open {
        canvas_w = (canvas_w - model.preview_w).max(240.0);
    }
    if model.dual {
        canvas_w / 2.0
    } else {
        canvas_w
    }
}

/// Columnas actuales de la grilla del panel activo (para que el wheel
/// scrollee por filas enteras).
fn grid_cols(model: &Model) -> usize {
    let nav = model.cur();
    let metrics = grid_metrics_for(nav.view);
    let (_, vh) = viewport_of(model);
    let win = ventana_visible(nav.visible_count(), grid_pane_w(model), vh - 120.0, 0, &metrics);
    win.cols.max(1)
}

/// Toolbar del shell: navegación + modos de vista + acciones, sobre el
/// widget `llimphi-widget-toolbar` (los grupos son datos → componibles).
fn shell_toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    use nahual_source_core::ViewMode as VM;
    let v = model.cur().view;
    let vista = |ic: Icon, modo: VM, activo: bool| {
        ToolbarItem::new(move |_s, c| icon_view(ic, c, 1.7), Msg::SetViewMode(modo)).active(activo)
    };
    let pane = model.cur_pane();
    let puede_atras = pane.hist_pos > 0;
    let puede_adelante = pane.hist_pos + 1 < pane.hist.len();
    toolbar_view(
        vec![
            // Navegación: atrás / adelante (historial browser) / subir.
            ToolbarGroup::new(vec![
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronLeft, c, 1.7), Msg::NavBack)
                    .enabled(puede_atras),
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronRight, c, 1.7), Msg::NavForward)
                    .enabled(puede_adelante),
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronUp, c, 1.7), Msg::Parent)
                    .with_label("subir"),
            ]),
            // Modos de vista (v cicla; acá acceso directo).
            ToolbarGroup::new(vec![
                vista(Icon::Rows, VM::List, matches!(v, VM::List)),
                vista(Icon::Table, VM::Details, matches!(v, VM::Details)),
                vista(Icon::Grid, VM::Icons, matches!(v, VM::Icons)),
                vista(Icon::Image, VM::Gallery, matches!(v, VM::Gallery)),
            ]),
            // Acciones.
            ToolbarGroup::new(vec![
                ToolbarItem::new(|_s, c| icon_view(Icon::Columns, c, 1.7), Msg::ToggleDual)
                    .active(model.dual),
                ToolbarItem::new(|_s, c| icon_view(Icon::Plus, c, 1.7), Msg::NewDirPrompt)
                    .with_label("carpeta")
                    .enabled(model.can_edit()),
            ]),
        ],
        34.0,
        &ToolbarPalette::from_theme(theme),
    )
}

fn navigator_icons_view(model: &Model, pane: usize, theme: &Theme) -> View<Msg> {
    let nav = model.panes[pane].nav();
    let marked = &model.panes[pane].marked;
    let gallery = matches!(nav.view, nahual_source_core::ViewMode::Gallery);
    let metrics = grid_metrics_for(nav.view);
    let modo = if gallery { "galería" } else { "iconos" };

    let (_, vh) = viewport_of(model);
    let pane_w = grid_pane_w(model);
    let total = nav.visible_count();
    let win = ventana_visible(total, pane_w, vh - 120.0, 0, &metrics);

    let visibles = nav.visible();
    let start = nav.visible_offset.min(visibles.len());
    let end = (start + MAX_ICON_TILES).min(visibles.len());
    let cells: Vec<GridCell<Msg>> = visibles[start..end]
        .iter()
        .map(|(idx, n)| {
            let mark = if marked.contains(&n.id) { "✓ " } else { "" };
            let label = format!("{mark}{}", n.name);
            GridCell {
                content: icon_tile_content(model, n, theme, metrics.tile_w - 12.0),
                label: Some(label),
                selected: *idx == nav.selected,
                on_click: Msg::SelectIn(pane, *idx),
            }
        })
        .collect();

    let mostrados = start + cells.len();
    let truncated_hint = (mostrados < total)
        .then(|| format!("… y {} más (rueda para ver más)", total - mostrados));

    grid_view(GridSpec {
        cells,
        cols: win.cols,
        metrics,
        caption: Some(format!(
            "{total} entradas · {modo} · ↑↓ navega · Enter abre · v cambia vista"
        )),
        truncated_hint,
        palette: GridPalette::from_theme(theme),
    })
}

/// Cuerpo de una celda de la grilla iconos/galería: la miniatura si está lista;
/// si no, un **ícono vectorial real** por tipo (carpeta, imagen pendiente,
/// archivo) o un aviso si la miniatura falló.
fn icon_tile_content(model: &Model, node: &Node, theme: &Theme, lado: f32) -> View<Msg> {
    let base = || Style {
        size: Size { width: length(lado), height: length(lado) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    };
    // Ícono vectorial centrado, dimensionado a la mitad del tile.
    let big = (lado * 0.5).clamp(28.0, 96.0);
    let centered = |icon: Icon, color: Color| {
        View::new(base()).fill(theme.bg_panel_alt).children(vec![View::new(Style {
            size: Size { width: length(big), height: length(big) },
            ..Default::default()
        })
        .children(vec![icon_view(icon, color, 1.6)])])
    };
    if node.is_container {
        let icon = match node.kind {
            NodeKind::Archive => Icon::Archive,
            _ => Icon::Folder,
        };
        return centered(icon, theme.fg_text);
    }
    let path = PathBuf::from(&node.id);
    if let Some(img) = model.thumbs.get(&path) {
        return View::new(base()).image(img.clone());
    }
    if model.thumbs_failed.contains(&path) {
        return centered(Icon::Warning, theme.fg_muted);
    }
    // Imagen aún decodificando → ícono de imagen; archivo común → ícono file.
    let icon = if es_imagen(&path) { Icon::Image } else { Icon::File };
    centered(icon, theme.fg_muted)
}

/// Color peniko de un label (para el tinte de fila en la vista detalle).
fn label_color(label: Label) -> Color {
    let (r, g, b) = label.rgb();
    Color::from_rgba8(r, g, b, 255)
}

/// Sufijo del caption con el estado del filtro y los atajos.
fn nav_caption(nav: &Navigator, filtering: bool) -> String {
    let f = nav.filter();
    if filtering || !f.is_empty() {
        let cursor = if filtering { "_" } else { "" };
        format!(
            "{} de {} · filtro: {f}{cursor}  (Esc sale · v vista)",
            nav.visible_count(),
            nav.children().len()
        )
    } else {
        format!(
            "{} entradas · ↑↓ navega · Enter abre · ⌫ vuelve · v cambia vista · / filtra",
            nav.children().len()
        )
    }
}

/// Pinta los hijos visibles (filtrados) del contenedor actual como una lista
/// `llimphi-widget-list` — el gemelo genérico de `file_explorer_view`.
fn navigator_list_view(
    nav: &Navigator,
    marked: &BTreeSet<nahual_source_core::NodeId>,
    state: &ShellState,
    palette: ListPalette,
    filtering: bool,
    pane: usize,
) -> View<Msg> {
    use std::cmp::min;
    let visibles = nav.visible();
    let start = nav.visible_offset.min(visibles.len());
    let end = min(visibles.len(), start + nav.visible_rows);
    let rows: Vec<ListRow<Msg>> = visibles[start..end]
        .iter()
        .map(|(idx, n)| {
            // Una fila marcada (selección múltiple) lleva un check al frente.
            let mark = if marked.contains(&n.id) { "✓" } else { " " };
            // Punto cuando el nodo tiene label (el color real se ve en detalle;
            // en lista es monocromo — la lista no pinta color por fila).
            let dot = if state.label_of(&n.id).is_some() { "●" } else { " " };
            // Indentación por profundidad (expansión inline) + chevron de
            // estado: ▾ expandida, ▸ colapsada. Click la alterna; doble
            // click la abre en el canvas.
            let sangria = "   ".repeat(nav.depth_of(*idx));
            let icon = if n.is_container {
                if nav.is_expanded(&n.id) { "▾ " } else { "▸ " }
            } else {
                "  "
            };
            let label = if n.is_container {
                format!("{mark}{dot}{sangria}{icon}{}/", n.name)
            } else {
                format!("{mark}{dot}{sangria}{icon}{}", n.name)
            };
            ListRow {
                label,
                selected: *idx == nav.selected,
                on_click: Msg::SelectIn(pane, *idx),
            }
        })
        .collect();
    let truncated_hint = if visibles.len() > end {
        Some(format!("… y {} más (rueda o ↓ para ver más)", visibles.len() - end))
    } else {
        None
    };
    list_view(ListSpec {
        rows,
        total: visibles.len(),
        caption: Some(nav_caption(nav, filtering)),
        truncated_hint,
        row_height: 22.0,
        palette,
    })
}

/// Pinta los hijos visibles como grilla detalle con columnas ordenables
/// (nombre · tamaño · modificado · tipo). Click en un encabezado emite
/// `NavSortBy`; click en una fila selecciona.
fn navigator_detail_view(
    nav: &Navigator,
    marked: &BTreeSet<nahual_source_core::NodeId>,
    state: &ShellState,
    theme: &Theme,
    filtering: bool,
    pane: usize,
) -> View<Msg> {
    use llimphi_widget_detail_table::{
        detail_table_view, Column, DetailPalette, DetailRow, DetailSpec, SortDir as DtDir,
    };
    use nahual_source_core::SortKey;

    let (skey, sdir) = nav.sort();
    let sort_col = match skey {
        SortKey::Name => 0,
        SortKey::Size => 1,
        SortKey::Mtime => 2,
        SortKey::Kind => 3,
    };
    let dt_dir = match sdir {
        nahual_source_core::SortDir::Asc => DtDir::Asc,
        nahual_source_core::SortDir::Desc => DtDir::Desc,
    };

    let visibles = nav.visible();
    let start = nav.visible_offset.min(visibles.len());
    let end = (start + nav.visible_rows).min(visibles.len());
    let rows: Vec<DetailRow<Msg>> = visibles[start..end]
        .iter()
        .map(|(idx, n)| {
            let icon = kind_icon(n.kind, n.is_container);
            let is_marked = marked.contains(&n.id);
            let mark = if is_marked { "✓" } else { " " };
            let label = state.label_of(&n.id);
            // El nombre lleva un punto del color del label, si tiene.
            let dot = if label.is_some() { "● " } else { "" };
            // Indentación por profundidad + chevron de expansión inline
            // (▾ expandida / ▸ colapsada). Click alterna; doble click abre.
            let sangria = "   ".repeat(nav.depth_of(*idx));
            let chev = if n.is_container {
                if nav.is_expanded(&n.id) { "▾ " } else { "▸ " }
            } else {
                "  "
            };
            DetailRow {
                cells: vec![
                    format!("{mark}{sangria}{chev}{icon} {dot}{}", n.name),
                    n.size.map(human_size).unwrap_or_default(),
                    n.mtime.map(epoch_ms_to_date).unwrap_or_default(),
                    kind_label(n.kind, &n.name).to_string(),
                ],
                selected: *idx == nav.selected,
                // El acento del nombre lleva el color del label si lo tiene; si
                // no, el acento neutro de las filas marcadas.
                accent: label
                    .map(label_color)
                    .or_else(|| is_marked.then_some(theme.accent)),
                on_click: Msg::SelectIn(pane, *idx),
            }
        })
        .collect();

    let columns = [
        Column::flex("Nombre", 1.0),
        Column::fixed("Tamaño", 88.0).right(),
        Column::fixed("Modificado", 140.0),
        Column::fixed("Tipo", 84.0),
    ];
    detail_table_view(
        DetailSpec {
            columns: &columns,
            rows,
            sort: Some((sort_col, dt_dir)),
            row_height: 22.0,
            caption: Some(nav_caption(nav, filtering)),
            palette: DetailPalette::from_theme(theme),
        },
        move |col| Msg::SortByIn(pane, col),
    )
}

/// Icono de una columna nombre según la naturaleza del nodo.
fn kind_icon(kind: nahual_source_core::NodeKind, is_container: bool) -> &'static str {
    use nahual_source_core::NodeKind::*;
    match kind {
        Dir => "▸",
        Synthetic => "◇",
        Archive => "▤",
        Symlink => "↪",
        File if is_container => "▸",
        File => " ",
    }
}

/// Rótulo de la columna "tipo".
fn kind_label(kind: nahual_source_core::NodeKind, name: &str) -> &'static str {
    use nahual_source_core::NodeKind::*;
    match kind {
        Dir => "carpeta",
        Synthetic => "mónada",
        Archive => "archivo",
        Symlink => "enlace",
        File => match name.rsplit_once('.').map(|(_, e)| e) {
            Some("rs") => "rust",
            Some("md") => "markdown",
            Some("toml") => "toml",
            Some("json") => "json",
            Some("png" | "jpg" | "jpeg" | "webp" | "gif") => "imagen",
            Some("txt") => "texto",
            _ => "archivo",
        },
    }
}

/// Tamaño humano compacto (B/KB/MB/GB/TB), una cifra decimal salvo bytes.
fn human_size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut val = b as f64;
    let mut i = 0;
    while val >= 1024.0 && i < U.len() - 1 {
        val /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{val:.1} {}", U[i])
    }
}

/// Epoch-ms → `YYYY-MM-DD HH:MM` en UTC (civil-from-days de Hinnant). Sin
/// dependencias de fechas — alcanza para la columna "modificado".
fn epoch_ms_to_date(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, min) = (tod / 3600, (tod % 3600) / 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}")
}

/// Releé el preview del entry seleccionado tras un cambio de selección.
/// Avanza el campo de choropleth: `None → campo₀ → campo₁ → … → None`.
fn next_in_cycle(fields: &[String], current: &Option<String>) -> Option<String> {
    if fields.is_empty() {
        return None;
    }
    match current {
        None => fields.first().cloned(),
        Some(c) => match fields.iter().position(|f| f == c) {
            Some(i) if i + 1 < fields.len() => Some(fields[i + 1].clone()),
            _ => None,
        },
    }
}

/// Releé el preview del nodo seleccionado en el navegador activo (POSIX o
/// fuente montada). Contenedor (o nada) → limpia. Hoja POSIX (id = ruta real)
/// → carga directa con `load_for`. Hoja no-POSIX → vuelca a tempfile y
/// previsualiza. Unifica los dos caminos viejos (POSIX y `*_nav`).
/// Abre la selección (Enter o doble click): contenedor → desciende al canvas
/// y **revela la carpeta en el árbol lateral**; hoja → monta (`.img` wawa) o
/// abre el visor en el sidebar derecho.
fn do_open_selected(m: &mut Model, handle: &Handle<Msg>) {
    match m.cur_mut().open_selected() {
        Ok(Some(Opened::Descended)) => {
            m.cur_pane_mut().marked.clear();
            m.canvas = None;
            clear_preview(m);
            apply_format(m);
            record_recent(m);
            // Revela la carpeta nueva en el árbol lateral (descolapsa la
            // cadena de ancestros) y sincroniza el nombre de la sesión.
            let cwd = cur_dir(m);
            if cwd.is_dir() {
                for anc in ancestors_set(&cwd) {
                    m.tree_expanded.insert(anc);
                }
                ensure_children_for_expanded(&mut m.tree_children, &m.tree_expanded);
            }
            let activa = m.active;
            m.sessions[activa].name = session_name(&cwd);
            record_history(m);
            // La nueva carpeta puede heredar vista iconos (folder format):
            // pedí sus miniaturas.
            if m.cur().view.is_grid() {
                request_thumbs(m, handle);
            }
        }
        Ok(Some(Opened::Leaf(id))) => {
            let nombre = m.cur().selected_node().map(|n| n.name.clone()).unwrap_or_default();
            let id_path = Path::new(&id);
            // Hoja POSIX (su id ES una ruta de archivo real):
            if id_path.is_file() {
                // Content-based: un `.img` wawa se MONTA (empuja su DAG);
                // cualquier otra cosa cae al open-with.
                match try_mount(id_path) {
                    Some(nav) => {
                        m.cur_pane_mut().nav_stack.push(nav);
                        clear_preview(m);
                    }
                    // Apertura integrada: texto → editor, imagen → visor con
                    // zoom, video/audio → media; el resto, preview derecho.
                    None => open_path(m, &id_path.to_path_buf()),
                }
            } else {
                // Hoja no-POSIX (wawa/nouser/minga): tempfile bridge.
                match m.cur().read(&id) {
                    Ok(bytes) => {
                        preview_from_bytes(m, bytes, &nombre);
                        m.viewer_open = true;
                    }
                    Err(_) => clear_preview(m),
                }
            }
        }
        Ok(None) | Err(_) => {}
    }
}

/// Registra la carpeta actual del panel activo en su historial (si cambió):
/// poda la cola forward y empuja el presente. Sólo carpetas POSIX.
fn record_history(m: &mut Model) {
    if m.is_foreign() {
        return;
    }
    let cwd = cur_dir(m);
    let pane = m.cur_pane_mut();
    if pane.hist.get(pane.hist_pos) == Some(&cwd) {
        return;
    }
    pane.hist.truncate(pane.hist_pos + 1);
    pane.hist.push(cwd);
    pane.hist_pos = pane.hist.len().saturating_sub(1);
}

/// Atrás/adelante por el historial del panel activo (delta = ±1), como un
/// navegador: moverse NO poda la cola. Revela la carpeta en el árbol.
fn nav_history_go(m: &mut Model, handle: &Handle<Msg>, delta: i64) {
    let pane = m.cur_pane();
    let destino = pane.hist_pos as i64 + delta;
    if destino < 0 || (destino as usize) >= pane.hist.len() {
        return;
    }
    let destino = destino as usize;
    let path = pane.hist[destino].clone();
    if !path.is_dir() {
        return;
    }
    {
        let pane = m.cur_pane_mut();
        pane.hist_pos = destino;
        pane.nav_stack = vec![posix_nav(&path)];
        pane.marked.clear();
    }
    m.canvas = None;
    apply_format(m);
    refresh_preview(m);
    for anc in ancestors_set(&path) {
        m.tree_expanded.insert(anc);
    }
    ensure_children_for_expanded(&mut m.tree_children, &m.tree_expanded);
    let activa = m.active;
    m.sessions[activa].name = session_name(&path);
    if m.cur().view.is_grid() {
        request_thumbs(m, handle);
    }
}

/// Tope de lectura para abrir un archivo en el editor del canvas. Más grande
/// que esto va al visor de texto del preview (read-only), no al editor.
const EDITOR_BYTES_MAX: u64 = 4 * 1024 * 1024;

/// Lenguaje de highlight por extensión (mismo mapeo que nada).
fn language_for_path(path: &Path) -> Language {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    Language::from_cell_language(ext)
}

/// Cuántas líneas del editor entran en el canvas.
fn canvas_editor_lines(m: &Model) -> usize {
    let (_, vh) = viewport_of(m);
    (((vh - 150.0) / (13.0 * 1.4)).floor() as usize).max(10)
}

/// Abre `path` de forma **integrada**: texto → editor potente en el canvas;
/// imagen → visor con zoom en el canvas; video/audio → player de media en el
/// canvas; cualquier otro tipo → el visor correspondiente en el sidebar
/// derecho de preview.
fn open_path(m: &mut Model, path: &PathBuf) {
    let pane = load_for(path);
    match pane {
        PreviewPane::Image(state) => {
            m.canvas = Some(CanvasApp::Imagen {
                path: path.clone(),
                state,
                viewport: ImageViewport::default(),
            });
        }
        PreviewPane::Video(state) => {
            m.canvas = Some(CanvasApp::Video(state));
        }
        PreviewPane::Audio(state) => {
            m.canvas = Some(CanvasApp::Audio(state));
        }
        PreviewPane::Text(_) | PreviewPane::Markdown(_) | PreviewPane::Web(_) => {
            // El HTML además lanza puriy (browser real), como siempre.
            if matches!(pane, PreviewPane::Web(_)) {
                launch_puriy(path);
            }
            let chico = std::fs::metadata(path).map(|md| md.len() <= EDITOR_BYTES_MAX);
            match chico.ok().filter(|c| *c).and_then(|_| std::fs::read_to_string(path).ok()) {
                Some(contenido) => {
                    let mut editor = EditorState::new();
                    editor.set_text(&contenido);
                    m.canvas = Some(CanvasApp::Texto {
                        path: path.clone(),
                        editor: Box::new(editor),
                        dirty: false,
                        saved: false,
                    });
                }
                // Muy grande o no-UTF8: visor de texto read-only a la derecha.
                None => open_in_preview(m, path, pane),
            }
        }
        otra => open_in_preview(m, path, otra),
    }
}

/// Deja `pane` como contenido del sidebar derecho de preview (y lo abre).
fn open_in_preview(m: &mut Model, path: &PathBuf, pane: PreviewPane) {
    m.preview = pane;
    m.basemap = open_basemap_if_pmtiles(path);
    m.basemap_dirty = m.basemap.is_some();
    m.preview_of = Some(path.clone());
    m.preview_temp = None;
    m.map_view.reset();
    m.map_view.color_field = None;
    m.viewer_open = true;
}

fn refresh_preview(m: &mut Model) {
    // Con el visor cerrado no hay nada que refrescar — y cargar/decodificar
    // en cada flecha sería I/O tirado. El visor carga fresco al abrirse
    // (OpenSelected / doble click).
    if !m.viewer_open {
        return;
    }
    // Resolvemos la acción soltando el préstamo de `cur()` antes de mutar el
    // preview (que toca el resto del modelo).
    enum Accion {
        Limpiar,
        Posix(PathBuf),
        Bytes(Vec<u8>, String),
    }
    let accion = match m.cur().selected_node() {
        Some(n) if !n.is_container => {
            let p = Path::new(&n.id);
            if p.is_file() {
                Accion::Posix(p.to_path_buf())
            } else {
                match m.cur().read(&n.id) {
                    Ok(bytes) => Accion::Bytes(bytes, n.name.clone()),
                    Err(_) => Accion::Limpiar,
                }
            }
        }
        _ => Accion::Limpiar,
    };
    match accion {
        Accion::Limpiar => clear_preview(m),
        Accion::Posix(path) => {
            m.preview = load_for(&path);
            m.basemap = open_basemap_if_pmtiles(&path);
            m.basemap_dirty = m.basemap.is_some();
            m.preview_of = Some(path);
            m.preview_temp = None;
            // Encuadre fresco para el nuevo archivo (si fuera un mapa).
            m.map_view.reset();
            m.map_view.color_field = None;
        }
        Accion::Bytes(bytes, nombre) => preview_from_bytes(m, bytes, &nombre),
    }
}

/// Decide qué viewer usar discerniendo el **contenido** del archivo (no
/// la extensión) y dispara la carga sync. Lee una muestra del header,
/// la pasa por `shuma-discern`, y `viewer_registry::pick` elige el visor.
/// Un .png con la extensión equivocada ahora se abre igual como imagen;
/// un archivo ilegible cae al text viewer (que degrada a "binario").
fn load_for(path: &Path) -> PreviewPane {
    let sample = read_header_sample(path, DISCERN_SAMPLE_BYTES);
    let pipeline = shuma_discern::DiscernPipeline::default();
    let hint = shuma_discern::Hint {
        path: path.to_str(),
        size_total: std::fs::metadata(path).ok().map(|m| m.len()),
    };
    let discernment = sample
        .as_deref()
        .and_then(|s| pipeline.discern(s, &hint));

    match viewer_registry::pick(discernment.as_ref()) {
        ViewerKind::Image => PreviewPane::Image(load_image(path, DEFAULT_IMAGE_BYTES_MAX)),
        ViewerKind::Video => PreviewPane::Video(open_video(path)),
        ViewerKind::Audio => PreviewPane::Audio(AudioViewerState::open(path)),
        ViewerKind::Card => PreviewPane::Card(load_card(path)),
        ViewerKind::Tree => PreviewPane::Tree(load_tree(path, DEFAULT_TREE_BYTES_MAX)),
        ViewerKind::Hex => PreviewPane::Hex(load_hex(path, DEFAULT_HEX_BYTES_MAX)),
        ViewerKind::Table => PreviewPane::Table(load_table(path, DEFAULT_TABLE_BYTES_MAX)),
        ViewerKind::Markdown => {
            PreviewPane::Markdown(load_markdown(path, DEFAULT_MARKDOWN_BYTES_MAX))
        }
        ViewerKind::Archive => PreviewPane::Archive(load_archive(path)),
        ViewerKind::Font => PreviewPane::Font(load_font(path, DEFAULT_FONT_BYTES_MAX)),
        ViewerKind::Map => PreviewPane::Map(load_map(path, DEFAULT_MAP_BYTES_MAX)),
        ViewerKind::Text => PreviewPane::Text(load_preview(path, DEFAULT_PREVIEW_BYTES_MAX)),
        // El panel muestra el fuente; el render lo hace puriy al abrir.
        ViewerKind::Web => PreviewPane::Web(load_preview(path, DEFAULT_PREVIEW_BYTES_MAX)),
    }
}

/// Lanza puriy (el navegador de la suite) sobre un archivo HTML local,
/// fuera de proceso, como un file manager abre el visor por defecto. La
/// ruta se entrega como `file://<abs>` (puriy resuelve `file://`). El
/// binario es `puriy`; `$PURIY_BIN` lo override (útil en dev:
/// `PURIY_BIN=target/debug/puriy`). Un fallo al spawnear se reporta a
/// stderr y no interrumpe el shell.
fn launch_puriy(path: &Path) {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let url = format!("file://{}", abs.display());
    let bin = std::env::var("PURIY_BIN").unwrap_or_else(|_| "puriy".to_string());
    match std::process::Command::new(&bin).arg(&url).spawn() {
        Ok(_) => {}
        Err(e) => eprintln!("[nahual] no pude lanzar puriy ({bin}) sobre {url}: {e}"),
    }
}

/// Abre un archivo de video con el constructor adecuado del visor. El
/// contenido ya se discernió como video; acá la extensión sólo decide
/// el *demuxer*: WebM/MKV (EBML) van por `media-source-webm`, el resto
/// (incluido `.ivf`) por el path AV1 crudo. Si la extensión miente, el
/// visor cae a estado de error y lo muestra en su header.
fn open_video(path: &Path) -> VideoViewerState {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("webm" | "mkv") => VideoViewerState::open_webm(path),
        Some("gif") => VideoViewerState::open_gif(path),
        _ => VideoViewerState::open_av1(path),
    }
}

/// Cuántos bytes del header alcanzan a `shuma-discern`. Los magic-bytes y
/// el arranque de JSON/TOML viven en los primeros KB; no hace falta leer
/// el archivo entero sólo para elegir visor.
const DISCERN_SAMPLE_BYTES: usize = 8 * 1024;

/// Lee hasta `max` bytes del inicio del archivo para discernir su tipo.
/// `None` si no se puede abrir/leer — el caller lo trata como "sin
/// discernimiento" y cae al text viewer.
fn read_header_sample(path: &Path, max: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patron_tokens_basicos() {
        // {name} = stem, {ext} = extensión, {n} = contador.
        assert_eq!(aplicar_patron("{name}.{ext}", "foto.png", 1), "foto.png");
        assert_eq!(aplicar_patron("img_{n}.{ext}", "foto.png", 3), "img_3.png");
        assert_eq!(aplicar_patron("{name}_copia", "notas.md", 1), "notas_copia");
    }

    #[test]
    fn patron_sin_extension() {
        // Un archivo sin extensión: {ext} queda vacío, {name} es el nombre.
        assert_eq!(aplicar_patron("{name}-{n}", "LICENSE", 7), "LICENSE-7");
        assert_eq!(aplicar_patron("{name}.{ext}", "LICENSE", 1), "LICENSE.");
    }

    #[test]
    fn batch_nuevo_nombre_respeta_vacio() {
        // Un patrón que rinde vacío conserva el original (no renombra a nada).
        let b = BatchRename {
            pattern: String::new(),
            targets: vec![("/x/a.txt".into(), "a.txt".into())],
        };
        assert_eq!(b.nuevo_nombre(0), "a.txt");
    }

    #[test]
    fn batch_contador_es_uno_based_y_ordenado() {
        let b = BatchRename {
            pattern: "f{n}".to_string(),
            targets: vec![
                ("/x/a".into(), "a".into()),
                ("/x/b".into(), "b".into()),
                ("/x/c".into(), "c".into()),
            ],
        };
        assert_eq!(b.nuevo_nombre(0), "f1");
        assert_eq!(b.nuevo_nombre(1), "f2");
        assert_eq!(b.nuevo_nombre(2), "f3");
    }
}
