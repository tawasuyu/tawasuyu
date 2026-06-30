//! Modelo del shell nahual: tipos, estado, mensajes y sus impls. Movido de
//! `main.rs` en el split de 2026-06-12 (puro movimiento de código).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nahual_source_core::{Navigator, Node, NodeKind, PosixSource};
use llimphi_ui::KeyEvent;
use llimphi_widget_text_editor::{EditorState, PointerEvent};
use tullpu_module as tullpu;
use media_module as mediamod;
use nahual_thumb_core::ThumbRgba;
use llimphi_ui::llimphi_raster::peniko::ImageBrush as Image;
use nahual_map_viewer_llimphi::{MapView, Basemap, MapPreview};
use nahual_text_viewer_llimphi::PreviewState;
use nahual_image_viewer_llimphi::ImagePreviewState;
use nahual_video_viewer_llimphi::VideoViewerState;
use nahual_audio_viewer_llimphi::AudioViewerState;
use nahual_card_viewer_llimphi::CardPreview;
use nahual_tree_viewer_llimphi::TreePreview;
use nahual_hex_viewer_llimphi::HexPreview;
use nahual_table_viewer_llimphi::TablePreview;
use nahual_markdown_viewer_llimphi::MarkdownPreview;
use nahual_archive_viewer_llimphi::ArchivePreview;
use nahual_font_viewer_llimphi::FontPreview;
use llimphi_theme::Theme;
use llimphi_motion::Tween;
use llimphi_module_command_palette::{Command as PaletteCommand, PaletteMsg, PaletteState};
use app_bus::AppRegistry;
use crate::ops::{OpKind, OpQueue};
use crate::state::{Label, ShellState};
use crate::helpers::ensure_children_for_expanded;

/// Qué viewer pinta el panel derecho. Lo decide [`crate::viewer_registry::pick`]
/// sobre el `Discernment` del **contenido** (no la extensión); los
/// archivos sin match caen como `Text` y el text viewer los muestra como
/// binarios si no son UTF-8 — fallback que pasa por la guard de
/// `load_preview`.
pub(crate) enum PreviewPane {
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
pub(crate) const FRAME_TICK: Duration = Duration::from_millis(33);

/// Alto de cada fila del árbol lateral (px) — debe coincidir con el
/// `row_height` que se le pasa a `tree_view`, para que el ventaneo cuadre.
pub(crate) const TREE_ROW_H: f32 = 22.0;

/// Ancho del rail de dientes (sesiones de trabajo), px.
pub(crate) const SESSION_RAIL_W: f32 = 40.0;

/// Intervalo mínimo entre re-streams del basemap PMTiles (debounce): los
/// pans/zooms se acumulan y se recalcula el viewport a lo sumo cada tanto,
/// para no rehacer la fusión de tiles en cada evento de arrastre.
pub(crate) const RESTREAM_THROTTLE: Duration = Duration::from_millis(90);

/// Un panel del file manager: su propia pila de navegación (mount stack). En
/// modo simple sólo el panel 0 se ve (panel 1 = visor); en modo dual ambos son
/// listas de archivos lado a lado (Fase 4.2c).
pub(crate) struct Pane {
    /// Pila de navegación: `[0]` = base POSIX (anclada en `/`, arrancada en el
    /// cwd con miga completa); montar una fuente no-POSIX empuja, desmontar
    /// saca. El navegador activo del panel es el tope. Nunca vacía.
    pub(crate) nav_stack: Vec<Navigator>,
    /// Selección **múltiple** marcada (Insert): ids de nodos sobre los que
    /// actúan las operaciones por lote (borrar/copiar/mover). Vacía = la
    /// operación recae sobre el cursor (`selected`). Se limpia al cambiar de
    /// directorio o tras ejecutar una operación.
    pub(crate) marked: BTreeSet<nahual_source_core::NodeId>,
    /// Historial de navegación estilo browser (sólo carpetas POSIX): la
    /// posición `hist_pos` es el presente; back/forward se mueven por acá
    /// sin truncar, y navegar a un lugar nuevo poda la cola forward.
    pub(crate) hist: Vec<PathBuf>,
    /// Posición actual dentro de `hist`.
    pub(crate) hist_pos: usize,
}

impl Pane {
    /// Panel vacío transitorio: sólo se usa como placeholder durante el swap
    /// de sesiones (`std::mem::replace`). Su `nav_stack` está vacío, así que
    /// **nunca** se le debe llamar `nav()`/`nav_mut()` mientras es placeholder.
    pub(crate) fn empty() -> Self {
        Pane { nav_stack: Vec::new(), marked: BTreeSet::new(), hist: Vec::new(), hist_pos: 0 }
    }

