//! Modelo de la app y mensajes del bucle Elm. La conducta vive en
//! `update`/`view` (en `main.rs`); acá sólo los datos y los enums de edición.

use dominium_control::StabilityController;
use dominium_iso::{IsoProjector, ZWeights};
use dominium_sim::Sim;
use dominium_render_plan::PlanConfig;
use llimphi_theme::Theme;
use llimphi_clipboard::SystemClipboard;
use llimphi_ui::KeyEvent;
use llimphi_widget_edit_menu::EditAction;
use llimphi_widget_text_input::TextInputState;
use llimphi_widget_toast::Toast;

pub(crate) struct Model {
    /// Sesión de simulación (estado de dominio + reloj + historia): `world`,
    /// `params`, `tick/epoch/rng_seed`, ring de snapshots (rewind), trails y
    /// clusters. El ciclo de vida (`advance`/`reseed`/…) vive en `Sim`
    /// (`dominium-sim`); el `Model` sólo guarda estado de vista.
    pub(crate) sim: Sim,
    /// Controlador de estabilidad (lazo cerrado). `Some` = activo: cada tick
    /// mide la población y ajusta una palanca blanda (regrowth) para sostener
    /// `setpoint`. `None` = lazo abierto (la sim corre con los params tal cual
    /// los dejaste). Lo togglea `Msg::ToggleController`.
    pub(crate) controller: Option<StabilityController>,
    /// Setpoint de población recordado (sobrevive al toggle del controlador).
    /// El slider del panel lo edita; al activar el lazo se siembra desde acá.
    pub(crate) setpoint: f32,
    pub(crate) iso: IsoProjector,
    /// Desplazamiento de cámara en píxeles de PANTALLA, sumado al centrado
    /// automático del canvas. `(0.0, 0.0)` = maqueta centrada. Lo mueve el
    /// drag de canvas (cuando no se arrastra un Concepto) y el zoom; el
    /// atajo `R` / botón "Recentrar" lo vuelve a `(0,0)`. La huella de
    /// render lo incluye (si no, panear no invalidaría la caché del plan).
    pub(crate) pan: (f32, f32),
    pub(crate) weights: ZWeights,
    /// Desplazamiento vertical del contenido del panel lateral, en píxeles
    /// (`0.0` = arriba). El panel tiene más secciones de las que entran en la
    /// ventana; la rueda sobre el panel lo desliza. Se clampa por tab (cada
    /// uno tiene distinto alto de contenido) y se resetea al cambiar de tab.
    pub(crate) panel_scroll: f32,
    pub(crate) cfg: PlanConfig,
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
    /// Cuál tab del panel lateral está activo. La UI muestra los grupos
    /// relevantes según esta selección — el modelo es simple, sin lazy load.
    pub(crate) panel_tab: PanelTab,
    /// Si el usuario ya entendió las gestures de canvas (click crea, drag
    /// mueve, segundo click selecciona). Cuando es `false` la app muestra
    /// un hint flotante sobre el canvas. Se apaga al primer click.
    pub(crate) onboarding_done: bool,
    /// Barra de menú principal: índice del menú raíz abierto (`None` cerrado).
    pub(crate) menu_open: Option<usize>,
    /// Fila resaltada por teclado en el menú principal (`usize::MAX` = ninguna).
    pub(crate) menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    pub(crate) menu_anim: llimphi_motion::Tween<f32>,
    /// Menú de edición contextual: ancla `(x, y)` en ventana (`None` cerrado).
    /// Opera sobre el editor del campo de texto focuseado (`id_input`).
    pub(crate) edit_menu: Option<(f32, f32)>,
    /// Fila resaltada por teclado en el menú de edición (`usize::MAX` = ninguna).
    pub(crate) edit_active: usize,
    /// Animación de aparición del menú de edición (0→1).
    pub(crate) edit_anim: llimphi_motion::Tween<f32>,
    /// Clipboard del sistema, compartido por el menú de edición y el
    /// text-input de renombre.
    pub(crate) clipboard: SystemClipboard,
    /// Caché del `RenderPlan` con memoización por huella. `view()` recibe
    /// `&Model`, así que la reconstrucción perezosa se hace por interior
    /// mutability: si la huella del estado de render (tick de sim, conceptos,
    /// weights, cfg, modo) no cambió desde el último frame, se reusa el
    /// `Arc<RenderPlan>` cacheado en vez de re-iterar las 57 600 celdas y
    /// re-emitir ~115 k polígonos. A grid 240 esto baja el costo de
    /// `build_plan` de ~14 ms/frame a ~0 en frames sin cambio (sim pausada,
    /// sólo UI). El `Arc` hace que el clon por frame sea barato.
    pub(crate) plan_cache: std::cell::RefCell<
        Option<(u64, std::sync::Arc<dominium_render_plan::RenderPlan>)>,
    >,
    /// Si el lienzo muestra la **vista voxel 3D** (`true`) o la maqueta iso 2D
    /// (`false`, default). Lo togglea `Msg::Toggle3D` / el menú Ver / el panel.
    pub(crate) mode3d: bool,
    /// Cámara orbital de la vista 3D: yaw/pitch (radianes) + distancia (voxels).
    /// El drag sobre el lienzo 3D la orbita; la rueda hace zoom.
    pub(crate) cam3d_yaw: f32,
    pub(crate) cam3d_pitch: f32,
    pub(crate) cam3d_dist: f32,
    /// Estado GPU persistente de la vista 3D (renderer ray-march + grilla voxel).
    /// Vive en `Arc<Mutex<_>>` porque el `VoxelRenderer` no es clonable y el
    /// `Model` sí se mueve/clona; la closure `gpu_paint_with` captura este Arc.
    pub(crate) view3d: std::sync::Arc<std::sync::Mutex<crate::view3d::View3d>>,
    /// Toasts efímeros vivos (confirmaciones de guardar/cargar pack/scenario).
    /// Cada uno se auto-descarta con `Msg::ToastExpire` tras su `duration`.
    pub(crate) toasts: Vec<Toast>,
    /// Id incremental para correlacionar un toast con su `Msg::ToastExpire`.
    pub(crate) next_toast: u64,
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

