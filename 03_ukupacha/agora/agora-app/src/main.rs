//! `agora-app` — UI Llimphi del ágora.
//!
//! Cuatro tiles draggables sobre el mismo `TrustGraph`: identidades,
//! atestaciones, compositor y política. Drag de la title bar de un
//! tile sobre otro los intercambia.
//!
//! ## Cómo arranca
//!
//! - Lee `~/.local/share/agora/graph.json` si existe; si no, parte vacío.
//! - Abre el [`agora_keystore::Keystore`] en `~/.local/share/agora/keys/`.
//! - La passphrase se toma de la env `AGORA_PASSPHRASE` o `"agora-dev"`
//!   por defecto (MVP — un unlock screen real queda para una iteración
//!   siguiente).
//!
//! ## Flujo típico
//!
//! 1. Tile **Identidades**: botón "nueva identidad" genera una seed
//!    CSPRNG, la cifra en el keystore y registra la pubkey en el grafo.
//!    Click en una fila la marca como *sujeto enfocado*; click en
//!    "actuar como" (visible sólo en identidades con seed propia) la
//!    elige como *firmante activo*.
//! 2. Tile **Compositor**: con un sujeto y un firmante seleccionados,
//!    edita predicado y valor; "atestar" firma y agrega al grafo.
//! 3. Tile **Atestaciones**: lista verificada del grafo. Click sobre
//!    una fila la selecciona para que la política aplique sobre su claim.
//! 4. Tile **Política**: slider `min_third_party` (0..=5), toggle
//!    `accept_self`, ciclo de `kind` (off/persona/comunidad/alianza/
//!    institución) + slider de mínimo cuando el kind está activo, y
//!    ciclo de `max_age` (off/1m/5m/1h/1d/7d). Veredicto en vivo abajo,
//!    basado en el claim de la atestación seleccionada y evaluado con
//!    todos los ejes activos.

use std::path::PathBuf;

use agora_core::{Attestation, Claim, IdentityKind, Keypair, MultiSignature};
use agora_graph::{Corroboration, TrustGraph, TrustPolicy};
use agora_keystore::Keystore;
use agora_core::IdentityId;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_tiled::{tiled_view_reorderable, TileSpec, TiledPalette};
use rand::RngCore;

// =============================================================================
//  Modelo
// =============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tile {
    Identidades,
    Compositor,
    Atestaciones,
    Politica,
    /// Compositor de [`MultiSignature`]: elige firmantes "míos", escribe
    /// el mensaje (típicamente una raíz canónica), elige umbral M, firma
    /// y exporta postcard en hex.
    Multifirma,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ComposeField {
    Predicate,
    Value,
}

/// Qué input recibe las teclas en la pantalla principal. Como el tile
/// del compositor de atestaciones y el tile multifirma comparten el
/// mismo `on_key`, necesitamos saber a cuál routear cada evento.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusedInput {
    Compose(ComposeField),
    MultiMessage,
}

/// Severidad de un mensaje de estado de servicio (persistencia, red,
/// keystore). Determina el color del banner.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusLevel {
    Info,
    Error,
}

/// Banner visible al pie de la ventana cuando hay un error o info que
/// vale la pena destacar (típicamente fallas de I/O o de red que antes
/// iban a stderr y nadie veía en una app de UI). `None` significa que
/// no hay nada que mostrar y el banner se oculta.
struct StatusBanner {
    level: StatusLevel,
    text: String,
}

/// Pantalla activa. `Unlock` pide la passphrase; `Main` muestra los
/// cuatro tiles. La transición la dispara `Msg::UnlockSubmit` cuando la
/// passphrase desbloquea al menos una seed (o el keystore está vacío).
enum Screen {
    Unlock {
        input: TextInputState,
        /// Mensaje al pie: vacío hasta el primer intento; al fallar,
        /// "passphrase incorrecta".
        status: String,
    },
    Main,
}

struct Model {
    graph: TrustGraph,
    keystore: Keystore,
    /// Seeds en RAM para las identidades "mías" (las que tienen archivo
    /// en el keystore). Se desbloquean al arrancar con la passphrase y
    /// se mantienen aquí mientras corre el proceso. No persisten al
    /// salir — siguen viviendo cifradas en el keystore.
    seeds: std::collections::HashMap<IdentityId, [u8; 32]>,
    passphrase: String,
    store_path: PathBuf,
    screen: Screen,
    tiles_order: Vec<Tile>,

    /// Identidad seleccionada como sujeto (objetivo del próximo claim).
    focused_subject: Option<IdentityId>,
    /// Identidad firmante activa (debe estar en `seeds`).
    active_signer: Option<IdentityId>,
    /// Atestación seleccionada en el tile de atestaciones, por índice.
    selected_attestation: Option<usize>,

    compose_predicate: TextInputState,
    compose_value: TextInputState,
    /// Input que recibe las teclas. Incluye los del compositor y el
    /// mensaje de la multifirma para que un solo `on_key` pueda
    /// rutearlas sin ambigüedad.
    focused_input: FocusedInput,
    /// Último mensaje al pie del compositor (éxito, error, hint).
    compose_status: String,

    policy: TrustPolicy,

    /// Texto del mensaje sobre el que se compone la multifirma. En el
    /// uso típico (raíz canónica) es ASCII corto; el compositor lo trata
    /// como bytes UTF-8.
    multi_message: TextInputState,
    /// Identidades "mías" elegidas como firmantes de la próxima
    /// multifirma. Sólo entran ids que estén en `seeds`.
    multi_selected: std::collections::BTreeSet<IdentityId>,
    /// Umbral M de la próxima multifirma. Se clampa a `[1, max(1, N)]`
    /// donde N = `multi_selected.len()` al renderizar el slider.
    multi_threshold: usize,
    /// Última multifirma producida por el compositor. `None` hasta que
    /// el usuario presione "firmar"; se descarta cuando cambian las
    /// selecciones o el mensaje y se vuelve a firmar.
    multi_current: Option<MultiSignature>,

    /// Último mensaje de estado de servicio (I/O, red, keystore). El
    /// `view` lo pinta como banner al pie. Se descarta con
    /// [`Msg::DescartarStatus`] o se sobreescribe automáticamente al
    /// salir otro evento de estado.
    status: Option<StatusBanner>,
}

