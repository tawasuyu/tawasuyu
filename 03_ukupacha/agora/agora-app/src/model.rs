//! Estado de la app y el catálogo de mensajes.
//!
//! `agora-app` es una sola ventana Llimphi con tiles draggables sobre el
//! mismo `TrustGraph`. Hay dos familias de tiles:
//!
//! - **Sociales** (sustrato): identidades · compositor · atestaciones ·
//!   política · multifirma. Construyen y evalúan la web-of-trust.
//! - **Plano de control de wawa** (el norte): release · capacidad. Firman
//!   y verifican los sobres Ed25519 que el kernel honra — `ManifiestoFirmado`
//!   (quién empuja una imagen) y `ConcesionCapacidad` (qué bytecode toca el
//!   hardware). Reusan `agora-channel`, el mismo código que valida el kernel.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use agora_core::{IdentityId, IdentityKind, Keypair, MultiSignature};
use agora_graph::{TrustGraph, TrustPolicy};
use agora_keystore::Keystore;
use format::{ConcesionCapacidad, ManifiestoFirmado, Permisos};
use llimphi_ui::KeyEvent;
use llimphi_widget_text_input::TextInputState;

// =============================================================================
//  Tiles
// =============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tile {
    Identidades,
    Compositor,
    Atestaciones,
    Politica,
    /// Compositor de [`MultiSignature`]: elige firmantes "míos", escribe
    /// el mensaje (típicamente una raíz canónica), elige umbral M, firma
    /// y exporta postcard en hex.
    Multifirma,
    /// **Plano de control wawa.** Firma un `ManifiestoFirmado` (hash de una
    /// imagen del sistema) con el firmante activo y verifica sobres pegados.
    /// Es el sobre que `apps/mudanza` empuja al kernel.
    Release,
    /// **Plano de control wawa (§14.1.3).** Concede capacidades por
    /// `(hash_bytecode, permisos)`: firma una `ConcesionCapacidad`. El kernel
    /// honra la intersección contra el `AGORA_AUTH_RING`.
    Capacidad,
}

/// Orden inicial de tiles en la grilla draggable.
pub(crate) const TILES_INICIALES: [Tile; 7] = [
    Tile::Identidades,
    Tile::Compositor,
    Tile::Atestaciones,
    Tile::Politica,
    Tile::Multifirma,
    Tile::Release,
    Tile::Capacidad,
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComposeField {
    Predicate,
    Value,
}

/// Qué input de texto recibe las teclas en la pantalla principal. Un solo
/// `on_key` rutea cada evento al `TextInputState` correcto vía
/// [`Model::focused_input_mut`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusedInput {
    Compose(ComposeField),
    MultiMessage,
    /// Hash hex del manifiesto a firmar (tile Release).
    ReleaseHash,
    /// Postcard hex de un `ManifiestoFirmado` a verificar (tile Release).
    ReleasePaste,
    /// Hash hex del bytecode a conceder (tile Capacidad).
    CapBytecode,
    /// Postcard hex de una `ConcesionCapacidad` a verificar (tile Capacidad).
    CapPaste,
}

/// Severidad de un mensaje de estado de servicio (persistencia, red,
/// keystore). Determina el color del banner.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusLevel {
    Info,
    Error,
}

/// Banner visible al pie de la ventana cuando hay un error o info que vale
/// la pena destacar (típicamente I/O, red, o un export que conviene poder
/// copiar). `None` lo oculta.
pub(crate) struct StatusBanner {
    pub level: StatusLevel,
    pub text: String,
}

/// Pantalla activa. `Unlock` pide la passphrase; `Main` muestra los tiles.
pub(crate) enum Screen {
    Unlock {
        input: TextInputState,
        /// Vacío hasta el primer intento; al fallar, "passphrase incorrecta".
        status: String,
    },
    Main,
}

pub(crate) struct Model {
    pub graph: TrustGraph,
    pub keystore: Keystore,
    /// Seeds en RAM de las identidades "mías" (con archivo en el keystore).
    /// Se desbloquean al arrancar y viven aquí mientras corre el proceso; no
    /// persisten al salir — siguen cifradas en el keystore.
    pub seeds: HashMap<IdentityId, [u8; 32]>,
    pub passphrase: String,
    pub store_path: PathBuf,
    pub screen: Screen,
    pub tiles_order: Vec<Tile>,

