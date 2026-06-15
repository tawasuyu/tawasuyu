//! `modelo` — tipos centrales de khipu-app: constantes, enums de foco y
//! mensajes, estructuras de modelo y estado P2P.
//!
//! Todo lo que `update`, `view` y los submódulos necesitan conocer para
//! tipar sus firmas; cero lógica de negocio aquí.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use agora_core::Keypair;
use khipu_core::{NoteId, NoteStore};
use khipu_gravity::{Gravity, Params, SemanticField};
use khipu_share::SignedBundle;
use llimphi_clipboard::SystemClipboard;
use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_widget_text_editor::EditorState;
use llimphi_widget_text_input::TextInputState;
use rimay_verbo::Provider;
use serde::{Deserialize, Serialize};

// =====================================================================
// Constantes globales
// =====================================================================

/// Dimensión del embebedor local (fallback sin daemon).
pub(crate) const EMBED_DIM: usize = 16;
pub(crate) const CLUSTER_THRESHOLD: f32 = 0.55;
pub(crate) const EDITOR_VISIBLE_LINES: usize = 24;
pub(crate) const LIST_WIDTH: f32 = 240.0;
/// Ancho del editor flotante (overlay derecho sobre el mapa).
pub(crate) const EDITOR_OVERLAY_W: f32 = 420.0;
/// Zoom a partir del cual el nodo seleccionado se abre in-situ.
pub(crate) const ZOOM_INJECT: f32 = 1.6;
pub(crate) const HEADER_H: f32 = 36.0;
pub(crate) const ROW_H: f32 = 26.0;
pub(crate) const FIELD_LABEL_SIZE: f32 = 10.0;

// =====================================================================
// Embedder
// =====================================================================

/// Fuente de vectores semánticos. Con un `verbo-daemon` en el socket por
/// defecto usa embeddings reales; si no hay daemon cae al hash-trigram
/// local de 16d — determinista, offline, sin runtime.
///
/// El arm remoto guarda el `Runtime` de tokio para resolver las llamadas
/// async del `Provider` con `block_on` desde el hilo worker que las
/// dispara (nunca el de UI). Es `Clone` (todo tras `Arc`) para viajar
/// barato dentro de la closure de `Handle::spawn`.
#[derive(Clone)]
pub(crate) enum Embedder {
    /// Daemon `rimay-verbo` por socket Unix.
    Remote {
        provider: Arc<dyn Provider>,
        rt: Arc<tokio::runtime::Runtime>,
        dim: usize,
        label: String,
    },
    /// Fallback local: hash-trigram → R^EMBED_DIM, sin red ni runtime.
    Local,
}

impl Embedder {
    /// Conecta al `verbo-daemon` en el socket por defecto. Si no hay
    /// ninguno (o no se pudo armar el runtime), devuelve el embebedor
    /// local — los demos arrancan igual, sin red.
    pub(crate) fn connect() -> Self {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => return Embedder::Local,
        };
        match rt.block_on(rimay_verbo::conectar()) {
            Ok(client) => {
                let id = client.model_id();
                let dim = id.dimension;
                let label = id.to_string();
                Embedder::Remote {
                    provider: Arc::new(client),
                    rt: Arc::new(rt),
                    dim,
                    label,
                }
            }
            Err(_) => Embedder::Local,
        }
    }

    /// Etiqueta del espacio vectorial. Si cambia entre dos arranques, los
    /// vectores persistidos son incomparables y hay que recalcularlos.
    pub(crate) fn label(&self) -> String {
        match self {
            Embedder::Remote { label, .. } => label.clone(),
            Embedder::Local => format!("khipu-trigram-{EMBED_DIM}d"),
        }
    }

    /// Embebe `text` de forma bloqueante. En el arm remoto resuelve el
    /// future con `block_on`; ante un error devuelve un vector de ceros.
    pub(crate) fn embed_blocking(&self, text: &str) -> Vec<f32> {
        match self {
            Embedder::Local => khipu_gravity::local_embed(text, EMBED_DIM),
            Embedder::Remote { provider, rt, dim, .. } => rt
                .block_on(provider.embed(text))
                .map(|v| v.values)
                .unwrap_or_else(|_| vec![0.0; *dim]),
        }
    }
}

// =====================================================================
// Focus
// =====================================================================

/// Foco activo del teclado. Cualquier `KeyEvent` se rutea al input
/// correspondiente; sin foco las teclas se ignoran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    None,
    Search,
    Title,
    Body,
    Tags,
    Passphrase,
    PeerAddr,
    /// Input del nombre de una región emergente (bautizo de un clúster).
    Region,
}

// =====================================================================
// Region
// =====================================================================

/// Una región del mapa: un nombre pinchado en una coordenada de mundo.
/// No es una carpeta — es un topónimo. Nace cuando el usuario bautiza un
/// clúster denso; los pensamientos cercanos "pertenecen" a esa zona por
/// vecindad, no por asignación.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Region {
    pub(crate) name: String,
    pub(crate) x: f32,
    pub(crate) y: f32,
}

