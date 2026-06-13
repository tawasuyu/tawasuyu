//! Modelo de la app y mensajes del bucle Elm.

use std::collections::HashMap;
use std::sync::Arc;

use llimphi_ui::{DragPhase, KeyEvent};
use llimphi_widget_text_editor::{EditorMetrics, PointerEvent};
use llimphi_widget_text_input::TextInputState;
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_llm::BackendKind;
use pluma_llm_core::ChatClient;
use pluma_store::PlumaStore;
use pluma_transform::Transformacion;
use uuid::Uuid;

use crate::clipboard::ArboardClipboard;

pub(crate) const METRICS: EditorMetrics = EditorMetrics::for_font_size(13.0);
pub(crate) const VISIBLE_LINES: usize = 200;

/// Ancho del rail de dientes, en px.
pub(crate) const RAIL_W: f32 = 46.0;
/// Ancho fijo de cada columna del multilienzo cuando hay ≥2 lienzos.
pub(crate) const ANCHO_COL: f32 = 360.0;
/// Ancho del carril entre columnas (= `ConfigMultilienzoEditor::ancho_carril`).
pub(crate) const ANCHO_CARRIL: f32 = 56.0;

/// Ancho total del contenido del multilienzo para `n` columnas fijas, o `0`
/// si `n < 2` (con una sola columna es elástica, sin scroll).
pub(crate) fn ancho_contenido(n: usize) -> f32 {
    if n < 2 {
        0.0
    } else {
        n as f32 * ANCHO_COL + (n as f32 - 1.0) * ANCHO_CARRIL
    }
}

/// Un filtro del grafo semántico: una etapa que transforma o acota el lienzo
/// que recibe. Encadenados de la fuente (lienzo activo) al sumidero, generan
/// una **línea de lienzo** nueva. Los tres primeros son transformaciones LLM
/// (las mismas que el diente Modelo); `Concepto` es un filtro semántico que
/// retiene sólo los párrafos afines a un término — MVP léxico (substring),
/// con el daemon de embeddings (rimay-verbo) como evolución natural.
#[derive(Clone, Debug)]
pub(crate) enum Filtro {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
    Concepto(String),
}

/// Un nodo-filtro posicionado en el lienzo del grafo (canvas coords del
/// nodegraph). El orden en `Model::grafo` es el orden del pipeline.
#[derive(Clone, Debug)]
pub(crate) struct NodoFiltro {
    pub(crate) filtro: Filtro,
    pub(crate) x: f32,
    pub(crate) y: f32,
}

pub(crate) const BACKENDS: [BackendKind; 6] = [
    BackendKind::Mock,
    BackendKind::Gemini,
    BackendKind::Anthropic,
    BackendKind::DeepSeek,
    BackendKind::Cohere,
    BackendKind::Ollama,
];

#[derive(Clone, Debug)]
pub(crate) enum Msg {
    EditorKey(KeyEvent),
    /// Click/drag dentro de la columna del cuerpo `Uuid` del multilienzo. Si
    /// ese cuerpo no es el activo, primero le pasa el foco (se apropia del
    /// teclado). Se identifica por `Uuid` y no por índice porque la lista de
    /// columnas visibles puede no coincidir 1-1 con `seleccionados`.
    MultiPointer(Uuid, PointerEvent),
    /// Abre un cuerpo como activo (lo agrega a la selección si no estaba).
    AbrirDoc(Uuid),
    /// Agrega/saca un cuerpo de la selección visible del multilienzo.
    ToggleSeleccion(Uuid),
    /// Reordena el tree de lienzos: mueve el lienzo en la posición `desde` a la
    /// posición `hasta` de `orden_lienzos` (drag&drop de filas). El orden del
    /// tree manda el orden de las columnas.
    ReordenarLienzo(usize, usize),
    /// Selecciona el diente del rail (0=Archivo,1=Lienzos,2=Derivar,3=LLM).
    SelectDiente(usize),
    /// Ctrl+Tab / Ctrl+Shift+Tab: mueve el foco al lienzo siguiente/anterior
    /// de la selección (cicla).
    FocoSiguiente,
    FocoAnterior,
    /// Activa/desactiva el foco por hover (pasar el cursor cambia el lienzo
    /// activo).
    ToggleFocoHover,
    /// Scroll horizontal del multilienzo, en píxeles (positivo = derecha).
    ScrollHoriz(f32),
    /// Scroll vertical del lienzo con foco, en "notches" de rueda (positivo =
    /// rueda hacia arriba). Los demás lienzos se nivelan al del foco.
    ScrollVert(f32),
    /// La ventana cambió de tamaño (ancho, alto) — para clampear el scroll.
    Resized(f32, f32),
    NuevoDoc,
    Guardar,
    PathInputKey(KeyEvent),
    FocusPath,
    DefocusPath,
    AbrirArchivo,
    ExportarMd,
    FindToggle,
    FindKey(KeyEvent),
    FindSiguiente,
    FindAnterior,
    FindClose,
    /// Togglea el modo "sólo activo" (una columna) vs "todos los
    /// seleccionados" (multilienzo completo) — antes era Diff.
    DiffToggle,
    /// Rail hospedado: pata reenvió un clic en un diente prestado — mapea
    /// directo a `SelectDiente`.
    HostActivate(u32),
    MoverAtomArriba,
    MoverAtomAbajo,
    TocarMadre,
    RegenerarStale,
    ToglearFusion,
    ZonaSiguiente,
    ZonaAnterior,
    CicloBackend,
    PedirTraducir(String),
    PedirTono(String),
    PedirResumir(Option<u32>),
    // --- Diente Derivar-IA: lienzo alterno desde prompt + presets ---
    /// Teclas hacia el input de prompt del diente Derivar.
    PresetInputKey(KeyEvent),
    FocusPreset,
    DefocusPreset,
    /// Deriva un lienzo alterno reescribiendo el activo con el prompt del input.
    CrearAlterno,
    /// Guarda el prompt actual del input como preset reutilizable.
    GuardarPreset,
    /// Re-corre el preset `usize` (lo reescribe sobre el activo).
    UsarPreset(usize),
    /// Borra el preset `usize` de la lista.
    BorrarPreset(usize),
    LlmListo {
        hija: Cuerpo,
        atoms_nuevos: Vec<NarrativeAtom>,
        carta: CartaHebras,
        transformacion: Transformacion,
    },
    LlmError(String),

