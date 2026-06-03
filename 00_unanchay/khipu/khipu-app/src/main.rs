//! `khipu-app` — cuaderno de notas sobre Llimphi.
//!
//! Tres regiones, todas en la misma ventana, sin modal:
//! - **Lista** (izquierda, 240 px): notas en orden de creación.
//!   Click selecciona. Botón `+ nueva` arriba.
//! - **Editor** (centro): título (input), cuerpo (text-editor con
//!   wiki-links `[[...]]`), etiquetas (input). Edición directa — la
//!   nota seleccionada se modifica al teclear, sin botón guardar.
//! - **Gravedad** (derecha): canvas vello que pinta las posiciones
//!   2D del [`SemanticField::gravity_layout`]. Color por clúster
//!   (umbral 0.55), la seleccionada va resaltada con borde acento.
//!
//! **Embeddings**: si hay un `verbo-daemon` corriendo en el socket por
//! defecto (`$XDG_RUNTIME_DIR/verbo.sock`) los vectores son reales
//! (fastembed e5, etc.) — clústeres y vecinos se vuelven semánticos de
//! verdad. Sin daemon caemos al hash trigram → R^16 local (random
//! projection 1-bit signed, normalizado): determinista, offline,
//! idéntico al comportamiento histórico. Ver [`Embedder`]. El cálculo
//! es async, así que viaja a un worker (`Handle::spawn`) y reentra al
//! `update` con [`Msg::EmbeddingReady`] — la UI nunca se bloquea.
//!
//! **Persistencia**: cada mutación graba `$XDG_DATA_HOME/khipu/notes.bin`
//! con postcard, anotando la etiqueta del espacio vectorial usado. Al
//! arrancar, si el archivo existe se carga; si el espacio cambió (otro
//! modelo o dimensión) los vectores se recalculan. Sin archivo se
//! siembra el cuaderno demo (siete notas en español).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use agora_core::Keypair;
use directories::ProjectDirs;
use khipu_share::{SharedNote, SignedBundle};
use rimay_verbo::Provider;
use khipu_core::{Note, NoteId, NoteStore};
use khipu_gravity::{Gravity, Params, SemanticField};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::winit::keyboard::{Key, NamedKey};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Rect, Size, Style},
    AlignItems, Dimension, JustifyContent,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle as KurboCircle, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, Alignment, TextBlock};
use llimphi_ui::{App, DragPhase, Handle, KeyEvent, KeyState, View};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_text_editor::{
    text_editor_view, EditorMetrics, EditorPalette, EditorState, PointerEvent,
};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_motion::{animate, motion, Tween};
use llimphi_clipboard::SystemClipboard;
use serde::{Deserialize, Serialize};

/// Dimensión del embebedor local (fallback sin daemon).
const EMBED_DIM: usize = 16;
const CLUSTER_THRESHOLD: f32 = 0.55;
const EDITOR_VISIBLE_LINES: usize = 24;
const LIST_WIDTH: f32 = 240.0;
/// Ancho del editor flotante (overlay derecho sobre el mapa).
const EDITOR_OVERLAY_W: f32 = 420.0;
/// Zoom a partir del cual el nodo seleccionado deja de editarse en el panel
/// lateral y pasa a abrirse como tarjeta anclada a su coordenada en el mapa
/// (zoom semántico). Por debajo, el editor vuelve al overlay derecho.
const ZOOM_INJECT: f32 = 1.6;
const HEADER_H: f32 = 36.0;
const ROW_H: f32 = 26.0;
const FIELD_LABEL_SIZE: f32 = 10.0;

/// Fuente de vectores semánticos. Con un `verbo-daemon` en el socket por
/// defecto usa embeddings reales; si no hay daemon cae al hash-trigram
/// local de 16d — determinista, offline, sin runtime.
///
/// El arm remoto guarda el `Runtime` de tokio para resolver las llamadas
/// async del `Provider` con `block_on` desde el hilo worker que las
/// dispara (nunca el de UI). Es `Clone` (todo tras `Arc`) para viajar
/// barato dentro de la closure de `Handle::spawn`.
#[derive(Clone)]
enum Embedder {
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
    fn connect() -> Self {
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
    /// vectores persistidos son incomparables (otro modelo o dimensión) y
    /// hay que recalcularlos — ver [`from_state`].
    fn label(&self) -> String {
        match self {
            Embedder::Remote { label, .. } => label.clone(),
            Embedder::Local => format!("khipu-trigram-{EMBED_DIM}d"),
        }
    }

    /// Embebe `text` de forma bloqueante. En el arm remoto resuelve el
    /// future con `block_on`; ante un error del backend devuelve un
    /// vector de ceros (afinidad nula con todo, nunca panic).
    fn embed_blocking(&self, text: &str) -> Vec<f32> {
        match self {
            Embedder::Local => embed(text, EMBED_DIM),
            Embedder::Remote { provider, rt, dim, .. } => rt
                .block_on(provider.embed(text))
                .map(|v| v.values)
                .unwrap_or_else(|_| vec![0.0; *dim]),
        }
    }
}

/// Foco activo del teclado. Cualquier `KeyEvent` se rutea al input
/// correspondiente; sin foco las teclas se ignoran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
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

/// Una región del mapa: un nombre pinchado en una coordenada de mundo. No
/// es una carpeta — es un topónimo. Nace cuando el usuario bautiza un
/// clúster denso que el mapa detectó, y de ahí queda como landmark fijo;
/// los pensamientos cercanos "pertenecen" a esa zona por vecindad, no por
/// asignación. Las placas tectónicas emergen del caos, no se imponen.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Region {
    name: String,
    x: f32,
    y: f32,
}

/// Nodo libp2p del cuaderno + su runtime tokio (que lo mantiene vivo). Se
/// arma perezosamente la primera vez que se usa P2P (`ensure_p2p`).
struct P2p {
    rt: Arc<tokio::runtime::Runtime>,
    node: Arc<khipu_brahman::KhipuNode>,
    /// Nuestra dirección para compartir (`/ip4/.../tcp/.../p2p/<id>`).
    dial_addr: String,
    /// `true` cuando ya estamos sirviendo el cuaderno por libp2p.
    serving: bool,
}

/// Un par descubierto en la red, en forma lista para la UI: dónde jalarle
/// el cuaderno y una etiqueta legible. Datos planos (no `PeerVisto`) para
/// viajar dentro de un `Msg`.
#[derive(Clone)]
struct PeerInfo {
    /// Dirección TCP de fetch, como string (`ip:puerto`).
    addr: String,
    /// Etiqueta para la fila: nombre · de:autor · dirección.
    label: String,
}

#[derive(Clone)]
enum Msg {
    SelectNote(NoteId),
    NewNote,
    DeleteSelected,
    ToggleArchive,
    Focus(Focus),
    Key(KeyEvent),
    EditorPointer(PointerEvent),
    /// Latido — fuerza el rerender para que la masa decaiga
    /// visiblemente aunque el usuario no esté tocando nada.
    Tick,
    /// Resultado async de un embed: `(nota, secuencia, vector)`. Se
    /// aplica sólo si `secuencia` sigue siendo la más reciente para esa
    /// nota — descarta cálculos que ediciones posteriores dejaron viejos.
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
    /// Jala de la dirección escrita a mano (input) — habilita WAN: cualquier
    /// `host:puerto` alcanzable, no sólo pares descubiertos en la LAN.
    FetchManual,
    /// Cierra el panel de recibir sin jalar nada.
    CancelPeers,
    /// Resultado async de un fetch: el sobre recibido o un error.
    Received(Result<SignedBundle, String>),
    /// Intenta desbloquear la identidad con la passphrase tipeada.
    Unlock,
    /// Cierra el prompt de passphrase sin desbloquear.
    CancelUnlock,
    /// Resultado async de reservar un circuito en un relay: la dirección de
    /// marcado vía circuito (o un mensaje de error) para mostrar.
    RelayReady(String),
    /// Abre/cierra un dropdown de la barra de menú principal (índice del menú).
    MenuOpen(Option<usize>),
    /// Comando elegido en la barra de menú (`command` de cada `MenuItem`).
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
    /// Abre el menú de edición contextual (right-click) en coords de ventana.
    EditMenuOpen(f32, f32),
    /// Acción de edición (undo/redo/cut/copy/paste/delete/selectall) sobre
    /// el campo focuseado.
    EditMenuAction(EditAction),
    /// Cierra cualquier menú abierto (principal o de edición).
    CloseMenus,
    /// Pan del mapa: delta de arrastre en pixels de pantalla.
    MapPan(f32, f32),
    /// Zoom del mapa: delta de rueda (líneas, signo winit). Acerca/aleja.
    MapZoom(f32),
    /// Click en el lienzo en coords locales `(lx, ly)` sobre un rect
    /// `(w, h)`: selecciona la nota más cercana bajo el cursor, si hay.
    MapClick(f32, f32, f32, f32),
    /// Abre/cierra el cajón de notas (overlay izquierdo).
    ToggleList,
    /// Cierra el editor flotante: deselecciona y vuelve al mapa limpio.
    Deselect,
    /// Escape en el mapa: cierra lo de más arriba (editor → cajón → foco).
    EscapeMap,
    /// Empieza a bautizar una región en la coordenada de mundo `(x, y)`
    /// (el centroide del clúster denso que se ofreció nombrar).
    BeginNaming(f32, f32),
    /// Confirma el bautizo: crea la región con el texto tipeado.
    CommitNaming,
    /// Cancela el bautizo sin crear región.
    CancelNaming,
}

struct Model {
    store: NoteStore,
    field: SemanticField,
    /// Orden de inserción (estable). La presentación se reordena por
    /// masa decreciente al renderizar.
    order: Vec<NoteId>,
    selected: Option<NoteId>,
    title: TextInputState,
    body: EditorState,
    tags: TextInputState,
    search: TextInputState,
    focus: Focus,
    theme: Theme,
    data_path: Option<PathBuf>,
    /// Física temporal: vida media + boost + horizonte.
    gravity: Gravity,
    /// `true` cuando el usuario quiere ver también las notas que
    /// cayeron del horizonte. Default `false`.
    show_archive: bool,
    /// Fuente de embeddings: daemon `verbo` o fallback trigram local.
    embedder: Embedder,
    /// Última secuencia de embedding pedida por nota. Un resultado async
    /// (`Msg::EmbeddingReady`) sólo se aplica si su secuencia coincide
    /// con la vigente aquí; así una edición rápida invalida el cálculo
    /// de la anterior sin condición de carrera.
    embed_latest: BTreeMap<NoteId, u64>,
    /// Contador monótono de pedidos de embedding.
    embed_seq: u64,
    /// Identidad Ed25519 del cuaderno, para firmar/exportar sobres
    /// (`khipu-share`). `None` si no hay directorio de datos.
    keypair: Option<Keypair>,
    /// Última línea de estado (export/import/red). Se pinta en una barra
    /// al pie cuando es `Some`.
    status: Option<String>,
    /// `true` cuando ya hay un servidor TCP sirviendo el cuaderno. Evita
    /// rebindear el puerto si se pulsa «publicar» dos veces.
    publishing: bool,
    /// `true` mientras el panel izquierdo está en modo "recibir": input de
    /// dirección + lista de pares descubiertos.
    receiving: bool,
    /// Pares descubiertos en la última búsqueda (filas clickeables).
    peers: Vec<PeerInfo>,
    /// Dirección manual del par para recibir (habilita WAN). Prellenada
    /// con `KHIPU_PEER` o el default; editable.
    peer_input: TextInputState,
    /// Input de la passphrase para desbloquear la identidad.
    passphrase: TextInputState,
    /// `true` mientras se muestra el prompt de passphrase (modal).
    unlocking: bool,
    /// Acción a reanudar tras desbloquear (lo que el usuario quiso hacer
    /// y disparó el prompt). Se redispatcha al lograr el unlock.
    pending: Option<Box<Msg>>,
    /// Nodo libp2p (perezoso): `Some` una vez que se usó P2P.
    p2p: Option<P2p>,
    /// Dropdown abierto de la barra de menú (índice), o `None` si cerrada.
    menu_open: Option<usize>,
    /// Fila resaltada por teclado en el menú principal (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    menu_anim: Tween<f32>,
    /// Posición (coords de ventana) del menú de edición contextual, si abierto.
    edit_menu: Option<(f32, f32)>,
    /// Fila resaltada por teclado en el menú de edición (`usize::MAX` = ninguna).
    edit_active: usize,
    /// Animación de aparición del menú de edición (0→1).
    edit_anim: Tween<f32>,
    /// Portapapeles del sistema, compartido por todas las acciones de edición.
    clipboard: SystemClipboard,
    /// Desplazamiento de la cámara del mapa, en coordenadas de mundo. El
    /// lienzo es infinito; arrastrar el fondo desplaza este vector.
    cam_pan: (f32, f32),
    /// Escala de la cámara del mapa (1.0 = mundo:pantalla). La rueda la
    /// cambia; el zoom semántico futuro decidirá qué se inyecta según ella.
    cam_zoom: f32,
    /// `true` mientras el cajón de notas (overlay izquierdo) está abierto.
    /// El mapa es la interfaz; la lista es un cajón invocable, no un panel
    /// permanente. Default `true` para no perder al usuario en el primer
    /// arranque.
    show_list: bool,
    /// Último tamaño conocido del lienzo `(w, h)` en pixels. Lo aprende de
    /// cada click (`on_click_at` lo trae) y sirve para anclar la tarjeta
    /// del nodo en su coordenada de pantalla durante el zoom semántico,
    /// que se calcula en `view()` antes de que corra el layout. Se corrige
    /// solo en el siguiente click tras un resize.
    canvas_size: (f32, f32),
    /// Topónimos del mapa: regiones bautizadas (landmarks persistidos).
    regions: Vec<Region>,
    /// Coordenada de mundo de la región que se está bautizando ahora mismo
    /// (input abierto), o `None`.
    naming: Option<(f32, f32)>,
    /// Input del nombre de la región en curso.
    region_input: TextInputState,
}