    /// Identidad seleccionada como sujeto (objetivo del próximo claim).
    pub focused_subject: Option<IdentityId>,
    /// Identidad firmante activa (debe estar en `seeds`). También es la que
    /// firma releases y concesiones.
    pub active_signer: Option<IdentityId>,
    /// Atestación seleccionada en el tile de atestaciones, por índice.
    pub selected_attestation: Option<usize>,

    pub compose_predicate: TextInputState,
    pub compose_value: TextInputState,
    /// Input que recibe las teclas (ver [`FocusedInput`]).
    pub focused_input: FocusedInput,
    /// Último mensaje al pie del compositor (éxito, error, hint).
    pub compose_status: String,

    pub policy: TrustPolicy,

    // ---- Multifirma --------------------------------------------------------
    /// Mensaje sobre el que se compone la multifirma (típicamente una raíz).
    pub multi_message: TextInputState,
    /// Identidades "mías" elegidas como firmantes. Sólo ids en `seeds`.
    pub multi_selected: BTreeSet<IdentityId>,
    /// Umbral M, clampado a `[1, max(1, N)]` con N = `multi_selected.len()`.
    pub multi_threshold: usize,
    /// Última multifirma producida. Se descarta al cambiar selección o mensaje.
    pub multi_current: Option<MultiSignature>,

    // ---- Release (plano de control wawa) -----------------------------------
    /// Hash hex (64 chars) del manifiesto a firmar.
    pub release_hash: TextInputState,
    /// Postcard hex de un `ManifiestoFirmado` ajeno a verificar.
    pub release_paste: TextInputState,
    /// Último release firmado por el firmante activo. Se descarta al editar
    /// el hash o cambiar de firmante.
    pub release_current: Option<ManifiestoFirmado>,
    /// Feedback in-situ del tile Release (verificación, errores de parseo).
    pub release_status: String,

    // ---- Capacidad (§14.1.3) -----------------------------------------------
    /// Hash hex (64 chars) del bytecode al que aplica la concesión.
    pub cap_bytecode: TextInputState,
    /// Bitfield de permisos a conceder (toggles de los `PERMISO_*`).
    pub cap_permisos: Permisos,
    /// Postcard hex de una `ConcesionCapacidad` ajena a verificar.
    pub cap_paste: TextInputState,
    /// Última concesión firmada. Se descarta al editar bytecode/permisos.
    pub cap_current: Option<ConcesionCapacidad>,
    /// Feedback in-situ del tile Capacidad.
    pub cap_status: String,

    /// Banner de estado de servicio al pie. Lo pinta el `view`.
    pub status: Option<StatusBanner>,
}

impl Model {
    pub fn set_status(&mut self, level: StatusLevel, text: impl Into<String>) {
        self.status = Some(StatusBanner {
            level,
            text: text.into(),
        });
    }

    pub fn save_graph(&mut self) {
        if let Err(e) = agora_store::save(&self.store_path, &self.graph) {
            self.set_status(StatusLevel::Error, format!("no pude persistir el grafo: {e}"));
        }
    }

    pub fn is_mine(&self, id: IdentityId) -> bool {
        self.seeds.contains_key(&id)
    }

    pub fn signer_keypair(&self) -> Option<Keypair> {
        self.active_signer
            .and_then(|id| self.seeds.get(&id).copied())
            .map(Keypair::from_seed)
    }

    /// El `TextInputState` que recibe las teclas según `focused_input`. Un
    /// solo punto de ruteo para todos los inputs de la pantalla principal.
    pub fn focused_input_mut(&mut self) -> &mut TextInputState {
        match self.focused_input {
            FocusedInput::Compose(ComposeField::Predicate) => &mut self.compose_predicate,
            FocusedInput::Compose(ComposeField::Value) => &mut self.compose_value,
            FocusedInput::MultiMessage => &mut self.multi_message,
            FocusedInput::ReleaseHash => &mut self.release_hash,
            FocusedInput::ReleasePaste => &mut self.release_paste,
            FocusedInput::CapBytecode => &mut self.cap_bytecode,
            FocusedInput::CapPaste => &mut self.cap_paste,
        }
    }

    /// Aplica una tecla al input focado e invalida el artefacto derivado del
    /// input editado (una firma vieja deja de corresponder al texto nuevo).
    pub fn edit_focused(&mut self, ev: &KeyEvent) {
        let focus = self.focused_input;
        self.focused_input_mut().apply_key(ev);
        match focus {
            FocusedInput::MultiMessage => self.multi_current = None,
            FocusedInput::ReleaseHash => self.release_current = None,
            FocusedInput::CapBytecode => self.cap_current = None,
            _ => {}
        }
    }