    // --- Diente Grafo: grafo semántico de filtros → línea de lienzo ---
    /// Agrega un nodo-filtro al final del pipeline.
    GrafoAdd(Filtro),
    /// Borra el nodo-filtro cuyo `NodeId` se pasa (right-click). La fuente
    /// (id 0) y el sumidero no se pueden borrar — se ignoran.
    GrafoDel(u32),
    /// Arrastra un nodo del grafo: `NodeId`, fase, delta (dx, dy).
    GrafoDrag(u32, DragPhase, f32, f32),
    /// Teclas hacia el input del término del filtro Concepto.
    GrafoInputKey(KeyEvent),
    FocusGrafo,
    DefocusGrafo,
    /// Corre el pipeline de filtros sobre el activo y agrega la línea generada.
    GenerarLinea,
    /// Vacía el grafo de filtros.
    GrafoLimpiar,
    /// Arrastra el divisor entre el panel del diente y el centro.
    ResizePanel(f32),

    // --- Menú principal + menú de edición contextual ---
    /// Abre/cierra un dropdown del menú principal (índice del menú raíz).
    MenuOpen(Option<usize>),
    /// Comando string del menú principal (rebota desde `on_command`).
    MenuCommand(String),
    /// Navegación por teclado en el menú principal (`+1` baja, `-1` sube).
    MenuNav(i32),
    /// Enter en el menú principal: ejecuta la fila activa.
    MenuActivate,
    /// Tick de animación de menús (sólo re-render).
    MenuTick,
    /// Navegación por teclado en el menú de edición.
    EditNav(i32),
    /// Enter en el menú de edición: ejecuta la fila activa.
    EditActivate,
    /// Right-click: abre el menú de edición anclado en (x, y) de ventana.
    EditMenuOpen(f32, f32),
    /// Acción elegida en el menú de edición contextual.
    EditMenuAction(llimphi_widget_edit_menu::EditAction),
    /// Cierra cualquier menú abierto (dropdown o edición).
    CloseMenus,
}

