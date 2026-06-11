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

use std::collections::BTreeSet;
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
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_theme::Theme;
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_breadcrumb::{breadcrumb_view, BreadcrumbPalette};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use app_bus::{AppMenu, AppRegistry, Menu, MenuItem};
use nahual_source_core::{
    MingaSource, Navigator, Node, NodeKind, NouserSource, Opened, PosixSource, WawaImgSource,
};
use nahual_image_viewer_llimphi::{
    image_viewer_view, load_image, ImagePreviewState, ImageViewerPalette,
    DEFAULT_IMAGE_BYTES_MAX,
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
}

impl Pane {
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

struct Model {
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
    /// Navega el panel enfocado a una ruta POSIX absoluta (favorito/reciente).
    GoTo(String),
    /// Agrega a favoritos la carpeta seleccionada (o la actual si no es dir).
    AddPlace,
    /// Quita un favorito por su ruta.
    RemovePlace(String),
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
        Model {
            // Ambos paneles arrancan en el cwd POSIX; el 1 se ve sólo en dual.
            panes: [
                Pane { nav_stack: vec![posix_nav(&cwd)], marked: BTreeSet::new() },
                Pane { nav_stack: vec![posix_nav(&cwd)], marked: BTreeSet::new() },
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
                if m.cur_mut().select(idx) {
                    refresh_preview(&mut m);
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
                // Abrir la selección por el navegador activo (POSIX o fuente
                // montada): contenedor → descender; hoja → montar/previsualizar.
                match m.cur_mut().open_selected() {
                    Ok(Some(Opened::Descended)) => {
                        m.cur_pane_mut().marked.clear();
                        clear_preview(&mut m);
                        apply_format(&mut m);
                        record_recent(&mut m);
                    }
                    Ok(Some(Opened::Leaf(id))) => {
                        let nombre =
                            m.cur().selected_node().map(|n| n.name.clone()).unwrap_or_default();
                        let id_path = Path::new(&id);
                        // Hoja POSIX (su id ES una ruta de archivo real):
                        if id_path.is_file() {
                            // Content-based: un `.img` wawa se MONTA (empuja su
                            // DAG); cualquier otra cosa cae al open-with.
                            match try_mount(id_path) {
                                Some(nav) => {
                                    m.cur_pane_mut().nav_stack.push(nav);
                                    clear_preview(&mut m);
                                }
                                None => {
                                    let path = id_path.to_path_buf();
                                    m.preview = load_for(&path);
                                    m.basemap = open_basemap_if_pmtiles(&path);
                                    m.basemap_dirty = m.basemap.is_some();
                                    if matches!(m.preview, PreviewPane::Web(_)) {
                                        launch_puriy(&path);
                                    }
                                    m.preview_of = Some(path);
                                    m.preview_temp = None;
                                    m.map_view.reset();
                                    m.map_view.color_field = None;
                                }
                            }
                        } else {
                            // Hoja no-POSIX (wawa/nouser/minga): tempfile bridge.
                            match m.cur().read(&id) {
                                Ok(bytes) => preview_from_bytes(&mut m, bytes, &nombre),
                                Err(_) => clear_preview(&mut m),
                            }
                        }
                    }
                    Ok(None) | Err(_) => {}
                }
            }
            Msg::Parent => {
                m.cur_pane_mut().marked.clear();
                match m.cur_mut().parent() {
                    Ok(true) => {
                        apply_format(&mut m);
                        refresh_preview(&mut m);
                    }
                    // Subir desde la raíz de una fuente montada la desmonta
                    // (vuelve al nivel de abajo de la pila). En POSIX, la raíz
                    // es `/` y no hay a dónde subir.
                    Ok(false) => {
                        if m.is_foreign() {
                            m.cur_pane_mut().nav_stack.pop();
                            clear_preview(&mut m);
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
                let nav = m.cur_mut();
                nav.view = match nav.view {
                    nahual_source_core::ViewMode::List => nahual_source_core::ViewMode::Details,
                    nahual_source_core::ViewMode::Details => nahual_source_core::ViewMode::List,
                };
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
                    apply_format(&mut m);
                    refresh_preview(&mut m);
                }
            }
            Msg::ResizeList(dx) => {
                m.list_width = (m.list_width + dx).clamp(220.0, 900.0);
            }
            Msg::Scroll(steps) => {
                // El navegador activo tiene su propio acumulador para touchpads
                // — le pasamos el delta crudo (en líneas).
                m.cur_mut().apply_wheel(steps as f32);
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
            Msg::TogglePlay => match &mut m.preview {
                PreviewPane::Video(state) => state.toggle_play(),
                PreviewPane::Audio(state) => state.toggle_play(),
                _ => {}
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
            Msg::GoTo(path) => {
                let p = PathBuf::from(&path);
                if p.is_dir() {
                    // Reemplaza la pila del panel enfocado por un POSIX fresco en
                    // esa ruta (desmonta cualquier fuente no-POSIX activa).
                    m.cur_pane_mut().nav_stack = vec![posix_nav(&p)];
                    m.cur_pane_mut().marked.clear();
                    apply_format(&mut m);
                    record_recent(&mut m);
                    refresh_preview(&mut m);
                }
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
            Msg::RemovePlace(path) => {
                m.state.remove_place(&path);
                m.state.save();
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

        // Modo simple: panel 0 (con su breadcrumb) + visor a la derecha.
        // Modo dual: dos columnas de archivos, la enfocada resaltada.
        let body = if model.dual {
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
            splitter_two(
                Direction::Row,
                pane_column(model, 0, false, &theme),
                PaneSize::Fixed(model.list_width),
                viewer_pane,
                PaneSize::Flex,
                |phase, dx| match phase {
                    DragPhase::Move => Some(Msg::ResizeList(dx)),
                    DragPhase::End => None,
                },
                &splitter_palette,
            )
        };

        // El cuerpo ocupa el alto restante (flex), dejando lugar al panel de
        // la cola de operaciones abajo si hay jobs. A la izquierda, el sidebar
        // de favoritos/recientes (ancho fijo); el resto es el split de paneles.
        let body_inner = View::new(Style {
            flex_grow: 1.0,
            min_size: Size { width: length(0.0), height: length(0.0) },
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![body]);
        let body_wrap = View::new(Style {
            flex_grow: 1.0,
            min_size: Size { width: length(0.0), height: length(0.0) },
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![sidebar_view(model, &theme), body_inner]);

        let mut col: Vec<View<Msg>> = vec![menubar, body_wrap];
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

/// Basename legible de una ruta POSIX (para las filas del sidebar). La raíz
/// `/` se muestra como tal.
fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Sidebar izquierdo (Fase 4.5): **favoritos** (places) + **recientes**. Cada
/// fila navega el panel enfocado a esa carpeta (`GoTo`); los favoritos llevan
/// una `✕` para quitarlos. Ancho fijo.
fn sidebar_view(model: &Model, theme: &Theme) -> View<Msg> {
    const WIDTH: f32 = 190.0;

    // Encabezado de sección.
    let seccion = |titulo: &str| {
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            padding: pad_h(12.0),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(titulo, 12.0, theme.fg_muted)
    };

    let mut hijos: Vec<View<Msg>> = vec![seccion("FAVORITOS")];

    if model.state.places.is_empty() {
        hijos.push(
            View::new(fila(22.0))
                .text("  (sin favoritos)", 12.0, theme.fg_placeholder),
        );
    } else {
        for path in &model.state.places {
            // Fila: nombre (navega) + ✕ (quita). Dos targets de click.
            let nombre = View::new(Style {
                flex_grow: 1.0,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                padding: pad_h(12.0),
                ..Default::default()
            })
            .on_click(Msg::GoTo(path.clone()))
            .text(format!("★ {}", basename(path)), 13.0, theme.fg_text);
            let quitar = View::new(Style {
                size: Size { width: length(24.0_f32), height: percent(1.0_f32) },
                justify_content: Some(JustifyContent::Center),
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .on_click(Msg::RemovePlace(path.clone()))
            .text("✕", 12.0, theme.fg_muted);
            hijos.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                })
                .children(vec![nombre, quitar]),
            );
        }
    }

    hijos.push(seccion("RECIENTES"));
    if model.state.recents.is_empty() {
        hijos.push(
            View::new(fila(22.0)).text("  (vacío)", 12.0, theme.fg_placeholder),
        );
    } else {
        for path in model.state.recents.iter().take(10) {
            hijos.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
                    align_items: Some(AlignItems::Center),
                    padding: pad_h(12.0),
                    ..Default::default()
                })
                .on_click(Msg::GoTo(path.clone()))
                .text(format!("🕘 {}", basename(path)), 12.5, theme.fg_muted),
            );
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(WIDTH), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(hijos)
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
    let content = nav_pane_view(
        model.panes[pane].nav(),
        &model.panes[pane].marked,
        &model.state,
        theme,
        filtering,
        pane,
    );
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
        details: matches!(nav.view, nahual_source_core::ViewMode::Details),
        sort_col: sortkey_to_col(key),
        sort_asc: matches!(dir, nahual_source_core::SortDir::Asc),
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
        nav.view = if fmt.details {
            nahual_source_core::ViewMode::Details
        } else {
            nahual_source_core::ViewMode::List
        };
        nav.set_sort_to(col_to_sortkey(fmt.sort_col), fmt.sort_asc);
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
    let src = WawaImgSource::abrir(path).ok()?;
    Navigator::open(Box::new(src)).ok()
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
    }
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
            "{} entradas · ↑↓ navega · Enter abre · ⌫ vuelve · v detalle · / filtra",
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
            let icon = if n.is_container { "▸ " } else { "  " };
            let label = if n.is_container {
                format!("{mark}{dot}{icon}{}/", n.name)
            } else {
                format!("{mark}{dot}{icon}{}", n.name)
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
            DetailRow {
                cells: vec![
                    format!("{mark}{icon} {dot}{}", n.name),
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
fn refresh_preview(m: &mut Model) {
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