    /// Intenta desbloquear todas las seeds del keystore con `self.passphrase`.
    /// Sólo guarda las que descifran; el resto se loguea a stderr y se omite.
    pub fn desbloquear_seeds_silencioso(&mut self) {
        let ids = self.keystore.list().unwrap_or_default();
        for id in ids {
            match self.keystore.load(id, &self.passphrase) {
                Ok(seed) => {
                    self.seeds.insert(id, seed);
                }
                Err(e) => eprintln!("agora-app: no pude desbloquear {id}: {e}"),
            }
        }
    }

    /// Versión estricta para la pantalla de unlock: requiere que **todas** las
    /// seeds del keystore descifren contra `passphrase`. `true` si pasó (y deja
    /// `self.seeds` poblada).
    pub fn intentar_unlock(&mut self, passphrase: &str) -> bool {
        let ids = self.keystore.list().unwrap_or_default();
        let mut nuevas = HashMap::new();
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

    /// Si el grafo no registra una identidad mía conocida (p. ej. el archivo
    /// se borró pero el keystore sobrevivió), la registra de nuevo como Person.
    pub fn registrar_identidades_huerfanas(&mut self) {
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

// =============================================================================
//  Mensajes
// =============================================================================

#[derive(Clone)]
pub(crate) enum Msg {
    /// Reordenar tiles por drag.
    SwapTile(usize, usize),

    /// Genera una identidad nueva (seed CSPRNG), la guarda y la registra.
    NuevaIdentidad,
    /// Selecciona el sujeto enfocado (objetivo del próximo claim).
    FocoSujeto(IdentityId),
    /// Cambia el firmante activo (entre identidades mías).
    ActuarComo(IdentityId),
    /// Selecciona una atestación para que la política evalúe su claim.
    SeleccionarAtestacion(usize),

    /// Cambia el input focado (cualquiera de la pantalla principal).
    Foco(FocusedInput),
    /// Tecla aplicada al input focado.
    EditFocused(KeyEvent),
    /// Firma + agrega la atestación con los valores actuales y persiste.
    Atestar,

    /// Drag del slider de `min_third_party`. Acumula el delta.
    SliderMinThird(llimphi_ui::DragPhase, f32),
    /// Toggle de `accept_self`.
    ToggleAcceptSelf,
    /// Cicla `min_attesters_of_kind`: off → Person → … → Institution → off.
    CycleKind,
    /// Drag del slider del N requerido para el kind activo.
    SliderMinKind(llimphi_ui::DragPhase, f32),
    /// Cicla `max_age_secs` por presets: off → 60 → 300 → 3600 → 86400 → 604800 → off.
    CycleMaxAge,

    /// `graph.json` cambió en disco (otro proceso). Recarga el snapshot.
    ArchivoCambio,

    /// Tecla aplicada al input de passphrase en la pantalla de unlock.
    UnlockKey(KeyEvent),
    /// Intenta desbloquear el keystore con la passphrase actual.
    UnlockSubmit,

    /// Cierra el banner de estado de servicio.
    DescartarStatus,

    // ---- Multifirma --------------------------------------------------------
    ToggleMultiFirmante(IdentityId),
    SliderMultiUmbral(llimphi_ui::DragPhase, f32),
    FirmarMulti,
    LimpiarMulti,
    ExportarMulti,

    // ---- Release -----------------------------------------------------------
    /// Firma `release_hash` (parseado de hex) con el firmante activo.
    FirmarRelease,
    /// Verifica el `ManifiestoFirmado` pegado en `release_paste` (postcard hex).
    VerificarRelease,
    /// Serializa `release_current` a postcard y lo presenta en hex en el banner.
    ExportarRelease,
    /// Limpia el estado del tile Release.
    LimpiarRelease,

    // ---- Capacidad ---------------------------------------------------------
    /// Toggle de un bit de `cap_permisos`.
    ToggleCapPermiso(Permisos),
    /// Firma una `ConcesionCapacidad` sobre `(cap_bytecode, cap_permisos)`.
    FirmarCapacidad,
    /// Verifica la `ConcesionCapacidad` pegada en `cap_paste` (postcard hex).
    VerificarCapacidad,
    /// Serializa `cap_current` a postcard y lo presenta en hex en el banner.
    ExportarCapacidad,
    /// Limpia el estado del tile Capacidad.
    LimpiarCapacidad,
}