pub(crate) struct Model {
    pub(crate) store: Arc<PlumaStore>,
    pub(crate) cuerpos: Vec<Cuerpo>,
    pub(crate) atoms: HashMap<Uuid, NarrativeAtom>,
    pub(crate) cartas: Vec<CartaHebras>,
    pub(crate) transformaciones: Vec<Transformacion>,
    /// `id` del `Cuerpo` activo (el editable en vivo, `ide`). `None` sólo
    /// si la lista de cuerpos está vacía — el init siembra uno para evitarlo.
    pub(crate) activo: Option<Uuid>,
    pub(crate) ide: CuerpoIde,
    /// Conjunto de cuerpos visibles en el multilienzo (membresía). Siempre
    /// contiene al `activo`. El ORDEN de columnas lo da `orden_lienzos`, no
    /// este vector.
    pub(crate) seleccionados: Vec<Uuid>,
    /// Orden maestro de todos los cuerpos en el tree de lienzos (reordenable por
    /// drag). Manda tanto el orden del tree como el de las columnas (filtrado
    /// por `seleccionados`).
    pub(crate) orden_lienzos: Vec<Uuid>,
    /// Editores read-only de los cuerpos seleccionados que no son el activo.
    /// Se reconstruyen al cambiar selección/activo/atoms.
    pub(crate) ides_ro: HashMap<Uuid, CuerpoIde>,
    /// Si `true`, el centro muestra sólo el cuerpo activo (una columna);
    /// si `false`, todo el multilienzo de `seleccionados`. Togglea con Ctrl+D.
    pub(crate) solo_activo: bool,
    /// Desplazamiento horizontal del multilienzo, en píxeles. Clampeado a
    /// `[0, ancho_contenido - ancho_centro]`.
    pub(crate) scroll_x: f32,
    /// Tamaño actual de la ventana (ancho, alto) en px lógicos. Lo actualiza
    /// `on_resize`; arranca en `initial_size`.
    pub(crate) viewport: (f32, f32),
    /// Diente activo del rail: 0=Archivo · 1=Lienzos · 2=Derivar · 3=LLM.
    pub(crate) diente_activo: usize,
    /// Si `true`, pasar el cursor sobre una columna le pasa el foco (off por
    /// defecto — se togglea desde el menú Multilienzo).
    pub(crate) foco_por_hover: bool,
    /// Ancho del panel del diente activo, en px (resizable con el divisor).
    pub(crate) panel_w: f32,
    pub(crate) clipboard: ArboardClipboard,
    pub(crate) drag_accum: (f32, f32),

    // --- Diente Derivar-IA ---
    /// Input del prompt para derivar un lienzo alterno.
    pub(crate) preset_input: TextInputState,
    /// Si el input de prompt tiene foco (las teclas van ahí).
    pub(crate) preset_focused: bool,
    /// Prompts guardados reutilizables. Persisten en `presets.txt` junto al sled.
    pub(crate) presets: Vec<String>,

    // --- Diente Grafo ---
    /// Pipeline de filtros (orden = fuente → ... → sumidero).
    pub(crate) grafo: Vec<NodoFiltro>,
    /// Posición del nodo fuente en el canvas del grafo (arrastrable).
    pub(crate) grafo_src: (f32, f32),
    /// Posición del nodo sumidero "→ nueva línea".
    pub(crate) grafo_sink: (f32, f32),
    /// Input del término para el filtro Concepto.
    pub(crate) grafo_input: TextInputState,
    pub(crate) grafo_input_focused: bool,

    pub(crate) chat: Arc<dyn ChatClient>,
    pub(crate) backend_idx: usize,
    pub(crate) en_curso: bool,
    pub(crate) ultimo_error: Option<String>,
    pub(crate) ultimo_status: String,

    /// Ruta del archivo a abrir/exportar — input compartido.
    /// Se interpreta según qué botón clickea el usuario.
    pub(crate) path_input: TextInputState,
    /// Cuando es `true`, las teclas del usuario van al `path_input` en
    /// vez del editor. Click sobre el input lo enciende; Esc, o un
    /// click fuera (en realidad, sólo Esc) lo apaga.
    pub(crate) path_focused: bool,

    /// Find-in-page sobre el cuerpo activo. `Ctrl+F` muestra el overlay
    /// y lo enfoca; Esc lo cierra; Enter/Shift+Enter cyclan matches.
    pub(crate) find_input: TextInputState,
    pub(crate) find_visible: bool,
    pub(crate) find_matches: Vec<(usize, usize)>,
    pub(crate) find_idx: usize,

    /// Índice del menú raíz cuyo dropdown está abierto (`None` = cerrado).
    pub(crate) menu_open: Option<usize>,
    /// Fila resaltada por teclado en el menú principal (`usize::MAX` = ninguna).
    pub(crate) menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    pub(crate) menu_anim: llimphi_motion::Tween<f32>,
    /// Ancla (x, y) en coords de ventana del menú de edición contextual,
    /// o `None` si no está abierto.
    pub(crate) edit_menu: Option<(f32, f32)>,
    /// Fila resaltada por teclado en el menú de edición (`usize::MAX` = ninguna).
    pub(crate) edit_active: usize,
    /// Animación de aparición del menú de edición (0→1).
    pub(crate) edit_anim: llimphi_motion::Tween<f32>,

    // --- Rail hospedado (dientes delegados a pata) ---
    /// `true` si pluma delega su rail a pata (`PLUMA_DELEGATE_SIDEBAR`): sus
    /// dientes aparecen en el rail de pata cuando tiene foco y pluma no dibuja
    /// su propio rail interno (sólo el panel del diente activo + el centro).
    pub(crate) delegated: bool,
    /// Cliente del rail hospedado; sólo se retiene (las activaciones llegan por
    /// callback). `_` evita el lint de campo sin leer.
    pub(crate) _host: Option<pata_host::HostClient>,
}
