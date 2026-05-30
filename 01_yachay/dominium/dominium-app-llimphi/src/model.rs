//! Modelo de la app y mensajes del bucle Elm. La conducta vive en
//! `update`/`view` (en `main.rs`); acá sólo los datos y los enums de edición.

use std::collections::VecDeque;

use dominium_core::{SimParams, World};
use dominium_iso::{IsoProjector, ZWeights};
use dominium_render_plan::PlanConfig;
use llimphi_theme::Theme;
use llimphi_ui::KeyEvent;
use llimphi_widget_text_input::TextInputState;

pub(crate) struct Model {
    pub(crate) world: World,
    pub(crate) params: SimParams,
    pub(crate) iso: IsoProjector,
    pub(crate) weights: ZWeights,
    pub(crate) cfg: PlanConfig,
    pub(crate) running: bool,
    pub(crate) tick: u64,
    pub(crate) epoch: u64,
    pub(crate) rng_seed: u64,
    /// Índice del Concepto seleccionado, si alguno. `None` cuando no hay
    /// selección. Si se "Limpia" la lista se resetea a `None`.
    pub(crate) selected: Option<usize>,
    /// Cuando está activo, editar `ZWeights` (relieve visual) también
    /// escribe a `params.relieve` (relieve físico) — lo que ves es lo
    /// que sienten los lemmings.
    pub(crate) sync_relieve: bool,
    /// Buffer de texto del input de renombre. `id_input_focused` decide
    /// si el panel muestra el text-input o el label estático.
    pub(crate) id_input: TextInputState,
    pub(crate) id_input_focused: bool,
    /// Índice del scenario actual en `scenario_packs()`. El picker del panel
    /// lo cicla; "Sembrar pack" instala el JSON correspondiente.
    pub(crate) scenario_idx: usize,
    /// Ring de snapshots del `World` — el último elemento es el más reciente
    /// ya cosechado. `rewind_offset == 0` significa "presente" y se renderiza
    /// `world`; `> 0` significa "mirar hacia atrás" y se renderiza
    /// `snapshots[len - 1 - offset]` en modo read-only.
    pub(crate) snapshots: VecDeque<World>,
    /// Cuántos pasos hacia atrás está mirando el usuario. `0` = vivo.
    /// Cuando `> 0`, el `Tick` deja de avanzar el mundo (la sim se
    /// auto-pausa visualmente, pero el reloj real podría seguir si se
    /// pidiera). Implementación: pausamos también el motor mientras hay
    /// rewind, así no acumula divergencia.
    pub(crate) rewind_offset: usize,
    /// Trails: para cada lemming vivo, las últimas `TRAIL_CAP` posiciones
    /// `(x, y)`. Como los lemmings se referencian por índice y `swap_remove`
    /// puede mover índices, el trail se reconstruye cada tick desde
    /// `lemmings.pos_x/pos_y` — sólo guardamos las posiciones, no su id.
    /// Estructura: `trails[k]` es el snapshot del frame `tick - k`.
    pub(crate) trails: VecDeque<Vec<(f32, f32)>>,
    /// Toggle para mostrar las trayectorias.
    pub(crate) show_trails: bool,
    /// Theme efectivo. Se construye en init desde `wawa-config` (con
    /// fallback a `Theme::dark()` si no hay archivo aún) y se rearma
    /// en cada `Msg::WawaConfigChanged`.
    pub(crate) theme: Theme,
    /// Subscripción al bus de configuración del SO. `Option` porque
    /// la creación puede fallar en plataformas sin ProjectDirs.
    /// Se mantiene viva mientras vive el `Model`.
    pub(crate) _wawa_watcher: Option<wawa_config::ConfigWatcher>,
    /// Asignación k-means → cluster por lemming. Vacío hasta que se entre
    /// al modo PsiCluster o se ejecute el primer refresh. `assignments[i]`
    /// ∈ `0..KMEANS_K` indica el cluster del lemming `i`. Si la población
    /// cambia entre refrescos (spawn/kill), los índices nuevos caen en `0`
    /// hasta el próximo refresh.
    pub(crate) cluster_assignments: Vec<u8>,
    /// Tick global en el que se calculó por última vez `cluster_assignments`.
    /// Usado para gated refresh cada `KMEANS_REFRESH_TICKS`.
    pub(crate) cluster_last_refresh: u64,
    /// Cuál tab del panel lateral está activo. La UI muestra los grupos
    /// relevantes según esta selección — el modelo es simple, sin lazy load.
    pub(crate) panel_tab: PanelTab,
    /// Si el usuario ya entendió las gestures de canvas (click crea, drag
    /// mueve, segundo click selecciona). Cuando es `false` la app muestra
    /// un hint flotante sobre el canvas. Se apaga al primer click.
    pub(crate) onboarding_done: bool,
}

/// Pestañas del panel lateral. El orden es el orden visual en el tab bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelTab {
    Mundo,
    Conceptos,
    Psique,
    Vista,
}

impl PanelTab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            PanelTab::Mundo => "Mundo",
            PanelTab::Conceptos => "Conceptos",
            PanelTab::Psique => "ψ",
            PanelTab::Vista => "Vista",
        }
    }

    pub(crate) fn all() -> [PanelTab; 4] {
        [
            PanelTab::Mundo,
            PanelTab::Conceptos,
            PanelTab::Psique,
            PanelTab::Vista,
        ]
    }
}

/// Una de las cuatro capas modificables de un `Concepto` (degradacion
/// queda fuera — es cicatriz emergente, no editable).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Layer {
    Materia,
    Psique,
    Poder,
    Oro,
}