    /// Tope de scroll del panel para este tab, en píxeles. Aproxima
    /// `alto_de_contenido − alto_visible`; generoso a propósito (mejor un
    /// pelín de vacío al final que no poder llegar al fondo). Mundo es el más
    /// largo (todas las secciones de params); Vista el más corto.
    pub(crate) fn max_scroll(self) -> f32 {
        match self {
            PanelTab::Mundo => 1700.0,
            PanelTab::Conceptos => 1000.0,
            PanelTab::Psique => 900.0,
            PanelTab::Vista => 500.0,
        }
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
    // — Economía: los levers termodinámicos del flujo de energía. Deciden
    //   si una población crece, se estabiliza o colapsa. Hasta ahora
    //   hardcoded; expuestos para que el escenario guardado los capture.
    /// Cantidad de materia que `Extraer` drena del suelo por acción.
    ExtractRate,
    /// Energía transferida por `Intercambiar`.
    TradeAmount,
    /// Fracción del espacio libre que la naturaleza repuebla por tick.
    RegrowthRate,
    /// Asíntota del regrowth: techo de materia por celda.
    CarryingCapacity,
    /// Drenaje basal de energía por tick a todo lemming vivo.
    MetabolicCost,
    /// Umbral de energía para que `Replicar` dispare.
    ReplicateThreshold,
    /// Umbral de abundancia por encima del cual el agente se fuerza a
    /// `Replicar` (0 = desactiva la transición).
    AbundanceThreshold,
    // — Cinética fina: los escalares de cada acción atómica y del ciclo de
    //   vida. Más quirúrgicos que la economía; tunean la "sensación" del
    //   motor sin redefinir su balance macro.
    /// Celdas por tick que avanza `Mover`.
    MoveSpeed,
    /// Tasa de convergencia del `vector_psi` en `Sincronizar` (0-1).
    SyncRate,
    /// Degradación añadida al suelo por cada `Extraer`.
    DegrPerExtract,
    /// Fracción de la energía del padre que hereda el hijo en `Replicar`.
    ChildEnergyFrac,
    /// Daño de energía que inflige `Degradar`.
    FightDamage,
    /// Fracción del daño que el atacante absorbe como energía.
    AbsorbFrac,
    /// Umbral de energía bajo el cual el agente se fuerza a `Degradar`.
    DesperationThreshold,
    /// Edad máxima; al superarla el agente muere (entero, en ticks).
    MaxEdad,
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
            // Economía — rangos calibrados alrededor de los defaults del
            // motor (ver `SimParams::default` / overrides de init).
            ParamSlot::ExtractRate => (0.0, 6.0),
            ParamSlot::TradeAmount => (0.0, 5.0),
            ParamSlot::RegrowthRate => (0.0, 0.1),
            ParamSlot::CarryingCapacity => (0.0, 100.0),
            ParamSlot::MetabolicCost => (0.0, 0.5),
            ParamSlot::ReplicateThreshold => (0.0, 100.0),
            ParamSlot::AbundanceThreshold => (0.0, 150.0),
            // Cinética fina — rangos alrededor de los defaults del motor.
            ParamSlot::MoveSpeed => (0.0, 4.0),
            ParamSlot::SyncRate => (0.0, 1.0),
            ParamSlot::DegrPerExtract => (0.0, 0.2),
            ParamSlot::ChildEnergyFrac => (0.0, 1.0),
            ParamSlot::FightDamage => (0.0, 20.0),
            ParamSlot::AbsorbFrac => (0.0, 1.0),
            ParamSlot::DesperationThreshold => (0.0, 30.0),
            // Edad máxima en ticks — 0 = inmortal (cuidado: sin cosecha por
            // vejez la población sólo cae por desesperación/metabolismo).
            ParamSlot::MaxEdad => (0.0, 20000.0),
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
    /// Activa/desactiva el controlador de estabilidad (lazo cerrado). Al
    /// activar, crea un `StabilityController` con el `setpoint` recordado.
    ToggleController,
    /// Delta sobre el `setpoint` de población (lo emite el slider del panel).
    /// Si el controlador está activo, lo sincroniza al instante.
    EditSetpoint(f32),
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
    /// Pan de cámara: delta en píxeles de PANTALLA. Lo emite el drag de
    /// canvas cuando NO se está arrastrando un Concepto (clic en vacío).
    /// Suma directo a `model.pan` (sin unproject — el pan es de pantalla).
    CanvasPan(f32, f32),
    /// Zoom de cámara por rueda. `delta` > 0 = acercar (notches hacia
    /// arriba), < 0 = alejar. Multiplica `iso.scale` por `ZOOM_STEP^delta`,
    /// clampeado a `[ZOOM_MIN, ZOOM_MAX]`. Zoom centrado (no focal): la
    /// rueda de `on_scroll` no entrega la posición del cursor.
    CanvasZoom(f32),
    /// Recentra la cámara: `pan = (0,0)` y `iso.scale`/`z_factor` al default.
    /// Atajo `R` y botón "Recentrar" en el tab Vista.
    ResetCamera,
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
    /// Rueda sobre el panel lateral → desliza su contenido. `dy > 0` baja
    /// (contenido sube). Se acumula y clampa en `Model::panel_scroll`.
    PanelScroll(f32),
    /// Alterna entre la maqueta iso 2D y la vista voxel 3D del mundo. Al
    /// activar el 3D, re-voxeliza el mundo actual.
    Toggle3D,
    /// Drag sobre el lienzo 3D → orbita la cámara (dx, dy en píxeles).
    Orbit3D(f32, f32),
    /// Rueda sobre el lienzo 3D → zoom de la cámara orbital.
    Zoom3D(f32),
    /// Cierra el hint flotante de onboarding (se cierra solo en el primer
    /// click sobre el canvas, pero también hay una X visible).
    DismissOnboarding,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
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
    /// Right-click en la ventana → abre el menú de edición en `(x, y)`,
    /// operando sobre el campo de texto focuseado (`id_input`).
    EditMenuOpen(f32, f32),
    /// Acción elegida en el menú de edición (sobre `id_input`).
    EditMenuAction(EditAction),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Un toast cumplió su `duration`: se descarta del stack.
    ToastExpire(u64),
}