struct KhipuApp;

impl App for KhipuApp {
    type Model = Model;
    type Msg = Msg;

    fn init(handle: &Handle<Msg>) -> Model {
        // Conectamos al daemon una sola vez al arrancar; el embebedor
        // resultante (remoto o local) se clona barato a cada worker.
        let embedder = Embedder::connect();
        let data_path = data_file_path();
        let mut model = match data_path.as_ref().and_then(load_state) {
            Some(state) => from_state(state, embedder),
            None => seeded_model(embedder),
        };
        model.data_path = data_path;
        // Identidad: si `KHIPU_PASSPHRASE` está en el entorno, desbloqueamos
        // (o creamos/migramos) sin prompt — útil headless. Si no, queda
        // bloqueada y se pide la passphrase al primer intento de compartir.
        model.keypair = std::env::var("KHIPU_PASSPHRASE")
            .ok()
            .and_then(|p| unlock_identity(&p));
        model.theme = Theme::dark();
        // Con bootstrap configurado, arrancamos el nodo libp2p ya, para que
        // la malla DHT esté caliente cuando el usuario quiera descubrir.
        if std::env::var("KHIPU_BOOTSTRAP").is_ok() {
            ensure_p2p(&mut model);
        }
        // Elegimos la primera nota más pesada (decayendo on-the-fly);
        // si todo el cuaderno está en archivo, caemos al orden de
        // inserción para no abrir vacío.
        let first = first_visible(&model).or_else(|| model.order.first().copied());
        if let Some(id) = first {
            reinforce_and_touch(&mut model, id);
            select(&mut model, id);
        }
        persist(&model);
        // Latido cada 30 s — la masa decae en disco como en pantalla.
        handle.spawn_periodic(std::time::Duration::from_secs(30), || Msg::Tick);
        model
    }