/// Slot de `SimParams` editable desde el panel. Los 4 más visibles más los
/// dos del ciclo estacional; los demás quedan al default.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ParamSlot {
    ClimbCost,
    DiffusionRate,
    EntropyRate,
    MoveCost,
    SeasonPeriod,
    SeasonAmplitude,
    /// Intensidad con la que el psi modula los efectos de las acciones.
    PsiModulation,
    /// Radio social del contagio (Fase B).
    SocialRadius,
    /// Tasa de convergencia del contagio social.
    ContagionRate,
    /// Umbral de homofilia (Fase B.2) — 0 = sin filtro.
    HomophilyThreshold,
}

impl ParamSlot {
    pub(crate) fn range(self) -> (f32, f32) {
        match self {
            ParamSlot::ClimbCost => (0.0, 0.5),
            ParamSlot::DiffusionRate => (0.0, 0.5),
            ParamSlot::EntropyRate => (0.0, 0.05),
            ParamSlot::MoveCost => (0.0, 0.5),
            // 0 = sin estaciones; hasta 500 ticks por ciclo (≈45 s a 11 Hz).
            ParamSlot::SeasonPeriod => (0.0, 500.0),
            ParamSlot::SeasonAmplitude => (0.0, 1.0),
            // Psi modulation: rango [0, 1] de uso típico; > 1 amplifica
            // demasiado y rompe calibraciones del default.
            ParamSlot::PsiModulation => (0.0, 1.0),
            // Radio social — hasta media diagonal del grid 80×80.
            ParamSlot::SocialRadius => (0.0, 30.0),
            // Tasa de contagio: > 0.5 produce conformismo brutal en pocos
            // ticks; típicos 0.05..0.20.
            ParamSlot::ContagionRate => (0.0, 0.5),
            // Homofilia 0..2 — > sqrt(4) = 2 incluye todo el psi space.
            ParamSlot::HomophilyThreshold => (0.0, 2.0),
        }
    }
}

/// Capa de `ZWeights` editable desde el panel — define el **relieve
/// visual** (cuánto eleva cada capa el render). Independiente del
/// `relieve` físico de `SimParams`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ZSlot {
    Materia,
    Psique,
    Poder,
    Oro,
    Degradacion,
}

#[derive(Clone)]
pub(crate) enum Msg {
    Tick,
    TogglePlay,
    Reseed,
    LimpiarConceptos,
    SembrarConceptos,
    SelectConcepto(usize),
    DeselectConcepto,
    EditMod(Layer, f32),
    EditRadius(f32),
    DeleteSelected,
    EditParam(ParamSlot, f32),
    EditZWeight(ZSlot, f32),
    GuardarPack,
    CargarPack,
    CrearConcepto,
    /// Click sobre el canvas, en coords de mundo. Si cae sobre un
    /// Concepto existente lo selecciona; si no, crea uno nuevo ahí.
    CanvasClick(f32, f32),
    ToggleSyncRelieve,
    ToggleAndina,
    // Editor de BehaviorHack del Concepto seleccionado.
    HackToggle,         // agrega o quita el hack.
    HackCycleTrigger,   // rota Always → EnergiaBajo → EdadSobre → Always.
    HackCycleAction,    // rota la acción forzada 0..5 → 0...
    HackEditTriggerParam(f32),
    HackEditDuration(f32),
    CycleSprite,
    /// Delta de un Move dentro de un drag activo, en coords de mundo.
    /// Mueve el Concepto seleccionado si hay uno.
    CanvasDragMove(f32, f32),
    FocusIdInput,
    BlurIdInput,
    IdInputKey(KeyEvent),
    /// Cicla al siguiente scenario embebido. Sólo cambia la selección
    /// (no lo aplica hasta que se toque "Cargar scenario").
    CycleScenario,
    /// Reemplaza los conceptos del mundo con el scenario actualmente
    /// seleccionado. Limpia hack_locks vivos y deselecciona.
    LoadScenario,
    /// Cicla `cfg.render_mode`: Composite → Heatmap(Materia) → … →
    /// Heatmap(Degradacion) → Composite.
    CycleRenderMode,
    /// Toggle de visualización de trayectorias.
    ToggleTrails,
    /// Toggle de texturización procedural sobre los techos.
    ToggleTexture,
    /// Delta sobre `rewind_offset` (positivo = más atrás; negativo = hacia
    /// el presente). El slider del panel emite estos deltas; un botón
    /// "vivo" emite `RewindHome`.
    RewindBy(f32),
    /// Vuelve `rewind_offset` a 0 (presente).
    RewindHome,
    /// El bus `wawa-config` publicó una versión nueva. Aplicamos
    /// theme y locale; los demás campos no nos competen.
    WawaConfigChanged(Box<wawa_config::WawaConfig>),
    /// Alterna `big_five` en SimParams. Si la población vino sin columna
    /// `psi5` (saves Big Four), la rellenamos al pasar a Big5.
    ToggleBigFive,
    /// Cicla `ActionPolicy` entre Fixed y PsiArgmax. Con periodo 0 nunca
    /// re-elige, así que también arrancamos un período sano la primera vez.
    CyclePsiPolicy,
    /// Cambia el tab activo del panel lateral.
    SelectTab(PanelTab),
    /// Cierra el hint flotante de onboarding (se cierra solo en el primer
    /// click sobre el canvas, pero también hay una X visible).
    DismissOnboarding,
}