impl Model {
    fn set_status(&mut self, level: StatusLevel, text: impl Into<String>) {
        self.status = Some(StatusBanner {
            level,
            text: text.into(),
        });
    }

    fn save_graph(&mut self) {
        if let Err(e) = agora_store::save(&self.store_path, &self.graph) {
            self.set_status(
                StatusLevel::Error,
                format!("no pude persistir el grafo: {e}"),
            );
        }
    }

    fn is_mine(&self, id: IdentityId) -> bool {
        self.seeds.contains_key(&id)
    }

    fn signer_keypair(&self) -> Option<Keypair> {
        self.active_signer
            .and_then(|id| self.seeds.get(&id).copied())
            .map(Keypair::from_seed)
    }

    /// Intenta desbloquear todas las seeds del keystore con
    /// `self.passphrase`. Sólo guarda las que descifran; el resto se
    /// loguea a stderr y se omite.
    fn desbloquear_seeds_silencioso(&mut self) {
        let ids = self.keystore.list().unwrap_or_default();
        for id in ids {
            match self.keystore.load(id, &self.passphrase) {
                Ok(seed) => {
                    self.seeds.insert(id, seed);
                }
                Err(e) => {
                    eprintln!("agora-app: no pude desbloquear {id}: {e}");
                }
            }
        }
    }

    /// Versión "estricta" para la pantalla de unlock: requiere que
    /// **todas** las seeds del keystore descifren contra `passphrase`.
    /// Devuelve `true` si pasó (y deja `self.seeds` poblada).
    fn intentar_unlock(&mut self, passphrase: &str) -> bool {
        let ids = self.keystore.list().unwrap_or_default();
        let mut nuevas = std::collections::HashMap::new();
        for id in &ids {
            match self.keystore.load(*id, passphrase) {
                Ok(seed) => {
                    nuevas.insert(*id, seed);
                }
                Err(_) => return false,
            }
        }
        self.seeds = nuevas;
        self.passphrase = passphrase.to_string();
        true
    }

    /// Si el grafo no registra una identidad mía conocida (p. ej. el
    /// archivo se borró pero el keystore sobrevivió), la registra
    /// de nuevo como Person con un nombre genérico.
    fn registrar_identidades_huerfanas(&mut self) {
        let huerfanas: Vec<_> = self
            .seeds
            .iter()
            .filter(|(id, _)| self.graph.identity(**id).is_none())
            .map(|(_id, seed)| {
                let kp = Keypair::from_seed(*seed);
                let n = self.graph.identity_count();
                kp.identity(IdentityKind::Person, format!("yo {}", n + 1))
            })
            .collect();
        for ident in huerfanas {
            self.graph.register(ident);
        }
    }
}

#[derive(Clone)]
enum Msg {
    /// Reordenar tiles por drag.
    SwapTile(usize, usize),

    /// Genera una identidad nueva con un seed CSPRNG, la guarda en el
    /// keystore y la registra en el grafo.
    NuevaIdentidad,
    /// Selecciona el sujeto enfocado (objetivo del próximo claim).
    FocoSujeto(IdentityId),
    /// Cambia el firmante activo (entre las identidades mías).
    ActuarComo(IdentityId),
    /// Selecciona una atestación para que la política evalúe su claim.
    SeleccionarAtestacion(usize),

    /// Cambia el campo focado en el compositor.
    FocoCompose(ComposeField),
    /// Tecla aplicada al input focado.
    EditCompose(KeyEvent),
    /// Firma + agrega la atestación con los valores actuales y persiste.
    Atestar,

    /// Drag del slider de `min_third_party`. Acumula el delta.
    SliderMinThird(DragPhase, f32),
    /// Toggle de `accept_self`.
    ToggleAcceptSelf,
    /// Cicla el eje `min_attesters_of_kind`:
    /// off → Person → Community → Alliance → Institution → off.
    /// Al pasar de off a un kind, el N arranca en 1.
    CycleKind,
    /// Drag del slider del N requerido para el kind activo.
    /// No tiene efecto si el kind está en off.
    SliderMinKind(DragPhase, f32),
    /// Cicla el eje `max_age_secs` por presets:
    /// off → 60 → 300 → 3_600 → 86_400 → 604_800 → off.
    CycleMaxAge,

    /// El archivo `graph.json` cambió en disco (lo escribió otro proceso,
    /// típicamente `agora-cli`). Recarga el grafo desde el snapshot.
    ArchivoCambio,

    /// Tecla aplicada al input de passphrase en la pantalla de unlock.
    UnlockKey(KeyEvent),
    /// Intenta desbloquear el keystore con la passphrase actual.
    UnlockSubmit,

    /// Cierra el banner de estado de servicio.
    DescartarStatus,

    /// Tecla aplicada al `multi_message` (el mensaje a multifirmar).
    EditMultiMessage(KeyEvent),
    /// Cambia el foco de edición hacia el `multi_message`.
    FocoMultiMessage,
    /// Toggle de inclusión/exclusión de una identidad propia en la
    /// próxima multifirma. Ignora ids que no estén en `seeds`.
    ToggleMultiFirmante(IdentityId),
    /// Drag del slider del umbral M.
    SliderMultiUmbral(DragPhase, f32),
    /// Firma `multi_message` con cada seed de `multi_selected` y guarda
    /// el resultado en `multi_current`. No persiste — la multifirma se
    /// queda en memoria hasta exportar o limpiar.
    FirmarMulti,
    /// Limpia `multi_current` y resetea las selecciones.
    LimpiarMulti,
    /// Serializa `multi_current` a postcard, lo presenta en hex en el
    /// banner de estado (Info). Si no hay multifirma vigente, muestra
    /// un error.
    ExportarMulti,
}

// =============================================================================
//  App
// =============================================================================

struct AgoraApp;