    fn update(mut model: Model, msg: Msg, h: &Handle<Msg>) -> Model {
        match msg {
            Msg::SelectNote(id) => {
                commit_edits(&mut model, h);
                reinforce_and_touch(&mut model, id);
                select(&mut model, id);
                persist(&model);
            }
            Msg::NewNote => {
                commit_edits(&mut model, h);
                let now = now_secs();
                let id = model.store.create("Nota nueva", "", Vec::new(), now);
                model.order.push(id);
                schedule_embedding(&mut model, id, h);
                select(&mut model, id);
                persist(&model);
            }
            Msg::ToggleArchive => {
                model.show_archive = !model.show_archive;
            }
            Msg::Tick => {
                // No muta nada: la masa vive en `current_mass` (decay
                // contra `last_access`). El Tick existe sólo para
                // pedirle al event loop un redraw.
            }
            Msg::EmbeddingReady(id, seq, v) => {
                // Aplicamos el vector sólo si sigue siendo el cálculo más
                // reciente para esa nota y la nota no fue borrada entre
                // medio. Tras insertarlo, persistimos para que el campo
                // semántico en disco quede al día.
                if model.embed_latest.get(&id) == Some(&seq)
                    && model.store.get(id).is_some()
                {
                    model.field.insert(id, v);
                    // Recién ahora la nota tiene vector: le damos domicilio
                    // en el mapa una sola vez, cerca de sus parientes. Si ya
                    // tenía posición (re-embed por edición), no se mueve.
                    place_note(&mut model, id);
                    persist(&model);
                }
            }
            Msg::Export => {
                // Firmar requiere identidad: si está bloqueada, pedimos la
                // passphrase y reanudamos el export al desbloquear.
                if model.keypair.is_none() {
                    start_unlock(&mut model, Msg::Export);
                } else {
                    commit_edits(&mut model, h);
                    model.status = Some(export_notebook(&model));
                }
            }
            Msg::Import => {
                let report = import_notebook(&mut model, h);
                persist(&model);
                model.status = Some(report);
            }
            Msg::Publish => {
                if model.keypair.is_none() {
                    start_unlock(&mut model, Msg::Publish);
                } else {
                    // Asegura que el sobre en disco refleje lo editado, luego
                    // levanta (una vez) el servidor TCP que lo sirve.
                    commit_edits(&mut model, h);
                    let _ = export_notebook(&model);
                    model.status = Some(start_publishing(&mut model, h));
                }
            }
            Msg::Receive => {
                let my_key = model.keypair.as_ref().map(|k| k.public_key());
                // Abrimos el panel de recibir ya: input de dirección
                // (prellenado, editable para WAN) + lista que se irá
                // poblando con lo que aparezca en la LAN.
                model.receiving = true;
                model.peers.clear();
                if model.peer_input.is_empty() {
                    model.peer_input.set_text(peer_addr());
                }
                model.focus = Focus::PeerAddr;
                model.status = Some("buscando pares (LAN + DHT)… o escribí una dirección".into());
                // Si hay nodo libp2p (bootstrap configurado), también
                // consultamos la DHT; lo capturamos para el worker.
                let dht = model.p2p.as_ref().map(|p| (p.rt.clone(), p.node.clone()));
                // El descubrimiento bloquea: va a un worker y reentra con la
                // lista de pares (LAN por UDP + DHT por libp2p, sin uno mismo).
                h.spawn(move || {
                    let mut infos: Vec<PeerInfo> =
                        khipu_share::discovery::descubrir(std::time::Duration::from_secs(3))
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|p| Some(p.beacon.author) != my_key)
                            .map(|p| PeerInfo {
                                addr: p.fetch_addr.to_string(),
                                label: format!(
                                    "LAN · {} · de:{} · {}",
                                    p.beacon.name,
                                    khipu_share::hex8(&p.beacon.author),
                                    p.fetch_addr
                                ),
                            })
                            .collect();
                    if let Some((rt, node)) = dht {
                        let me = node.peer_id();
                        for pid in rt.block_on(node.descubrir()) {
                            if pid == me {
                                continue;
                            }
                            let s = pid.to_string();
                            let corto: String = s.chars().rev().take(8).collect::<Vec<_>>()
                                .into_iter().rev().collect();
                            infos.push(PeerInfo {
                                label: format!("DHT · …{corto}"),
                                addr: s,
                            });
                        }
                    }
                    Msg::PeersFound(infos)
                });
            }
            Msg::PeersFound(peers) => {
                // Sólo aplica si seguimos en modo recibir (no cancelado).
                if model.receiving {
                    model.status = Some(if peers.is_empty() {
                        "ningún par en la LAN — escribí una dirección y jalá".into()
                    } else {
                        format!("{} pares en la red — elegí uno o escribí una dirección", peers.len())
                    });
                    model.peers = peers;
                }
            }
            Msg::FetchManual => {
                let addr = model.peer_input.text().trim().to_string();
                if addr.is_empty() {
                    model.status = Some("escribí una dirección host:puerto".into());
                } else {
                    h.dispatch(Msg::FetchFrom(addr));
                }
            }
            Msg::FetchFrom(addr) => {
                model.receiving = false;
                model.peers.clear();
                model.focus = Focus::None;
                let destino = addr.trim().to_string();
                if destino.starts_with('/') || !destino.contains(':') {
                    // Vía libp2p: multiaddr (`/ip4/…/p2p/<id>`, incluido
                    // circuito) o un peer-id pelado (descubierto por DHT).
                    // Arma el nodo si hace falta.
                    if ensure_p2p(&mut model) {
                        let p = model.p2p.as_ref().expect("p2p recién armado");
                        let (rt, node) = (p.rt.clone(), p.node.clone());
                        let es_multiaddr = destino.starts_with('/');
                        model.status = Some(format!("jalando por libp2p de {destino}…"));
                        h.spawn(move || {
                            let res = if es_multiaddr {
                                rt.block_on(node.fetch_addr_str(&destino))
                            } else {
                                rt.block_on(node.fetch_peer_str(&destino))
                            };
                            match res {
                                Ok(s) => Msg::Received(Ok(s)),
                                Err(e) => Msg::Received(Err(format!("p2p: {e}"))),
                            }
                        });
                    } else {
                        model.status = Some("no se pudo iniciar el nodo libp2p".into());
                    }
                } else {
                    // Dirección TCP `host:puerto` (LAN/WAN directa).
                    model.status = Some(format!("jalando de {destino}…"));
                    h.spawn(move || match khipu_share::net::fetch(&destino) {
                        Ok(s) => Msg::Received(Ok(s)),
                        Err(e) => {
                            Msg::Received(Err(format!("no se pudo recibir de {destino}: {e}")))
                        }
                    });
                }
            }
            Msg::CancelPeers => {
                model.receiving = false;
                model.peers.clear();
                model.focus = Focus::None;
                model.status = Some("recibir cancelado".into());
            }
            Msg::Received(res) => {
                model.receiving = false;
                model.peers.clear();
                model.status = Some(match res {
                    Ok(sobre) => match khipu_share::open(&sobre) {
                        Ok(bundle) => {
                            let now = now_secs();
                            let outcome =
                                khipu_share::import_into(&mut model.store, bundle, now);
                            for id in &outcome.created {
                                model.order.push(*id);
                                schedule_embedding(&mut model, *id, h);
                            }
                            persist(&model);
                            format!(
                                "recibidas {} · omitidas {} (ya existían)",
                                outcome.created.len(),
                                outcome.skipped
                            )
                        }
                        Err(_) => "firma inválida — sobre rechazado".into(),
                    },
                    Err(e) => e,
                });
            }
            Msg::Unlock => {
                let pass = model.passphrase.text();
                match unlock_identity(&pass) {
                    Some(kp) => {
                        let id = khipu_share::hex8(&kp.public_key());
                        model.keypair = Some(kp);
                        model.unlocking = false;
                        model.passphrase.clear();
                        model.focus = Focus::None;
                        model.status = Some(format!("identidad desbloqueada · {id}"));
                        // Reanudar lo que el usuario quería hacer.
                        if let Some(accion) = model.pending.take() {
                            h.dispatch(*accion);
                        }
                    }
                    None => {
                        model.status =
                            Some("passphrase incorrecta o sin acceso al keystore".into());
                    }
                }
            }
            Msg::CancelUnlock => {
                model.unlocking = false;
                model.pending = None;
                model.passphrase.clear();
                model.focus = Focus::None;
                model.status = Some("desbloqueo cancelado".into());
            }
            Msg::RelayReady(addr) => {
                model.status = Some(format!("alcanzable vía relay: {addr}"));
            }
            Msg::DeleteSelected => {
                if let Some(id) = model.selected {
                    model.store.remove(id);
                    model.order.retain(|x| *x != id);
                    model.field.remove(id);
                    let next = model.order.first().copied();
                    model.selected = None;
                    model.title.clear();
                    model.body = EditorState::default();
                    model.tags.clear();
                    if let Some(n) = next {
                        select(&mut model, n);
                    }
                    persist(&model);
                }
            }
            Msg::Focus(f) => {
                commit_edits(&mut model, h);
                model.focus = f;
            }
            Msg::Key(ev) => {
                let changed = match model.focus {
                    Focus::Title => model.title.apply_key(&ev),
                    Focus::Body => model.body.apply_key(&ev).touched(),
                    Focus::Tags => model.tags.apply_key(&ev),
                    Focus::Search => {
                        // El search no muta el store: filtramos al
                        // renderizar. Sólo consumimos el evento.
                        let _ = model.search.apply_key(&ev);
                        false
                    }
                    Focus::Passphrase => {
                        // La passphrase no toca el store; sólo el input.
                        let _ = model.passphrase.apply_key(&ev);
                        false
                    }
                    Focus::PeerAddr => {
                        let _ = model.peer_input.apply_key(&ev);
                        false
                    }
                    Focus::Region => {
                        let _ = model.region_input.apply_key(&ev);
                        false
                    }
                    Focus::None => false,
                };
                if changed {
                    commit_edits(&mut model, h);
                }
            }
            Msg::EditorPointer(ev) => {
                let metrics = EditorMetrics::for_font_size(13.0);
                match ev {
                    PointerEvent::Click { x, y } => {
                        let (line, col) = metrics.screen_to_pos(x, y, model.body.scroll_offset);
                        model.body.set_caret_at(line, col);
                    }
                    PointerEvent::Drag { initial_x, initial_y, dx, dy } => {
                        let (l0, c0) = metrics.screen_to_pos(
                            initial_x,
                            initial_y,
                            model.body.scroll_offset,
                        );
                        let (l1, c1) = metrics.screen_to_pos(
                            initial_x + dx,
                            initial_y + dy,
                            model.body.scroll_offset,
                        );
                        model.body.set_caret_at(l0, c0);
                        model.body.extend_selection_to(l1, c1);
                    }
                }
                model.focus = Focus::Body;
            }
            Msg::MenuOpen(idx) => {
                model.menu_open = idx;
                model.menu_active = usize::MAX;
                model.edit_menu = None;
                if idx.is_some() {
                    model.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(h, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuCommand(cmd) => {
                return handle_menu_command(model, cmd, h);
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    model.menu_active = menubar_nav(&menu, mi, model.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    if let Some(cmd) = menubar_command_at(&menu, mi, model.menu_active) {
                        return handle_menu_command(model, cmd, h);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::EditNav(dir) => {
                let flags = focused_edit_flags(&model);
                model.edit_active = editmenu::edit_menu_step(flags, model.edit_active, dir);
            }
            Msg::EditActivate => {
                let flags = focused_edit_flags(&model);
                if let Some(action) = editmenu::edit_menu_action_at(flags, model.edit_active) {
                    return apply_edit_menu_action(model, action, h);
                }
            }
            Msg::EditMenuOpen(x, y) => {
                model.edit_menu = Some((x, y));
                model.edit_active = usize::MAX;
                model.menu_open = None;
                model.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(h, motion::FAST, || Msg::MenuTick);
            }
            Msg::EditMenuAction(action) => {
                return apply_edit_menu_action(model, action, h);
            }
            Msg::CloseMenus => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
                model.edit_menu = None;
                model.edit_active = usize::MAX;
            }
            Msg::MapPan(dx, dy) => {
                // El delta viene en pixels de pantalla; lo llevamos a
                // mundo dividiendo por el zoom para que el arrastre se
                // sienta 1:1 con el cursor a cualquier escala.
                let z = model.cam_zoom.max(0.01);
                model.cam_pan.0 += dx / z;
                model.cam_pan.1 += dy / z;
            }
            Msg::MapZoom(dy) => {
                // dy>0 = rueda hacia el usuario (winit invierte el signo en
                // el event loop) → alejar. Factor multiplicativo, clamp para
                // no perder el mapa.
                let factor = (1.0 - dy * 0.12).clamp(0.5, 2.0);
                model.cam_zoom = (model.cam_zoom * factor).clamp(0.15, 6.0);
            }
            Msg::MapClick(lx, ly, rw, rh) => {
                model.canvas_size = (rw, rh);
                if let Some(id) = pick_note(&model, lx, ly, rw, rh) {
                    commit_edits(&mut model, h);
                    reinforce_and_touch(&mut model, id);
                    select(&mut model, id);
                    persist(&model);
                }
            }
            Msg::ToggleList => {
                model.show_list = !model.show_list;
            }
            Msg::Deselect => {
                commit_edits(&mut model, h);
                deselect(&mut model);
                persist(&model);
            }
            Msg::BeginNaming(x, y) => {
                model.naming = Some((x, y));
                model.region_input.clear();
                model.focus = Focus::Region;
            }
            Msg::CommitNaming => {
                if let Some((x, y)) = model.naming.take() {
                    let name = model.region_input.text().trim().to_string();
                    if !name.is_empty() {
                        model.regions.push(Region { name, x, y });
                        persist(&model);
                    }
                }
                model.region_input.clear();
                model.focus = Focus::None;
            }
            Msg::CancelNaming => {
                model.naming = None;
                model.region_input.clear();
                model.focus = Focus::None;
            }
            Msg::EscapeMap => {
                // Cierra la capa más cercana al usuario, en orden.
                if model.naming.is_some() {
                    model.naming = None;
                    model.region_input.clear();
                    model.focus = Focus::None;
                } else if model.selected.is_some() {
                    commit_edits(&mut model, h);
                    deselect(&mut model);
                    persist(&model);
                } else if model.show_list {
                    model.show_list = false;
                } else {
                    model.focus = Focus::None;
                }
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let palette = ListPalette::from_theme(&model.theme);
        let input_palette = TextInputPalette::from_theme(&model.theme);
        let editor_palette = EditorPalette::from_theme(&model.theme);

        // Prompt de passphrase: ocupa toda la ventana hasta resolverse.
        if model.unlocking {
            let mut children = vec![header_view(model), unlock_view(model, &input_palette)];
            if let Some(bar) = status_bar(model) {
                children.push(bar);
            }
            return View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                ..Default::default()
            })
            .fill(model.theme.bg_app)
            .children(children);
        }

        let header = header_view(model);

        // Zoom semántico: con una nota seleccionada y el mapa lo bastante
        // cerca, el nodo se "abre" como tarjeta anclada a su coordenada
        // (in-situ). Lejos, el editor cae al panel lateral — un fallback
        // para editar sin tener que acercarse.
        let inplace = model
            .selected
            .filter(|_| model.cam_zoom >= ZOOM_INJECT)
            .and_then(|id| node_screen_pos(model, id).map(|p| (id, p)));

        // El mapa es la interfaz: ocupa todo el cuerpo como capa de fondo.
        // Sobre él viajan, como hijos del canvas: la tarjeta del nodo
        // abierto (zoom semántico), los chips para bautizar clústeres
        // densos, y el input del bautizo en curso.
        let mut injected: Vec<View<Msg>> = Vec::new();
        if let Some((_, (nx, ny))) = inplace {
            let editor = editor_panel(model, &input_palette, &editor_palette);
            injected.push(node_card(editor, nx, ny, model.canvas_size, &model.theme));
        }
        // Sugerencias de bautizo (sólo si no estamos editando in-situ, para
        // no encimar la tarjeta).
        if inplace.is_none() {
            for (wx, wy) in unnamed_cluster_centroids(model) {
                let (sx, sy) = world_screen(model, wx, wy);
                injected.push(pinned(
                    name_region_chip(wx, wy, &model.theme),
                    sx,
                    sy,
                    132.0,
                    24.0,
                    model.canvas_size,
                ));
            }
        }
        // Input del bautizo en curso, anclado al centroide elegido.
        if let Some((wx, wy)) = model.naming {
            let (sx, sy) = world_screen(model, wx, wy);
            injected.push(pinned(
                naming_input(model, &input_palette),
                sx,
                sy,
                220.0,
                34.0,
                model.canvas_size,
            ));
        }
        let map = gravity_panel(model, injected);
        let mut layers: Vec<View<Msg>> = vec![map];

        // Cajón de notas (izquierda): abierto a pedido, o forzado en modo
        // recibir (muestra los pares en vez de las notas).
        if model.show_list || model.receiving {
            let drawer = if model.receiving {
                receive_panel(model, &palette, &input_palette)
            } else {
                list_panel(model, &palette, &input_palette)
            };
            layers.push(overlay_left(drawer, LIST_WIDTH));
        }

        // Editor lateral: sólo si hay selección y NO se abrió in-situ.
        if model.selected.is_some() && inplace.is_none() {
            let editor = editor_panel(model, &input_palette, &editor_palette);
            layers.push(overlay_right(editor, EDITOR_OVERLAY_W, &model.theme));
        }

        let body = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            min_size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(layers);

        // Barra de menú principal: primer hijo del column raíz.
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &model.theme));

        let mut children = vec![menubar, header, body];
        if let Some(bar) = status_bar(model) {
            children.push(bar);
        }

        // El right-click se engancha en la raíz (origen 0,0 → coords
        // locales == coords de ventana) y abre el menú de edición sobre
        // el campo focuseado.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
        .children(children)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // El prompt modal de passphrase no convive con menús.
        if model.unlocking {
            return None;
        }
        // Prioridad: menú de edición contextual sobre el menú principal.
        if let Some((x, y)) = model.edit_menu {
            let flags = focused_edit_flags(model);
            let (w, h) = Self::initial_size();
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                (w as f32, h as f32),
                &model.theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras {
                    appear: model.edit_anim.value(),
                    ..Default::default()
                },
            ));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &model.theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        // Menús abiertos: las flechas navegan y tienen prioridad sobre todo.
        if event.state == KeyState::Pressed {
            if let Some(mi) = model.menu_open {
                let n = app_menu(model).menus.len().max(1);
                return match &event.key {
                    Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                    Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                    Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                    Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                    Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                    Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                    _ => None,
                };
            }
            if model.edit_menu.is_some() {
                return match &event.key {
                    Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                    Key::Named(NamedKey::ArrowDown) => Some(Msg::EditNav(1)),
                    Key::Named(NamedKey::ArrowUp) => Some(Msg::EditNav(-1)),
                    Key::Named(NamedKey::Enter) => Some(Msg::EditActivate),
                    _ => None,
                };
            }
        }
        // Con el prompt de passphrase abierto, las teclas son sólo suyas:
        // Enter desbloquea, Esc cancela, el resto va al input.
        if model.unlocking {
            if event.state == KeyState::Pressed && !event.repeat {
                if matches!(&event.key, Key::Named(NamedKey::Enter)) {
                    return Some(Msg::Unlock);
                }
                if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                    return Some(Msg::CancelUnlock);
                }
            }
            return Some(Msg::Key(event.clone()));
        }
        // En modo recibir con foco en la dirección: Enter jala, Esc cancela.
        if model.receiving && model.focus == Focus::PeerAddr {
            if event.state == KeyState::Pressed && !event.repeat {
                if matches!(&event.key, Key::Named(NamedKey::Enter)) {
                    return Some(Msg::FetchManual);
                }
                if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                    return Some(Msg::CancelPeers);
                }
            }
            return Some(Msg::Key(event.clone()));
        }
        // Bautizando una región: Enter confirma, Esc cancela, resto al input.
        if model.naming.is_some() && model.focus == Focus::Region {
            if event.state == KeyState::Pressed && !event.repeat {
                if matches!(&event.key, Key::Named(NamedKey::Enter)) {
                    return Some(Msg::CommitNaming);
                }
                if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                    return Some(Msg::CancelNaming);
                }
            }
            return Some(Msg::Key(event.clone()));
        }
        // Atajo global: Ctrl+N (sin foco en input necesario) crea
        // nota. Esc libera el foco. Cualquier otra tecla la dispatcha
        // como `Key` al input/editor focado.
        if event.state == KeyState::Pressed && !event.repeat {
            if event.modifiers.ctrl
                && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("n"))
            {
                return Some(Msg::NewNote);
            }
            if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::EscapeMap);
            }
        }
        Some(Msg::Key(event.clone()))
    }

    fn title() -> &'static str {
        "khipu"
    }

    fn app_id() -> Option<&'static str> {
        Some("gioser.khipu")
    }

    fn initial_size() -> (u32, u32) {
        (1280, 760)
    }
}