    pub(crate) fn nav(&self) -> &Navigator {
        self.nav_stack.last().expect("nav_stack nunca vacía")
    }
    pub(crate) fn nav_mut(&mut self) -> &mut Navigator {
        self.nav_stack.last_mut().expect("nav_stack nunca vacía")
    }
    pub(crate) fn is_foreign(&self) -> bool {
        self.nav_stack.len() > 1
    }
    /// Los ids objetivo de una operación por lote: la marca si hay, si no el
    /// nodo bajo el cursor. Cada uno con su nombre, para el rótulo del job.
    pub(crate) fn op_targets(&self) -> Vec<(nahual_source_core::NodeId, String)> {
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
pub(crate) struct Prompt {
    pub(crate) kind: PromptKind,
    pub(crate) text: String,
}

/// Qué operación dispara el [`Prompt`] al confirmarse.
pub(crate) enum PromptKind {
    /// Crear un directorio dentro del id contenedor.
    NewDir { parent: nahual_source_core::NodeId },
    /// Crear un archivo vacío dentro del id contenedor.
    NewFile { parent: nahual_source_core::NodeId },
    /// Renombrar el nodo `id` (el texto arranca con su nombre actual).
    Rename { id: nahual_source_core::NodeId },
    /// Seleccionar (marcar) por patrón: el texto es un glob (`*.png`, `foto*`).
    /// No opera sobre el filesystem — marca los hijos visibles que matchean.
    SelectPattern,
    /// **Submonadizar**: el texto es el nombre de la Mónada hija que agrupa a
    /// `members` (la selección) dentro de la Mónada `parent` (la actual). No
    /// toca el filesystem — reorganiza el grafo vía `MonadGraphMut`.
    Submonadize { parent: nahual_source_core::NodeId, members: Vec<nahual_source_core::NodeId> },
    /// **Renombrar una Mónada**: el texto es el nuevo nombre de la Mónada `id`.
    /// Edición de grafo (no de archivo), vía `MonadGraphMut::rename_monad`.
    RenameMonad { id: nahual_source_core::NodeId },
}

impl Prompt {
    /// Título humano localizado del overlay.
    pub(crate) fn title(&self) -> String {
        let key = match self.kind {
            PromptKind::NewDir { .. } => "nahual-shell-new-dir",
            PromptKind::NewFile { .. } => "nahual-shell-new-file",
            PromptKind::Rename { .. } => "nahual-shell-rename",
            PromptKind::SelectPattern => "nahual-shell-select-pattern-title",
            PromptKind::Submonadize { .. } => "nahual-shell-submonadize",
            PromptKind::RenameMonad { .. } => "nahual-shell-rename-monad",
        };
        rimay_localize::t(key)
    }
}

/// Estado del **renombrado por lote** (Fase 4.5): un patrón en edición + los
/// nodos objetivo (la marca del panel). El patrón soporta tokens `{name}`
/// (nombre sin extensión), `{ext}` (extensión sin punto) y `{n}` (contador
/// 1-based, en el orden de los objetivos). El overlay pinta la previsualización
/// `viejo → nuevo` antes de aplicar.
pub(crate) struct BatchRename {
    /// Patrón en edición (p. ej. `foto_{n}.{ext}`).
    pub(crate) pattern: String,
    /// `(id, nombre_original)` de cada nodo a renombrar, en orden estable.
    pub(crate) targets: Vec<(nahual_source_core::NodeId, String)>,
    /// Nombres **explícitos** (alineados a `targets`), cuando los propone la IA
    /// en vez de derivarse de un patrón. `Some` desactiva el patrón: el overlay
    /// es de revisión (old→new), no de edición de tokens.
    pub(crate) explicit: Option<Vec<String>>,
}

impl BatchRename {
    /// Crea un batch por **patrón** (el flujo clásico, tokens `{name}` etc.).
    pub(crate) fn por_patron(
        pattern: String,
        targets: Vec<(nahual_source_core::NodeId, String)>,
    ) -> Self {
        Self { pattern, targets, explicit: None }
    }

    /// Crea un batch con **nombres explícitos** (los propone la IA). `names`
    /// debe estar alineado a `targets`.
    pub(crate) fn explicitos(
        targets: Vec<(nahual_source_core::NodeId, String)>,
        names: Vec<String>,
    ) -> Self {
        Self { pattern: String::new(), targets, explicit: Some(names) }
    }

    /// `true` si los nombres vienen de la IA (no de un patrón) — el overlay
    /// cambia de "editar patrón" a "revisar propuesta".
    pub(crate) fn es_ia(&self) -> bool {
        self.explicit.is_some()
    }

    /// Calcula el nuevo nombre del objetivo `idx`: el explícito si lo hay, o el
    /// patrón aplicado al nombre original. Vacío conserva el original (no se
    /// renombra a "nada").
    pub(crate) fn nuevo_nombre(&self, idx: usize) -> String {
        let original = &self.targets[idx].1;
        let out = match &self.explicit {
            Some(names) => names.get(idx).cloned().unwrap_or_else(|| original.clone()),
            None => aplicar_patron(&self.pattern, original, idx + 1),
        };
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
pub(crate) fn aplicar_patron(pattern: &str, original: &str, n: usize) -> String {
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
pub(crate) enum CanvasApp {
    Texto { path: PathBuf, editor: Box<EditorState>, dirty: bool, saved: bool },
    /// Editor de imágenes por capas (tullpu-module sobre tullpu-core/render/
    /// ops/paint): pincel, ops derivadas, capas, undo, guardar.
    Imagen(Box<tullpu::State>),
    /// Player de media embebible (media-module) con controles en dientes.
    Media(Box<mediamod::State>),
}

// Los tipos de dominio del find (`FindMode`/`SemIndex`/`FindHit`) y el motor de
// búsqueda viven en el core agnóstico `nahual-shell-core` (Regla 2). Se
// re-exportan acá para que el resto del crate los siga viendo como
// `crate::modelo::*` sin cambios. `FindState` (abajo) es estado de UI y se queda.
pub(crate) use nahual_shell_core::{FindHit, FindMode, SemIndex};

/// Estado del **find recursivo** (Ctrl+F): un modal que camina el árbol bajo
/// `root` en un worker y lista los matches. `gen` descarta resultados de
/// búsquedas viejas (si el usuario relanza antes de que termine la anterior).
pub(crate) struct FindState {
    pub(crate) query: String,
    pub(crate) mode: FindMode,
    pub(crate) root: PathBuf,
    pub(crate) results: Vec<FindHit>,
    pub(crate) selected: usize,
    pub(crate) searching: bool,
    /// Generación de la búsqueda en curso (monótona).
    pub(crate) gen: u64,
    /// El `(query, mode)` con que se lanzó la última búsqueda — para que Enter
    /// distinga "correr" de "abrir el resultado seleccionado".
    pub(crate) ran: Option<(String, FindMode)>,
}

impl FindState {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            query: String::new(),
            mode: FindMode::Name,
            root,
            results: Vec::new(),
            selected: 0,
            searching: false,
            gen: 0,
            ran: None,
        }
    }
}

/// Estado del panel de **IA** (acción "Preguntar a la IA sobre la selección"):
/// el título del contexto, la respuesta (cuando llega) y si sigue en vuelo.
pub(crate) struct AiState {
    pub(crate) titulo: String,
    pub(crate) respuesta: Option<String>,
    pub(crate) pendiente: bool,
}

/// Clipboard del sistema para el editor del canvas (mismo backend que nada).
pub(crate) struct ShellClipboard {
    pub(crate) inner: Option<arboard::Clipboard>,
}

impl ShellClipboard {
    pub(crate) fn new() -> Self {
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

/// Qué hace la **rueda** con una app abierta en el canvas (modo alternable
/// desde el toolbox): `Zoom` = la rueda es de la app (zoom del lienzo de
/// imagen, scroll del editor); `Lista` = pasa al archivo siguiente/anterior
/// de la carpeta (como un visor de fotos). Los botones atrás/adelante de la
/// navegación también pasan de archivo mientras hay canvas abierto.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WheelMode {
    Zoom,
    Lista,
}

/// Estado *movible* de una sesión de trabajo (diente del rail): todo lo que
/// cambia al saltar de una sesión a otra. La sesión activa **no** guarda su
/// snap aquí — sus campos viven directamente en `Model` (los `panes`,
/// `preview`, el árbol expandido, etc.). Al cambiar de diente se hace swap:
/// los campos vivos del `Model` se vuelcan a un `SessionSnap` y los de la
/// sesión destino se restauran.
pub(crate) struct SessionSnap {
    pub(crate) panes: [Pane; 2],
    pub(crate) focus: usize,
    pub(crate) dual: bool,
    pub(crate) list_width: f32,
    pub(crate) nav_filtering: bool,
    pub(crate) preview: PreviewPane,
    pub(crate) preview_of: Option<PathBuf>,
    pub(crate) preview_temp: Option<tempfile::TempDir>,
    pub(crate) map_view: MapView,
    pub(crate) basemap: Option<Basemap>,
    pub(crate) basemap_dirty: bool,
    pub(crate) last_restream: Option<Instant>,
    /// Carpetas descolapsadas del árbol lateral — **por sesión**.
    pub(crate) tree_expanded: BTreeSet<PathBuf>,
    /// Offset de scroll del árbol lateral (en filas) — por sesión.
    pub(crate) tree_scroll: usize,
    /// `true` = el sidebar derecho de preview está abierto.
    pub(crate) viewer_open: bool,
    /// `true` = el diente derecho de tools (tullpu) está abierto.
    pub(crate) tools_open: bool,
    /// App integrada abierta en el canvas (editor/imagen/media), si hay.
    pub(crate) canvas: Option<CanvasApp>,
}

/// Una sesión de trabajo, representada por un **diente** del rail. `snap` es
/// `None` para la sesión **activa** (sus campos están vivos en `Model`) y
/// `Some(_)` para las inactivas.
pub(crate) struct Session {
    pub(crate) name: String,
    pub(crate) snap: Option<SessionSnap>,
}

pub(crate) struct Model {
    /// Sesiones de trabajo abiertas (los dientes del rail). `sessions[active]`
    /// es la viva (sus campos de navegación/preview/árbol están en los campos
    /// sueltos de abajo).
    pub(crate) sessions: Vec<Session>,
    /// Índice de la sesión activa.
    pub(crate) active: usize,
    /// Carpetas descolapsadas del árbol lateral de la sesión activa.
    pub(crate) tree_expanded: BTreeSet<PathBuf>,
    /// Offset de scroll del árbol lateral (en filas) de la sesión activa.
    pub(crate) tree_scroll: usize,
    /// Cache **global** de subcarpetas por carpeta (sólo directorios, ya
    /// ordenados). No es por-sesión: el contenido de un dir es el mismo para
    /// todas; sólo el set de expandidas cambia por sesión.
    pub(crate) tree_children: HashMap<PathBuf, Vec<PathBuf>>,
    /// Ancho del sidebar (árbol de carpetas) en px. Lo muta su splitter.
    pub(crate) tree_w: f32,
    /// Ancho del sidebar derecho del visor (preview), px. Lo muta su splitter.
    pub(crate) preview_w: f32,
    /// `true` = el sidebar derecho de preview está abierto (lo togglea su
    /// diente; Esc/⌫ también lo cierra). Por sesión (va al snap).
    pub(crate) viewer_open: bool,
    /// `true` = el diente derecho de **tools** del editor de imágenes está
    /// abierto (sólo aplica con `CanvasApp::Imagen`). Excluyente con
    /// `viewer_open`: comparten el panel derecho. Por sesión (va al snap).
    pub(crate) tools_open: bool,
    /// Ancho del panel de tools del diente derecho, px. Lo muta su splitter.
    pub(crate) tools_w: f32,
    /// Modo de la rueda con una app de canvas abierta (toolbox). Global.
    pub(crate) wheel_mode: WheelMode,
    /// Tamaño real de la ventana en px (lo actualiza `on_resize`) — de acá
    /// salen las columnas de la grilla, el ventaneo del árbol y el clamp de
    /// los overlays. Antes era `initial_size()` constante y la grilla
    /// quedaba clavada en las columnas del tamaño inicial.
    pub(crate) win: (f32, f32),
    /// App integrada abierta en el canvas (editor/imagen/media), si hay.
    /// Por sesión (va al snap). Esc/⌫ la cierra.
    pub(crate) canvas: Option<CanvasApp>,
    /// Clipboard del sistema para el editor del canvas. Transitorio.
    pub(crate) clipboard: ShellClipboard,
    /// Último click en una fila (pane, idx, instante) — para detectar el
    /// doble click que abre carpeta/archivo. Transitorio, no va al snap.
    pub(crate) last_click: Option<(usize, usize, Instant)>,
    /// Acumulador del drag del editor del canvas (como `drag_accum` de nada).
    pub(crate) canvas_drag: (f32, f32),
    /// Los dos paneles (Fase 4.2c). `panes[focus]` es el activo (recibe
    /// teclado). En modo simple sólo se ve el 0; en dual, ambos.
    pub(crate) panes: [Pane; 2],
    /// Panel activo: 0 o 1.
    pub(crate) focus: usize,
    /// `true` = dos paneles de archivos lado a lado; `false` = panel + visor.
    pub(crate) dual: bool,
    /// Ancho del panel izquierdo en px. Lo muta el drag del splitter.
    pub(crate) list_width: f32,
    /// `true` mientras se teclea el filtro vivo sobre la fuente montada
    /// (entra con `/`, sale con Esc/Enter). El teclado se captura al filtro.
    pub(crate) nav_filtering: bool,
    pub(crate) preview: PreviewPane,
    /// Path del archivo previsualizado (header del panel derecho).
    pub(crate) preview_of: Option<PathBuf>,
    /// Materialización temporal de una hoja no-POSIX: los visores son
    /// path-based (`load_image(path)`), así que los bytes de un objeto wawa
    /// se vuelcan a un tempfile y se previsualizan por ahí. Vive mientras el
    /// visor lo lea (audio/video streamean del path); se reemplaza al cambiar
    /// de preview.
    pub(crate) preview_temp: Option<tempfile::TempDir>,
    pub(crate) theme: Theme,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    pub(crate) menu_open: Option<usize>,
    /// Fila activa dentro del dropdown abierto (`usize::MAX` = ninguna).
    pub(crate) menu_active: usize,
    /// Animación de aparición del dropdown.
    pub(crate) menu_anim: Tween<f32>,
    /// Menú contextual sobre el nodo/archivo seleccionado: ancla `(x, y)`
    /// en coords de ventana. `None` cerrado. No hay edición de texto en el
    /// shell, así que el contextual lista acciones de navegación/montaje.
    pub(crate) context_menu: Option<(f32, f32)>,
    /// Cámara del visor de mapas (zoom/pan). Se resetea al cambiar de
    /// preview; la mutan el arrastre y la rueda sobre el panel del mapa.
    pub(crate) map_view: MapView,
    /// Basemap PMTiles vivo, si el archivo abierto es un `.pmtiles`. Mantiene
    /// el contenedor + caché de tiles para el streaming por viewport.
    pub(crate) basemap: Option<Basemap>,
    /// La cámara cambió y el basemap necesita re-streamear. El Tick lo procesa
    /// con throttle (debounce): coalesce muchos pans en pocos recálculos.
    pub(crate) basemap_dirty: bool,
    /// Último instante en que se re-streameó (para el throttle).
    pub(crate) last_restream: Option<Instant>,
    /// Suscripción al bus de configuración del SO.
    pub(crate) _wawa_watcher: Option<wawa_config::ConfigWatcher>,
    /// Catálogo de apps de la suite (AppBus): qué app abre qué mime. Se
    /// consulta al abrir el menú contextual sobre un archivo (open-with).
    pub(crate) registry: AppRegistry,
    /// Opciones "Abrir con <app>" precomputadas al abrir el contextual:
    /// `(app_id, label)`. El render del menú las pinta sin tocar el registro.
    pub(crate) ctx_open_with: Vec<(String, String)>,
    /// El archivo que el contextual abriría: ruta POSIX real, o un tempfile
    /// materializado de una hoja no-POSIX (Mónada/wawa). `None` si la
    /// selección no es un archivo abrible.
    pub(crate) ctx_target: Option<PathBuf>,
    /// Tempfile de la hoja no-POSIX materializada (lo mantiene vivo mientras
    /// la app externa lo lee). Se reemplaza al recomputar el contextual.
    pub(crate) ctx_temp: Option<tempfile::TempDir>,
    /// Cola de operaciones de archivo en vuelo / historial (Fase 4.3). El panel
    /// inferior colapsable la lista.
    pub(crate) queue: OpQueue,
    /// Pedido de nombre activo (nueva carpeta/archivo, renombrar). `None` =
    /// sin overlay; mientras esté `Some`, el teclado va al texto.
    pub(crate) prompt: Option<Prompt>,
    /// Confirmación de borrado pendiente: los `(id, nombre)` a borrar. `None` =
    /// sin diálogo. Borrar es destructivo, así que pasa por este sí/no.
    pub(crate) confirm_delete: Option<Vec<(nahual_source_core::NodeId, String)>>,
    /// Renombrado por lote en curso (Fase 4.5): patrón + objetivos + preview.
    /// `None` = sin overlay; mientras esté `Some`, el teclado va al patrón.
    pub(crate) batch: Option<BatchRename>,
    /// Preferencias persistidas (Fase 4.5): labels de color por archivo,
    /// favoritos, recientes, folder formats. Se relee al arrancar y se reescribe
    /// tras cada cambio.
    pub(crate) state: ShellState,
    /// Cache RAM de miniaturas listas para pintar (vista iconos, Fase 4.8).
    /// Clave = ruta POSIX del archivo. Se llena async vía `Handle::spawn`.
    pub(crate) thumbs: HashMap<PathBuf, Image>,
    /// Miniaturas pedidas y aún en vuelo (dedup: no relanzar el mismo path).
    pub(crate) thumbs_pending: HashSet<PathBuf>,
    /// Miniaturas que fallaron al generarse (se pinta un ⚠, no se reintenta).
    pub(crate) thumbs_failed: HashSet<PathBuf>,
    /// Panel de IA (acción LLM sobre la selección): `None` cerrado.
    pub(crate) ai: Option<AiState>,
    /// Find recursivo (Ctrl+F): `None` cerrado. Mientras esté `Some`, captura
    /// todo el teclado (es un modal).
    pub(crate) find: Option<FindState>,
    /// Índice de embeddings de una carpeta (búsqueda semántica instantánea).
    /// `None` = sin índice (la semántica embebe por consulta). Se invalida solo
    /// cuando la búsqueda se posa en otra carpeta.
    pub(crate) sem_index: Option<SemIndex>,
    /// `true` mientras un índice se está construyendo en background.
    pub(crate) sem_indexing: bool,
    /// Command palette (Ctrl+Shift+P / Ctrl+P): `None` cerrado. Mientras esté
    /// `Some`, el módulo se lleva todo el teclado.
    pub(crate) palette: Option<PaletteState>,
    /// Catálogo de comandos del palette. Se arma una vez en `init` y se reusa
    /// en cada apertura (el palette guarda índices, no copia los comandos).
    pub(crate) palette_commands: Vec<PaletteCommand>,
}

#[derive(Clone)]
pub(crate) enum Msg {
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
    /// Abre el prompt para **submonadizar** la selección (marca o cursor) de la
    /// Mónada actual en una sub-Mónada nueva (texto = nombre). Sólo aplica
    /// dentro de un grafo de Mónadas (`monad_graph()` presente).
    SubmonadizePrompt,
    /// Abre el prompt para **renombrar** la Mónada seleccionada (texto = nombre).
    RenameMonadPrompt,
    /// **Borra** la Mónada seleccionada (disuelve el agrupamiento; no borra
    /// archivos ni sub-Mónadas). Edición de grafo, no destructiva.
    DeleteMonad,
    /// **Fusiona** las Mónadas marcadas dentro de la Mónada bajo el cursor.
    MergeMonads,
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
    /// Diente derecho: abre/cierra el panel de tools del editor de imágenes.
    ToggleToolsPanel,
    /// Drag del splitter del panel de tools: ajusta su ancho.
    ResizeTools(f32),
    /// Fija el modo de la rueda con canvas abierto (toolbox: zoom/lista).
    SetWheelMode(WheelMode),
    /// Pasa al archivo siguiente (+1) / anterior (−1) de la carpeta con una
    /// app de canvas abierta (rueda en modo lista, botones atrás/adelante).
    CanvasNav(i32),
    /// La ventana cambió de tamaño (px) — `App::on_resize`.
    Resized(f32, f32),
    /// Cierra la app integrada del canvas (editor/imagen/media).
    CanvasClose,
    /// Tecla rumbo al editor de texto del canvas.
    CanvasEditKey(KeyEvent),
    /// Click/drag sobre el editor del canvas (posicionar caret / seleccionar).
    CanvasEditPointer(PointerEvent),
    /// Guarda el archivo del editor del canvas (Ctrl+S).
    CanvasSave,
    /// Msg lifteado del editor de imágenes del canvas (tullpu-module).
    CanvasTullpu(tullpu::Msg),
    /// Msg lifteado del player de media del canvas (media-module).
    CanvasMedia(mediamod::Msg),

    // ---- Fase 4.8: vista iconos con miniaturas ----
    /// Una miniatura terminó de generarse (llega del worker).
    ThumbReady(PathBuf, ThumbRgba),
    /// La miniatura de este path falló (formato no soportado / I/O).
    ThumbFailed(PathBuf),

    // ---- Command palette (Ctrl+Shift+P / Ctrl+P) ----
    /// Mensaje del módulo command-palette (abrir/cerrar/teclear/navegar/aplicar).
    Palette(PaletteMsg),

    // ---- Selección (parity dOpus) ----
    /// Marca todos los hijos visibles del panel enfocado (Ctrl+A).
    SelectAll,
    /// Limpia la marca del panel enfocado.
    SelectNone,
    /// Invierte la marca: lo marcado se desmarca y viceversa (`*`).
    InvertSelection,
    /// Abre el prompt de selección por patrón (glob `*.png`, `foto*`).
    SelectByPattern,

    // ---- Find recursivo (Ctrl+F) ----
    /// Abre el find recursivo posado en la carpeta actual.
    FindOpen,
    /// Agrega texto a la consulta del find.
    FindInput(String),
    /// Borra el último carácter de la consulta.
    FindBackspace,
    /// Enter: corre la búsqueda, o abre el resultado seleccionado si ya corrió.
    FindSubmit,
    /// Navega la lista de resultados (+1/-1).
    FindNav(i32),
    /// Alterna el modo de búsqueda (nombre ↔ contenido).
    FindToggleMode,
    /// Resultados del worker (con su generación para descartar los viejos).
    FindResults { gen: u64, hits: Vec<FindHit> },
    /// Construye el índice de embeddings de la carpeta actual (background).
    SemIndexBuild,
    /// El índice terminó de construirse (`None` si el daemon no estaba).
    SemIndexReady(Option<Box<SemIndex>>),
    /// Cierra el find.
    FindClose,

    // ---- Acción LLM ----
    /// Pregunta a la IA sobre la selección (archivo/carpeta/marca).
    AiAsk,
    /// Respuesta del worker LLM (texto o error humano).
    AiResult(Result<String, String>),
    /// Cierra el panel de IA.
    AiClose,
    /// Pide a la IA nombres nuevos para la selección (marca o cursor).
    AiRename,
    /// Nombres propuestos por la IA: `(id, original, propuesto)` por archivo.
    /// Abre el overlay de batch rename para revisarlos antes de aplicar.
    AiRenameResult(Vec<(nahual_source_core::NodeId, String, String)>),
}

impl Model {
    /// El navegador activo: el tope de la pila del panel enfocado.
    pub(crate) fn cur(&self) -> &Navigator {
        self.panes[self.focus].nav()
    }

    /// El navegador activo, mutable.
    pub(crate) fn cur_mut(&mut self) -> &mut Navigator {
        self.panes[self.focus].nav_mut()
    }

    /// `true` si el panel enfocado tiene una fuente no-POSIX montada (pila > 1).
    /// Gatea el montaje (no se anidan fuentes) y el desmontaje.
    pub(crate) fn is_foreign(&self) -> bool {
        self.panes[self.focus].is_foreign()
    }

    /// El panel enfocado, mutable (para empujar/sacar de su pila de montaje).
    pub(crate) fn cur_pane_mut(&mut self) -> &mut Pane {
        let f = self.focus;
        &mut self.panes[f]
    }

    /// El panel enfocado (lectura).
    pub(crate) fn cur_pane(&self) -> &Pane {
        &self.panes[self.focus]
    }

    /// `true` si la fuente activa admite operaciones de archivo (POSIX). Las
    /// fuentes montadas (wawa/minga/nouser) son read-only → sin `SourceMut`.
    pub(crate) fn can_edit(&self) -> bool {
        self.cur().writable().is_some()
    }

    /// Vuelca los campos vivos de navegación/preview a un `SessionSnap`
    /// (deja los campos del `Model` en su estado por defecto, listos para
    /// recibir los de otra sesión).
    pub(crate) fn snapshot_active(&mut self) -> SessionSnap {
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
            tools_open: self.tools_open,
            canvas: self.canvas.take(),
        }
    }

    /// Restaura los campos vivos desde un `SessionSnap` (al activar su diente).
    pub(crate) fn restore(&mut self, snap: SessionSnap) {
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
        self.tools_open = snap.tools_open;
        self.canvas = snap.canvas;
        // Asegurá el cache de subcarpetas de lo que esta sesión tiene abierto.
        ensure_children_for_expanded(&mut self.tree_children, &self.tree_expanded);
    }

    /// Activa la sesión `i`: guarda la sesión viva en su slot y restaura la
    /// destino. No hace nada si `i` ya es la activa o está fuera de rango.
    pub(crate) fn switch_to(&mut self, i: usize) {
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
pub(crate) fn cur_dir(m: &Model) -> PathBuf {
    PathBuf::from(m.cur().current_id().as_str())
}

/// Nombre corto de una sesión a partir de su carpeta de arranque.
pub(crate) fn session_name(cwd: &Path) -> String {
    cwd.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/".to_string())
}

/// Snapshot fresco para una sesión nueva, posada en `cwd`. El árbol arranca
/// descolapsado a lo largo de la cadena de ancestros de `cwd`.
pub(crate) fn fresh_snap(cwd: &Path) -> SessionSnap {
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
        tree_expanded: crate::helpers::ancestors_set(cwd),
        tree_scroll: 0,
        viewer_open: false,
        tools_open: false,
        canvas: None,
    }
}

/// Construye el navegador **POSIX base**: ancla la fuente en `/` (para poder
/// subir hasta la raíz del filesystem) y arranca parado en `cwd`, sembrando la
/// pila de ancestros para que el breadcrumb tenga la ruta completa. Si algo
/// falla, cae a la raíz `/`.
pub(crate) fn posix_nav(cwd: &Path) -> Navigator {
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
        let b = BatchRename::por_patron(
            String::new(),
            vec![("/x/a.txt".into(), "a.txt".into())],
        );
        assert_eq!(b.nuevo_nombre(0), "a.txt");
    }

    #[test]
    fn batch_contador_es_uno_based_y_ordenado() {
        let b = BatchRename::por_patron(
            "f{n}".to_string(),
            vec![
                ("/x/a".into(), "a".into()),
                ("/x/b".into(), "b".into()),
                ("/x/c".into(), "c".into()),
            ],
        );
        assert_eq!(b.nuevo_nombre(0), "f1");
        assert_eq!(b.nuevo_nombre(1), "f2");
        assert_eq!(b.nuevo_nombre(2), "f3");
    }

    #[test]
    fn batch_explicito_usa_los_nombres_de_ia() {
        // Nombres explícitos (propuestos por IA): alineados a targets, ignoran
        // el patrón; un explícito vacío conserva el original.
        let b = BatchRename::explicitos(
            vec![
                ("/x/IMG_001.jpg".into(), "IMG_001.jpg".into()),
                ("/x/IMG_002.jpg".into(), "IMG_002.jpg".into()),
            ],
            vec!["atardecer_playa.jpg".to_string(), String::new()],
        );
        assert!(b.es_ia());
        assert_eq!(b.nuevo_nombre(0), "atardecer_playa.jpg");
        assert_eq!(b.nuevo_nombre(1), "IMG_002.jpg"); // vacío → original
    }
}