// =====================================================================
// P2p
// =====================================================================

/// Nodo libp2p del cuaderno + su runtime tokio. Se arma perezosamente la
/// primera vez que se usa P2P (`ensure_p2p`).
pub(crate) struct P2p {
    pub(crate) rt: Arc<tokio::runtime::Runtime>,
    pub(crate) node: Arc<khipu_brahman::KhipuNode>,
    /// Nuestra dirección para compartir (`/ip4/.../tcp/.../p2p/<id>`).
    pub(crate) dial_addr: String,
    /// `true` cuando ya estamos sirviendo el cuaderno por libp2p.
    pub(crate) serving: bool,
}

// =====================================================================
// PeerInfo
// =====================================================================

/// Un par descubierto en la red, en forma lista para la UI.
#[derive(Clone)]
pub(crate) struct PeerInfo {
    /// Dirección TCP de fetch, como string (`ip:puerto`).
    pub(crate) addr: String,
    /// Etiqueta para la fila: nombre · de:autor · dirección.
    pub(crate) label: String,
}

// =====================================================================
// Msg
// =====================================================================

#[derive(Clone)]
pub(crate) enum Msg {
    SelectNote(NoteId),
    NewNote,
    DeleteSelected,
    ToggleArchive,
    Focus(Focus),
    Key(llimphi_ui::KeyEvent),
    EditorPointer(llimphi_widget_text_editor::PointerEvent),
    /// Latido — fuerza el rerender para que la masa decaiga
    /// visiblemente aunque el usuario no esté tocando nada.
    Tick,
    /// Resultado async de un embed: `(nota, secuencia, vector)`.
    EmbeddingReady(NoteId, u64, Vec<f32>),
    /// Sella todo el cuaderno en un sobre firmado (`compartido.khipu`).
    Export,
    /// Verifica e ingiere `compartido.khipu` como notas nuevas.
    Import,
    /// Empieza a servir el cuaderno por TCP para que un par lo jale.
    Publish,
    /// Busca pares en la LAN para jalarles el cuaderno.
    Receive,
    /// Resultado del descubrimiento: los pares vistos (ya sin uno mismo).
    PeersFound(Vec<PeerInfo>),
    /// Jala el cuaderno del par en esta dirección TCP.
    FetchFrom(String),
    /// Jala de la dirección escrita a mano (input) — habilita WAN.
    FetchManual,
    /// Cierra el panel de recibir sin jalar nada.
    CancelPeers,
    /// Resultado async de un fetch: el sobre recibido o un error.
    Received(Result<SignedBundle, String>),
    /// Intenta desbloquear la identidad con la passphrase tipeada.
    Unlock,
    /// Cierra el prompt de passphrase sin desbloquear.
    CancelUnlock,
    /// Resultado async de reservar un circuito en un relay.
    RelayReady(String),
    /// Abre/cierra un dropdown de la barra de menú principal.
    MenuOpen(Option<usize>),
    /// Comando elegido en la barra de menú.
    MenuCommand(String),
    /// Navegación por teclado en el menú principal.
    MenuNav(i32),
    /// Enter en el menú principal: ejecuta la fila activa.
    MenuActivate,
    /// Tick de animación de menús (sólo re-render).
    MenuTick,
    /// Navegación por teclado en el menú de edición.
    EditNav(i32),
    /// Enter en el menú de edición: ejecuta la fila activa.
    EditActivate,
    /// Abre el menú de edición contextual (right-click) en coords de ventana.
    EditMenuOpen(f32, f32),
    /// Acción de edición sobre el campo focuseado.
    EditMenuAction(llimphi_widget_edit_menu::EditAction),
    /// Cierra cualquier menú abierto (principal o de edición).
    CloseMenus,
    /// Pan del mapa: delta de arrastre en pixels de pantalla.
    MapPan(f32, f32),
    /// Zoom del mapa: delta de rueda (líneas, signo winit).
    MapZoom(f32),
    /// Click en el lienzo en coords locales `(lx, ly)` sobre un rect `(w, h)`.
    MapClick(f32, f32, f32, f32),
    /// Abre/cierra el cajón de notas (overlay izquierdo).
    ToggleList,
    /// Cierra el editor flotante: deselecciona y vuelve al mapa limpio.
    Deselect,
    /// Escape en el mapa: cierra lo de más arriba.
    EscapeMap,
    /// Empieza a bautizar una región en la coordenada de mundo `(x, y)`,
    /// precargando el input con el nombre propuesto del clúster.
    BeginNaming(f32, f32, String),
    /// Confirma el bautizo: crea la región con el texto tipeado.
    CommitNaming,
    /// Cancela el bautizo sin crear región.
    CancelNaming,
}

// =====================================================================
// Model
// =====================================================================