fn header_view(model: &Model) -> View<Msg> {
    let title = format!("khipu · {} notas", model.store.len());
    let title_node = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(title, 14.0, model.theme.fg_text, Alignment::Start);

    let list_label = if model.show_list { "ocultar notas" } else { "☰ notas" };
    let list_btn = button(
        list_label,
        model.theme.bg_button,
        if model.show_list { model.theme.accent } else { model.theme.fg_muted },
        Msg::ToggleList,
    );
    let new_btn = button(
        "+ nueva  (Ctrl+N)",
        model.theme.bg_button,
        model.theme.fg_text,
        Msg::NewNote,
    );
    let archive_label = if model.show_archive {
        "ocultar archivo"
    } else {
        "ver archivo"
    };
    let archive_btn = button(
        archive_label,
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::ToggleArchive,
    );
    let del_btn = button(
        "borrar",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::DeleteSelected,
    );
    let export_btn = button(
        "exportar",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::Export,
    );
    let import_btn = button(
        "importar",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::Import,
    );
    let publish_label = if model.publishing {
        "publicando"
    } else {
        "publicar"
    };
    let publish_btn = button(
        publish_label,
        model.theme.bg_button,
        if model.publishing {
            model.theme.accent
        } else {
            model.theme.fg_muted
        },
        Msg::Publish,
    );
    let receive_btn = button(
        "recibir",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::Receive,
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_H),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(0.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel_alt)
    .children(vec![
        title_node,
        list_btn,
        new_btn,
        archive_btn,
        del_btn,
        export_btn,
        import_btn,
        publish_btn,
        receive_btn,
    ])
}

fn button(label: &str, bg: Color, fg: Color, msg: Msg) -> View<Msg> {
    // El ancho crece con el largo del texto — los labels más
    // explícitos («+ nueva (Ctrl+N)», «ocultar archivo») piden más
    // espacio que un «borrar» seco.
    let chars = label.chars().count() as f32;
    let width = (chars * 7.2 + 22.0).max(86.0);
    View::new(Style {
        size: Size {
            width: length(width),
            height: length(26.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(4.0)
    .text_aligned(label.to_string(), 11.0, fg, Alignment::Center)
    .on_click(msg)
}

fn list_panel(
    model: &Model,
    palette: &ListPalette,
    input_palette: &TextInputPalette,
) -> View<Msg> {
    let now = now_secs();
    let query = model.search.text();
    let q = query.trim();

    // Particionamos en horizonte vs archivo y ordenamos cada parte por
    // masa viva decreciente. Si hay query, ambas listas quedan
    // pre-filtradas por coincidencia en título/cuerpo/etiquetas.
    let mut visible: Vec<(NoteId, f32, &Note)> = Vec::new();
    let mut archive: Vec<(NoteId, f32, &Note)> = Vec::new();
    let mut hidden_by_query = 0usize;
    for id in &model.order {
        let Some(n) = model.store.get(*id) else {
            continue;
        };
        if !q.is_empty() && !note_matches(n, q) {
            hidden_by_query += 1;
            continue;
        }
        let m = current_mass(&model.gravity, n, now);
        if model.gravity.is_visible(m) {
            visible.push((*id, m, n));
        } else {
            archive.push((*id, m, n));
        }
    }
    let by_mass_desc = |a: &(NoteId, f32, &Note), b: &(NoteId, f32, &Note)| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    };
    visible.sort_by(by_mass_desc);
    archive.sort_by(by_mass_desc);

    let mut chain: Vec<(NoteId, f32, &Note)> = visible.clone();
    if model.show_archive {
        chain.extend(archive.iter().cloned());
    }

    let rows: Vec<ListRow<Msg>> = chain
        .into_iter()
        .map(|(id, mass, n)| ListRow {
            label: row_label(n, mass),
            selected: Some(id) == model.selected,
            on_click: Msg::SelectNote(id),
        })
        .collect();

    let caption = if !q.is_empty() {
        format!(
            "buscar «{}» · {}/{} coinciden",
            q,
            visible.len() + if model.show_archive { archive.len() } else { 0 },
            visible.len() + archive.len() + hidden_by_query
        )
    } else if archive.is_empty() {
        format!("notas · {}", visible.len())
    } else if model.show_archive {
        format!(
            "notas · {} horizonte + {} archivo",
            visible.len(),
            archive.len()
        )
    } else {
        format!(
            "notas · {} horizonte (+{} archivo)",
            visible.len(),
            archive.len()
        )
    };

    let spec = ListSpec {
        total: rows.len(),
        rows,
        caption: Some(caption),
        truncated_hint: None,
        row_height: ROW_H,
        palette: *palette,
    };

    let search_input = text_input_view(
        &model.search,
        "buscar (título, cuerpo, etiquetas)",
        model.focus == Focus::Search,
        input_palette,
        Msg::Focus(Focus::Search),
    );
    let search_row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel_alt)
    .children(vec![search_input]);

    let list_wrap = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![list_view(spec)]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(LIST_WIDTH),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![search_row, list_wrap])
}

/// Panel izquierdo en modo "recibir": arriba un input de dirección manual
/// (`host:puerto`, habilita WAN) con botones jalar/cancelar; debajo, la
/// lista de pares descubiertos en la LAN (click ⇒ jalar de él). Reemplaza
/// transitoriamente la lista de notas.
fn receive_panel(
    model: &Model,
    palette: &ListPalette,
    input_palette: &TextInputPalette,
) -> View<Msg> {
    // Fila de dirección manual + jalar.
    let addr_input = text_input_view(
        &model.peer_input,
        "host:puerto  o  /ip4/…/p2p/…",
        model.focus == Focus::PeerAddr,
        input_palette,
        Msg::Focus(Focus::PeerAddr),
    );
    let addr_wrap = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(26.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![addr_input]);
    let jalar = button(
        "jalar",
        model.theme.bg_button,
        model.theme.accent,
        Msg::FetchManual,
    );
    let addr_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel_alt)
    .children(vec![addr_wrap, jalar]);

    let cancel = button(
        "cancelar",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::CancelPeers,
    );
    let cancel_row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(2.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(model.theme.bg_panel_alt)
    .children(vec![cancel]);

    let rows: Vec<ListRow<Msg>> = model
        .peers
        .iter()
        .map(|p| ListRow {
            label: p.label.clone(),
            selected: false,
            on_click: Msg::FetchFrom(p.addr.clone()),
        })
        .collect();
    let caption = if model.peers.is_empty() {
        "pares en la LAN: ninguno aún".to_string()
    } else {
        format!("pares en la LAN · {} (click para jalar)", model.peers.len())
    };
    let spec = ListSpec {
        total: rows.len(),
        rows,
        caption: Some(caption),
        truncated_hint: None,
        row_height: ROW_H,
        palette: *palette,
    };
    let list_wrap = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![list_view(spec)]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(LIST_WIDTH),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![addr_row, cancel_row, list_wrap])
}

/// Coincidencia sobre título, cuerpo y etiquetas. Case-insensitive.
fn note_matches(n: &Note, query: &str) -> bool {
    if n.matches(query) {
        return true;
    }
    let q = query.to_lowercase();
    n.tags.iter().any(|t| t.to_lowercase().contains(&q))
}

fn row_label(n: &Note, mass: f32) -> String {
    let title = if n.title.is_empty() {
        "(sin título)"
    } else {
        n.title.as_str()
    };
    // Una barra de tres bloques visualiza la masa (0..1.5 mapeada a
    // 0..3). Sobre el horizonte se ve llena; cayendo, se vacía.
    let bars = (mass.clamp(0.0, 1.5) / 0.5).round() as usize;
    let glyph: String = (0..3)
        .map(|i| if i < bars { '▮' } else { '▯' })
        .collect();
    format!("{glyph}  {title}")
}

fn editor_panel(
    model: &Model,
    input_palette: &TextInputPalette,
    editor_palette: &EditorPalette,
) -> View<Msg> {
    let none_view = || -> View<Msg> {
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(model.theme.bg_panel)
        .text_aligned(
            "selecciona o crea una nota".to_string(),
            12.0,
            model.theme.fg_muted,
            Alignment::Center,
        )
    };

    if model.selected.is_none() {
        return wrap_panel(model, none_view());
    }

    let metrics = EditorMetrics::for_font_size(13.0);

    let title_field = field(
        model,
        "título",
        text_input_view(
            &model.title,
            "(sin título)",
            model.focus == Focus::Title,
            input_palette,
            Msg::Focus(Focus::Title),
        ),
    );

    let body_input = text_editor_view(
        &model.body,
        editor_palette,
        metrics,
        EDITOR_VISIBLE_LINES,
        |ev| Some(Msg::EditorPointer(ev)),
    );
    let body_field = body_field_view(model, body_input);

    let tags_field = field(
        model,
        "etiquetas (coma separadas)",
        text_input_view(
            &model.tags,
            "p. ej. cocina, jardín",
            model.focus == Focus::Tags,
            input_palette,
            Msg::Focus(Focus::Tags),
        ),
    );

    let stats = stats_view(model);

    let column = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel)
    .children(vec![title_field, body_field, tags_field, stats]);

    wrap_panel(model, column)
}

fn wrap_panel(_model: &Model, child: View<Msg>) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![child])
}

fn field(model: &Model, label: &str, control: View<Msg>) -> View<Msg> {
    let label_node = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        label.to_string(),
        FIELD_LABEL_SIZE,
        model.theme.fg_muted,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_shrink: 0.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(vec![label_node, control])
}

fn body_field_view(model: &Model, editor: View<Msg>) -> View<Msg> {
    let label_node = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "cuerpo (wiki-links con [[Título]])".to_string(),
        FIELD_LABEL_SIZE,
        model.theme.fg_muted,
        Alignment::Start,
    );

    let focused = model.focus == Focus::Body;
    let border = if focused {
        model.theme.border_focus
    } else {
        model.theme.border
    };

    let editor_wrap = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(border)
    .radius(4.0)
    .on_click(Msg::Focus(Focus::Body))
    .children(vec![editor]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(vec![label_node, editor_wrap])
}

