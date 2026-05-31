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
use khipu_gravity::{Gravity, GravityConfig, NotePlacement, Params, SemanticField};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::winit::keyboard::{Key, NamedKey};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Rect, Size, Style},
    AlignItems, Dimension, JustifyContent,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle as KurboCircle, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, KeyEvent, KeyState, View};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_text_editor::{
    text_editor_view, EditorMetrics, EditorPalette, EditorState, PointerEvent,
};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use serde::{Deserialize, Serialize};

/// Dimensión del embebedor local (fallback sin daemon).
const EMBED_DIM: usize = 16;
const CLUSTER_THRESHOLD: f32 = 0.55;
const EDITOR_VISIBLE_LINES: usize = 24;
const LIST_WIDTH: f32 = 240.0;
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
    /// Descarta la lista de pares sin jalar nada.
    CancelPeers,
    /// Resultado async de un fetch: el sobre recibido o un error.
    Received(Result<SignedBundle, String>),
    /// Intenta desbloquear la identidad con la passphrase tipeada.
    Unlock,
    /// Cierra el prompt de passphrase sin desbloquear.
    CancelUnlock,
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
    /// Pares descubiertos pendientes de elegir. No vacío ⇒ el panel
    /// izquierdo muestra la lista de pares en vez de las notas.
    peers: Vec<PeerInfo>,
    /// Input de la passphrase para desbloquear la identidad.
    passphrase: TextInputState,
    /// `true` mientras se muestra el prompt de passphrase (modal).
    unlocking: bool,
    /// Acción a reanudar tras desbloquear (lo que el usuario quiso hacer
    /// y disparó el prompt). Se redispatcha al lograr el unlock.
    pending: Option<Box<Msg>>,
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
                    model.status = Some(start_publishing(&mut model));
                }
            }
            Msg::Receive => {
                let my_key = model.keypair.as_ref().map(|k| k.public_key());
                model.status = Some("buscando pares en la LAN…".into());
                // El descubrimiento bloquea ~3s: va a un worker y reentra
                // con la lista de pares (ya sin uno mismo).
                h.spawn(move || {
                    let pares = khipu_share::discovery::descubrir(
                        std::time::Duration::from_secs(3),
                    )
                    .unwrap_or_default();
                    let infos: Vec<PeerInfo> = pares
                        .into_iter()
                        .filter(|p| Some(p.beacon.author) != my_key)
                        .map(|p| PeerInfo {
                            addr: p.fetch_addr.to_string(),
                            label: format!(
                                "{} · de:{} · {}",
                                p.beacon.name,
                                khipu_share::hex8(&p.beacon.author),
                                p.fetch_addr
                            ),
                        })
                        .collect();
                    Msg::PeersFound(infos)
                });
            }
            Msg::PeersFound(peers) => {
                if peers.is_empty() {
                    // Nadie anunció: caer al par explícito (KHIPU_PEER).
                    let peer = peer_addr();
                    model.status = Some(format!("ningún par en la LAN; probando {peer}…"));
                    h.spawn(move || match khipu_share::net::fetch(&peer) {
                        Ok(s) => Msg::Received(Ok(s)),
                        Err(e) => Msg::Received(Err(format!("no se pudo recibir de {peer}: {e}"))),
                    });
                } else {
                    // Dejamos que el usuario elija de quién jalar — el panel
                    // izquierdo pasa a mostrar la lista de pares.
                    model.status = Some(format!("{} pares en la red — elegí uno", peers.len()));
                    model.peers = peers;
                }
            }
            Msg::FetchFrom(addr) => {
                model.peers.clear();
                model.status = Some(format!("jalando de {addr}…"));
                let destino = addr;
                h.spawn(move || match khipu_share::net::fetch(&destino) {
                    Ok(s) => Msg::Received(Ok(s)),
                    Err(e) => Msg::Received(Err(format!("no se pudo recibir de {destino}: {e}"))),
                });
            }
            Msg::CancelPeers => {
                model.peers.clear();
                model.status = Some("descarté la lista de pares".into());
            }
            Msg::Received(res) => {
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
        // Mientras hay pares pendientes de elegir, el panel izquierdo
        // muestra la lista de pares en vez de las notas.
        let list = if model.peers.is_empty() {
            list_panel(model, &palette, &input_palette)
        } else {
            peers_panel(model, &palette)
        };
        let editor = editor_panel(model, &input_palette, &editor_palette);
        let gravity = gravity_panel(model);

        let body = View::new(Style {
            flex_direction: FlexDirection::Row,
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
        .children(vec![list, editor, gravity]);

        let mut children = vec![header, body];
        if let Some(bar) = status_bar(model) {
            children.push(bar);
        }

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .children(children)
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
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
                return Some(Msg::Focus(Focus::None));
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

/// Panel izquierdo en modo "elegir par": una fila por par descubierto
/// (click ⇒ jalar de él) y un botón para descartar la lista. Reemplaza
/// transitoriamente la lista de notas.
fn peers_panel(model: &Model, palette: &ListPalette) -> View<Msg> {
    let rows: Vec<ListRow<Msg>> = model
        .peers
        .iter()
        .map(|p| ListRow {
            label: p.label.clone(),
            selected: false,
            on_click: Msg::FetchFrom(p.addr.clone()),
        })
        .collect();

    let spec = ListSpec {
        total: rows.len(),
        rows,
        caption: Some(format!("pares en la red · {} (click para jalar)", model.peers.len())),
        truncated_hint: None,
        row_height: ROW_H,
        palette: *palette,
    };

    let cancel = button(
        "cancelar",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::CancelPeers,
    );
    let cancel_row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(36.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(model.theme.bg_panel_alt)
    .children(vec![cancel]);

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
    .children(vec![cancel_row, list_wrap])
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

fn gravity_panel(model: &Model) -> View<Msg> {
    let placements = model.field.gravity_layout(&GravityConfig::default());
    let clusters = model.field.clusters(CLUSTER_THRESHOLD);
    let selected = model.selected;
    let theme = model.theme;
    let labels: Vec<(NoteId, String)> = placements
        .iter()
        .filter_map(|p| model.store.get(p.id).map(|n| (p.id, short_label(&n.title))))
        .collect();

    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .paint_with(move |scene, _ts, rect| {
        paint_gravity(scene, rect, &placements, &clusters, &labels, selected, theme);
    });

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

fn paint_gravity(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: llimphi_ui::PaintRect,
    placements: &[NotePlacement],
    clusters: &[Vec<NoteId>],
    labels: &[(NoteId, String)],
    selected: Option<NoteId>,
    theme: Theme,
) {
    if placements.is_empty() || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let (min_x, max_x, min_y, max_y) = placements.iter().fold(
        (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY),
        |(mnx, mxx, mny, mxy), p| {
            (
                mnx.min(p.x),
                mxx.max(p.x),
                mny.min(p.y),
                mxy.max(p.y),
            )
        },
    );
    let pad = 36.0_f32;
    let span_x = (max_x - min_x).max(1.0);
    let span_y = (max_y - min_y).max(1.0);
    let scale = ((rect.w - pad * 2.0).max(10.0) / span_x)
        .min((rect.h - pad * 2.0).max(10.0) / span_y);
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    let mx = (min_x + max_x) * 0.5;
    let my = (min_y + max_y) * 0.5;
    let project = |p: &NotePlacement| -> (f32, f32) {
        (cx + (p.x - mx) * scale, cy + (p.y - my) * scale)
    };

    for p in placements {
        let (px, py) = project(p);
        let color = cluster_color(p.id, clusters, theme);
        let r = if selected == Some(p.id) { 9.0 } else { 6.0 };
        let circle = KurboCircle::new((px as f64, py as f64), r);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &circle);
        if selected == Some(p.id) {
            let ring = KurboCircle::new((px as f64, py as f64), (r + 3.0) as f64);
            scene.stroke(
                &Stroke::new(2.0),
                Affine::IDENTITY,
                theme.accent,
                None,
                &ring,
            );
        }
    }

    let line_color = with_alpha(theme.border, 0.55);
    for cluster in clusters {
        if cluster.len() < 2 {
            continue;
        }
        let pts: Vec<(f32, f32)> = cluster
            .iter()
            .filter_map(|cid| placements.iter().find(|p| p.id == *cid).map(project))
            .collect();
        if pts.len() < 2 {
            continue;
        }
        let (sx, sy) = pts.iter().fold((0.0, 0.0), |(ax, ay), (x, y)| (ax + x, ay + y));
        let cx_g = sx / pts.len() as f32;
        let cy_g = sy / pts.len() as f32;
        let mut path = BezPath::new();
        for (x, y) in &pts {
            path.move_to((cx_g as f64, cy_g as f64));
            path.line_to((*x as f64, *y as f64));
        }
        scene.stroke(
            &Stroke::new(1.0).with_dashes(0.0, [3.0, 3.0]),
            Affine::IDENTITY,
            line_color,
            None,
            &path,
        );
    }

    // Etiquetas como rectángulos diminutos al lado de cada nodo
    // serían un trabajo de typesetter; en MVP imprimimos sólo el
    // título de la nota seleccionada arriba.
    if let Some(sel) = selected {
        if let Some((_, label)) = labels.iter().find(|(id, _)| *id == sel) {
            let _ = label; // intencional: el texto va por View::text en otro nodo.
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
}

/// Formato histórico, sin `model`. postcard no es self-describing, así
/// que lo intentamos como fallback cuando el formato actual no parsea
/// (archivos escritos antes de enchufar `verbo`).
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

/// Levanta (una sola vez) el servidor TCP que sirve `compartido.khipu`.
/// El hilo lee el archivo en cada conexión, así sirve siempre la versión
/// vigente; vive hasta que el proceso termina. Devuelve la línea de estado.
fn start_publishing(model: &mut Model) -> String {
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
    format!("publicando el cuaderno en {addr} (anunciando en LAN)")
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
    let v1: PersistedStateV1 = postcard::from_bytes(&bytes).ok()?;
    Some(PersistedState {
        store: v1.store,
        embeddings: v1.embeddings,
        order: v1.order,
        model: String::new(),
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
        peers: Vec::new(),
        passphrase: TextInputState::masked(),
        unlocking: false,
        pending: None,
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
        }
    } else {
        let ids: Vec<NoteId> = model.order.clone();
        for id in ids {
            embed_now(&mut model, id);
        }
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
        peers: Vec::new(),
        passphrase: TextInputState::masked(),
        unlocking: false,
        pending: None,
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