pub(crate) struct Model {
    pub(crate) store: NoteStore,
    pub(crate) field: SemanticField,
    /// Orden de inserción (estable).
    pub(crate) order: Vec<NoteId>,
    pub(crate) selected: Option<NoteId>,
    pub(crate) title: TextInputState,
    pub(crate) body: EditorState,
    pub(crate) tags: TextInputState,
    pub(crate) search: TextInputState,
    pub(crate) focus: Focus,
    pub(crate) theme: Theme,
    pub(crate) data_path: Option<PathBuf>,
    /// Física temporal: vida media + boost + horizonte.
    pub(crate) gravity: Gravity,
    /// `true` cuando el usuario quiere ver también las notas archivadas.
    pub(crate) show_archive: bool,
    /// Fuente de embeddings: daemon `verbo` o fallback trigram local.
    pub(crate) embedder: Embedder,
    /// Última secuencia de embedding pedida por nota.
    pub(crate) embed_latest: BTreeMap<NoteId, u64>,
    /// Contador monótono de pedidos de embedding.
    pub(crate) embed_seq: u64,
    /// Identidad Ed25519 del cuaderno.
    pub(crate) keypair: Option<Keypair>,
    /// Última línea de estado (export/import/red).
    pub(crate) status: Option<String>,
    /// `true` cuando ya hay un servidor TCP sirviendo el cuaderno.
    pub(crate) publishing: bool,
    /// `true` mientras el panel izquierdo está en modo "recibir".
    pub(crate) receiving: bool,
    /// Pares descubiertos en la última búsqueda.
    pub(crate) peers: Vec<PeerInfo>,
    /// Dirección manual del par para recibir (habilita WAN).
    pub(crate) peer_input: TextInputState,
    /// Input de la passphrase para desbloquear la identidad.
    pub(crate) passphrase: TextInputState,
    /// `true` mientras se muestra el prompt de passphrase (modal).
    pub(crate) unlocking: bool,
    /// Acción a reanudar tras desbloquear.
    pub(crate) pending: Option<Box<Msg>>,
    /// Nodo libp2p (perezoso).
    pub(crate) p2p: Option<P2p>,
    /// Dropdown abierto de la barra de menú (índice), o `None`.
    pub(crate) menu_open: Option<usize>,
    /// Fila resaltada por teclado en el menú principal.
    pub(crate) menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal.
    pub(crate) menu_anim: Tween<f32>,
    /// Posición del menú de edición contextual, si abierto.
    pub(crate) edit_menu: Option<(f32, f32)>,
    /// Fila resaltada por teclado en el menú de edición.
    pub(crate) edit_active: usize,
    /// Animación de aparición del menú de edición.
    pub(crate) edit_anim: Tween<f32>,
    /// Portapapeles del sistema.
    pub(crate) clipboard: SystemClipboard,
    /// Desplazamiento de la cámara del mapa, en coordenadas de mundo.
    pub(crate) cam_pan: (f32, f32),
    /// Escala de la cámara del mapa (1.0 = mundo:pantalla).
    pub(crate) cam_zoom: f32,
    /// `true` mientras el cajón de notas (overlay izquierdo) está abierto.
    pub(crate) show_list: bool,
    /// Último tamaño conocido del lienzo `(w, h)` en pixels.
    pub(crate) canvas_size: (f32, f32),
    /// Topónimos del mapa: regiones bautizadas.
    pub(crate) regions: Vec<Region>,
    /// Coordenada de mundo de la región que se está bautizando.
    pub(crate) naming: Option<(f32, f32)>,
    /// Input del nombre de la región en curso.
    pub(crate) region_input: TextInputState,
}

impl Model {
    /// Constructor vacío base; lo usan `from_state` y `seeded_model`
    /// para no repetir la inicialización de todos los campos.
    pub(crate) fn blank(embedder: Embedder) -> Self {
        Model {
            store: NoteStore::new(),
            field: SemanticField::new(),
            order: Vec::new(),
            selected: None,
            title: TextInputState::new(),
            body: EditorState::default(),
            tags: TextInputState::new(),
            search: TextInputState::new(),
            focus: Focus::None,
            theme: Theme::dark(),
            data_path: None,
            gravity: Gravity::new(Params::default()),
            show_archive: false,
            embedder,
            embed_latest: BTreeMap::new(),
            embed_seq: 0,
            keypair: None,
            status: None,
            publishing: false,
            receiving: false,
            peers: Vec::new(),
            peer_input: TextInputState::new(),
            passphrase: TextInputState::masked(),
            unlocking: false,
            pending: None,
            p2p: None,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_menu: None,
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
            clipboard: SystemClipboard::new(),
            cam_pan: (0.0, 0.0),
            cam_zoom: 1.0,
            show_list: true,
            canvas_size: (1280.0, 640.0),
            regions: Vec::new(),
            naming: None,
            region_input: TextInputState::new(),
        }
    }
}