fn stats_view(model: &Model) -> View<Msg> {
    let Some(id) = model.selected else {
        return View::new(Style::default());
    };
    let fwd = model.store.forward_links(id);
    let back = model.store.backlinks(id);
    let fwd_titles: Vec<String> = fwd
        .iter()
        .filter_map(|i| model.store.get(*i).map(|n| n.title.clone()))
        .collect();
    let back_titles: Vec<String> = back
        .iter()
        .filter_map(|i| model.store.get(*i).map(|n| n.title.clone()))
        .collect();
    let nearest: Vec<String> = model
        .field
        .nearest(id, 3)
        .into_iter()
        .filter_map(|(nid, score)| {
            model
                .store
                .get(nid)
                .map(|n| format!("{} ({:.2})", n.title, score))
        })
        .collect();

    let mut lines = vec![
        format!("→ enlaza a: {}", join_or_dash(&fwd_titles)),
        format!("← backlinks: {}", join_or_dash(&back_titles)),
        format!("∼ vecinos: {}", join_or_dash(&nearest)),
    ];
    // Procedencia: si la nota llegó por compartir, lleva una etiqueta
    // `de:<autor>`. La mostramos explícita.
    if let Some(n) = model.store.get(id) {
        let autores: Vec<&str> = n
            .tags
            .iter()
            .filter_map(|t| t.strip_prefix("de:"))
            .collect();
        if !autores.is_empty() {
            lines.push(format!("✎ de: {}", autores.join(", ")));
        }
    }

    let nodes: Vec<View<Msg>> = lines
        .into_iter()
        .map(|s| {
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(16.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(s, 11.0, model.theme.fg_muted, Alignment::Start)
        })
        .collect();

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(nodes)
}

fn join_or_dash(items: &[String]) -> String {
    if items.is_empty() {
        "—".to_string()
    } else {
        items.join(", ")
    }
}

/// Un nodo del mapa, ya resuelto a coordenadas de mundo + su masa viva.
/// Datos planos para viajar dentro de la closure de pintura.
struct MapNode {
    id: NoteId,
    /// Coordenadas de mundo (el domicilio fijo de la nota).
    x: f32,
    y: f32,
    /// Masa "vivida" en el instante del render: enciende el brillo y el
    /// tamaño. Decae con el tiempo → el mapa respira sin que toques nada.
    mass: f32,
    /// `false` si cayó bajo el horizonte (sólo se ve con archivo activo).
    visible: bool,
    color: Color,
    label: String,
}

/// Mundo → pantalla local (relativa al rect del lienzo). El centro del
/// rect es el ancla del zoom; `pan` se suma en mundo, luego se escala.
fn world_to_local(wx: f32, wy: f32, w: f32, h: f32, pan: (f32, f32), zoom: f32) -> (f32, f32) {
    (w * 0.5 + (wx + pan.0) * zoom, h * 0.5 + (wy + pan.1) * zoom)
}

/// Inversa de [`world_to_local`]: pantalla local → mundo. Para resolver
/// qué nota cae bajo un click.
fn local_to_world(lx: f32, ly: f32, w: f32, h: f32, pan: (f32, f32), zoom: f32) -> (f32, f32) {
    let z = zoom.max(1e-3);
    ((lx - w * 0.5) / z - pan.0, (ly - h * 0.5) / z - pan.1)
}

/// La nota colocada más cercana a un click en coords locales, dentro de un
/// radio de tolerancia (~18 px de pantalla). `None` si el click cae en el
/// vacío — así arrastrar el fondo no cambia la selección.
fn pick_note(model: &Model, lx: f32, ly: f32, w: f32, h: f32) -> Option<NoteId> {
    let (wx, wy) = local_to_world(lx, ly, w, h, model.cam_pan, model.cam_zoom);
    let now = now_secs();
    let mut best: Option<(NoteId, f32)> = None;
    for id in &model.order {
        let Some(n) = model.store.get(*id) else { continue };
        let Some((px, py)) = n.pos else { continue };
        if !model.show_archive {
            let m = current_mass(&model.gravity, n, now);
            if !model.gravity.is_visible(m) {
                continue;
            }
        }
        let d2 = (px - wx).powi(2) + (py - wy).powi(2);
        if best.map(|(_, bd)| d2 < bd).unwrap_or(true) {
            best = Some((*id, d2));
        }
    }
    let tol = (18.0 / model.cam_zoom.max(1e-3)).powi(2);
    best.filter(|(_, d2)| *d2 <= tol).map(|(id, _)| id)
}

/// Separación mínima entre nodos al colocarlos (coordenadas de mundo).
const MAP_MIN_SEP: f32 = 30.0;
/// Ángulo áureo en radianes — reparte determinísticamente lo que no tiene
/// parentela semántica sin amontonarlo.
const GOLDEN_ANGLE: f32 = 2.399_963_2;

/// Le da a `id` un domicilio fijo en el mapa, **una sola vez**: cae en el
/// baricentro de sus parientes semánticos (ponderado por afinidad) y, si
/// quedó pegada a otra nota, se separa apenas. Determinista y dependiente
/// sólo de las notas ya asentadas, así el orden de inserción es estable y
/// el mapa nunca se reacomoda solo.
fn place_note(model: &mut Model, id: NoteId) {
    if model.store.get(id).map(|n| n.pos.is_some()).unwrap_or(true) {
        return; // ya tiene domicilio (o no existe): no se mueve.
    }
    // Vecinos ya colocados: su afinidad con la nota nueva y su posición.
    let mut kin: Vec<(f32, (f32, f32))> = Vec::new();
    for other in &model.order {
        if *other == id {
            continue;
        }
        let Some(pos) = model.store.get(*other).and_then(|n| n.pos) else { continue };
        let aff = model.field.affinity(id, *other).unwrap_or(0.0).max(0.0);
        kin.push((aff, pos));
    }

    let target = if kin.is_empty() {
        (0.0, 0.0) // primera nota del cuaderno: centro del mundo.
    } else {
        let wsum: f32 = kin.iter().map(|(w, _)| *w).sum();
        if wsum > 1e-3 {
            // Cae junto a su parentela: baricentro ponderado por afinidad.
            let (mut tx, mut ty) = (0.0_f32, 0.0_f32);
            for (w, (x, y)) in &kin {
                tx += w * x;
                ty += w * y;
            }
            (tx / wsum, ty / wsum)
        } else {
            // Ortogonal a todo: anillo determinista por id, lejos del núcleo.
            let ang = id as f32 * GOLDEN_ANGLE;
            let rad = 180.0 + 14.0 * (id as f32).sqrt();
            (rad * ang.cos(), rad * ang.sin())
        }
    };

    // Separación: empuja el target hasta despegarlo de cada vecino cercano.
    let mut p = target;
    for _ in 0..12 {
        let mut moved = false;
        for (_, q) in &kin {
            let dx = p.0 - q.0;
            let dy = p.1 - q.1;
            let d = (dx * dx + dy * dy).sqrt();
            if d < MAP_MIN_SEP {
                let (ux, uy) = if d > 1e-3 {
                    (dx / d, dy / d)
                } else {
                    let a = id as f32 * GOLDEN_ANGLE;
                    (a.cos(), a.sin())
                };
                let push = MAP_MIN_SEP - d;
                p.0 += ux * push;
                p.1 += uy * push;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }

    model.store.set_pos(id, p.0, p.1);
}

/// Envuelve `child` como cajón absoluto pegado al borde izquierdo, alto
/// completo con un margen. El mapa de fondo sigue recibiendo pan/zoom en
/// el resto de la ventana; sólo los clicks sobre el cajón los come él.
fn overlay_left(child: View<Msg>, width: f32) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
            right: auto(),
        },
        size: Size {
            width: length(width),
            height: auto(),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![child])
}

/// Columna interna del editor: barra de cierre (× ⇒ deselecciona) arriba +
/// el editor abajo, sobre `bg_panel`. La comparten el overlay lateral y la
/// tarjeta anclada del zoom semántico.
fn editor_shell(child: View<Msg>, theme: &Theme) -> View<Msg> {
    let close = button("× cerrar", theme.bg_button, theme.fg_muted, Msg::Deselect);
    let close_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        justify_content: Some(JustifyContent::End),
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![close]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![close_row, child])
}

/// Envuelve `child` como panel absoluto pegado al borde derecho, alto
/// completo, con barra de cierre. El editor del nodo abierto cuando se lo
/// edita de lejos (zoom bajo): un fallback práctico al anclaje in-situ.
fn overlay_right(child: View<Msg>, width: f32, theme: &Theme) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: auto(),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
            right: length(8.0_f32),
        },
        size: Size {
            width: length(width),
            height: auto(),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![editor_shell(child, theme)])
}

/// Mundo → pantalla local usando el último tamaño de lienzo conocido + la
/// cámara. La versión de `view()`, donde el rect real aún no se sabe.
fn world_screen(model: &Model, wx: f32, wy: f32) -> (f32, f32) {
    let (w, h) = model.canvas_size;
    world_to_local(wx, wy, w, h, model.cam_pan, model.cam_zoom)
}

/// Posición de pantalla (local al lienzo) del nodo `id`. `None` si la nota
/// no tiene domicilio todavía.
fn node_screen_pos(model: &Model, id: NoteId) -> Option<(f32, f32)> {
    let (wx, wy) = model.store.get(id).and_then(|n| n.pos)?;
    Some(world_screen(model, wx, wy))
}

/// Mínimo de notas para que un clúster cuente como región candidata.
const REGION_MIN_MEMBERS: usize = 3;
/// Distancia de mundo dentro de la cual una región ya "posee" un clúster:
/// si hay un topónimo así de cerca del centroide, no se vuelve a ofrecer.
const REGION_MATCH_DIST: f32 = 140.0;

/// Centroides (mundo) de los clústeres densos que todavía no tienen una
/// región cerca — los lugares que el mapa ofrece bautizar. Sólo cuentan
/// miembros colocados y visibles.
fn unnamed_cluster_centroids(model: &Model) -> Vec<(f32, f32)> {
    let now = now_secs();
    let mut out = Vec::new();
    for cluster in model.field.clusters(CLUSTER_THRESHOLD) {
        let pts: Vec<(f32, f32)> = cluster
            .iter()
            .filter_map(|id| {
                let n = model.store.get(*id)?;
                let p = n.pos?;
                let m = current_mass(&model.gravity, n, now);
                (model.show_archive || model.gravity.is_visible(m)).then_some(p)
            })
            .collect();
        if pts.len() < REGION_MIN_MEMBERS {
            continue;
        }
        let (sx, sy) = pts.iter().fold((0.0, 0.0), |(ax, ay), (x, y)| (ax + x, ay + y));
        let c = (sx / pts.len() as f32, sy / pts.len() as f32);
        let d2 = REGION_MATCH_DIST * REGION_MATCH_DIST;
        let near_named = model
            .regions
            .iter()
            .any(|r| (r.x - c.0).powi(2) + (r.y - c.1).powi(2) <= d2);
        let naming_here = model
            .naming
            .map(|(nx, ny)| (nx - c.0).powi(2) + (ny - c.1).powi(2) <= d2)
            .unwrap_or(false);
        if !near_named && !naming_here {
            out.push(c);
        }
    }
    out
}

/// Chip clickeable "✛ nombrar zona" que ofrece bautizar el clúster denso
/// en `(wx, wy)`. Al click abre el input de bautizo en esa coordenada.
fn name_region_chip(wx: f32, wy: f32, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .radius(12.0)
    .hover_fill(theme.bg_button_hover)
    .text_aligned("✛ nombrar zona", 11.0, theme.fg_muted, Alignment::Center)
    .on_click(Msg::BeginNaming(wx, wy))
}

/// Mini-input del bautizo en curso: una tarjeta con el campo de texto
/// enfocado. Enter confirma, Esc cancela (en `on_key`).
fn naming_input(model: &Model, input_palette: &TextInputPalette) -> View<Msg> {
    let input = text_input_view(
        &model.region_input,
        "nombre de la zona…",
        model.focus == Focus::Region,
        input_palette,
        Msg::Focus(Focus::Region),
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel)
    .radius(8.0)
    .children(vec![input])
}

/// Posiciona `child` como vista absoluta de tamaño `(w, h)` centrada en la
/// pantalla `(sx, sy)`, clampeada al lienzo. Para chips y mini-inputs que
/// viven en el mapa (sugerencia de bautizo, input de nombre).
fn pinned(child: View<Msg>, sx: f32, sy: f32, w: f32, h: f32, canvas: (f32, f32)) -> View<Msg> {
    let left = (sx - w * 0.5).clamp(4.0, (canvas.0 - w - 4.0).max(4.0));
    let top = (sy - h * 0.5).clamp(4.0, (canvas.1 - h - 4.0).max(4.0));
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(left),
            top: length(top),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(w),
            height: length(h),
        },
        ..Default::default()
    })
    .children(vec![child])
}

/// La tarjeta del nodo abierto, anclada a su coordenada `(nx, ny)` de
/// pantalla: el zoom semántico hecho carne — el editor vive EN el lugar del
/// pensamiento, no en un panel aparte. Se clampea para no salirse del
/// lienzo. Hija del canvas, así pan/zoom la arrastran con el nodo.
fn node_card(child: View<Msg>, nx: f32, ny: f32, canvas: (f32, f32), theme: &Theme) -> View<Msg> {
    let (cw_max, ch_max) = canvas;
    let cw = 380.0_f32.min((cw_max - 16.0).max(220.0));
    let ch = 440.0_f32.min((ch_max - 16.0).max(200.0));
    // Anclada bajo el nodo, centrada en X, clampeada a la ventana.
    let left = (nx - cw * 0.5).clamp(8.0, (cw_max - cw - 8.0).max(8.0));
    let top = (ny + 16.0).clamp(8.0, (ch_max - ch - 8.0).max(8.0));

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(left),
            top: length(top),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(cw),
            height: length(ch),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .radius(8.0)
    .children(vec![editor_shell(child, theme)])
}