impl App for AgoraApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "ágora · red de confianza federada"
    }

    fn initial_size() -> (u32, u32) {
        (1200, 760)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let data_dir = directories::ProjectDirs::from("net", "gioser", "agora")
            .map(|p| p.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&data_dir).ok();
        let store_path = data_dir.join("graph.json");

        // Watcher del directorio padre — vigila renames y crear/borrar
        // del archivo aunque aún no exista (agora-store::save escribe a
        // un tmp y rename atómico).
        arranca_watcher(handle.clone(), data_dir.clone());

        // Si AGORA_PASSPHRASE está en env, vamos directo a Main
        // intentando desbloquear con esa passphrase. Si no, mostramos
        // la pantalla de unlock — salvo que el keystore esté vacío,
        // en cuyo caso no hay nada que desbloquear y vamos a Main con
        // la passphrase default ("agora-dev").
        let env_pass = std::env::var("AGORA_PASSPHRASE").ok();
        let keystore = Keystore::open_default()
            .unwrap_or_else(|e| panic!("agora-app: no pude abrir el keystore: {e}"));
        let graph = if store_path.exists() {
            agora_store::load(&store_path).unwrap_or_else(|e| {
                eprintln!("agora-app: no pude cargar el grafo ({e}); empiezo vacío.");
                TrustGraph::new()
            })
        } else {
            TrustGraph::new()
        };

        let ids_keystore = keystore.list().unwrap_or_default();
        let necesita_unlock = !ids_keystore.is_empty() && env_pass.is_none();

        let mut model = Model {
            graph,
            keystore,
            seeds: std::collections::HashMap::new(),
            passphrase: env_pass.unwrap_or_else(|| "agora-dev".to_string()),
            store_path,
            screen: if necesita_unlock {
                Screen::Unlock {
                    input: TextInputState::masked(),
                    status: String::new(),
                }
            } else {
                Screen::Main
            },
            tiles_order: vec![
                Tile::Identidades,
                Tile::Compositor,
                Tile::Atestaciones,
                Tile::Politica,
                Tile::Multifirma,
            ],
            focused_subject: None,
            active_signer: None,
            selected_attestation: None,
            compose_predicate: TextInputState::new(),
            compose_value: TextInputState::new(),
            focused_input: FocusedInput::Compose(ComposeField::Predicate),
            compose_status: String::new(),
            policy: TrustPolicy::default(),
            multi_message: TextInputState::new(),
            multi_selected: std::collections::BTreeSet::new(),
            multi_threshold: 1,
            multi_current: None,
            status: None,
        };

        if !necesita_unlock {
            model.desbloquear_seeds_silencioso();
            model.active_signer = model.seeds.keys().next().copied();
            model.registrar_identidades_huerfanas();
        }
        model
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::SwapTile(from, to) => {
                if from != to && from < model.tiles_order.len() && to < model.tiles_order.len() {
                    model.tiles_order.swap(from, to);
                }
            }

            Msg::NuevaIdentidad => {
                let mut seed = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut seed);
                let kp = Keypair::from_seed(seed);
                let id = kp.identity_id();
                match model.keystore.save(id, &seed, &model.passphrase) {
                    Ok(()) => {
                        model.seeds.insert(id, seed);
                        let n = model.graph.identity_count();
                        model
                            .graph
                            .register(kp.identity(IdentityKind::Person, format!("yo {}", n + 1)));
                        if model.active_signer.is_none() {
                            model.active_signer = Some(id);
                        }
                        model.save_graph();
                        model.compose_status = format!("identidad nueva: {id}");
                    }
                    Err(e) => {
                        model.compose_status = format!("no pude guardar la seed: {e}");
                    }
                }
            }

            Msg::FocoSujeto(id) => {
                model.focused_subject = Some(id);
            }

            Msg::ActuarComo(id) => {
                if model.seeds.contains_key(&id) {
                    model.active_signer = Some(id);
                }
            }

            Msg::SeleccionarAtestacion(idx) => {
                if idx < model.graph.attestations().len() {
                    model.selected_attestation = Some(idx);
                }
            }

            Msg::FocoCompose(field) => {
                model.focused_input = FocusedInput::Compose(field);
            }

            Msg::EditCompose(ev) => {
                if let FocusedInput::Compose(field) = model.focused_input {
                    match field {
                        ComposeField::Predicate => {
                            model.compose_predicate.apply_key(&ev);
                        }
                        ComposeField::Value => {
                            model.compose_value.apply_key(&ev);
                        }
                    }
                }
            }

            Msg::Atestar => {
                let signer = model.signer_keypair();
                let subject = model.focused_subject;
                let predicate = model.compose_predicate.text();
                let value = model.compose_value.text();
                let predicate = predicate.trim();
                let value = value.trim();
                match (signer, subject) {
                    (None, _) => {
                        model.compose_status =
                            "no hay firmante activo (creá una identidad o seleccioná \"actuar como\")".into();
                    }
                    (_, None) => {
                        model.compose_status = "elegí un sujeto en el tile de identidades".into();
                    }
                    _ if predicate.is_empty() || value.is_empty() => {
                        model.compose_status =
                            "predicate y value son obligatorios".into();
                    }
                    (Some(kp), Some(subject)) => {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let claim = Claim::new(subject, predicate, value, now);
                        let att = Attestation::create(&kp, claim);
                        match model.graph.add_attestation(att.clone()) {
                            Ok(()) => {
                                model.compose_predicate.clear();
                                model.compose_value.clear();
                                model.compose_status = "atestación agregada y persistida".into();
                                // Append al log, no save completo.
                                if let Err(e) =
                                    agora_store::append_attestation(&model.store_path, &att)
                                {
                                    model.set_status(
                                        StatusLevel::Error,
                                        format!("no pude appendear la atestación: {e}"),
                                    );
                                }
                            }
                            Err(e) => {
                                model.compose_status = format!("rechazada: {e}");
                            }
                        }
                    }
                }
            }

            Msg::SliderMinThird(_phase, dv) => {
                let cur = model.policy.min_third_party as f32 + dv;
                let new = cur.clamp(0.0, 5.0).round() as usize;
                model.policy.min_third_party = new;
            }

            Msg::ToggleAcceptSelf => {
                model.policy.accept_self = !model.policy.accept_self;
            }

            Msg::CycleKind => {
                model.policy.min_attesters_of_kind = match model.policy.min_attesters_of_kind {
                    None => Some((IdentityKind::Person, 1)),
                    Some((IdentityKind::Person, n)) => Some((IdentityKind::Community, n)),
                    Some((IdentityKind::Community, n)) => Some((IdentityKind::Alliance, n)),
                    Some((IdentityKind::Alliance, n)) => Some((IdentityKind::Institution, n)),
                    Some((IdentityKind::Institution, _)) => None,
                };
            }

            Msg::SliderMinKind(_phase, dv) => {
                if let Some((kind, n)) = model.policy.min_attesters_of_kind {
                    let cur = n as f32 + dv;
                    let new = cur.clamp(1.0, 5.0).round() as usize;
                    model.policy.min_attesters_of_kind = Some((kind, new));
                }
            }

            Msg::CycleMaxAge => {
                model.policy.max_age_secs = match model.policy.max_age_secs {
                    None => Some(60),
                    Some(60) => Some(300),
                    Some(300) => Some(3_600),
                    Some(3_600) => Some(86_400),
                    Some(86_400) => Some(604_800),
                    _ => None,
                };
            }

            Msg::UnlockKey(ev) => {
                if let Screen::Unlock { input, .. } = &mut model.screen {
                    input.apply_key(&ev);
                }
            }

            Msg::UnlockSubmit => {
                // Sacamos primero la passphrase del input para liberar
                // el borrow de `model.screen` antes de mutar otras
                // partes de `model` desde `intentar_unlock`.
                let pass_intentada = if let Screen::Unlock { input, .. } = &model.screen {
                    Some(input.text())
                } else {
                    None
                };
                if let Some(pass) = pass_intentada {
                    if model.intentar_unlock(&pass) {
                        model.registrar_identidades_huerfanas();
                        model.active_signer = model.seeds.keys().next().copied();
                        model.screen = Screen::Main;
                    } else if let Screen::Unlock { input, status } = &mut model.screen {
                        *status = "passphrase incorrecta".into();
                        input.clear();
                    }
                }
            }

            Msg::ArchivoCambio => {
                // Otro proceso (típicamente agora-cli) escribió
                // graph.json. Releemos. agora-store::save es atómico
                // (tmp+rename), así que load siempre ve estado
                // consistente. Si falla, dejamos el grafo en memoria
                // intacto y logueamos.
                match agora_store::load(&model.store_path) {
                    Ok(g) => {
                        // Conservamos selecciones si las identidades
                        // siguen existiendo en el nuevo grafo. Si no,
                        // las limpiamos.
                        if let Some(id) = model.focused_subject {
                            if g.identity(id).is_none() {
                                model.focused_subject = None;
                            }
                        }
                        if let Some(idx) = model.selected_attestation {
                            if idx >= g.attestations().len() {
                                model.selected_attestation = None;
                            }
                        }
                        let antes_atts = model.graph.attestation_count();
                        let antes_idents = model.graph.identity_count();
                        model.graph = g;
                        let delta_atts =
                            model.graph.attestation_count() as isize - antes_atts as isize;
                        let delta_idents =
                            model.graph.identity_count() as isize - antes_idents as isize;
                        if delta_atts != 0 || delta_idents != 0 {
                            model.set_status(
                                StatusLevel::Info,
                                format!(
                                    "grafo recargado desde disco · {:+} atestaciones · {:+} identidades",
                                    delta_atts, delta_idents
                                ),
                            );
                        }
                    }
                    Err(e) => {
                        model.set_status(
                            StatusLevel::Error,
                            format!(
                                "no pude recargar graph.json ({e}); sigo con el grafo en memoria"
                            ),
                        );
                    }
                }
            }

            Msg::DescartarStatus => {
                model.status = None;
            }

            Msg::FocoMultiMessage => {
                model.focused_input = FocusedInput::MultiMessage;
            }

            Msg::EditMultiMessage(ev) => {
                if matches!(model.focused_input, FocusedInput::MultiMessage) {
                    model.multi_message.apply_key(&ev);
                    model.multi_current = None;
                }
            }

            Msg::ToggleMultiFirmante(id) => {
                if !model.seeds.contains_key(&id) {
                    // Sólo permitimos seleccionar identidades propias
                    // (las que tienen seed en el keystore desbloqueado).
                    return model;
                }
                if !model.multi_selected.insert(id) {
                    model.multi_selected.remove(&id);
                }
                // Cualquier cambio en el conjunto invalida la firma
                // vigente y reclampa el umbral.
                model.multi_current = None;
                let n = model.multi_selected.len();
                if n == 0 {
                    model.multi_threshold = 1;
                } else if model.multi_threshold > n {
                    model.multi_threshold = n;
                } else if model.multi_threshold == 0 {
                    model.multi_threshold = 1;
                }
            }

            Msg::SliderMultiUmbral(_phase, dv) => {
                let n = model.multi_selected.len().max(1);
                let cur = model.multi_threshold as f32 + dv;
                model.multi_threshold = cur.clamp(1.0, n as f32).round() as usize;
            }

            Msg::FirmarMulti => {
                if model.multi_selected.is_empty() {
                    model.set_status(
                        StatusLevel::Error,
                        "elegí al menos una identidad propia como firmante",
                    );
                    return model;
                }
                let mensaje_txt = model.multi_message.text();
                if mensaje_txt.trim().is_empty() {
                    model.set_status(
                        StatusLevel::Error,
                        "el mensaje de la multifirma no puede estar vacío",
                    );
                    return model;
                }
                // Materializamos un keypair por seed seleccionada y
                // firmamos `mensaje` (bytes UTF-8 crudos).
                let pares: Vec<Keypair> = model
                    .multi_selected
                    .iter()
                    .filter_map(|id| model.seeds.get(id).copied().map(Keypair::from_seed))
                    .collect();
                let refs: Vec<&Keypair> = pares.iter().collect();
                let multi = MultiSignature::create(&refs, mensaje_txt.as_bytes());
                model.multi_current = Some(multi);
            }

            Msg::LimpiarMulti => {
                model.multi_selected.clear();
                model.multi_threshold = 1;
                model.multi_current = None;
                model.multi_message.clear();
            }

            Msg::ExportarMulti => {
                match model.multi_current.as_ref() {
                    None => {
                        model.set_status(
                            StatusLevel::Error,
                            "no hay multifirma vigente — firmá primero",
                        );
                    }
                    Some(multi) => match postcard::to_allocvec(multi) {
                        Ok(bytes) => {
                            let hex = bytes_to_hex(&bytes);
                            model.set_status(
                                StatusLevel::Info,
                                format!(
                                    "multifirma postcard ({n} bytes): {hex}",
                                    n = bytes.len()
                                ),
                            );
                        }
                        Err(e) => {
                            model.set_status(
                                StatusLevel::Error,
                                format!("no pude serializar la multifirma: {e}"),
                            );
                        }
                    },
                }
            }
        }
        model
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // En la pantalla de unlock todas las teclas van al input;
        // Enter dispara el intento de desbloqueo.
        if matches!(model.screen, Screen::Unlock { .. }) {
            if let Key::Named(NamedKey::Enter) = &event.key {
                return Some(Msg::UnlockSubmit);
            }
            return Some(Msg::UnlockKey(event.clone()));
        }
        // Tab cicla campos del compositor cuando el foco está ahí. Si
        // el foco está en el multi_message, Tab lo manda al compositor.
        if let Key::Named(NamedKey::Tab) = &event.key {
            return Some(match model.focused_input {
                FocusedInput::Compose(ComposeField::Predicate) => {
                    Msg::FocoCompose(ComposeField::Value)
                }
                FocusedInput::Compose(ComposeField::Value) => {
                    Msg::FocoCompose(ComposeField::Predicate)
                }
                FocusedInput::MultiMessage => Msg::FocoCompose(ComposeField::Predicate),
            });
        }
        match model.focused_input {
            FocusedInput::Compose(_) => {
                // Enter firma la atestación; el resto edita el input
                // del compositor.
                if let Key::Named(NamedKey::Enter) = &event.key {
                    return Some(Msg::Atestar);
                }
                Some(Msg::EditCompose(event.clone()))
            }
            FocusedInput::MultiMessage => {
                // Enter dispara la firma de la multifirma.
                if let Key::Named(NamedKey::Enter) = &event.key {
                    return Some(Msg::FirmarMulti);
                }
                Some(Msg::EditMultiMessage(event.clone()))
            }
        }
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        if matches!(model.screen, Screen::Unlock { .. }) {
            return unlock_view(model, &theme);
        }
        let palette = TiledPalette::from_theme(&theme);

        let tiles: Vec<TileSpec<Msg>> = model
            .tiles_order
            .iter()
            .map(|t| match t {
                Tile::Identidades => TileSpec {
                    label: "identidades".into(),
                    content: identidades_view(model, &theme),
                },
                Tile::Compositor => TileSpec {
                    label: "compositor".into(),
                    content: compositor_view(model, &theme),
                },
                Tile::Atestaciones => TileSpec {
                    label: "atestaciones".into(),
                    content: atestaciones_view(model, &theme),
                },
                Tile::Politica => TileSpec {
                    label: "política".into(),
                    content: politica_view(model, &theme),
                },
                Tile::Multifirma => TileSpec {
                    label: "multifirma".into(),
                    content: multifirma_view(model, &theme),
                },
            })
            .collect();

        let tiled =
            tiled_view_reorderable(tiles, |from, to| Some(Msg::SwapTile(from, to)), &palette);

        match &model.status {
            None => tiled,
            Some(banner) => status_layout(&theme, tiled, banner),
        }
    }
}

