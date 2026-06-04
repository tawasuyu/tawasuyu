//! Modelo de la app y mensajes del bucle Elm.

use std::collections::HashMap;
use std::sync::Arc;

use llimphi_ui::KeyEvent;
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
    EditorPointer(PointerEvent),
    AbrirDoc(Uuid),
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
    DiffToggle,
    /// Rail hospedado: pata reenvió un clic en un diente prestado (0=Documentos,
    /// 1=LLM, 2=Buscar, 3=Diff). Togglea esa sección.
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
    LlmListo {
        hija: Cuerpo,
        atoms_nuevos: Vec<NarrativeAtom>,
        carta: CartaHebras,
        transformacion: Transformacion,
    },
    LlmError(String),
    ResizeIzq(f32),
    ResizeDer(f32),

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
    /// `id` del `Cuerpo` activo (el que se ve en `ide`). `None` sólo si
    /// la lista de cuerpos está vacía — el init siembra uno para evitarlo.
    pub(crate) activo: Option<Uuid>,
    pub(crate) ide: CuerpoIde,
    pub(crate) clipboard: ArboardClipboard,
    pub(crate) drag_accum: (f32, f32),

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

    /// Cuando es `true` y el cuerpo activo es una hija, el centro
    /// muestra la madre y la hija lado a lado con las hebras pintadas
    /// (read-only). Cuando el activo es Original o no se encuentra la
    /// madre, el flag igual existe pero la vista cae al cuerpo_ide
    /// normal con un cartel.
    pub(crate) diff_visible: bool,

    pub(crate) side_izq_w: f32,
    pub(crate) side_der_w: f32,

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

    // --- Rail hospedado (sidebar delegado a pata) ---
    /// `true` si pluma delega su sidebar a pata (`PLUMA_DELEGATE_SIDEBAR`): sus
    /// secciones aparecen como dientes en el rail de pata cuando tiene foco, y las
    /// columnas laterales se pueden colapsar (editor a pantalla completa).
    pub(crate) delegated: bool,
    /// Visibilidad de la columna de Documentos (sólo aplica en modo delegado).
    pub(crate) side_izq_visible: bool,
    /// Visibilidad de la columna LLM (sólo aplica en modo delegado).
    pub(crate) side_der_visible: bool,
    /// Cliente del rail hospedado; sólo se retiene (las activaciones llegan por
    /// callback). `_` evita el lint de campo sin leer.
    pub(crate) _host: Option<pata_host::HostClient>,
}