fn gravity_panel(model: &Model, injected: Vec<View<Msg>>) -> View<Msg> {
    let theme = model.theme;
    let now = now_secs();
    let clusters = model.field.clusters(CLUSTER_THRESHOLD);
    let selected = model.selected;
    let pan = model.cam_pan;
    let zoom = model.cam_zoom;

    // Nodos colocados (los que ya tienen domicilio), con su masa viva.
    let mut nodes: Vec<MapNode> = Vec::new();
    for id in &model.order {
        let Some(n) = model.store.get(*id) else { continue };
        let Some((x, y)) = n.pos else { continue };
        let mass = current_mass(&model.gravity, n, now);
        let visible = model.gravity.is_visible(mass);
        if !visible && !model.show_archive {
            continue;
        }
        nodes.push(MapNode {
            id: *id,
            x,
            y,
            mass,
            visible,
            color: cluster_color(*id, &clusters, theme),
            label: short_label(&n.title),
        });
    }

    // Topónimos: las regiones bautizadas, para pintarlas como rótulos de
    // continente detrás de los nodos.
    let regions: Vec<(String, f32, f32)> = model
        .regions
        .iter()
        .map(|r| (r.name.clone(), r.x, r.y))
        .collect();

    // Filamentos del nodo seleccionado: sus parientes más afines ya
    // colocados. Elegir un pensamiento enciende sus vecinos (activación
    // por difusión) — el motor de serendipia.
    let mut links: Vec<((f32, f32), (f32, f32), f32)> = Vec::new();
    if let Some(sel) = selected {
        if let Some(sp) = model.store.get(sel).and_then(|n| n.pos) {
            for (nid, aff) in model.field.nearest(sel, 6) {
                if aff < 0.20 {
                    continue;
                }
                if let Some(np) = model.store.get(nid).and_then(|n| n.pos) {
                    links.push((sp, np, aff));
                }
            }
        }
    }

    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .paint_with(move |scene, ts, rect| {
        paint_map(scene, ts, rect, &nodes, &links, &regions, selected, pan, zoom, theme);
    })
    .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
        DragPhase::Move => Some(Msg::MapPan(dx, dy)),
        DragPhase::End => None,
    })
    .on_scroll(|_dx, dy| Some(Msg::MapZoom(dy)))
    .on_click_at(|lx, ly, w, h| Some(Msg::MapClick(lx, ly, w, h)))
    // La tarjeta del nodo abierto (zoom semántico) viaja como hija del
    // canvas: se pinta encima de los nodos y la cámara la arrastra con el
    // pensamiento al que pertenece.
    .children(injected);

    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![canvas])
}

#[allow(clippy::too_many_arguments)]
fn paint_map(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: llimphi_ui::PaintRect,
    nodes: &[MapNode],
    links: &[((f32, f32), (f32, f32), f32)],
    regions: &[(String, f32, f32)],
    selected: Option<NoteId>,
    pan: (f32, f32),
    zoom: f32,
    theme: Theme,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    // Pantalla absoluta = origen del rect + pantalla local.
    let to_screen = |wx: f32, wy: f32| -> (f64, f64) {
        let (lx, ly) = world_to_local(wx, wy, rect.w, rect.h, pan, zoom);
        ((rect.x + lx) as f64, (rect.y + ly) as f64)
    };

    // Topónimos al fondo: el nombre de cada región, grande y tenue, como
    // rótulo de continente; un halo suave insinúa su territorio.
    for (name, rx, ry) in regions {
        let (cx, cy) = to_screen(*rx, *ry);
        let blob = KurboCircle::new((cx, cy), (96.0 * zoom as f64).max(34.0));
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(theme.accent, 0.05),
            None,
            &blob,
        );
        let size = (15.0 * zoom).clamp(11.0, 28.0);
        // Centrado aproximado: `simple` alinea a la izquierda en (x, y).
        let est_w = name.chars().count() as f64 * size as f64 * 0.52;
        draw_block(
            scene,
            ts,
            &TextBlock::simple(
                name,
                size,
                with_alpha(theme.fg_text, 0.30),
                (cx - est_w * 0.5, cy - size as f64 * 0.6),
            ),
        );
    }

    // Filamentos primero (debajo de los nodos). Más opacos cuanto más afín.
    for (a, b, aff) in links {
        let (ax, ay) = to_screen(a.0, a.1);
        let (bx, by) = to_screen(b.0, b.1);
        let mut path = BezPath::new();
        path.move_to((ax, ay));
        path.line_to((bx, by));
        let alpha = (0.18 + aff * 0.55).clamp(0.0, 0.85);
        scene.stroke(
            &Stroke::new((0.8 + *aff as f64 * 1.6).max(0.6)),
            Affine::IDENTITY,
            with_alpha(theme.accent, alpha),
            None,
            &path,
        );
    }

    // Nodos: tamaño y brillo crecen con la masa viva (el mapa respira).
    for n in nodes {
        let (px, py) = to_screen(n.x, n.y);
        let m = n.mass.clamp(0.0, 2.0);
        // Radio base por masa, escalado apenas por zoom para no inflarse.
        let r = (3.0 + m * 4.5) * (0.6 + 0.4 * zoom.clamp(0.5, 1.5));
        // Brillo: las notas frescas arden; las que se enfrían se apagan
        // hacia el fondo. Bajo el horizonte (archivo) van casi transparentes.
        let glow = if n.visible {
            (0.35 + m * 0.45).clamp(0.0, 1.0)
        } else {
            0.18
        };
        let color = with_alpha(n.color, glow);
        // Halo tenue alrededor de las notas más encendidas.
        if n.visible && m > 0.6 {
            let halo = KurboCircle::new((px, py), (r + 5.0) as f64);
            scene.fill(Fill::NonZero, Affine::IDENTITY, with_alpha(n.color, 0.10), None, &halo);
        }
        let circle = KurboCircle::new((px, py), r as f64);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &circle);

        if selected == Some(n.id) {
            let ring = KurboCircle::new((px, py), (r + 3.0) as f64);
            scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, theme.accent, None, &ring);
        }

        // Etiqueta: sólo si el zoom da espacio o es la seleccionada — para
        // no saturar el mapa lejano. El texto sale del Typesetter.
        if (zoom >= 0.9 || selected == Some(n.id)) && n.visible {
            let lbl_col = with_alpha(theme.fg_text, (glow + 0.25).clamp(0.0, 1.0));
            draw_block(
                scene,
                ts,
                &TextBlock::simple(&n.label, 10.0, lbl_col, (px + r as f64 + 4.0, py - 7.0)),
            );
        }
    }
}

fn cluster_color(id: NoteId, clusters: &[Vec<NoteId>], theme: Theme) -> Color {
    let idx = clusters.iter().position(|c| c.contains(&id)).unwrap_or(0);
    // Paleta tomada del theme + matices generados por golden-ratio
    // sobre el hue del accent. Determinista por índice.
    let palette: [Color; 6] = [
        theme.accent,
        with_alpha(rotate_hue(theme.accent, 0.16), 1.0),
        with_alpha(rotate_hue(theme.accent, 0.33), 1.0),
        with_alpha(rotate_hue(theme.accent, 0.50), 1.0),
        with_alpha(rotate_hue(theme.accent, 0.66), 1.0),
        with_alpha(rotate_hue(theme.accent, 0.83), 1.0),
    ];
    palette[idx % palette.len()]
}

fn with_alpha(c: Color, alpha: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, alpha])
}

fn rotate_hue(c: Color, dh: f32) -> Color {
    // RGB → HSV → rota H → RGB. Aproximación, alpha fijo.
    let [r, g, b, a] = c.components;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { (max - min) / max };
    let h = if (max - min).abs() < 1e-6 {
        0.0
    } else if max == r {
        ((g - b) / (max - min)) % 6.0
    } else if max == g {
        (b - r) / (max - min) + 2.0
    } else {
        (r - g) / (max - min) + 4.0
    };
    let h2 = ((h / 6.0) + dh).rem_euclid(1.0) * 6.0;
    let c2 = v * s;
    let x = c2 * (1.0 - ((h2 % 2.0) - 1.0).abs());
    let (r2, g2, b2) = match h2 as i32 {
        0 => (c2, x, 0.0),
        1 => (x, c2, 0.0),
        2 => (0.0, c2, x),
        3 => (0.0, x, c2),
        4 => (x, 0.0, c2),
        _ => (c2, 0.0, x),
    };
    let m = v - c2;
    Color::new([r2 + m, g2 + m, b2 + m, a])
}

fn short_label(s: &str) -> String {
    let mut out: String = s.chars().take(24).collect();
    if s.chars().count() > 24 {
        out.push('…');
    }
    out
}

/// Sincroniza inputs/editor → store/field + persiste si cambió algo.
fn commit_edits(model: &mut Model, h: &Handle<Msg>) {
    let Some(id) = model.selected else {
        return;
    };
    let mut changed = false;
    let new_title = model.title.text();
    let new_body = model.body.text();
    let new_tags = parse_tags(&model.tags.text());
    let now = now_secs();
    if let Some(note) = model.store.get_mut(id) {
        if note.title != new_title {
            note.title = new_title;
            note.updated_at = now;
            changed = true;
        }
        if note.body != new_body {
            note.body = new_body;
            note.updated_at = now;
            changed = true;
        }
        if note.tags != new_tags {
            note.tags = new_tags;
            note.updated_at = now;
            changed = true;
        }
    }
    if changed {
        // El texto ya está en el store: persistimos de inmediato para no
        // perderlo. El embedding viaja a un worker y persistirá de nuevo
        // cuando llegue (`Msg::EmbeddingReady`).
        persist(model);
        schedule_embedding(model, id, h);
    }
}

// =====================================================================
// Menú principal + menú de edición contextual
// =====================================================================

/// Devuelve el `EditorState` del campo focuseado (referencia inmutable) y
/// si está enmascarado (passphrase). Search/PeerAddr/Title/Tags son
/// `TextInputState` (su `.editor()`); Body es el `EditorState` directo.
/// Sin foco editable devuelve `None`.
fn focused_editor(model: &Model) -> (Option<&EditorState>, bool) {
    match model.focus {
        Focus::Body => (Some(&model.body), false),
        Focus::Title => (Some(model.title.editor()), false),
        Focus::Tags => (Some(model.tags.editor()), false),
        Focus::Search => (Some(model.search.editor()), false),
        Focus::PeerAddr => (Some(model.peer_input.editor()), false),
        Focus::Region => (Some(model.region_input.editor()), false),
        Focus::Passphrase => (Some(model.passphrase.editor()), model.passphrase.is_masked()),
        Focus::None => (None, false),
    }
}

/// `EditFlags` del campo focuseado, para nav/ejecución por teclado del
/// menú de edición. Sin campo focuseado, flags vacíos (todo gris).
fn focused_edit_flags(model: &Model) -> EditFlags {
    let (editor, masked) = focused_editor(model);
    match editor {
        Some(ed) => EditFlags::from_editor(ed, masked),
        None => EditFlags::default(),
    }
}

/// Construye el menú principal de khipu reflejando el estado del campo
/// focuseado (ítems de Editar grises sin selección / historial).
fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};
    let (editor, _masked) = focused_editor(model);
    let has_sel = editor.map(|e| e.has_selection()).unwrap_or(false);
    let can_undo = editor.map(|e| e.can_undo()).unwrap_or(false);
    let can_redo = editor.map(|e| e.can_redo()).unwrap_or(false);
    let has_field = editor.is_some();
    let has_sel_note = model.selected.is_some();

    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo {
        undo = undo.disabled();
    }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y");
    if !can_redo {
        redo = redo.disabled();
    }
    let mut cut = MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C");
    if !has_sel {
        cut = cut.disabled();
        copy = copy.disabled();
    }
    let mut paste = MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V");
    if !has_field {
        paste = paste.disabled();
    }
    let mut sel_all = MenuItem::new("Seleccionar todo", "edit.selectall")
        .shortcut("Ctrl+A")
        .separated();
    if !has_field {
        sel_all = sel_all.disabled();
    }

    let mut delete_note = MenuItem::new("Borrar nota", "note.delete");
    if !has_sel_note {
        delete_note = delete_note.disabled();
    }
    let archive_label = if model.show_archive {
        "Ocultar archivadas"
    } else {
        "Ver archivadas"
    };

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Nueva nota", "note.new").shortcut("Ctrl+N"))
                .item(delete_note)
                .item(MenuItem::new(archive_label, "note.archive").separated())
                .item(MenuItem::new("Exportar sobre…", "share.export"))
                .item(MenuItem::new("Importar sobre…", "share.import")),
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
            Menu::new("Compartir")
                .item(MenuItem::new("Publicar (P2P)", "share.publish"))
                .item(MenuItem::new("Recibir de un par…", "share.receive")),
        )
        .menu(
            Menu::new("Ayuda")
                .item(MenuItem::new("Buscar (foco)", "view.search").shortcut("Ctrl+F"))
                .item(MenuItem::new("Acerca de khipu", "help.about")),
        )
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a app_bus::AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
    let (w, h) = KhipuApp::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: std::sync::Arc::new(Msg::MenuOpen),
        on_command: std::sync::Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Traduce el `command` del menú principal al `Msg` real y lo redespacha