/// Compone el tiled view con un banner al pie. El banner ocupa
/// 34 px fijos y empuja al tiled hacia arriba.
fn status_layout(theme: &Theme, tiled: View<Msg>, banner: &StatusBanner) -> View<Msg> {
    let (bg, fg) = match banner.level {
        StatusLevel::Info => (theme.bg_panel, theme.fg_text),
        StatusLevel::Error => (theme.fg_destructive, theme.bg_app),
    };

    let texto = View::new(Style {
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: edge_padding(12.0, 0.0),
        ..Default::default()
    })
    .text_aligned(banner.text.clone(), 12.0, fg, Alignment::Start);

    let cerrar = button_styled(
        "×",
        Style {
            size: Size {
                width: length(34.0_f32),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &ButtonPalette {
            bg,
            bg_hover: theme.bg_button_hover,
            fg,
            radius: 0.0,
        },
        Msg::DescartarStatus,
    );

    let banner_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .children(vec![texto, cerrar]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![grow(tiled), banner_row])
}

// =============================================================================
//  Pantalla: Unlock
// =============================================================================

fn unlock_view(model: &Model, theme: &Theme) -> View<Msg> {
    let (input, status_text) = match &model.screen {
        Screen::Unlock { input, status } => (input, status.as_str()),
        Screen::Main => unreachable!("unlock_view sólo se llama con Screen::Unlock"),
    };
    let input_palette = TextInputPalette::from_theme(theme);

    let titulo = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        "ágora · desbloqueo".to_string(),
        24.0,
        theme.accent,
        Alignment::Center,
    );

    let hint = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(
        "passphrase del keystore (Enter desbloquea)".to_string(),
        12.0,
        theme.fg_muted,
        Alignment::Center,
    );

    let input_view = View::new(Style {
        size: Size {
            width: length(360.0_f32),
            height: length(36.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![text_input_view(
        input,
        "•••",
        true,
        &input_palette,
        Msg::UnlockSubmit, // click en el input no cambia foco — sólo hay uno
    )]);

    let status = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(
        if status_text.is_empty() {
            String::new()
        } else {
            status_text.to_string()
        },
        12.0,
        theme.fg_destructive,
        Alignment::Center,
    );

    let boton = button_styled(
        "desbloquear",
        Style {
            size: Size {
                width: length(360.0_f32),
                height: length(34.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_primary(theme),
        Msg::UnlockSubmit,
    );

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(420.0_f32),
            height: length(260.0_f32),
        },
        padding: Rect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(24.0_f32),
            bottom: length(20.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(8.0)
    .children(vec![titulo, hint, spacer(8.0), input_view, boton, status]);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![card])
}

// =============================================================================
//  Tile: Identidades
// =============================================================================

fn identidades_view(model: &Model, theme: &Theme) -> View<Msg> {
    let list_palette = ListPalette::from_theme(theme);

    // Orden estable: por id bytes, ya que `graph.identities()` itera
    // sobre el HashMap interno.
    let mut idents: Vec<_> = model.graph.identities().collect();
    idents.sort_by(|a, b| a.id().as_bytes().cmp(b.id().as_bytes()));

    let rows: Vec<ListRow<Msg>> = idents
        .iter()
        .map(|ident| {
            let id = ident.id();
            let prefix = if model.is_mine(id) { "★ " } else { "  " };
            let active = Some(id) == model.active_signer;
            let mark_active = if active { " ← activa" } else { "" };
            let kind = match ident.kind {
                IdentityKind::Person => "persona",
                IdentityKind::Community => "comunidad",
                IdentityKind::Alliance => "alianza",
                IdentityKind::Institution => "institución",
            };
            ListRow {
                label: format!(
                    "{prefix}{id}  {kind}  {name}{mark_active}",
                    name = ident.display_name
                ),
                selected: model.focused_subject == Some(id),
                on_click: Msg::FocoSujeto(id),
            }
        })
        .collect();

    let caption = format!(
        "{} identidades · {} mías · enfocada: {}",
        idents.len(),
        model.seeds.len(),
        model
            .focused_subject
            .map(|id| format!("{id}"))
            .unwrap_or_else(|| "—".into())
    );

    let list = list_view(ListSpec {
        rows,
        total: idents.len(),
        caption: Some(caption),
        truncated_hint: None,
        row_height: 22.0,
        palette: list_palette,
    });

    let mut footer_buttons: Vec<View<Msg>> = vec![button_styled(
        "+ nueva identidad",
        Style {
            size: Size {
                width: percent(0.5_f32),
                height: length(30.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_primary(theme),
        Msg::NuevaIdentidad,
    )];

    // Botón "actuar como" — sólo se habilita si la enfocada es mía y
    // distinta de la activa actual.
    let can_act_as = model
        .focused_subject
        .filter(|id| model.is_mine(*id) && Some(*id) != model.active_signer);
    if let Some(id) = can_act_as {
        footer_buttons.push(button_styled(
            "actuar como ★ enfocada",
            Style {
                size: Size {
                    width: percent(0.5_f32),
                    height: length(30.0_f32),
                },
                padding: edge_padding(10.0, 0.0),
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            },
            Alignment::Center,
            &button_palette_secondary(theme),
            Msg::ActuarComo(id),
        ));
    }

    let footer = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(38.0_f32),
        },
        flex_shrink: 0.0,
        padding: edge_padding(8.0, 4.0),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(footer_buttons);

    column(vec![grow(list), footer])
}

// =============================================================================
//  Tile: Compositor
// =============================================================================

fn compositor_view(model: &Model, theme: &Theme) -> View<Msg> {
    let input_palette = TextInputPalette::from_theme(theme);

    let signer_line = format!(
        "yo: {}",
        model
            .active_signer
            .map(|id| format!("★ {id}"))
            .unwrap_or_else(|| "(ninguna — creá una identidad)".into())
    );
    let subject_line = format!(
        "sobre: {}",
        model
            .focused_subject
            .map(|id| format!("{id}"))
            .unwrap_or_else(|| "(elegí una identidad en el tile de la izquierda)".into())
    );

    let header_signer = label_line(&signer_line, 13.0, theme.fg_text);
    let header_subject = label_line(&subject_line, 13.0, theme.fg_text);

    let label_predicate = label_line("predicado", 10.0, theme.fg_muted);
    let label_value = label_line("valor", 10.0, theme.fg_muted);

    let input_predicate = input_row(
        &model.compose_predicate,
        "nacionalidad / miembro-de / habilidad …",
        model.focused_input == FocusedInput::Compose(ComposeField::Predicate),
        &input_palette,
        Msg::FocoCompose(ComposeField::Predicate),
    );
    let input_value = input_row(
        &model.compose_value,
        "venezolana / El Valle / soldadura …",
        model.focused_input == FocusedInput::Compose(ComposeField::Value),
        &input_palette,
        Msg::FocoCompose(ComposeField::Value),
    );

    let firmar = button_styled(
        "atestar (Enter)",
        Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(34.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_primary(theme),
        Msg::Atestar,
    );

    let status_color = if model.compose_status.starts_with("atestación") {
        theme.accent
    } else if model.compose_status.is_empty() {
        theme.fg_muted
    } else {
        theme.fg_destructive
    };
    let status = label_line(
        if model.compose_status.is_empty() {
            "Tab cicla campos · Enter firma"
        } else {
            &model.compose_status
        },
        11.0,
        status_color,
    );

    column(vec![
        spacer(6.0),
        header_signer,
        header_subject,
        spacer(8.0),
        label_predicate,
        input_predicate,
        spacer(4.0),
        label_value,
        input_value,
        spacer(8.0),
        firmar,
        spacer(6.0),
        status,
        grow(empty()),
    ])
}

fn input_row(
    state: &TextInputState,
    placeholder: &str,
    focused: bool,
    palette: &TextInputPalette,
    on_focus: Msg,
) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![text_input_view(state, placeholder, focused, palette, on_focus)])
}

// =============================================================================
//  Tile: Atestaciones
// =============================================================================

fn atestaciones_view(model: &Model, theme: &Theme) -> View<Msg> {
    let list_palette = ListPalette::from_theme(theme);

    let rows: Vec<ListRow<Msg>> = model
        .graph
        .attestations()
        .iter()
        .enumerate()
        .map(|(idx, att)| {
            let mark = if att.is_self_attested() { "[self]" } else { "      " };
            let attester_name = model
                .graph
                .identity(att.attester)
                .map(|i| i.display_name.as_str())
                .unwrap_or("?");
            ListRow {
                label: format!(
                    "{mark}  {att}  ←  {attester} · {pred} = {val}",
                    att = att.attester,
                    attester = attester_name,
                    pred = att.claim.predicate,
                    val = att.claim.value,
                ),
                selected: model.selected_attestation == Some(idx),
                on_click: Msg::SeleccionarAtestacion(idx),
            }
        })
        .collect();

    let total = rows.len();
    let caption = format!(
        "{total} atestaciones verificadas · seleccioná una para evaluar política"
    );

    list_view(ListSpec {
        rows,
        total,
        caption: Some(caption),
        truncated_hint: None,
        row_height: 22.0,
        palette: list_palette,
    })
}

// =============================================================================
//  Tile: Política
// =============================================================================

fn politica_view(model: &Model, theme: &Theme) -> View<Msg> {
    let slider_palette = SliderPalette::from_theme(theme);

    let slider = slider_view(
        "min terceros",
        model.policy.min_third_party as f32,
        0.0,
        5.0,
        &slider_palette,
        |phase, dv| Some(Msg::SliderMinThird(phase, dv)),
    );

    let toggle = button_styled(
        format!(
            "accept_self: {}",
            if model.policy.accept_self { "sí" } else { "no" }
        ),
        Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(30.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_secondary(theme),
        Msg::ToggleAcceptSelf,
    );

    let kind_label = match model.policy.min_attesters_of_kind {
        None => "kind: off".to_string(),
        Some((k, _)) => format!("kind: {}", kind_str(k)),
    };
    let kind_button = button_styled(
        kind_label,
        Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(30.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_secondary(theme),
        Msg::CycleKind,
    );

    // Slider del N sólo aparece si el eje kind está activo. Cuando está
    // en off mostramos un hint discreto para que el tile no salte de
    // alto entre estados.
    let kind_n_view: View<Msg> = match model.policy.min_attesters_of_kind {
        Some((_, n)) => slider_view(
            "min de kind",
            n as f32,
            1.0,
            5.0,
            &slider_palette,
            |phase, dv| Some(Msg::SliderMinKind(phase, dv)),
        ),
        None => label_line(
            "(activá un kind para exigir un mínimo)",
            10.0,
            theme.fg_muted,
        ),
    };

    let max_age_button = button_styled(
        format!("edad máx: {}", format_max_age(model.policy.max_age_secs)),
        Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(30.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_secondary(theme),
        Msg::CycleMaxAge,
    );

    // Veredicto sobre la atestación seleccionada.
    let verdict_block = match model.selected_attestation.and_then(|i| {
        let atts = model.graph.attestations();
        atts.get(i).cloned()
    }) {
        None => column(vec![label_line(
            "seleccioná una atestación para ver el veredicto",
            12.0,
            theme.fg_muted,
        )]),
        Some(att) => {
            let cor: Corroboration = model.graph.corroboration(
                att.claim.subject,
                &att.claim.predicate,
                &att.claim.value,
            );
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let ok = model.graph.is_accepted_at(
                att.claim.subject,
                &att.claim.predicate,
                &att.claim.value,
                &model.policy,
                now,
            );
            let veredicto_color = if ok { theme.accent } else { theme.fg_destructive };
            let veredicto = label_line(
                if ok { "ACEPTA" } else { "rechaza" },
                26.0,
                veredicto_color,
            );

            // Desglose por eje: indica cuál falla y cuál pasa, para que
            // el usuario entienda por qué el veredicto es lo que es.
            let eje_basico = model.policy.accepts(&cor);
            let eje_kind = match model.policy.min_attesters_of_kind {
                None => None,
                Some((kind, n)) => {
                    let count = cor
                        .attesters
                        .iter()
                        .filter(|id| {
                            model
                                .graph
                                .identity(**id)
                                .map(|i| i.kind == kind)
                                .unwrap_or(false)
                        })
                        .count();
                    Some((kind, n, count))
                }
            };
            let eje_edad: Option<(u64, u64)> = match model.policy.max_age_secs {
                None => None,
                Some(max_age) => {
                    let mas_reciente = model
                        .graph
                        .attestations()
                        .iter()
                        .filter(|a| {
                            a.claim.subject == att.claim.subject
                                && a.claim.predicate == att.claim.predicate
                                && a.claim.value == att.claim.value
                        })
                        .map(|a| a.claim.issued_at)
                        .max()
                        .unwrap_or(0);
                    let edad = now.saturating_sub(mas_reciente);
                    Some((edad, max_age))
                }
            };

            let mut detail: Vec<View<Msg>> = vec![
                label_line(
                    &format!("claim: {} = {}", att.claim.predicate, att.claim.value),
                    12.0,
                    theme.fg_text,
                ),
                label_line(
                    &format!("sujeto: {}", att.claim.subject),
                    11.0,
                    theme.fg_muted,
                ),
                spacer(4.0),
                label_line(
                    &format!(
                        "{}  básico: terceros {} / {} · auto {}",
                        if eje_basico { "✓" } else { "✗" },
                        cor.third_party(),
                        model.policy.min_third_party,
                        if cor.self_attested { "sí" } else { "no" }
                    ),
                    11.0,
                    if eje_basico { theme.fg_muted } else { theme.fg_destructive },
                ),
            ];
            if let Some((kind, requeridos, count)) = eje_kind {
                let pasa = count >= requeridos;
                detail.push(label_line(
                    &format!(
                        "{}  kind: {} {} / {}",
                        if pasa { "✓" } else { "✗" },
                        kind_str(kind),
                        count,
                        requeridos
                    ),
                    11.0,
                    if pasa { theme.fg_muted } else { theme.fg_destructive },
                ));
            }
            if let Some((edad, max_age)) = eje_edad {
                let pasa = edad <= max_age;
                detail.push(label_line(
                    &format!(
                        "{}  edad: {} / {} máx",
                        if pasa { "✓" } else { "✗" },
                        format_duration(edad),
                        format_duration(max_age)
                    ),
                    11.0,
                    if pasa { theme.fg_muted } else { theme.fg_destructive },
                ));
            }
            detail.push(spacer(6.0));
            detail.push(veredicto);

            column(detail)
        }
    };

    column(vec![
        spacer(8.0),
        slider,
        spacer(8.0),
        toggle,
        spacer(8.0),
        kind_button,
        kind_n_view,
        spacer(8.0),
        max_age_button,
        spacer(12.0),
        verdict_block,
        grow(empty()),
    ])
}

fn kind_str(k: IdentityKind) -> &'static str {
    match k {
        IdentityKind::Person => "persona",
        IdentityKind::Community => "comunidad",
        IdentityKind::Alliance => "alianza",
        IdentityKind::Institution => "institución",
    }
}

fn format_max_age(v: Option<u64>) -> String {
    match v {
        None => "off".into(),
        Some(s) => format_duration(s),
    }
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

// =============================================================================
//  Tile: Multifirma
// =============================================================================

fn multifirma_view(model: &Model, theme: &Theme) -> View<Msg> {
    let input_palette = TextInputPalette::from_theme(theme);
    let list_palette = ListPalette::from_theme(theme);
    let slider_palette = SliderPalette::from_theme(theme);

    let mensaje_label = label_line("mensaje a multifirmar", 10.0, theme.fg_muted);
    let mensaje_input = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![text_input_view(
        &model.multi_message,
        "raíz canónica / hash de manifiesto / …",
        matches!(model.focused_input, FocusedInput::MultiMessage),
        &input_palette,
        Msg::FocoMultiMessage,
    )]);

    // Lista de identidades propias con check ☑/☐ indicando selección.
    // Click toggle: la fila NO se pinta como selected porque eso ya lo
    // hace el check; selected lo reservamos para focused_subject.
    let mut mias: Vec<_> = model
        .seeds
        .keys()
        .copied()
        .filter_map(|id| model.graph.identity(id).map(|i| (id, i)))
        .collect();
    mias.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let rows: Vec<ListRow<Msg>> = mias
        .iter()
        .map(|(id, ident)| {
            let check = if model.multi_selected.contains(id) {
                "☑"
            } else {
                "☐"
            };
            ListRow {
                label: format!(
                    "{check}  {id}  {kind}  {name}",
                    id = id,
                    kind = kind_str(ident.kind),
                    name = ident.display_name
                ),
                selected: false,
                on_click: Msg::ToggleMultiFirmante(*id),
            }
        })
        .collect();
    let firmantes_list = list_view(ListSpec {
        rows,
        total: mias.len(),
        caption: Some(format!(
            "{} identidades propias · {} elegidas",
            mias.len(),
            model.multi_selected.len()
        )),
        truncated_hint: None,
        row_height: 22.0,
        palette: list_palette,
    });

    let n_seleccionados = model.multi_selected.len().max(1);
    let umbral_slider = slider_view(
        "umbral M",
        model.multi_threshold as f32,
        1.0,
        n_seleccionados as f32,
        &slider_palette,
        |phase, dv| Some(Msg::SliderMultiUmbral(phase, dv)),
    );

    let firmar = button_styled(
        "firmar las elegidas (Enter)",
        Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(30.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_primary(theme),
        Msg::FirmarMulti,
    );

    let exportar = button_styled(
        "exportar postcard hex →",
        Style {
            size: Size {
                width: percent(0.5_f32),
                height: length(30.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_secondary(theme),
        Msg::ExportarMulti,
    );

    let limpiar = button_styled(
        "limpiar",
        Style {
            size: Size {
                width: percent(0.5_f32),
                height: length(30.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_secondary(theme),
        Msg::LimpiarMulti,
    );

    let acciones_secundarias = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        flex_shrink: 0.0,
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![exportar, limpiar]);

    // Veredicto en vivo: si hay multifirma vigente, evaluamos su umbral
    // contra el mensaje actual y mostramos firmantes_distintos / N.
    let veredicto_block: View<Msg> = match &model.multi_current {
        None => label_line(
            "(sin multifirma vigente — elegí firmantes, escribí el mensaje y Enter)",
            11.0,
            theme.fg_muted,
        ),
        Some(multi) => {
            let mensaje_bytes = model.multi_message.text();
            let v = multi.verdict(mensaje_bytes.as_bytes());
            let pasa = v.firmantes_distintos >= model.multi_threshold;
            let color = if pasa { theme.accent } else { theme.fg_destructive };
            column(vec![
                label_line(
                    &format!(
                        "verdict: {} válidas · {} distintas · umbral {}",
                        v.validas, v.firmantes_distintos, model.multi_threshold
                    ),
                    11.0,
                    color,
                ),
                label_line(
                    if pasa {
                        "ACEPTA (umbral alcanzado)"
                    } else {
                        "rechaza (faltan firmantes distintos)"
                    },
                    14.0,
                    color,
                ),
            ])
        }
    };

    column(vec![
        spacer(6.0),
        mensaje_label,
        mensaje_input,
        spacer(8.0),
        grow(firmantes_list),
        spacer(6.0),
        umbral_slider,
        spacer(6.0),
        firmar,
        spacer(4.0),
        acciones_secundarias,
        spacer(8.0),
        veredicto_block,
    ])
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// =============================================================================
//  Helpers de layout y paletas
// =============================================================================

fn column<Msg: 'static>(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: edge_padding(10.0, 6.0),
        gap: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

fn grow<Msg: 'static>(v: View<Msg>) -> View<Msg> {
    let mut v = v;
    v.style.flex_grow = 1.0;
    v.style.flex_basis = length(0.0_f32);
    v.style.min_size = Size {
        width: length(0.0_f32),
        height: length(0.0_f32),
    };
    v
}

fn empty<Msg: 'static>() -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
}

fn spacer<Msg: 'static>(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

fn label_line<Msg: 'static>(text: &str, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size + 8.0),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text.to_string(), size, color, Alignment::Start)
}

fn edge_padding(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

fn button_palette_primary(t: &Theme) -> ButtonPalette {
    ButtonPalette {
        bg: t.accent,
        bg_hover: t.bg_button_hover,
        fg: t.bg_app,
        radius: 4.0,
    }
}

fn button_palette_secondary(t: &Theme) -> ButtonPalette {
    ButtonPalette {
        bg: t.bg_button,
        bg_hover: t.bg_button_hover,
        fg: t.fg_text,
        radius: 4.0,
    }
}

// =============================================================================
//  Entrypoint
// =============================================================================

/// Lanza un watcher de filesystem sobre `dir` y dispatcha
/// [`Msg::ArchivoCambio`] cada vez que notify reporta un evento OK.
/// El callback es coarse — no distingue qué archivo cambió ni qué tipo
/// de evento; el `update` decide si el cambio es relevante reintentando
/// el `load`. El `Watcher` se "leakea" deliberadamente con
/// `mem::forget`: tiene que vivir mientras el proceso corra y la app
/// no tiene un buen lugar donde almacenarlo dentro del Model (es
/// `!Send` en algunos backends de notify y el Model debe ser
/// estructurado simple).
fn arranca_watcher(handle: Handle<Msg>, dir: PathBuf) {
    use notify::{RecursiveMode, Watcher};
    let watcher_res = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            handle.dispatch(Msg::ArchivoCambio);
        }
    });
    let mut watcher = match watcher_res {
        Ok(w) => w,
        Err(e) => {
            eprintln!("agora-app: no pude arrancar el watcher ({e}); cambios externos al grafo no se reflejarán hasta reiniciar.");
            return;
        }
    };
    if let Err(e) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
        eprintln!("agora-app: no pude vigilar {} ({e}); cambios externos al grafo no se reflejarán.", dir.display());
        return;
    }
    std::mem::forget(watcher);
}

fn main() {
    llimphi_ui::run::<AgoraApp>();
}