/// por el `update`. Cierra el menú antes de actuar.
fn handle_menu_command(mut model: Model, command: String, h: &Handle<Msg>) -> Model {
    model.menu_open = None;
    let target = match command.as_str() {
        "note.new" => Some(Msg::NewNote),
        "note.delete" => Some(Msg::DeleteSelected),
        "note.archive" => Some(Msg::ToggleArchive),
        "share.export" => Some(Msg::Export),
        "share.import" => Some(Msg::Import),
        "share.publish" => Some(Msg::Publish),
        "share.receive" => Some(Msg::Receive),
        "view.search" => Some(Msg::Focus(Focus::Search)),
        "edit.undo" => Some(Msg::EditMenuAction(EditAction::Undo)),
        "edit.redo" => Some(Msg::EditMenuAction(EditAction::Redo)),
        "edit.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "edit.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "edit.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "edit.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        "help.about" => {
            model.status = Some("khipu · cuaderno de notas P2P soberano".into());
            None
        }
        _ => None,
    };
    match target {
        Some(msg) => KhipuApp::update(model, msg, h),
        None => model,
    }
}

/// Aplica una acción del menú de edición al editor del campo focuseado,
/// usando el portapapeles del sistema, y replica el bookkeeping que khipu
/// hace tras editar (commit al store + embedding si cambió un campo que
/// vive en la nota). Cierra el menú de edición.
fn apply_edit_menu_action(mut model: Model, action: EditAction, h: &Handle<Msg>) -> Model {
    model.edit_menu = None;
    let focus = model.focus;
    let clip = &mut model.clipboard;
    let result = match focus {
        Focus::Body => Some(editmenu::apply(&mut model.body, action, clip)),
        Focus::Title => Some(editmenu::apply(model.title.editor_mut(), action, clip)),
        Focus::Tags => Some(editmenu::apply(model.tags.editor_mut(), action, clip)),
        Focus::Search => Some(editmenu::apply(model.search.editor_mut(), action, clip)),
        Focus::PeerAddr => Some(editmenu::apply(model.peer_input.editor_mut(), action, clip)),
        Focus::Region => Some(editmenu::apply(model.region_input.editor_mut(), action, clip)),
        Focus::Passphrase => Some(editmenu::apply(model.passphrase.editor_mut(), action, clip)),
        Focus::None => None,
    };
    // Si la acción cambió un campo persistente de la nota (título, cuerpo o
    // tags), corremos el mismo commit que las teclas. Search/PeerAddr/
    // Passphrase no tocan el store.
    if let Some(r) = result {
        if r.changed() && matches!(focus, Focus::Body | Focus::Title | Focus::Tags) {
            commit_edits(&mut model, h);
        }
    }
    model
}

fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn select(model: &mut Model, id: NoteId) {
    let Some(note) = model.store.get(id) else {
        return;
    };
    model.selected = Some(id);
    model.title.set_text(note.title.clone());
    model.body = EditorState::default();
    model.body.set_text(&note.body);
    model.tags.set_text(note.tags.join(", "));
    model.focus = Focus::Body;
}

/// Suelta la nota seleccionada y limpia los campos del editor. El editor
/// flotante desaparece y el mapa queda libre — el equivalente a alejarse
/// del nodo (precursor del zoom semántico).
fn deselect(model: &mut Model) {
    model.selected = None;
    model.title.set_text(String::new());
    model.body = EditorState::default();
    model.tags.set_text(String::new());
    model.focus = Focus::None;
}

/// Pide el embedding de `id` en segundo plano. Asigna una secuencia
/// nueva, la marca como vigente, y dispara un worker (`Handle::spawn`)
/// que al terminar reentra al `update` con [`Msg::EmbeddingReady`]. Así
/// el `block_on` del arm remoto nunca corre en el hilo de UI.
fn schedule_embedding(model: &mut Model, id: NoteId, h: &Handle<Msg>) {
    let Some(note) = model.store.get(id) else {
        return;
    };
    let combined = format!("{} {}", note.title, note.body);
    model.embed_seq += 1;
    let seq = model.embed_seq;
    model.embed_latest.insert(id, seq);
    let embedder = model.embedder.clone();
    h.spawn(move || {
        let v = embedder.embed_blocking(&combined);
        Msg::EmbeddingReady(id, seq, v)
    });
}

/// Versión síncrona para el arranque (seed y migración de formato):
/// calcula el vector en línea y lo inserta. En init todavía no hay nada
/// que repintar, así que bloquear un instante es lo correcto — y deja el
/// campo semántico listo antes del primer layout.
fn embed_now(model: &mut Model, id: NoteId) {
    let Some(note) = model.store.get(id) else {
        return;
    };
    let combined = format!("{} {}", note.title, note.body);
    let v = model.embedder.embed_blocking(&combined);
    model.field.insert(id, v);
}

/// Hash trigram → R^EMBED_DIM con signos +/-1 (random projection
/// 1-bit signed), normalizado por L2. Determinista, independiente de
/// idioma, sin red.
fn embed(text: &str, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; dim];
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    if bytes.len() < 3 {
        for (i, b) in bytes.iter().enumerate() {
            v[i % dim] += *b as f32 / 255.0;
        }
    } else {
        for w in bytes.windows(3) {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in w {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            let idx = (h as usize) % dim;
            let sign = if h & 1 == 0 { 1.0 } else { -1.0 };
            v[idx] += sign;
        }
    }
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        for x in &mut v {
            *x /= n;
        }
    }
    v
}

#[derive(Serialize, Deserialize)]
struct PersistedState {
    store: NoteStore,
    embeddings: Vec<(NoteId, Vec<f32>)>,
    order: Vec<NoteId>,
    /// Etiqueta del espacio vectorial con que se guardaron los
    /// `embeddings` (ver [`Embedder::label`]). Si al cargar no coincide
    /// con el embebedor activo, los vectores se recalculan.
    model: String,
    /// Topónimos bautizados. Trailing → archivos previos a las regiones
    /// no parsean como esta forma y caen al fallback `PersistedStateV2`.
    #[serde(default)]
    regions: Vec<Region>,
}

/// Formato previo a las regiones (postcard no es self-describing, así que
/// un campo trailing rompe el parseo y hay que intentar la forma vieja).
#[derive(Deserialize)]
struct PersistedStateV2 {
    store: NoteStore,
    embeddings: Vec<(NoteId, Vec<f32>)>,
    order: Vec<NoteId>,
    model: String,
}

/// Formato histórico, sin `model`. Fallback cuando ni el actual ni el V2
/// parsean (archivos escritos antes de enchufar `verbo`).
#[derive(Deserialize)]
struct PersistedStateV1 {
    store: NoteStore,
    embeddings: Vec<(NoteId, Vec<f32>)>,
    order: Vec<NoteId>,
}

/// Directorio de datos de khipu (`$XDG_DATA_HOME/khipu/`), creándolo si
/// hace falta. Raíz de `notes.bin`, `identidad.seed` y `compartido.khipu`.
fn khipu_dir() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("org", "gioser", "khipu")?;
    let dir = dirs.data_dir().to_path_buf();
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn data_file_path() -> Option<PathBuf> {
    Some(khipu_dir()?.join("notes.bin"))
}

/// Desbloquea (o crea, o migra) la identidad del cuaderno con `passphrase`,
/// vía [`khipu_share::identity::unlock`]. La semilla vive cifrada en
/// `<datos>/keys/`; si existe un `identidad.seed` en claro de versiones
/// viejas, se migra al keystore y se borra el claro. `None` si no hay
/// directorio de datos o la passphrase no descifra.
fn unlock_identity(passphrase: &str) -> Option<Keypair> {
    let dir = khipu_dir()?;
    let legacy = dir.join("identidad.seed");
    khipu_share::identity::unlock(&dir.join("keys"), Some(&legacy), passphrase).ok()
}

/// Arranca el prompt de passphrase y memoriza la acción a reanudar.
fn start_unlock(model: &mut Model, accion: Msg) {
    model.unlocking = true;
    model.pending = Some(Box::new(accion));
    model.focus = Focus::Passphrase;
    model.passphrase.clear();
    model.status = Some("ingresá tu passphrase para desbloquear la identidad".into());
}

/// Sella todas las notas del cuaderno en `compartido.khipu` (sobre
/// firmado, direccionado por contenido). Devuelve la línea de estado.
fn export_notebook(model: &Model) -> String {
    let Some(kp) = model.keypair.as_ref() else {
        return "sin identidad para firmar".into();
    };
    let Some(dir) = khipu_dir() else {
        return "sin directorio de datos".into();
    };
    // Compartir selectivo: si hay texto en el buscador, exportamos sólo
    // las notas que filtra (mismo criterio que la lista); si está vacío,
    // todo el cuaderno.
    let query = model.search.text();
    let q = query.trim();
    let notes: Vec<SharedNote> = model
        .order
        .iter()
        .filter_map(|id| model.store.get(*id))
        .filter(|n| q.is_empty() || note_matches(n, q))
        .map(SharedNote::from_note)
        .collect();
    if notes.is_empty() {
        return "no hay notas para exportar (¿el filtro no coincide?)".into();
    }
    let n = notes.len();
    let sobre = match khipu_share::seal(kp, notes, now_secs()) {
        Ok(s) => s,
        Err(_) => return "falló el sellado".into(),
    };
    let Ok(bytes) = sobre.to_bytes() else {
        return "falló serializar el sobre".into();
    };
    let path = dir.join("compartido.khipu");
    let tmp = path.with_extension("khipu.tmp");
    if std::fs::write(&tmp, &bytes)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .is_err()
    {
        return "no se pudo escribir el sobre".into();
    }
    let hash = sobre.content_address().unwrap_or([0u8; 32]);
    let filtro = if q.is_empty() {
        String::new()
    } else {
        format!(" (filtro «{q}»)")
    };
    format!(
        "exportadas {n} notas{filtro} → compartido.khipu · {}",
        hex8(&hash)
    )
}

/// Verifica e ingiere `compartido.khipu`. Las notas nuevas nacen con
/// gravedad fresca; sus embeddings se recalculan en segundo plano. Un
/// sobre con firma inválida se rechaza entero. Devuelve la línea de estado.
fn import_notebook(model: &mut Model, h: &Handle<Msg>) -> String {
    let Some(dir) = khipu_dir() else {
        return "sin directorio de datos".into();
    };
    let path = dir.join("compartido.khipu");
    let Ok(bytes) = std::fs::read(&path) else {
        return "no hay compartido.khipu para importar".into();
    };
    let sobre = match SignedBundle::from_bytes(&bytes) {
        Ok(s) => s,
        Err(_) => return "sobre ilegible".into(),
    };
    let outcome = match khipu_share::open(&sobre) {
        Ok(bundle) => khipu_share::import_into(&mut model.store, bundle, now_secs()),
        Err(_) => return "firma inválida — sobre rechazado".into(),
    };
    for id in &outcome.created {
        model.order.push(*id);
        schedule_embedding(model, *id, h);
    }
    format!(
        "importadas {} · omitidas {} (ya existían)",
        outcome.created.len(),
        outcome.skipped
    )
}

/// Dirección donde el servidor escucha. `KHIPU_BIND` la sobrescribe;
/// default localhost para no exponerse sin querer.
fn bind_addr() -> String {
    std::env::var("KHIPU_BIND").unwrap_or_else(|_| "127.0.0.1:7700".into())
}

/// Dirección del par a quien jalarle el cuaderno. `KHIPU_PEER` la
/// sobrescribe; default coincide con [`bind_addr`] para probar en local.
fn peer_addr() -> String {
    std::env::var("KHIPU_PEER").unwrap_or_else(|_| "127.0.0.1:7700".into())
}

/// Arma el nodo libp2p la primera vez que se necesita: runtime tokio
/// dedicado + `KhipuNode` que empieza a escuchar (para ser alcanzable y
/// obtener nuestra dirección de marcado). Idempotente. `false` si no se
/// pudo (sin runtime o sin red).
fn ensure_p2p(model: &mut Model) -> bool {
    if model.p2p.is_some() {
        return true;
    }
    let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    else {
        return false;
    };
    // `KhipuNode::standalone` arranca el swarm con `tokio::spawn`: hay que
    // estar dentro del runtime.
    let node = {
        let _g = rt.enter();
        match khipu_brahman::KhipuNode::standalone() {
            Ok(n) => Arc::new(n),
            Err(_) => return false,
        }
    };
    let dial_addr = rt
        .block_on(node.listen_str("/ip4/0.0.0.0/tcp/0"))
        .unwrap_or_default();
    // Si hay un nodo bootstrap configurado, nos unimos a la malla DHT para
    // poder descubrir y ser descubiertos (`anunciar`/`descubrir`).
    if let Ok(boot) = std::env::var("KHIPU_BOOTSTRAP") {
        let _ = node.dial_str(&boot);
    }
    model.p2p = Some(P2p {
        rt: Arc::new(rt),
        node,
        dial_addr,
        serving: false,
    });
    true
}

/// Levanta (una sola vez) el servidor TCP que sirve `compartido.khipu`.
/// El hilo lee el archivo en cada conexión, así sirve siempre la versión
/// vigente; vive hasta que el proceso termina. Devuelve la línea de estado.
fn start_publishing(model: &mut Model, h: &Handle<Msg>) -> String {
    if model.publishing {
        return format!("ya publicando en {}", bind_addr());
    }
    let Some(dir) = khipu_dir() else {
        return "sin directorio de datos".into();
    };
    let addr = bind_addr();
    let listener = match std::net::TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => return format!("no se pudo escuchar en {addr}: {e}"),
    };
    // Puerto efectivo (resuelve `:0` si se usara) para anunciarlo en la baliza.
    let tcp_port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let path = dir.join("compartido.khipu");
    std::thread::spawn(move || {
        khipu_share::net::serve_loop(listener, move || std::fs::read(&path));
    });
    // Baliza periódica para que los pares nos descubran sin saber la IP.
    let beacon = khipu_share::discovery::Beacon {
        author: model.keypair.as_ref().map(|k| k.public_key()).unwrap_or([0u8; 32]),
        port: tcp_port,
        name: "khipu".into(),
    };
    std::thread::spawn(move || loop {
        let _ = khipu_share::discovery::anunciar(&beacon);
        std::thread::sleep(std::time::Duration::from_secs(2));
    });
    model.publishing = true;

    // Además del TCP/LAN, servimos por libp2p (cifrado, WAN). El nodo se
    // arma perezoso; servimos `compartido.khipu` y nos anunciamos en la DHT.
    let p2p_status = if ensure_p2p(model) {
        let dir2 = dir.clone();
        if let Some(p) = model.p2p.as_mut() {
            if !p.serving {
                let path2 = dir2.join("compartido.khipu");
                let node = p.node.clone();
                let _g = p.rt.enter();
                node.run_serve(move || std::fs::read(&path2).ok());
                node.anunciar();
                p.serving = true;
            }
            // Si hay un relay configurado (KHIPU_RELAY=/ip4/.../p2p/<id>),
            // reservamos un circuito ahí para ser alcanzables detrás de NAT.
            // Async (dial + identify + reserva tardan ~2s): cuando termina,
            // reentra con Msg::RelayReady para mostrar la dirección.
            if let Ok(relay) = std::env::var("KHIPU_RELAY") {
                let (rt, node, h2) = (p.rt.clone(), p.node.clone(), h.clone());
                rt.spawn(async move {
                    let _ = node.dial_str(&relay);
                    // Esperamos a que AutoNAT confirme la dirección del relay
                    // (boot_delay + dial-back) antes de pedir la reserva.
                    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                    let circuit = format!("{relay}/p2p-circuit");
                    let msg = match node.listen_str(&circuit).await {
                        Ok(addr) => addr,
                        Err(e) => format!("falló reservar circuito: {e}"),
                    };
                    h2.dispatch(Msg::RelayReady(msg));
                });
            }
            format!(" · libp2p: {}", p.dial_addr)
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    format!("publicando en {addr} (LAN){p2p_status}")
}

/// Prefijo hex (4 bytes / 8 hex) de un hash, para mostrar una dirección
/// de contenido sin abrumar.
fn hex8(hash: &[u8; 32]) -> String {
    hash[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Prompt modal de passphrase: tarjeta centrada con el input (enmascarado
/// con •) y dos botones. Enter desbloquea, Esc cancela (ver `on_key`).
fn unlock_view(model: &Model, input_palette: &TextInputPalette) -> View<Msg> {
    let titulo = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "Desbloqueá tu identidad para firmar".to_string(),
        14.0,
        model.theme.fg_text,
        Alignment::Start,
    );

    let hint = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "La semilla vive cifrada (Argon2id). La primera vez, esta passphrase la crea."
            .to_string(),
        11.0,
        model.theme.fg_muted,
        Alignment::Start,
    );

    let input = text_input_view(
        &model.passphrase,
        "passphrase",
        model.focus == Focus::Passphrase,
        input_palette,
        Msg::Focus(Focus::Passphrase),
    );
    let input_row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![input]);

    let unlock_btn = button(
        "desbloquear (Enter)",
        model.theme.bg_button,
        model.theme.accent,
        Msg::Unlock,
    );
    let cancel_btn = button(
        "cancelar (Esc)",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::CancelUnlock,
    );
    let buttons = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![unlock_btn, cancel_btn]);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(420.0_f32),
            height: Dimension::auto(),
        },
        padding: Rect {
            left: length(18.0_f32),
            right: length(18.0_f32),
            top: length(16.0_f32),
            bottom: length(16.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel)
    .radius(6.0)
    .children(vec![titulo, hint, input_row, buttons]);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(model.theme.bg_app)
    .children(vec![card])
}

/// Barra de estado al pie: muestra el último mensaje de export/import.
fn status_bar(model: &Model) -> Option<View<Msg>> {
    let text = model.status.as_ref()?;
    Some(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(22.0_f32),
            },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(2.0_f32),
                bottom: length(2.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(model.theme.bg_panel_alt)
        .text_aligned(text.clone(), 11.0, model.theme.fg_muted, Alignment::Start),
    )
}

fn load_state(path: &PathBuf) -> Option<PersistedState> {
    let bytes = std::fs::read(path).ok()?;
    // Formato actual primero; si no parsea (payload viejo sin `model`)
    // caemos al V1 y lo migramos con `model` vacío → fuerza recálculo.
    if let Ok(state) = postcard::from_bytes::<PersistedState>(&bytes) {
        return Some(state);
    }
    // Archivo previo a las regiones: misma forma sin el campo trailing.
    if let Ok(v2) = postcard::from_bytes::<PersistedStateV2>(&bytes) {
        return Some(PersistedState {
            store: v2.store,
            embeddings: v2.embeddings,
            order: v2.order,
            model: v2.model,
            regions: Vec::new(),
        });
    }
    let v1: PersistedStateV1 = postcard::from_bytes(&bytes).ok()?;
    Some(PersistedState {
        store: v1.store,
        embeddings: v1.embeddings,
        order: v1.order,
        model: String::new(),
        regions: Vec::new(),
    })
}

fn persist(model: &Model) {
    let Some(path) = model.data_path.as_ref() else {
        return;
    };
    let state = PersistedState {
        store: model.store.clone(),
        embeddings: model
            .field
            .iter()
            .map(|(id, v)| (id, v.to_vec()))
            .collect(),
        order: model.order.clone(),
        model: model.embedder.label(),
        regions: model.regions.clone(),
    };
    if let Ok(bytes) = postcard::to_allocvec(&state) {
        let tmp = path.with_extension("bin.tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

fn from_state(state: PersistedState, embedder: Embedder) -> Model {
    // ¿Los vectores guardados son del mismo espacio que el embebedor
    // activo? Si cambió el modelo o la dimensión (p. ej. arrancó el
    // daemon, o se cayó y volvimos al trigram local), son incomparables:
    // se descartan y se recalcula todo el cuaderno.
    let same_space = !state.model.is_empty() && state.model == embedder.label();
    let regions = state.regions;
    let mut model = Model {
        store: state.store,
        field: SemanticField::new(),
        order: state.order,
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
        regions,
        naming: None,
        region_input: TextInputState::new(),
    };
    if same_space {
        let restored: std::collections::HashSet<NoteId> =
            state.embeddings.iter().map(|(id, _)| *id).collect();
        for (id, v) in &state.embeddings {
            if !v.is_empty() {
                model.field.insert(*id, v.clone());
            }
        }
        // Notas sin vector persistido (nota nueva que no alcanzó a
        // guardar su embedding async): recalcular sólo esas.
        let missing: Vec<NoteId> = model
            .order
            .iter()
            .copied()
            .filter(|id| !restored.contains(id))
            .collect();
        for id in missing {
            embed_now(&mut model, id);
            place_note(&mut model, id);
        }
    } else {
        let ids: Vec<NoteId> = model.order.clone();
        for id in ids {
            embed_now(&mut model, id);
        }
    }
    // Notas cargadas de disco sin posición (payloads viejos previos al
    // anclaje) reciben domicilio ahora, en orden, contra las ya asentadas.
    let unplaced: Vec<NoteId> = model
        .order
        .iter()
        .copied()
        .filter(|id| model.store.get(*id).map(|n| n.pos.is_none()).unwrap_or(false))
        .collect();
    for id in unplaced {
        place_note(&mut model, id);
    }
    model
}

fn seeded_model(embedder: Embedder) -> Model {
    let mut model = Model {
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
    };
    let now = now_secs();
    let seed: [(&str, &str, &[&str]); 7] = [
        (
            "Índice",
            "mi cuaderno: [[Recetas de la abuela]], [[Jardín]] y [[Oficina]]",
            &["meta"],
        ),
        (
            "Recetas de la abuela",
            "sopa de auyama; ver también [[Lista del mercado]]",
            &["cocina"],
        ),
        (
            "Lista del mercado",
            "auyama, cilantro, pan; vuelve al [[Índice]]",
            &["cocina"],
        ),
        (
            "Jardín",
            "riego semanal; las [[Semillas de cilantro]] van en marzo",
            &["jardín"],
        ),
        (
            "Semillas de cilantro",
            "germinan en diez días",
            &["jardín"],
        ),
        (
            "Oficina",
            "[[Reunión del lunes]] y pendientes varios",
            &["trabajo"],
        ),
        (
            "Diario sin enlaces",
            "una nota suelta, no la enlaza nadie",
            &["personal"],
        ),
    ];
    for (title, body, tags) in seed {
        let tags: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
        let id = model.store.create(title, body, tags, now);
        model.order.push(id);
        embed_now(&mut model, id);
        place_note(&mut model, id);
    }
    model
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// La masa "vivida" de una nota en `now`: la guardada decae contra
/// el tiempo transcurrido desde `last_access`. Las notas con
/// `last_access == 0` (payloads viejos sin el campo) toman su `mass`
/// tal cual — equivale a tratar `now` como su primer acceso.
fn current_mass(gravity: &Gravity, n: &Note, now: u64) -> f32 {
    if n.last_access == 0 {
        return n.mass;
    }
    let dt = if now > n.last_access {
        (now - n.last_access) as f32
    } else {
        0.0
    };
    gravity.decay(n.mass, dt)
}

/// Refuerza la masa de `id` y marca `last_access`. El gesto canónico
/// cuando el usuario selecciona o abre una nota: primero decaemos el
/// valor guardado al "ahora" y sobre ese decaído sumamos el boost.
fn reinforce_and_touch(model: &mut Model, id: NoteId) {
    let now = now_secs();
    let Some(n) = model.store.get(id) else {
        return;
    };
    let lived = current_mass(&model.gravity, n, now);
    let reinforced = model.gravity.reinforce(lived);
    model.store.set_mass(id, reinforced);
    model.store.touch(id, now);
}

/// Primera nota sobre el horizonte, ordenada por masa "viva".
fn first_visible(model: &Model) -> Option<NoteId> {
    let now = now_secs();
    let mut visible: Vec<(NoteId, f32)> = model
        .order
        .iter()
        .filter_map(|id| {
            model.store.get(*id).and_then(|n| {
                let m = current_mass(&model.gravity, n, now);
                model.gravity.is_visible(m).then_some((*id, m))
            })
        })
        .collect();
    visible.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    visible.first().map(|(id, _)| *id)
}

fn main() {
    llimphi_ui::run::<KhipuApp>();
}
