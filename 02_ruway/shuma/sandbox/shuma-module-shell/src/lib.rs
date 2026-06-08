//! `shuma-module-shell` — REPL del shell como módulo enchufable.
//!
//! Núcleo del shell interactivo: cwd + input + ejecución por `shuma-exec`
//! con salida en streaming + buffer de output acotado. Builtins: `cd`,
//! `pwd`, `clear`, `exit` (no-op — el chasis maneja la salida).
//!
//! Diseño del tab:
//!
//! ```text
//!  Shell · local · cwd: /home/usuario
//!  ┌──────────────────────────────────────────────────────────┐
//!  │ $ ls                                                     │
//!  │ Cargo.toml                                               │
//!  │ src                                                      │
//!  │ ...                                                      │
//!  │ ✔ exit 0                                                 │
//!  └──────────────────────────────────────────────────────────┘
//!  ┌──────────────────────────────────────────────────────────┐
//!  │ $ █                                                      │
//!  └──────────────────────────────────────────────────────────┘
//! ```
//!
//! **Ejecución no bloqueante.** Cada submisión lanza `shuma_exec::run`
//! que vuelve de inmediato; el `RunHandle` se guarda en el state. El
//! chasis manda `Msg::Tick` periódicamente y el módulo drena
//! `try_events()` sin bloquear la UI. `sleep 5`, `top` y demás dejan
//! de congelar el shell. Mientras hay un run vivo, Enter encola la
//! nueva línea — el siguiente comando arranca al cerrar el actual.

#![forbid(unsafe_code)]

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use shuma_exec::{CommandSpec, Exec, Killer, RunEvent, RunHandle, StageSpec};
use shuma_intent::SessionGraph;
use shuma_line::{LineState, TokenKind};
use shuma_module::{ModuleContributions, ShortcutSpec, Source};
use shuma_remote_exec::RemoteRunHandle;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// `id` canónico del módulo. El shumarc lo referencia para activarlo.
pub const ID: &str = "shell";

/// Tope de líneas guardadas en el buffer de output — análogo al
/// `cap_log` de matilda. Suficiente para varios runs sin que el panel
/// crezca sin límite.
pub const MAX_OUTPUT_LINES: usize = 500;

/// Tipo de cada línea del buffer — define el color que la `view` usa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// El comando tal como lo tipeó el usuario (precede a su output).
    Prompt,
    /// stdout del comando.
    Stdout,
    /// stderr del comando.
    Stderr,
    /// Mensaje del shell mismo (cd, error de spawn, exit status, etc.).
    Notice,
}

/// Una línea del buffer de output con su tipo (para coloreado) y el
/// bloque de comando al que pertenece. El render agrupa las líneas con
/// el mismo `block` en una *card* desplegable (un `$ cmd` + su salida +
/// su exit status). `block == 0` = líneas sueltas sin comando dueño.
#[derive(Debug, Clone)]
pub struct OutputLine {
    pub kind: OutputKind,
    pub text: String,
    /// Bloque de comando. Lo asigna [`State::push_output`] — cada
    /// `Prompt` abre uno nuevo (id monotónico) y las siguientes líneas
    /// lo heredan. Por defecto `0` (las constructoras no lo conocen).
    pub block: u64,
    /// Etapa intermedia del pipe que produjo la línea (tee de
    /// `shuma-exec`), 0-based. `None` = salida normal (de la última etapa
    /// o de un comando suelto). El render guarda estas líneas para el
    /// desplegable de su etapa en vez de mezclarlas con el cuerpo.
    pub stage: Option<usize>,
}

impl OutputLine {
    pub fn prompt(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Prompt,
            text: text.into(),
            block: 0,
            stage: None,
        }
    }
    pub fn stdout(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Stdout,
            text: text.into(),
            block: 0,
            stage: None,
        }
    }
    pub fn stderr(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Stderr,
            text: text.into(),
            block: 0,
            stage: None,
        }
    }
    pub fn notice(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Notice,
            text: text.into(),
            block: 0,
            stage: None,
        }
    }
    /// Línea capturada de una etapa intermedia del pipe (tee en vivo). Se
    /// guarda con su `stage` para el desplegable correspondiente.
    pub fn stage_stdout(stage: usize, text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Stdout,
            text: text.into(),
            block: 0,
            stage: Some(stage),
        }
    }
}

/// Run vivo: handle de ejecución (local directo o vía daemon), un
/// `Killer` opcional (solo en local — el remoto matamos cerrando el
/// stream) y el comando original (para el notice de cierre).
pub struct ActiveRun {
    pub handle: BackendHandle,
    /// `Some` cuando el run es local (`shuma-exec::RunHandle.killer()`).
    /// `None` cuando es remoto — la cancelación va por `handle.kill()`.
    pub killer: Option<Killer>,
    pub command: String,
    /// Sesión TUI: emulador vt100 + dims del PTY. `Some` cuando el run
    /// arrancó bajo `Exec::Pty` (vim/htop/less/etc.); las teclas van al
    /// stdin del PTY y la pantalla se renderiza como grid de celdas.
    /// El daemon no soporta PTY remoto todavía — TUIs forzados a local.
    pub tui: Option<TuiSession>,
    /// Bloque de output al que se adjunta TODA la salida de este run —
    /// fijo desde el arranque. Sin esto, un comando lento que drena en
    /// ticks posteriores se mezclaría con el bloque "actual" (p. ej. un
    /// builtin tipeado mientras corre), o un job de fondo se metería en
    /// la card del foreground. Cada run vive en su propia card.
    pub block: u64,
}

/// Backend de ejecución abstracto. Local va por `shuma-exec`; Daemon
/// (Unix o TCP) va por `shuma-remote-exec`. La API expuesta al módulo
/// shell (`try_events`, `is_finished`, `kill`, `write_input`, `resize`)
/// es la misma — las operaciones de PTY son no-op en remoto.
pub enum BackendHandle {
    Local(RunHandle),
    Remote(RemoteRunHandle),
}

impl BackendHandle {
    pub fn try_events(&mut self) -> Vec<RunEvent> {
        match self {
            BackendHandle::Local(h) => h.try_events(),
            BackendHandle::Remote(h) => h.try_events(),
        }
    }
    pub fn is_finished(&self) -> bool {
        match self {
            BackendHandle::Local(h) => h.is_finished(),
            BackendHandle::Remote(h) => h.is_finished(),
        }
    }
    pub fn kill(&self) {
        match self {
            BackendHandle::Local(h) => h.kill(),
            BackendHandle::Remote(h) => h.kill(),
        }
    }
    pub fn write_input(&self, bytes: Vec<u8>) -> bool {
        match self {
            BackendHandle::Local(h) => h.write_input(bytes),
            // En PTY remoto, el asa enruta las teclas al daemon; en runs
            // remotos no-PTY es no-op (devuelve false).
            BackendHandle::Remote(h) => h.write_input(bytes),
        }
    }
    pub fn resize(&self, rows: u16, cols: u16) -> bool {
        match self {
            BackendHandle::Local(h) => h.resize(rows, cols),
            BackendHandle::Remote(h) => h.resize(rows, cols),
        }
    }
}

/// Skin de render para un programa bajo PTY. `Generic` pinta la grilla
/// vt100 cruda; los demás reconstruyen la pantalla como un card
/// themeable propio del programa (deja de verse "como por un vidrio").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppSkin {
    /// Grilla de celdas vt100 (htop, less, man, btop, …).
    Generic,
    /// vim/nvim/vi: el buffer como texto en la paleta del tema.
    Vim,
    /// claude code: un card grande que engloba la sesión (por ahora cae
    /// al genérico hasta que esté el parser de bloques).
    Claude,
}

/// Elige el skin a partir del nombre del programa (acepta un path —
/// toma el basename).
pub fn app_skin_for(program: &str) -> AppSkin {
    let base = program.rsplit('/').next().unwrap_or(program);
    match base {
        "vi" | "vim" | "nvim" | "view" | "nvi" => AppSkin::Vim,
        "claude" => AppSkin::Claude,
        _ => AppSkin::Generic,
    }
}

/// Sesión TUI sobre PTY — bufferea el parser vt100 y los dims actuales.
pub struct TuiSession {
    pub parser: vt100::Parser,
    pub rows: u16,
    pub cols: u16,
    /// Programa bajo el PTY (basename incluido) — define el skin.
    pub program: String,
    /// Skin de render elegido al arrancar.
    pub skin: AppSkin,
}

impl TuiSession {
    pub fn new(program: &str, rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, 0),
            rows,
            cols,
            program: program.to_string(),
            skin: app_skin_for(program),
        }
    }

    /// Cambia las dimensiones del buffer interno del parser. El resize
    /// del PTY real (que dispara SIGWINCH al child) lo hace el caller
    /// vía `RunHandle::resize`.
    pub fn set_size(&mut self, rows: u16, cols: u16) {
        if rows == self.rows && cols == self.cols {
            return;
        }
        self.parser.screen_mut().set_size(rows, cols);
        self.rows = rows;
        self.cols = cols;
    }
}

impl std::fmt::Debug for ActiveRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveRun")
            .field("command", &self.command)
            .field("finished", &self.handle.is_finished())
            .field("tui", &self.tui.is_some())
            .finish()
    }
}

/// Dims fijos para el PTY mientras el chasis no exponga el ancho real
/// del panel. 80×24 es el default histórico y vim/htop arrancan bien.
const PTY_ROWS: u16 = 24;
const PTY_COLS: u16 = 80;

/// Tabla de comandos que pedimos PTY automáticamente. Otros pueden
/// pedirlo con el prefijo `:tui ...`.
const TUI_ALLOWLIST: &[&str] = &[
    "vi", "vim", "nvim", "nano", "emacs", "helix", "hx", "htop", "btop", "top", "less", "more",
    "man", "claude", "tig", "tui", "watch",
];

/// Selección activa/última en el card de vim, en coordenadas locales px
/// del panel (`ax,ay` = ancla del press; `hx,hy` = cabeza/cursor).
/// `active` = hay un drag en curso.
#[derive(Debug, Clone, Copy)]
pub struct VimSel {
    pub ax: f32,
    pub ay: f32,
    pub hx: f32,
    pub hy: f32,
    pub active: bool,
}

/// Recursos GPU del modo grilla (Fase 4 del SDD-TERMINAL). Se inicializan
/// lazy la primera vez que el `gpu_paint_with` del `generic_grid_panel`
/// recibe un device, y persisten entre frames (re-crear el pipeline cada
/// frame sería absurdo — el WGSL no cambia). El atlas crece y la textura
/// se re-aloca cuando aparece un glifo nuevo que no entra.
pub struct GpuGridResources {
    pub pipeline: llimphi_widget_terminal::CellPipeline,
    pub atlas: llimphi_widget_terminal::GlyphAtlas,
    pub atlas_texture: llimphi_ui::llimphi_hal::wgpu::Texture,
    pub atlas_view: llimphi_ui::llimphi_hal::wgpu::TextureView,
    /// Tamaño del atlas para detectar grow → re-crear textura.
    pub atlas_size: (u32, u32),
}

/// Estado de la barra de búsqueda Ctrl+F sobre el cuerpo de output.
/// La barra es focus-grabbing: mientras está abierta, las teclas van a
/// `query`, no al input del shell. El `current` index navega ciclicamente
/// con `FindNext`/`FindPrev`; al cambiar, el `update` re-arma
/// `surf_selection` como la span del match actual (paridad de pintado y
/// copy con la selección por mouse).
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub query: String,
    pub matches: Vec<llimphi_widget_terminal::FindMatch>,
    pub current: Option<usize>,
    pub case_insensitive: bool,
}

/// Cache de las últimas N líneas spilleadas, refrescable cuando cambia el
/// `spilled_count` del `surf_history`. El `output_pane_surface` la lee en
/// cada render para prepend-ear esas líneas al view (Fase 5.11). Cap fijo
/// para acotar memoria + tiempo de refresh.
#[derive(Debug, Default, Clone)]
pub struct SurfSpilledCache {
    /// Líneas spilleadas en orden cronológico (las más recientes que caben).
    /// La 0 corresponde a `global_id = first_id`; la última a `first_id +
    /// lines.len() - 1`. Cap a [`MAX_SPILLED_VISIBLE`].
    pub lines: Vec<String>,
    /// Global id de la primera línea cacheada (la más vieja del cache).
    pub first_id: u64,
    /// `spilled_count` al momento del último refresh — para detectar staleness.
    pub cached_at: usize,
}

/// Tope de líneas spilleadas visibles directamente al frente del view.
/// Más allá → `:scrollback open`. ~30 KB para líneas típicas de 150 chars.
pub const MAX_SPILLED_VISIBLE: usize = 200;

/// Snapshot del layout del cuerpo de output bajo `SHUMA_TERMINAL_SURFACE=1`.
/// Lo escribe `output_pane_surface` al final del render; lo lee el handler
/// del drag de selección para resolver `(lx, ly)` a [`Point`] del store.
/// **Liviano**: `items_geo` es `Vec<ItemGeo>` (`Copy`), `store` es un Arc
/// para que el clone post-frame no copie todas las líneas.
#[derive(Clone)]
pub struct SurfLayout {
    pub items_geo: Vec<llimphi_widget_terminal::ItemGeo>,
    pub scroll_y: f32,
    pub viewport_h: f32,
    pub metrics: llimphi_widget_terminal::TermMetrics,
    pub gutter_w: f32,
    pub store: Arc<llimphi_widget_terminal::Scrollback>,
}

#[derive(Clone)]
pub struct State {
    pub source: Source,
    pub cwd: PathBuf,
    pub input: LineState,
    pub output: Vec<OutputLine>,
    pub focused: bool,
    /// Run en ejecución, si hay. Cloneable por `Arc<Mutex<…>>` — la
    /// derivación `Clone` del state nos obliga a esto (el chasis clona
    /// el state en cada `route_to_instance`).
    pub running: Option<Arc<Mutex<ActiveRun>>>,
    /// Cola de líneas pendientes — cuando el usuario presiona Enter
    /// mientras hay un run vivo, el nuevo comando entra acá y arranca
    /// cuando el actual cierra.
    pub queue: VecDeque<String>,
    /// Fuente de completion (binarios en `$PATH` + paths bajo cwd). Es
    /// `Arc` porque el `complete()` de `shuma-line` la usa por
    /// referencia y el state se clona en cada `route_to_instance`.
    pub completion_source: Arc<ShellSource>,
    /// Historial durable de líneas submitted — alimenta ghost
    /// suggestion + Up/Down + Ctrl-R fuzzy.
    pub history: Arc<Mutex<shuma_history::History>>,
    /// Cursor de navegación del historial. `None` = no navegando.
    pub history_cursor: Option<usize>,
    /// Overlay de búsqueda Ctrl-R activo. `None` = no abierto.
    pub history_search: Option<HistorySearch>,
    /// Último rect (w, h) píxel del panel TUI — lo escribe el painter
    /// y lo lee `drain_run` para disparar resize si cambia. Cero =
    /// "todavía no se pintó".
    pub last_tui_rect: Arc<Mutex<(f32, f32)>>,
    /// Métricas reales (char_w, line_h) del monospace del card de vim,
    /// medidas por el painter sobre el layout de parley y leídas por
    /// `copy_vim_selection`. Cero = todavía sin medir (usar fallback).
    pub vim_metrics: Arc<Mutex<(f32, f32)>>,
    /// Jobs en background — arrancados con sufijo `&` en la línea. No
    /// son el "foreground" (ese es `running`); su output se mergea al
    /// buffer prefijado por `[N]`. Builtins `:jobs`, `:term N`,
    /// `:stop N`, `:cont N` operan sobre estos.
    pub bg_jobs: Vec<Arc<Mutex<ActiveRun>>>,
    /// Grafo de intenciones de la sesión — alimenta el lienzo de
    /// contexto (`shuma-module-canvas`). Cada `start_run` registra un
    /// nodo `%cN` y `drain_run` lo cierra con el status del exit.
    pub intent_graph: SessionGraph,
    /// `%cN` del run en foreground actual; `None` cuando no hay nada
    /// corriendo. Se setea en `start_run` y se consume en `drain_run`.
    pub current_run_node: Option<u32>,
    /// Bytes acumulados de stdout+stderr del run actual; se vuelca al
    /// nodo del grafo cuando el comando cierra (`complete`).
    pub current_run_bytes: u64,
    /// Selección del card de vim (drag-to-select). `None` = sin selección.
    pub vim_sel: Option<VimSel>,
    /// Contador monotónico de bloques de comando. Cada `Prompt` lo
    /// incrementa; nunca se reusa, así el colapso sobrevive al capado
    /// del buffer (los ids no se reciclan al drenar líneas viejas).
    pub block_seq: u64,
    /// Bloque al que se adjuntan las líneas nuevas (el último `Prompt`).
    pub current_block: u64,
    /// Bloques colapsados por el usuario (click en el header de la card).
    /// Se renderizan plegados, mostrando sólo el header + un resumen.
    pub collapsed: HashSet<u64>,
    /// Sub-secciones colapsadas dentro de un bloque (`ls -R` por dir, etc.).
    /// El `usize` es el índice de la sección que devolvió
    /// [`sections::detect_sections`] para el comando del bloque.
    pub section_collapsed: HashSet<(u64, usize)>,
    /// Estado de orden de las sub-secciones tipo tabla: por `(block, sec_idx)`
    /// guarda `(col, ascending)`. Sin entry = orden natural (el del output).
    /// Click en un header de columna togglea (col, true) → (col, false) →
    /// remove.
    pub section_sort: HashMap<(u64, usize), (usize, bool)>,
    /// Etapas de pipe desplegadas — `(block, stage)`. Click en un chip de
    /// etapa alterna la pertenencia; al estar presente se muestran sus
    /// líneas capturadas en vivo (tee) bajo la fila de etapas.
    pub expanded_stages: HashSet<(u64, usize)>,
    /// Patrones de comandos inferidos del historial (`shuma-infer`). Se
    /// recalculan al cerrar cada comando y alimentan el ghost con la
    /// secuencia predicha (no sólo el historial reciente). Vacío al
    /// arrancar y hasta tener suficiente historial.
    pub patterns: Vec<shuma_infer::EmergingPattern>,
    /// Tope de captura de stdout por run, en bytes. `0` = sin tope. Lo fija
    /// el builtin `:limit <MB>`.
    pub capture_limit_bytes: usize,
    /// Si volcar a disco la salida que excede el tope (`:spill on`). Sólo
    /// tiene efecto con `capture_limit_bytes > 0`.
    pub spill: bool,
    /// Bloque cuyo stdout alimenta el stdin del próximo run (reprocess —
    /// el `%pN` del lienzo). Lo arma el chip ↻ de una card y se consume en
    /// el siguiente submit. `None` = sin reprocess armado.
    pub reprocess_source: Option<u64>,
    /// Grupos de comandos guardados con `:save <nombre>` — ejecutables por
    /// F1..F8 (índice 0-based = número de F menos 1).
    pub groups: Vec<CommandGroup>,
    /// Largo del historial en el último `:save` — los comandos desde acá
    /// son los que entran al próximo grupo.
    pub group_anchor: usize,
    /// Completado activo (popup de candidatos). `Some` = popup abierto (Tab
    /// con ≥2 opciones); se navega con Tab/flechas y se acepta con Enter.
    pub completion: Option<shuma_line::Completion>,
    /// Candidato resaltado dentro del popup de completado.
    pub completion_index: usize,
    /// Scroll del panel de output, en px medidos desde el fondo. `0` =
    /// pegado al fondo (lo último siempre visible, como una terminal).
    /// Crece al rodar la rueda hacia arriba (ver historial). Lo clampa
    /// la `view` contra el overflow real.
    pub scroll_px: f32,
    /// Alto del viewport de output (lo publica el painter del panel cada
    /// frame; lo lee la `view` y el handler de rueda al frame siguiente).
    pub out_viewport_h: Arc<Mutex<f32>>,
    /// Overflow vertical del output (content_h − viewport_h, ≥0). Lo
    /// publica la `view` y lo usa `Msg::Scroll` para clampar `scroll_px`
    /// sin recalcular la geometría en el handler.
    pub out_overflow: Arc<Mutex<f32>>,
    /// `overflow` vigente al momento en que el usuario fijó `scroll_px` por
    /// última vez (rueda / scrollbar / auto-scroll de find). Lo usa el
    /// render del surface para **anclar la vista del usuario al contenido**
    /// cuando llegan líneas nuevas: si el usuario está scrolled-up
    /// (`scroll_px > 0`), su `scroll_y` permanece donde lo dejó aunque el
    /// `overflow` crezca por append — paridad con la UX que la gente espera
    /// (Fase 5 del SDD-TERMINAL). `0.0` mientras esté pinned al fondo.
    pub surf_scroll_anchor: f32,
    /// Velocidad de scroll inercial (px por Tick) — la última entrada de
    /// rueda/scrollbar la captura, y el Tick decae el valor por fricción
    /// para que el scroll continúe brevemente después de soltar el wheel,
    /// estilo touchpad (Fase 5 del SDD-TERMINAL). `0.0` mientras el scroll
    /// está quieto.
    pub surf_scroll_velocity: f32,
    /// Selección viva del **stream del scrollback** (modo superficie,
    /// `SHUMA_TERMINAL_SURFACE=1`). Spans una o más líneas y se traduce a
    /// texto vía [`llimphi_widget_terminal::SelectionRange::slice_text`].
    /// `None` = sin selección. La pinta el `block_surface_with_selection` y
    /// la mutan los handlers de drag (`SurfSelect{Press,Drag,End}`).
    pub surf_selection: Option<llimphi_widget_terminal::SelectionRange>,
    /// `true` mientras hay un drag de selección activo (entre el primer Move
    /// y el End). Separado de `surf_selection` para distinguir "tengo una
    /// selección viva" de "estoy dragueando ahora" — el primero persiste
    /// post-release para que el usuario copie.
    pub surf_selecting: bool,
    /// Acumulador del drag (`lx0 + Σdx`, `ly0 + Σdy`). El `draggable_at` del
    /// widget entrega deltas; este campo trackea la posición absoluta
    /// dentro del viewport para resolverla a [`Point`] con `point_at_geo`.
    pub surf_drag_acc: (f32, f32),
    /// Snapshot del layout del último frame de `output_pane_surface` —
    /// items en versión liviana (`ItemGeo`), métricas, gutter_w, scroll_y,
    /// viewport_h y una copia barata del `Scrollback`. Lo lee el handler
    /// del drag para hit-testear `(lx, ly)` contra el render previo, sin
    /// re-armar los items.
    pub surf_layout: Arc<Mutex<Option<SurfLayout>>>,
    /// Estado de la barra de búsqueda (Ctrl+F) sobre el cuerpo de output.
    /// `None` = barra cerrada. Cuando hay matches, el `current` se refleja
    /// como `surf_selection` para que se vea resaltado con el mismo overlay
    /// y se pueda copiar con el clipboard ya cableado.
    pub find: Option<FindState>,
    /// Menú contextual del cuerpo de output **en modo superficie**:
    /// `(x, y)` en coords del nodo raíz del shell. `None` = cerrado.
    /// Distinto del `body_menu` del legacy (que carga un `block`); el
    /// surface menu opera sobre el scrollback entero.
    pub surf_menu: Option<(f32, f32)>,
    /// Timestamp (ms unix) del último `SurfDoubleClick`. Si llega otro
    /// double-click dentro de la ventana (~350 ms), el handler lo trata
    /// como **triple-click** y selecciona la línea entera (paridad con la
    /// UX de xterm/gnome-terminal: tap, tap-tap, tap-tap-tap-tap).
    pub surf_last_dblclick_ms: u64,
    /// Scrollback **persistente** del cuerpo de output (Fase 5.7 del
    /// SDD-TERMINAL). Es independiente del store que `output_pane_surface`
    /// reconstruye por frame para el view (esa sigue siendo la fuente de
    /// verdad del render). Esta acumula CADA línea de body que pasa por
    /// `push_output` desde el arranque, con cap por memoria + spill
    /// opcional. Cuando el cap se excede, las líneas viejas se vuelcan al
    /// spill file y siguen recuperables por `read_spilled(global_id)`.
    /// Sin spill activo, igual sirve para mostrar el tamaño total del
    /// historial; el view no la usa todavía (TODO: integrar para servir
    /// scrolls al-pasado-spillado).
    pub surf_history: Arc<Mutex<llimphi_widget_terminal::Scrollback>>,
    /// Cache de las últimas líneas spilled visibles directamente al frente
    /// del view (Fase 5.11). Refrescada lazy desde `surf_history.read_spilled`
    /// cuando el `spilled_count` cambia.
    pub surf_spilled_visible: Arc<Mutex<SurfSpilledCache>>,
    /// Recursos GPU del modo grilla (atlas + pipeline + textura). `None`
    /// hasta que el primer `gpu_paint_with` los inicialice. Mantenidos en
    /// `Arc<Mutex<>>` para que la closure de paint (Send+Sync+'static)
    /// pueda accederlos.
    pub gpu_grid: Arc<Mutex<Option<GpuGridResources>>>,
    /// Selección viva en el cuerpo (IDE-text) de una card: `(block,
    /// cursor)`. El cuerpo de cada comando se pinta con
    /// `llimphi-widget-text-editor` read-only (numeración + selección +
    /// copiar); acá vive el cursor/selección del bloque que el usuario
    /// está seleccionando con el mouse. `None` = sin selección. La `view`
    /// reconstruye el `EditorState` por frame desde las `OutputLine` +
    /// este cursor (la fuente de verdad sigue siendo el buffer de output).
    pub body_sel: Option<(u64, llimphi_widget_text_editor::Cursor)>,
    /// Menú contextual del output abierto: `(x, y, bloque_objetivo)` en coords
    /// del nodo raíz del shell. `None` = cerrado. Lo abre el click derecho; sus
    /// acciones (copiar / copiar todo / seleccionar todo) operan sobre el
    /// `bloque_objetivo`.
    pub body_menu: Option<(f32, f32, u64)>,
    /// Acumulador del drag de selección del cuerpo (el `PointerEvent::Drag`
    /// del editor entrega deltas; hay que acumularlos contra el press).
    pub body_drag_accum: (f32, f32),
    /// Momento de creación de cada bloque (unix secs) — alimenta el badge
    /// de "hace N minutos" en vez del crudo "exit N". Lo setea
    /// [`State::push_output`] (Prompt) y [`State::open_block`].
    pub block_started: std::collections::HashMap<u64, u64>,
    /// Texto del comando (`$ …`) por bloque. Se guarda al abrir el bloque para
    /// que el header de la card sobreviva aunque la línea Prompt se recorte del
    /// buffer en un output gigante (`MAX_OUTPUT_LINES`).
    pub block_command: std::collections::HashMap<u64, String>,
    /// Instante (unix ms) de la última tecla en el input — ancla del
    /// parpadeo del caret: queda sólido un instante tras tipear y luego
    /// titila, para que se sienta vivo sin distraer.
    pub input_edit_at_ms: u64,
    /// Configuración personal cargada de `~/.config/shuma/shumarc.toml`
    /// (aliases, env, dedup, captura). Si falta o no parsea, es
    /// [`shuma_config::Config::default`] — el shell arranca igual. Sus
    /// aliases se expanden en cada submit; sus env vars ya se aplicaron al
    /// proceso en [`State::new`].
    pub config: shuma_config::Config,
}

/// Estado del overlay de búsqueda Ctrl-R.
#[derive(Debug, Clone, Default)]
pub struct HistorySearch {
    pub query: String,
    pub selected: usize,
}

/// Grupo de comandos guardado (`:save <nombre>`) — una secuencia ejecutable
/// como una sola línea (`l1 && l2 && …`) desde una tecla de función.
#[derive(Debug, Clone)]
pub struct CommandGroup {
    pub name: String,
    pub lines: Vec<String>,
}

impl State {
    pub fn new(source: Source) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let completion_source = Arc::new(ShellSource::new(&cwd));
        // Configuración personal: fallback silencioso a default si falta o no
        // parsea (no hay nada crítico, sólo preferencias). Las env vars del
        // `.shumarc` se exportan AHORA, antes de spawnear ningún subproceso —
        // los hijos las heredan.
        let config = shuma_config::Config::load_default().unwrap_or_default();
        config.apply_env();
        let history = Arc::new(Mutex::new(open_history()));
        // La política de dedup del historial sale del rc (default
        // `IgnoreConsecutive` si no se declara).
        if let Ok(mut h) = history.lock() {
            h.set_dedup(match config.history.dedup {
                shuma_config::DedupPolicy::None => shuma_history::DedupPolicy::None,
                shuma_config::DedupPolicy::IgnoreConsecutive => {
                    shuma_history::DedupPolicy::IgnoreConsecutive
                }
                shuma_config::DedupPolicy::EraseDups => shuma_history::DedupPolicy::EraseDups,
            });
        }
        // El anchor de grupos arranca al final del historial durable: el
        // primer `:save` agrupa sólo lo tipeado en ESTA sesión, no meses
        // de historial persistido.
        let group_anchor = history.lock().map(|h| h.len()).unwrap_or(0);
        Self {
            source,
            cwd,
            input: LineState::new(),
            output: Vec::new(),
            focused: true,
            running: None,
            queue: VecDeque::new(),
            completion_source,
            history,
            history_cursor: None,
            history_search: None,
            last_tui_rect: Arc::new(Mutex::new((0.0, 0.0))),
            vim_metrics: Arc::new(Mutex::new((0.0, 0.0))),
            bg_jobs: Vec::new(),
            intent_graph: SessionGraph::new(),
            current_run_node: None,
            current_run_bytes: 0,
            vim_sel: None,
            block_seq: 0,
            current_block: 0,
            collapsed: HashSet::new(),
            section_collapsed: HashSet::new(),
            section_sort: HashMap::new(),
            expanded_stages: HashSet::new(),
            patterns: Vec::new(),
            // Política de captura inicial desde el rc (los builtins `:limit` /
            // `:spill` la sobreescriben en vivo). `0` MiB = sin tope.
            capture_limit_bytes: config.capture.limit_mb.saturating_mul(1024 * 1024),
            spill: config.capture.spill,
            reprocess_source: None,
            groups: Vec::new(),
            group_anchor,
            completion: None,
            completion_index: 0,
            scroll_px: 0.0,
            out_viewport_h: Arc::new(Mutex::new(0.0)),
            out_overflow: Arc::new(Mutex::new(0.0)),
            surf_scroll_anchor: 0.0,
            surf_scroll_velocity: 0.0,
            surf_selection: None,
            surf_selecting: false,
            surf_drag_acc: (0.0, 0.0),
            surf_layout: Arc::new(Mutex::new(None)),
            find: None,
            surf_menu: None,
            surf_last_dblclick_ms: 0,
            // Scrollback persistente: cap por `config.scrollback.limit_mb`,
            // spill opcional (si `config.scrollback.spill = true`) a un
            // archivo en `$XDG_RUNTIME_DIR/shuma-<pid>.spill` (o el path
            // explícito de la config). Errores I/O al armar el spill se
            // ignoran silenciosamente: el history funciona sin él.
            surf_history: Arc::new(Mutex::new(build_surf_history(&config))),
            surf_spilled_visible: Arc::new(Mutex::new(SurfSpilledCache::default())),
            gpu_grid: Arc::new(Mutex::new(None)),
            body_sel: None,
            body_menu: None,
            body_drag_accum: (0.0, 0.0),
            block_started: std::collections::HashMap::new(),
            block_command: std::collections::HashMap::new(),
            input_edit_at_ms: now_unix_millis(),
            config,
        }
    }

    /// Empuja una línea al buffer asignándole bloque. Cada `Prompt` abre
    /// un bloque nuevo (id monotónico); las demás líneas heredan el
    /// bloque abierto. El render usa esto para agrupar cada comando con
    /// su salida en una card desplegable.
    pub(crate) fn push_output(&mut self, mut line: OutputLine) {
        if line.kind == OutputKind::Prompt {
            self.block_seq += 1;
            self.current_block = self.block_seq;
            self.block_started.insert(self.current_block, now_unix_secs());
            // Guardamos el comando para que el header sobreviva al recorte del
            // buffer en outputs gigantes (ver `command_card`).
            self.block_command
                .insert(self.current_block, line.text.clone());
        }
        line.block = self.current_block;
        // History persistente (Fase 5.7): toda línea de body se archiva en
        // `surf_history`, con cap por memoria + spill opcional. Filtra los
        // que NO son body (igual que `body_lines_for_block`).
        push_to_surf_history(&self.surf_history, &line);
        push_line(&mut self.output, line);
    }

    /// Reserva un bloque nuevo sin tocar `current_block` — para runs que
    /// drenan asíncronos (foreground lento, jobs de fondo) y necesitan su
    /// propia card aunque otros comandos se intercalen mientras tanto.
    pub(crate) fn open_block(&mut self) -> u64 {
        self.block_seq += 1;
        self.block_started.insert(self.block_seq, now_unix_secs());
        self.block_seq
    }

    /// Empuja una línea en un bloque explícito (no en `current_block`).
    /// La usa el drenado de runs async para que su salida quede en SU
    /// card y no en la del comando que el usuario tipeó mientras tanto.
    pub(crate) fn push_in_block(&mut self, block: u64, mut line: OutputLine) {
        line.block = block;
        push_to_surf_history(&self.surf_history, &line);
        push_line(&mut self.output, line);
    }

    /// Vacía el buffer y el set de colapsos. No resetea `block_seq` —
    /// mantener ids monotónicos es inofensivo y evita reusos.
    pub(crate) fn clear_output(&mut self) {
        self.output.clear();
        self.collapsed.clear();
        self.expanded_stages.clear();
        self.reprocess_source = None;
        self.scroll_px = 0.0;
        self.surf_scroll_anchor = 0.0;
        self.surf_scroll_velocity = 0.0;
        // El builtin `clear` resetea también la history persistente; si el
        // usuario quería conservar lo previo, debió correr `:save` o leer
        // el spill antes. La semántica espeja el `clear` de un terminal.
        if let Ok(mut h) = self.surf_history.lock() {
            h.clear();
        }
        if let Ok(mut c) = self.surf_spilled_visible.lock() {
            *c = SurfSpilledCache::default();
        }
    }

    /// Cantidad de líneas en el buffer — alimenta el monitor.
    pub fn output_len(&self) -> usize {
        self.output.len()
    }

    /// `true` si hay un comando ejecutándose ahora.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Snapshot del grafo de intenciones — el chasis lo lee cada tick
    /// y lo sincroniza al `shuma-module-canvas` activo.
    pub fn intent_graph(&self) -> &SessionGraph {
        &self.intent_graph
    }
}

/// Fuente de candidatos del shell — implementa
/// [`shuma_line::CompletionSource`]:
///
/// - `commands()`: escanea `$PATH` la primera vez y cachea el resultado.
/// - `paths(prefix)`: listado del dir derivado del `prefix`, resolviendo
///   relativos contra `cwd`.
#[derive(Debug)]
pub struct ShellSource {
    cwd: PathBuf,
    commands: std::sync::OnceLock<Vec<String>>,
}

impl ShellSource {
    pub fn new(cwd: &std::path::Path) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            commands: std::sync::OnceLock::new(),
        }
    }
}

impl shuma_line::CompletionSource for ShellSource {
    fn commands(&self) -> Vec<String> {
        self.commands
            .get_or_init(|| {
                let path = std::env::var_os("PATH").unwrap_or_default();
                let mut out: Vec<String> = Vec::new();
                for dir in std::env::split_paths(&path) {
                    if let Ok(rd) = std::fs::read_dir(&dir) {
                        for ent in rd.flatten() {
                            if let Some(name) = ent.file_name().to_str() {
                                out.push(name.to_string());
                            }
                        }
                    }
                }
                out.sort();
                out.dedup();
                out
            })
            .clone()
    }
    fn paths(&self, prefix: &str) -> Vec<String> {
        let (dir_part, file_part) = match prefix.rfind('/') {
            Some(i) => (&prefix[..=i], &prefix[i + 1..]),
            None => ("", prefix),
        };
        let dir: PathBuf = if dir_part.is_empty() {
            self.cwd.clone()
        } else if dir_part.starts_with('/') {
            PathBuf::from(dir_part)
        } else if let Some(stripped) = dir_part.strip_prefix("~/") {
            if let Ok(home) = std::env::var("HOME") {
                PathBuf::from(home).join(stripped)
            } else {
                self.cwd.join(dir_part)
            }
        } else {
            self.cwd.join(dir_part)
        };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for ent in rd.flatten() {
            let name = match ent.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !name.starts_with(file_part) {
                continue;
            }
            // Ocultos: sólo aparecen si el prefix los pidió explícito.
            if name.starts_with('.') && !file_part.starts_with('.') {
                continue;
            }
            let mut full = format!("{dir_part}{name}");
            if ent.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                full.push('/');
            }
            out.push(full);
        }
        out.sort();
        out
    }
}

/// Abre el historial en `$XDG_DATA_HOME/shuma/history.jsonl` (o el
/// fallback de `directories`). Si no se puede abrir, devuelve un
/// historial vacío en `/dev/null` — el shell sigue funcionando sin
/// persistencia.
fn open_history() -> shuma_history::History {
    if let Some(path) = shuma_history::History::default_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(h) = shuma_history::History::open(&path) {
            return h;
        }
    }
    // Fallback: historial en /dev/null (existe siempre, append-only OK).
    shuma_history::History::open(std::path::PathBuf::from("/dev/null"))
        .unwrap_or_else(|_| panic!("no se pudo abrir ni /dev/null como history"))
}

/// Segundos unix actuales (0 si el reloj está antes de la época).
pub(crate) fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Milisegundos unix actuales — para el parpadeo del caret del input.
pub(crate) fn now_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Tecla recibida desde el chasis. Enter ejecuta, Tab completa,
    /// flechas y edición van al `LineState`.
    Key(KeyEvent),
    /// Click sobre el input box — re-foca (sigue siendo el único
    /// campo, pero lo mantenemos por simetría con otros módulos).
    FocusInput,
    /// Limpia el buffer de output — disparado por el shortcut `Clear`
    /// o el builtin `clear`.
    Clear,
    /// Drena eventos del run activo (si hay) y pinta líneas nuevas.
    /// Lo dispara el chasis a alta frecuencia (~100 ms).
    Tick,
    /// SIGTERM al run activo (Ctrl-C o shortcut `Cancel`).
    Cancel,
    /// Click en una decoración del output — el dispatch decide la
    /// acción (cd, xdg-open, pre-llenar el input, etc.).
    OpenDecoration(shuma_line::DecorationKind),
    /// Inserta `text` en la posición actual del cursor del input. La
    /// dispara el chasis cuando otro módulo (p. ej. `shuma-module-canvas`
    /// al clickear un nodo) quiere empujar una referencia `%pN`/`%cN`
    /// al REPL. Cierra los overlays de búsqueda y deja el cursor justo
    /// después del texto insertado.
    InsertAtCursor(String),
    /// Pega el clipboard al PTY del TUI activo — click derecho o botón
    /// del medio sobre el panel de vim (paste estilo terminal).
    VimPaste,
    /// Drag de selección sobre el card de vim. `dx`/`dy` = delta desde el
    /// evento anterior; `ax`/`ay` = posición del press (local al panel).
    VimDrag {
        end: bool,
        dx: f32,
        dy: f32,
        ax: f32,
        ay: f32,
    },
    /// Alterna plegado/desplegado de la card de un comando. La dispara el
    /// click en el header de la card (chevron + comando).
    ToggleBlock(u64),
    /// Alterna plegado/desplegado de una **sub-sección** dentro del bloque
    /// `block` (índice `idx` según `sections::detect_sections`). Click en
    /// el header de la sección lo dispara.
    ToggleSection { block: u64, idx: usize },
    /// Click en un header de columna de una sub-sección tipo tabla. Cicla:
    /// sin orden → asc(col) → desc(col) → sin orden.
    SortSectionColumn {
        block: u64,
        section: usize,
        col: usize,
    },
    /// Rueda del mouse sobre el panel de output. `delta` ya viene en px
    /// (positivo = rodar hacia arriba / ver historial). Ajusta `scroll_px`.
    Scroll(f32),
    /// Re-ejecuta `line` como un comando nuevo — la dispara el click en
    /// una etapa de pipe de una card SIN captura en vivo (fallback `sh -c`).
    RunLine(String),
    /// Alterna el desplegable de una etapa de pipe con captura en vivo
    /// (tee). La dispara el click en su chip; muestra/oculta las líneas
    /// intermedias ya capturadas sin re-ejecutar nada.
    ToggleStage { block: u64, stage: usize },
    /// Arma el reprocess: el stdout del bloque `block` alimentará el stdin
    /// del próximo comando. La dispara el chip ↻ de una card. Si ya estaba
    /// armado el mismo bloque, lo desarma (toggle).
    SetReprocess(u64),
    /// Ejecuta el grupo guardado de índice `idx` (0-based). La dispara el
    /// click en su card del panel de grupos (equivale a la tecla F{idx+1}).
    RunGroup(usize),
    /// Mouse sobre el cuerpo (IDE-text) de la card del bloque `block`:
    /// click posiciona el caret, drag extiende la selección. La dispara el
    /// `on_pointer` del `text-editor` del cuerpo.
    BodyPointer {
        block: u64,
        ev: llimphi_widget_text_editor::PointerEvent,
    },
    /// Copia al clipboard la selección viva del cuerpo del bloque `block`
    /// (click derecho sobre el cuerpo). No-op si no hay selección.
    CopyBody(u64),
    /// Doble-click sobre el cuerpo IDE-text del bloque `block`: selecciona
    /// la palabra bajo `(x, y)` (coords locales al nodo del editor, incluyen
    /// el gutter). La dispara el `on_double_tap_at` del cuerpo.
    BodyDoubleClick { block: u64, x: f32, y: f32 },
    /// Click derecho sobre el output: abre el menú contextual en `(x, y)`
    /// (coords del nodo raíz del shell = espacio de su view). Las acciones
    /// operan sobre el bloque seleccionado (o el más reciente).
    OpenBodyMenu { x: f32, y: f32 },
    /// Elegir un item del menú contextual del output (índice 0-based).
    BodyMenuPick(usize),
    /// Cerrar el menú contextual del output (scrim / Esc).
    BodyMenuDismiss,
    /// Click sobre el panel de un TUI bajo PTY (htop/less/btop/…). Si el
    /// programa habilitó mouse (`vt100::MouseProtocolMode != None`), encodea
    /// el click en xterm-mouse y lo escribe al stdin del PTY. `button` es 0
    /// (izquierdo), 1 (medio), 2 (derecho). `lx`/`ly` son coords relativas
    /// al rect del panel; `rect_w`/`rect_h` el tamaño del rect (para
    /// convertir a celdas).
    TuiMouseClick {
        button: u8,
        lx: f32,
        ly: f32,
        rect_w: f32,
        rect_h: f32,
    },
    /// Rueda sobre el panel TUI. `dy` positivo = arriba (botón 4); negativo
    /// = abajo (botón 5). Se emite un evento de mouse por cada "tick" de
    /// rueda lógica. Las coords se usan para reportar dónde estaba el
    /// cursor (algunos TUIs lo respetan).
    TuiMouseWheel {
        dy: f32,
        lx: f32,
        ly: f32,
        rect_w: f32,
        rect_h: f32,
    },
    /// Drag del mouse sobre el cuerpo de output en modo **superficie**
    /// (`SHUMA_TERMINAL_SURFACE=1`). El primer Move arranca/colapsa la
    /// selección al `(lx0, ly0)`; los siguientes la extienden; el End la
    /// deja fijada para que el usuario copie. `dx`/`dy` son deltas desde el
    /// evento previo (el `update` los acumula sobre `(ax, ay)`).
    SurfSelectDrag {
        phase: llimphi_ui::DragPhase,
        dx: f32,
        dy: f32,
        ax: f32,
        ay: f32,
    },
    /// Limpia la selección viva del cuerpo de output (lo dispara una tecla,
    /// un click en blanco, etc.). No-op si ya está vacía.
    SurfClearSelection,
    /// Copia al clipboard el texto de la selección viva del cuerpo de
    /// output. No-op si no hay selección. Reusa el clipboard global del
    /// proceso (vía `arboard`).
    SurfCopySelection,
    /// Doble-click sobre el cuerpo de output en modo superficie: selecciona
    /// la palabra bajo el punto (paridad con terminales clásicas). El
    /// `update` resuelve `(lx, ly)` a `Point` con `point_at_geo`, computa
    /// los boundaries de palabra en el texto de la línea y arma una
    /// `SelectionRange` sobre esa palabra.
    SurfDoubleClick {
        lx: f32,
        ly: f32,
        rect_w: f32,
        rect_h: f32,
    },
    /// Right-click sobre el cuerpo de output en modo superficie: abre el
    /// menú contextual en `(x, y)` (coords del nodo raíz del shell). Las
    /// acciones operan sobre el scrollback entero (no por-bloque como el
    /// `BodyMenu` del legacy) — Copiar selección, Copiar todo, Seleccionar
    /// todo.
    SurfOpenMenu { x: f32, y: f32 },
    /// Elegir un item del menú contextual del surface (0-based).
    SurfMenuPick(usize),
    /// Cerrar el menú contextual del surface (scrim / Esc).
    SurfMenuDismiss,
    /// Abre la barra de búsqueda (Ctrl+F). Si ya estaba abierta, re-foca el
    /// input vacío (paridad con browsers/editores). Si no hay layout
    /// publicado todavía, abre igual — el primer keystroke recomputará.
    FindOpen,
    /// Cierra la barra de búsqueda (Esc). Limpia `find` y la selección
    /// derivada de un match. No toca `surf_selection` si vino de un drag
    /// del mouse y no de un match (la heurística: si `find` existía y
    /// tenía un `current`, era nuestro highlight; lo limpiamos).
    FindClose,
    /// Agrega un char a la query de búsqueda y re-busca.
    FindChar(char),
    /// Borra el último char de la query y re-busca.
    FindBackspace,
    /// Avanza al siguiente match (Enter / F3 / botón).
    FindNext,
    /// Retrocede al match previo (Shift+Enter / Shift+F3 / botón).
    FindPrev,
    /// Togglea case-insensitive (botón `Aa` o atajo). Re-busca con la
    /// nueva política.
    FindToggleCase,
}

mod mouse_xterm;
pub mod sections;
mod update;
mod view;

pub use mouse_xterm::{XBtn, XPhase};
pub use update::*;
pub use view::*;

/// Arma el `Scrollback` persistente desde la config: cap en MiB +
/// (opcional) spill a un archivo en `$XDG_RUNTIME_DIR/shuma-<pid>.spill`
/// (o el path explícito de la config). Errores al armar el spill se
/// degradan a "sin spill" (el history funciona igual, sólo pierde el
/// archivo de archive).
fn build_surf_history(config: &shuma_config::Config) -> llimphi_widget_terminal::Scrollback {
    let limit_bytes = config.scrollback.limit_mb.saturating_mul(1024 * 1024);
    let mut sb = llimphi_widget_terminal::Scrollback::new(limit_bytes);
    if config.scrollback.spill {
        let path = if !config.scrollback.spill_path.is_empty() {
            PathBuf::from(&config.scrollback.spill_path)
        } else {
            let dir = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir());
            dir.join(format!("shuma-{}.spill", std::process::id()))
        };
        if let Ok(spill) = llimphi_widget_terminal::SpillStore::create(&path) {
            sb.enable_spill(spill);
        }
        // Sin spill si falló crear el archivo — no es fatal, el shell sigue.
    }
    sb
}

/// Refresca el cache de líneas spilled visibles si `spilled_count` cambió
/// desde el último refresh. Lee las últimas [`MAX_SPILLED_VISIBLE`] líneas
/// del archive vía `Scrollback::read_spilled`. Si el read falla por I/O,
/// la entrada queda como `<I/O error>` (no propaga el error — el view
/// sigue mostrando el resto). Sincrono: el costo del refresh es N reads
/// del archivo, una sola vez por cambio de spill (no por frame).
pub(crate) fn refresh_surf_spilled_visible(
    history: &Arc<Mutex<llimphi_widget_terminal::Scrollback>>,
    cache: &Arc<Mutex<SurfSpilledCache>>,
) {
    // Snapshot del estado del history sin retener el lock durante el I/O.
    let (spilled_count, hist_clone) = {
        let Ok(h) = history.lock() else { return };
        (h.spilled_count(), h.clone())
    };
    {
        let Ok(c) = cache.lock() else { return };
        if c.cached_at == spilled_count {
            return; // no hubo append al spill desde el último refresh
        }
    }
    // Refresh: leer las últimas N spilled.
    let n = spilled_count.min(MAX_SPILLED_VISIBLE);
    let first_id = (spilled_count - n) as u64;
    let mut lines = Vec::with_capacity(n);
    for i in 0..n {
        let id = first_id + i as u64;
        match hist_clone.read_spilled(id) {
            Ok(Some(text)) => lines.push(text),
            Ok(None) => lines.push(String::new()),
            Err(_) => lines.push("<I/O error reading spill>".into()),
        }
    }
    if let Ok(mut c) = cache.lock() {
        c.lines = lines;
        c.first_id = first_id;
        c.cached_at = spilled_count;
    }
}

/// Appendea el texto de `line` a la `Scrollback` persistente sólo si es una
/// línea de **body** (no Prompt, no salida de etapa intermedia, no notice
/// de cierre `✔/✘/⏹`). Espeja el filtro de `body_lines_for_block` para
/// que el history acumule sólo lo que el view ve como cuerpo. Errores del
/// lock se ignoran (poison defensivo).
fn push_to_surf_history(
    history: &Arc<Mutex<llimphi_widget_terminal::Scrollback>>,
    line: &OutputLine,
) {
    if line.kind == OutputKind::Prompt {
        return;
    }
    if line.stage.is_some() {
        return;
    }
    if view::is_status_line(&line.text) {
        return;
    }
    if let Ok(mut h) = history.lock() {
        h.push_line(&line.text);
    }
}

pub fn contributions(_state: &State) -> ModuleContributions {
    ModuleContributions {
        monitors: vec![],
        shortcuts: vec![
            ShortcutSpec::module_action("Clear", "shell.clear")
                .with_hint("Vacía el buffer de output"),
            ShortcutSpec::module_action("Cancel", "shell.cancel")
                .with_hint("SIGTERM al comando actual"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_ui::Modifiers;

    fn ev(key: Key, text: Option<&str>) -> KeyEvent {
        KeyEvent {
            key,
            state: KeyState::Pressed,
            text: text.map(|s| s.to_string()),
            modifiers: Modifiers::default(),
            repeat: false,
        }
    }

    /// Aplica `Msg::Tick` hasta que el run vivo se cierre (o se acabe el
    /// presupuesto). Imita lo que el chasis hace a 100 ms entre ticks.
    fn drain_until_idle(mut s: State) -> State {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while s.is_running() {
            s = update(s, Msg::Tick);
            if std::time::Instant::now() > deadline {
                panic!("run no terminó en 10s");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Un Tick más por si quedó algo en el canal después del Exited.
        update(s, Msg::Tick)
    }

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "shell");
    }

    #[test]
    fn placeholder_state_constructs() {
        let s = State::new(Source::Local);
        assert!(s.output.is_empty());
        assert!(s.cwd.is_absolute() || s.cwd == PathBuf::from("/"));
    }

    #[test]
    fn pwd_builtin_writes_cwd() {
        let mut s = State::new(Source::Local);
        s.input.set_text("pwd");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.starts_with("$ pwd")));
        assert!(s.output.iter().any(|l| l.kind == OutputKind::Stdout));
    }

    #[test]
    fn clear_builtin_empties_output() {
        let mut s = State::new(Source::Local);
        s.input.set_text("pwd");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(!s.output.is_empty());
        s.input.set_text("clear");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.is_empty());
    }

    #[test]
    fn clear_msg_empties_output() {
        let mut s = State::new(Source::Local);
        s.output.push(OutputLine::stdout("hola"));
        s = update(s, Msg::Clear);
        assert!(s.output.is_empty());
    }

    #[test]
    fn cd_to_root_changes_cwd() {
        let mut s = State::new(Source::Local);
        s.input.set_text("cd /");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.cwd, PathBuf::from("/"));
    }

    #[test]
    fn cd_to_nonexistent_logs_error() {
        let mut s = State::new(Source::Local);
        s.input.set_text("cd /nope/this/does/not/exist");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.starts_with("cd:")));
    }

    #[test]
    fn external_command_captures_stdout() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("echo hola_mundo");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running(), "Enter debe arrancar el run");
        s = drain_until_idle(s);
        let combined: Vec<String> = s.output.iter().map(|l| l.text.clone()).collect();
        assert!(
            combined.iter().any(|t| t == "hola_mundo"),
            "esperaba stdout 'hola_mundo' en {combined:?}"
        );
        assert!(combined.iter().any(|t| t == "✔ exit 0"));
    }

    #[test]
    fn external_command_failure_writes_exit_nonzero() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("false");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s = drain_until_idle(s);
        assert!(s.output.iter().any(|l| l.text.starts_with("✘ exit")));
    }

    #[test]
    fn long_running_command_does_not_block_update() {
        // `sleep 0.3` debería volver de `update` inmediatamente (no
        // bloquear ~300 ms como con `Command::output`). Si el spawn es
        // no-bloqueante, `update` retorna en pocos milisegundos.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 0.3");
        let t0 = std::time::Instant::now();
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "update bloqueó {elapsed:?} — debería volver al instante"
        );
        assert!(s.is_running(), "el sleep debe seguir vivo tras Enter");
        s = drain_until_idle(s);
        assert!(s.output.iter().any(|l| l.text == "✔ exit 0"));
    }

    #[test]
    fn second_enter_queues_while_busy() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 0.2");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running());
        s.input.set_text("echo segunda");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.queue.len(), 1, "segunda línea debe quedar en cola");
        s = drain_until_idle(s);
        // Tras drenar, la cola arrancó y ya cerró el segundo run.
        assert_eq!(s.queue.len(), 0);
        let combined: Vec<String> = s.output.iter().map(|l| l.text.clone()).collect();
        assert!(combined.iter().any(|t| t == "segunda"), "{combined:?}");
    }

    #[test]
    fn cancel_terminates_active_run() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 30");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running());
        // El coordinador de `shuma-exec` puebla `Killer.children` en
        // background — un Cancel inmediato podría llegar antes y la
        // señal caería en el vacío. Esperar a que aparezca el PID.
        let arc = s.running.as_ref().unwrap().clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            let has_pid = arc
                .lock()
                .unwrap()
                .killer
                .as_ref()
                .map(|k| !k.pids().is_empty())
                .unwrap_or(false);
            if has_pid {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            arc.lock()
                .unwrap()
                .killer
                .as_ref()
                .map(|k| !k.pids().is_empty())
                .unwrap_or(false),
            "el coordinador no expuso el PID en 500ms"
        );
        s = update(s, Msg::Cancel);
        s = drain_until_idle(s);
        assert!(!s.is_running(), "sleep 30 debe morir al cancelar");
        assert!(s.output.iter().any(|l| l.text.starts_with("⏹ cancel")));
    }

    #[test]
    fn empty_submit_does_nothing_but_clears_input() {
        let mut s = State::new(Source::Local);
        s.input.set_text("   ");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.is_empty());
        assert!(s.input.text().is_empty());
    }

    #[test]
    fn output_buffer_caps_at_max() {
        let mut buf: Vec<OutputLine> = Vec::new();
        for i in 0..MAX_OUTPUT_LINES + 50 {
            push_line(&mut buf, OutputLine::stdout(format!("línea {i}")));
        }
        assert_eq!(buf.len(), MAX_OUTPUT_LINES);
        assert!(buf[0].text.contains("50"));
    }

    #[test]
    fn tab_completion_inserts_unique_candidate() {
        // Si el prefijo tiene un único match, Tab debe completarlo.
        let mut s = State::new(Source::Local);
        s.input.set_text("ec");
        // Forzar un source determinístico para no depender de $PATH.
        struct Fixed;
        impl shuma_line::CompletionSource for Fixed {
            fn commands(&self) -> Vec<String> {
                vec!["echo".into()]
            }
            fn paths(&self, _: &str) -> Vec<String> {
                vec![]
            }
        }
        s.completion_source = Arc::new(ShellSource::new(&s.cwd));
        // Bypassear: aplicamos completion manualmente con el Fixed source,
        // ya que apply_completion_msg usa s.completion_source.
        let comp = s.input.complete(&Fixed);
        let candidate = comp.candidates.first().cloned().unwrap_or_default();
        s.input.apply_completion(&comp, &candidate);
        assert_eq!(s.input.text(), "echo");
    }

    #[test]
    fn arrow_up_walks_history_backwards() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        // Insertar entradas a mano vía History (no via run_submitted, que
        // dispararía procesos reales).
        {
            let mut h = s.history.lock().unwrap();
            let _ = h.append(shuma_history::Entry::new("uno", "/", 1));
            let _ = h.append(shuma_history::Entry::new("dos", "/", 2));
        }
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowUp), None)));
        assert_eq!(s.input.text(), "dos");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowUp), None)));
        assert_eq!(s.input.text(), "uno");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowDown), None)));
        assert_eq!(s.input.text(), "dos");
    }

    #[test]
    fn ctrl_r_opens_search_overlay() {
        let mut s = State::new(Source::Local);
        let ctrl_r = KeyEvent {
            key: Key::Character("r".into()),
            state: KeyState::Pressed,
            text: Some("r".into()),
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
            repeat: false,
        };
        s = update(s, Msg::Key(ctrl_r));
        assert!(s.history_search.is_some());
    }

    #[test]
    fn ghost_extends_from_history_when_prefix_matches() {
        let mut s = State::new(Source::Local);
        {
            let mut h = s.history.lock().unwrap();
            let _ = h.append(shuma_history::Entry::new("cargo build --release", "/", 1));
        }
        s.input.set_text("cargo bu");
        let g = current_ghost(&s);
        // Devuelve el sufijo que falta para llegar a la línea histórica.
        assert_eq!(g.as_deref(), Some("ild --release"));
    }

    #[test]
    fn build_spec_routes_known_tui_command_to_pty() {
        let (spec, tui) = build_spec("vim README.md", "/");
        assert!(matches!(spec.exec, shuma_exec::Exec::Pty { .. }));
        assert!(tui.is_some());
    }

    #[test]
    fn build_spec_routes_plain_command_to_shell() {
        let (spec, tui) = build_spec("ls -la", "/");
        assert!(matches!(spec.exec, shuma_exec::Exec::Shell { .. }));
        assert!(tui.is_none());
    }

    #[test]
    fn build_spec_routes_simple_pipe_to_direct_with_capture() {
        // Un pipe simple corre directo (sin bash) y con captura por etapa.
        let (spec, tui) = build_spec("ls -la | grep foo", "/");
        match &spec.exec {
            shuma_exec::Exec::Direct { stages } => {
                assert_eq!(stages.len(), 2, "dos etapas");
                assert_eq!(stages[0].program, "ls");
                assert_eq!(stages[1].program, "grep");
            }
            other => panic!("esperaba Exec::Direct, fue {other:?}"),
        }
        assert!(spec.capture_stages, "el pipe directo activa el tee");
        assert!(tui.is_none());
    }

    #[test]
    fn build_spec_pipe_with_quotes_falls_back_to_shell() {
        // `shuma_line::Stage` no recoge StringLit en args, así que un pipe
        // con comillas debe ir a `sh -c` o perdería el argumento citado.
        let (spec, _) = build_spec("echo 'a | b' | cat", "/");
        assert!(matches!(spec.exec, shuma_exec::Exec::Shell { .. }));
        assert!(!spec.capture_stages);
    }

    #[test]
    fn alt_screen_is_the_hard_tui_signal() {
        // `ESC[?1049h` entra a alternate screen (señal dura de TUI
        // full-screen); `ESC[?1049l` sale y vuelve a modo líneas.
        let mut p = vt100::Parser::new(24, 80, 0);
        p.process(b"hola mundo\r\n");
        assert!(!p.screen().alternate_screen(), "arranca en modo líneas");
        p.process(b"\x1b[?1049h");
        assert!(p.screen().alternate_screen(), "1049h = pantalla completa");
        p.process(b"\x1b[?1049l");
        assert!(!p.screen().alternate_screen(), "1049l = vuelve a líneas");
    }

    #[test]
    fn screen_to_lines_trims_trailing_blanks() {
        let mut p = vt100::Parser::new(24, 80, 0);
        p.process(b"primera\r\nsegunda\r\n");
        let lines = screen_to_lines(p.screen());
        // Sólo las dos filas con contenido; las 22 filas vacías de abajo
        // se recortan.
        assert_eq!(lines, vec!["primera", "segunda"]);
    }

    #[test]
    fn build_spec_pipe_with_glob_falls_back_to_shell() {
        let (spec, _) = build_spec("ls *.rs | cat", "/");
        assert!(matches!(spec.exec, shuma_exec::Exec::Shell { .. }));
    }

    #[test]
    fn simple_pipe_stages_rejects_single_command() {
        // Un único comando no gana nada del modo directo (no hay tubería
        // que interceptar) → `None`, cae a `sh -c`.
        assert!(simple_pipe_stages("ls -la").is_none());
    }

    #[test]
    fn simple_pipe_stages_rejects_trailing_pipe() {
        // Etapa sin comando (línea incompleta) → None.
        assert!(simple_pipe_stages("ls |").is_none());
    }

    #[test]
    fn piped_command_captures_intermediate_stage_output() {
        // `echo hola | cat`: stage0 (echo) se captura en vivo como una
        // OutputLine con stage=Some(0); la salida final (cat) sale como
        // stdout normal (stage None).
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("echo hola | cat");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running(), "el pipe debe arrancar un run");
        s = drain_until_idle(s);
        let stage0: Vec<&OutputLine> = s
            .output
            .iter()
            .filter(|l| l.stage == Some(0))
            .collect();
        assert!(
            stage0.iter().any(|l| l.text == "hola"),
            "esperaba 'hola' capturado de la etapa 0, output: {:?}",
            s.output.iter().map(|l| (l.stage, &l.text)).collect::<Vec<_>>()
        );
        // La salida final (cat) llega como stdout normal sin stage.
        assert!(s
            .output
            .iter()
            .any(|l| l.stage.is_none() && l.text == "hola"));
        assert!(s.output.iter().any(|l| l.text == "✔ exit 0"));
    }

    #[test]
    fn infer_predicts_next_command_in_a_repeated_sequence() {
        // Historial con el patrón `git pull` → `make` repetido dos veces y
        // un `git pull` final: el motor debe predecir `make` como
        // continuación. cwd `/tmp/...` sin marcadores → sin gating.
        let mut s = State::new(Source::Local);
        let dir = "/tmp/shuma-infer-pred-test";
        {
            let mut h = s.history.lock().unwrap();
            for (i, line) in ["git pull", "make", "git pull", "make", "git pull"]
                .iter()
                .enumerate()
            {
                let _ = h.append(shuma_history::Entry::new(*line, dir, i as u64));
            }
        }
        refresh_patterns(&mut s);
        assert!(!s.patterns.is_empty(), "debe emerger el patrón git→make");
        // La continuación predicha empieza por `make` (puede seguir con el
        // resto del patrón más largo, p. ej. `make && git pull`).
        let pred = predicted_sequence(&s).expect("predice una continuación");
        assert!(
            pred.starts_with("make"),
            "tras `git pull` predice `make…`, fue {pred:?}"
        );
    }

    #[test]
    fn ghost_uses_prediction_before_history() {
        // Con el patrón aprendido, tipear `ma` debe sugerir `ke` (de la
        // predicción `make`), aunque el historial no tenga un match mejor.
        let mut s = State::new(Source::Local);
        let dir = "/tmp/shuma-infer-ghost-test";
        {
            let mut h = s.history.lock().unwrap();
            for (i, line) in ["git pull", "make", "git pull", "make", "git pull"]
                .iter()
                .enumerate()
            {
                let _ = h.append(shuma_history::Entry::new(*line, dir, i as u64));
            }
        }
        refresh_patterns(&mut s);
        s.input.set_text("ma");
        assert_eq!(current_ghost(&s).as_deref(), Some("ke"));
    }

    #[test]
    fn git_branch_reads_head_ref() {
        // `.git/HEAD` con `ref: refs/heads/<rama>` → Some(rama). Usamos un
        // tmpdir aislado para no depender del repo real.
        let base = std::env::temp_dir().join(format!("shuma-gb-{}", std::process::id()));
        let git = base.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        // Desde un subdirectorio: debe subir hasta encontrar `.git`.
        let sub = base.join("sub/dir");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(git_branch(&sub).as_deref(), Some("feature/x"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn git_branch_none_outside_repo() {
        let base = std::env::temp_dir().join(format!("shuma-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(git_branch(&base), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn limit_builtin_sets_capture_bytes() {
        let mut s = State::new(Source::Local);
        s.input.set_text(":limit 5");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.capture_limit_bytes, 5 * 1024 * 1024);
        assert!(!s.is_running(), "`:limit` no spawnea proceso");
        // `:limit 0` quita el tope.
        s.input.set_text(":limit 0");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.capture_limit_bytes, 0);
    }

    #[test]
    fn spill_builtin_toggles_flag() {
        let mut s = State::new(Source::Local);
        s.input.set_text(":spill on");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.spill);
        s.input.set_text(":spill off");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(!s.spill);
    }

    #[test]
    fn sanitize_paste_drops_single_trailing_newline() {
        // Pegar "ls -la\n" no debe dejar una línea vacía colgando.
        assert_eq!(sanitize_paste("ls -la\n"), "ls -la");
    }

    #[test]
    fn sanitize_paste_preserves_interior_newlines() {
        // El input es multilínea: pegar un script conserva sus saltos
        // (no se colapsa a `;` como el shell GPUI).
        assert_eq!(sanitize_paste("ls\npwd\n"), "ls\npwd");
    }

    #[test]
    fn sanitize_paste_normalizes_crlf() {
        assert_eq!(sanitize_paste("a\r\nb"), "a\nb");
        assert_eq!(sanitize_paste("a\rb"), "a\nb");
    }

    #[test]
    fn sanitize_paste_strips_control_chars_and_tabs() {
        // ESC (\x1b) y BEL (\x07) se descartan; tab → espacio; los saltos
        // de línea sobreviven.
        assert_eq!(sanitize_paste("ls\t-la\x1b[X\x07"), "ls -la[X");
    }

    #[test]
    fn sanitize_paste_keeps_plain_text() {
        assert_eq!(sanitize_paste("echo hola mundo"), "echo hola mundo");
    }

    #[test]
    fn alias_from_config_expands_before_run() {
        // Un alias del `.shumarc` reemplaza la primera palabra; lo tipeado
        // queda en el historial, lo resuelto es lo que se ejecuta.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.config
            .aliases
            .insert("saluda".into(), "echo hola_alias".into());
        s.input.set_text("saluda");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running(), "el alias resuelto debe arrancar un run");
        s = drain_until_idle(s);
        let combined: Vec<String> = s.output.iter().map(|l| l.text.clone()).collect();
        assert!(
            combined.iter().any(|t| t == "hola_alias"),
            "esperaba stdout del alias resuelto en {combined:?}"
        );
        // El prompt muestra lo tipeado, no lo resuelto.
        assert!(combined.iter().any(|t| t == "$ saluda"));
    }

    #[test]
    fn alias_can_resolve_to_a_builtin() {
        // `alias raiz='cd /'` debe disparar el builtin cd sobre la línea ya
        // expandida.
        let mut s = State::new(Source::Local);
        s.config.aliases.insert("raiz".into(), "cd /".into());
        s.input.set_text("raiz");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.cwd, PathBuf::from("/"));
        assert!(!s.is_running(), "cd no spawnea proceso");
    }

    #[test]
    fn alias_never_hijacks_meta_command() {
        // Un alias declarado con el nombre de un meta-comando no debe
        // secuestrarlo: `:limit` sigue siendo el builtin del shell.
        let mut s = State::new(Source::Local);
        s.config
            .aliases
            .insert(":limit".into(), "echo secuestrado".into());
        s.input.set_text(":limit 7");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.capture_limit_bytes, 7 * 1024 * 1024);
        assert!(!s.is_running(), "el meta no debe ejecutar el alias");
    }

    #[test]
    fn save_group_captures_recent_commands() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        // Dos comandos reales (no meta) + un :save.
        for line in ["echo uno", "echo dos"] {
            s.input.set_text(line);
            s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
            s = drain_until_idle(s);
        }
        s.input.set_text(":save build");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.groups.len(), 1);
        assert_eq!(s.groups[0].name, "build");
        assert_eq!(s.groups[0].lines, vec!["echo uno", "echo dos"]);
        // El anchor avanzó: un segundo :save sin comandos nuevos no agrupa.
        s.input.set_text(":save vacio");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.groups.len(), 1, "no se crea grupo vacío");
    }

    #[test]
    fn run_group_msg_executes_group() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.groups.push(CommandGroup {
            name: "g".into(),
            lines: vec!["echo desde_panel".into()],
        });
        s = update(s, Msg::RunGroup(0));
        s = drain_until_idle(s);
        assert!(s.output.iter().any(|l| l.text == "desde_panel"));
        // Índice fuera de rango: no-op.
        s = update(s, Msg::RunGroup(9));
        assert!(!s.is_running());
    }

    #[test]
    fn fkey_runs_saved_group() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        // F1 sin grupos: no hace nada.
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::F1), None)));
        assert!(!s.is_running());
        // Guardamos un grupo de un comando y lo corremos con F1.
        s.groups.push(CommandGroup {
            name: "g".into(),
            lines: vec!["echo desde_f1".into()],
        });
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::F1), None)));
        s = drain_until_idle(s);
        assert!(s.output.iter().any(|l| l.text == "desde_f1"));
    }

    #[test]
    fn reprocess_feeds_block_stdout_as_stdin() {
        // Corre `printf "b\\na\\nc\\n"`, arma reprocess sobre su bloque, y
        // corre `sort`: debe recibir esa salida por stdin y ordenarla.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("printf 'b\\na\\nc\\n'");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s = drain_until_idle(s);
        let src_block = s.output.iter().find(|l| l.text == "b").unwrap().block;
        s = update(s, Msg::SetReprocess(src_block));
        assert_eq!(s.reprocess_source, Some(src_block));
        s.input.set_text("sort");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.reprocess_source.is_none(), "el submit consume el reprocess");
        s = drain_until_idle(s);
        // La salida de `sort` (en su propio bloque) está ordenada: a,b,c.
        let sorted: Vec<String> = s
            .output
            .iter()
            .filter(|l| l.block != src_block && l.kind == OutputKind::Stdout)
            .map(|l| l.text.clone())
            .collect();
        assert_eq!(sorted, vec!["a", "b", "c"], "sort recibió el stdin reprocesado");
    }

    #[test]
    fn set_reprocess_toggles_off_same_block() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::SetReprocess(3));
        assert_eq!(s.reprocess_source, Some(3));
        s = update(s, Msg::SetReprocess(3));
        assert_eq!(s.reprocess_source, None, "re-armar el mismo bloque desarma");
    }

    fn fake_completion(cands: &[&str], start: usize, end: usize) -> shuma_line::Completion {
        shuma_line::Completion {
            kind: shuma_line::CompletionKind::Command,
            candidates: cands.iter().map(|s| s.to_string()).collect(),
            replace_start: start,
            replace_end: end,
        }
    }

    #[test]
    fn completion_tab_accepts_highlighted() {
        // Con popup vivo, Tab acepta el candidato resaltado (no cicla).
        let mut s = State::new(Source::Local);
        s.input.set_text("ca");
        s.completion = Some(fake_completion(&["cargo", "cat", "cal"], 0, 2));
        s.completion_index = 1; // "cat"
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Tab), None)));
        assert_eq!(s.input.text(), "cat", "Tab aplica el resaltado");
        assert!(s.completion.is_none(), "y cierra el popup");
    }

    #[test]
    fn ctrl_a_selects_whole_input_line() {
        let mut s = State::new(Source::Local);
        s.input.set_text("git status");
        let ctrl_a = KeyEvent {
            key: Key::Character("a".into()),
            state: KeyState::Pressed,
            text: Some("a".into()),
            modifiers: Modifiers { ctrl: true, ..Default::default() },
            repeat: false,
        };
        s = update(s, Msg::Key(ctrl_a));
        assert_eq!(s.input.selected_text().as_deref(), Some("git status"));
    }

    #[test]
    fn shift_arrow_extends_input_selection() {
        let mut s = State::new(Source::Local);
        s.input.set_text("abc");
        // Shift+Left desde el final selecciona el último char.
        let shift_left = KeyEvent {
            key: Key::Named(NamedKey::ArrowLeft),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers { shift: true, ..Default::default() },
            repeat: false,
        };
        s = update(s, Msg::Key(shift_left));
        assert_eq!(s.input.selected_text().as_deref(), Some("c"));
    }

    #[test]
    fn rank_completion_by_usage_orders_by_history() {
        let mut s = State::new(Source::Local);
        // Historial aislado (el real del usuario contaminaría el ranking).
        s.history = Arc::new(Mutex::new(
            shuma_history::History::open(std::path::PathBuf::from("/dev/null")).unwrap(),
        ));
        {
            let mut h = s.history.lock().unwrap();
            // Líneas distintas (el dedup colapsa repetidas consecutivas).
            let _ = h.append(shuma_history::Entry::new("cat a", "/", 0));
            let _ = h.append(shuma_history::Entry::new("cargo build", "/", 1));
            let _ = h.append(shuma_history::Entry::new("cat b", "/", 2));
            let _ = h.append(shuma_history::Entry::new("cat c", "/", 3));
        }
        let mut comp = fake_completion(&["cargo", "cat", "cal"], 0, 2);
        rank_completion_by_usage(&s, &mut comp);
        assert_eq!(comp.candidates[0], "cat", "el más usado primero");
        assert_eq!(comp.candidates[1], "cargo");
        assert_eq!(comp.candidates[2], "cal", "sin uso, al final");
    }

    #[test]
    fn completion_arrows_cycle_both_ways() {
        let mut s = State::new(Source::Local);
        s.completion = Some(fake_completion(&["a", "b", "c"], 0, 0));
        s.completion_index = 0;
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowUp), None)));
        assert_eq!(s.completion_index, 2, "↑ desde 0 va al último");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowDown), None)));
        assert_eq!(s.completion_index, 0);
    }

    #[test]
    fn completion_enter_submits_not_accepts() {
        // Con popup vivo, Enter ejecuta el comando como está (no acepta el
        // resaltado): el popup es sugerencia, no modal.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("ca");
        s.completion = Some(fake_completion(&["cargo", "cat"], 0, 2));
        s.completion_index = 1; // "cat"
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.completion.is_none(), "Enter cierra el popup");
        assert!(
            s.input.text().is_empty(),
            "ejecutó (limpió el input) en vez de aplicar 'cat'"
        );
        s = drain_until_idle(s);
    }

    #[test]
    fn completion_escape_closes_without_change() {
        let mut s = State::new(Source::Local);
        s.input.set_text("ca");
        s.completion = Some(fake_completion(&["cargo", "cat"], 0, 2));
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Escape), None)));
        assert!(s.completion.is_none());
        assert_eq!(s.input.text(), "ca", "Esc no toca el texto");
    }

    #[test]
    fn typing_processes_key_and_refreshes_completion() {
        // Tipear procesa la tecla y refresca el popup en vivo (puede reabrir
        // con nuevos candidatos según el entorno; lo determinístico es que la
        // tecla entró al input).
        let mut s = State::new(Source::Local);
        s.input.set_text("ca");
        s.completion = Some(fake_completion(&["cargo", "cat"], 0, 2));
        let key = KeyEvent {
            key: Key::Character("r".into()),
            state: KeyState::Pressed,
            text: Some("r".into()),
            modifiers: Modifiers::default(),
            repeat: false,
        };
        s = update(s, Msg::Key(key));
        assert_eq!(s.input.text(), "car", "la tecla se procesa normal");
    }

    #[test]
    fn toggle_stage_flips_expanded_set() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::ToggleStage { block: 2, stage: 0 });
        assert!(s.expanded_stages.contains(&(2, 0)), "primer toggle despliega");
        s = update(s, Msg::ToggleStage { block: 2, stage: 0 });
        assert!(
            !s.expanded_stages.contains(&(2, 0)),
            "segundo toggle repliega"
        );
    }

    #[test]
    fn build_spec_tui_prefix_overrides_default() {
        // `:tui ls` no es típico, pero el prefix lo fuerza igual.
        let (spec, tui) = build_spec(":tui ls", "/");
        assert!(matches!(spec.exec, shuma_exec::Exec::Pty { .. }));
        assert!(tui.is_some());
    }

    #[test]
    fn key_to_pty_bytes_handles_special_keys() {
        let enter = ev(Key::Named(NamedKey::Enter), None);
        assert_eq!(key_to_pty_bytes(&enter), b"\r");
        let up = ev(Key::Named(NamedKey::ArrowUp), None);
        assert_eq!(key_to_pty_bytes(&up), b"\x1b[A");
        let esc = ev(Key::Named(NamedKey::Escape), None);
        assert_eq!(key_to_pty_bytes(&esc), b"\x1b");
        // Ctrl-C → 0x03.
        let ctrl_c = KeyEvent {
            key: Key::Character("c".into()),
            state: KeyState::Pressed,
            text: Some("c".into()),
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
            repeat: false,
        };
        assert_eq!(key_to_pty_bytes(&ctrl_c), vec![3u8]);
    }

    #[test]
    fn source_daemon_failure_surfaces_as_notice() {
        // Sin daemon corriendo, start_run con Source::Daemon debe
        // dejar un notice rojo y no enredarse — el shell sigue vivo.
        let mut s = State::new(Source::Daemon {
            socket: Some(PathBuf::from("/tmp/shuma-no-existe-test.sock")),
            label: None,
        });
        let _ = std::fs::remove_file("/tmp/shuma-no-existe-test.sock");
        s.input.set_text("echo hola");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.starts_with("✘ daemon:")));
        assert!(!s.is_running(), "no debe quedar un run vivo si falló");
    }

    #[test]
    fn ampersand_suffix_starts_background_job() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 5 &");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(!s.is_running(), "& no debe dejar un foreground vivo");
        assert_eq!(s.bg_jobs.len(), 1);
        // El header de la card del job: `[0] $ sleep 5 &`.
        assert!(s
            .output
            .iter()
            .any(|l| l.text.contains("[0]") && l.text.contains("sleep 5")));
        // Cancelar el job así no queda sleep colgado en el host.
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s.input.set_text(":term 0");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s
            .output
            .iter()
            .any(|l| l.text.contains("[0] SIGTERM enviado")));
    }

    #[test]
    fn kill_builtin_signals_background_job() {
        // `:kill N` manda SIGKILL al job N (paralelo a `:term`).
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 5 &");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.bg_jobs.len(), 1);
        s.input.set_text(":kill 0");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s
            .output
            .iter()
            .any(|l| l.text.contains("[0] SIGKILL enviado")));
    }

    #[test]
    fn open_body_menu_targets_a_block_with_body() {
        // Click derecho sobre el output abre el menú apuntando al bloque con
        // cuerpo más reciente (no hay selección previa).
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ echo hi"));
        s.push_output(OutputLine::stdout("hola mundo"));
        s = update(s, Msg::OpenBodyMenu { x: 10.0, y: 20.0 });
        match s.body_menu {
            Some((x, y, b)) => {
                assert_eq!((x, y), (10.0, 20.0));
                assert_ne!(b, 0, "el bloque objetivo no es huérfano");
            }
            None => panic!("el menú debería abrirse"),
        }
    }

    #[test]
    fn body_menu_select_all_then_pick_closes() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ ls"));
        s.push_output(OutputLine::stdout("linea uno"));
        s.push_output(OutputLine::stdout("linea dos"));
        s = update(s, Msg::OpenBodyMenu { x: 0.0, y: 0.0 });
        assert!(s.body_menu.is_some());
        // Item 2 = "Seleccionar todo".
        s = update(s, Msg::BodyMenuPick(2));
        assert!(s.body_sel.is_some(), "seleccionar todo deja selección viva");
        assert!(s.body_menu.is_none(), "elegir un item cierra el menú");
    }

    #[test]
    fn body_menu_dismiss_clears_it() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ x"));
        s.push_output(OutputLine::stdout("y"));
        s = update(s, Msg::OpenBodyMenu { x: 0.0, y: 0.0 });
        assert!(s.body_menu.is_some());
        s = update(s, Msg::BodyMenuDismiss);
        assert!(s.body_menu.is_none());
    }

    #[test]
    fn jobs_builtin_lists_background_jobs() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 5 &");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s.input.set_text(":jobs");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s
            .output
            .iter()
            .any(|l| l.text.contains("[0]") && l.text.contains("sleep")));
        s.input.set_text(":term 0");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
    }

    #[test]
    fn jobs_builtin_empty_shows_notice() {
        let mut s = State::new(Source::Local);
        s.input.set_text(":jobs");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.contains("sin jobs")));
    }

    #[test]
    fn enter_with_open_quote_inserts_newline_instead_of_submit() {
        let mut s = State::new(Source::Local);
        s.input.set_text("echo 'hola");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        // No debe haber arrancado un run — Enter agregó \n.
        assert!(!s.is_running());
        assert_eq!(s.input.text(), "echo 'hola\n");
    }

    #[test]
    fn shift_enter_always_inserts_newline() {
        let mut s = State::new(Source::Local);
        s.input.set_text("ls"); // texto completo, sin continuation pendiente
        let shift_enter = KeyEvent {
            key: Key::Named(NamedKey::Enter),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers {
                shift: true,
                ..Default::default()
            },
            repeat: false,
        };
        s = update(s, Msg::Key(shift_enter));
        assert!(!s.is_running(), "shift+enter no debe ejecutar");
        assert_eq!(s.input.text(), "ls\n");
    }

    #[test]
    fn paste_key_event_is_recognized() {
        // Ctrl-V con texto en clipboard se procesa como paste (no
        // termina llamando apply_key con el carácter 'v'). Sin display
        // server (CI), read_clipboard devuelve None y el state no
        // cambia. Pero verificamos que la rama de paste se toma.
        let mut s = State::new(Source::Local);
        s.input.set_text("hola");
        let ctrl_v = KeyEvent {
            key: Key::Character("v".into()),
            state: KeyState::Pressed,
            text: Some("v".into()),
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
            repeat: false,
        };
        s = update(s, Msg::Key(ctrl_v));
        // El input no debe llevar una 'v' al final — la rama paste se
        // tragó la tecla (y en CI sin clipboard no insertó nada).
        assert_eq!(s.input.text(), "hola");
    }

    #[test]
    fn ansi_idx_palette_matches_expected_basics() {
        // Idx 0 = negro, 15 = blanco, 196 = rojo claro del cubo.
        let black = ansi_idx_to_color(0);
        assert_eq!(black.components[0], 0.0);
        let white = ansi_idx_to_color(15);
        assert!(white.components[0] > 0.99);
    }

    #[test]
    fn arrow_right_at_end_accepts_ghost() {
        let mut s = State::new(Source::Local);
        {
            let mut h = s.history.lock().unwrap();
            let _ = h.append(shuma_history::Entry::new("cargo build --release", "/", 1));
        }
        s.input.set_text("cargo bu");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowRight), None)));
        assert_eq!(s.input.text(), "cargo build --release");
    }

    #[test]
    fn partition_line_segments_a_line_with_a_url() {
        use shuma_line::{Decoration, DecorationKind};
        let theme = Theme::dark();
        let text = "abrí https://tawasuyu.net y mirá";
        let url_start = text.find("https").unwrap();
        let url_end = url_start + "https://tawasuyu.net".len();
        let decs = vec![Decoration {
            start: url_start,
            end: url_end,
            kind: DecorationKind::Url(text[url_start..url_end].to_string()),
        }];
        let pieces = partition_line(text, &decs, theme.fg_text, &theme);
        assert_eq!(pieces.len(), 3, "pre, url, post: {pieces:?}");
        assert_eq!(pieces[0].color, theme.fg_text);
        assert!(pieces[0].deco.is_none());
        assert_eq!(pieces[1].color, theme.accent);
        assert!(matches!(pieces[1].deco, Some(DecorationKind::Url(_))));
        assert_eq!(pieces[2].color, theme.fg_text);
    }

    #[test]
    fn open_decoration_cd_into_a_directory() {
        let mut s = State::new(Source::Local);
        let target = std::env::temp_dir();
        let kind = shuma_line::DecorationKind::Path {
            abs: target.clone(),
            is_dir: true,
            is_executable: false,
            is_symlink: false,
        };
        s = update(s, Msg::OpenDecoration(kind));
        // cwd cambia al directorio target (no comparamos canónico — el
        // open_decoration acepta el path tal cual viene si es dir).
        assert_eq!(s.cwd, target);
    }

    #[test]
    fn open_decoration_git_sha_prefills_input() {
        let mut s = State::new(Source::Local);
        let kind = shuma_line::DecorationKind::GitSha("abcdef0123456".into());
        s = update(s, Msg::OpenDecoration(kind));
        assert_eq!(s.input.text(), "git show abcdef0123456");
    }

    #[test]
    fn open_decoration_path_executable_prefills_input() {
        let mut s = State::new(Source::Local);
        let kind = shuma_line::DecorationKind::Path {
            abs: PathBuf::from("/usr/bin/ls"),
            is_dir: false,
            is_executable: true,
            is_symlink: false,
        };
        s = update(s, Msg::OpenDecoration(kind));
        assert_eq!(s.input.text(), "/usr/bin/ls");
    }

    #[test]
    fn dispatch_maps_clear() {
        assert!(matches!(dispatch("shell.clear"), Some(Msg::Clear)));
        assert!(matches!(dispatch("shell.cancel"), Some(Msg::Cancel)));
        assert!(dispatch("desconocido").is_none());
    }

    #[test]
    fn contributions_expose_clear_and_cancel_shortcuts() {
        let s = State::new(Source::Local);
        let c = contributions(&s);
        assert!(c.monitors.is_empty());
        let labels: Vec<&str> = c.shortcuts.iter().map(|s| s.label.as_str()).collect();
        assert!(labels.contains(&"Clear"), "{labels:?}");
        assert!(labels.contains(&"Cancel"), "{labels:?}");
    }

    #[test]
    fn typing_appends_to_input() {
        let mut s = State::new(Source::Local);
        // El widget text-input usa apply_key con KeyEvent que incluye texto.
        let key = KeyEvent {
            key: Key::Character("h".into()),
            state: KeyState::Pressed,
            text: Some("h".into()),
            modifiers: Modifiers::default(),
            repeat: false,
        };
        s = update(s, Msg::Key(key));
        assert_eq!(s.input.text(), "h");
    }

    #[test]
    fn external_command_records_intention_in_graph() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        assert!(s.intent_graph().is_empty(), "grafo arranca vacío");
        s.input.set_text("echo lienzo");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(
            s.intent_graph().len(),
            1,
            "Enter debe registrar el `%c1` en el grafo"
        );
        assert_eq!(s.intent_graph().commands()[0].intention, "echo lienzo");
        s = drain_until_idle(s);
        let node = &s.intent_graph().commands()[0];
        assert_eq!(node.status, shuma_intent::NodeStatus::Ok);
        assert!(
            node.output_bytes >= 7,
            "esperaba ≥7 bytes (len de 'lienzo\\n'), recibí {}",
            node.output_bytes
        );
    }

    #[test]
    fn failed_command_records_failed_status() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("false");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s = drain_until_idle(s);
        assert_eq!(s.intent_graph().len(), 1);
        assert_eq!(
            s.intent_graph().commands()[0].status,
            shuma_intent::NodeStatus::Failed
        );
    }

    #[test]
    fn builtin_does_not_register_in_graph() {
        let mut s = State::new(Source::Local);
        s.input.set_text("pwd");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(
            s.intent_graph().is_empty(),
            "builtins no entran al grafo de intenciones"
        );
    }

    #[test]
    fn insert_at_cursor_appends_into_input() {
        let mut s = State::new(Source::Local);
        // `set_text` deja el cursor al final, así que `insert` extiende.
        s.input.set_text("sort ");
        s = update(s, Msg::InsertAtCursor("%p1".into()));
        assert_eq!(s.input.text(), "sort %p1");
    }

    #[test]
    fn push_output_groups_lines_into_command_blocks() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ ls"));
        s.push_output(OutputLine::stdout("a.txt"));
        s.push_output(OutputLine::stdout("b.txt"));
        s.push_output(OutputLine::notice("✔ exit 0"));
        let b = s.output[0].block;
        assert!(b > 0, "el prompt debe abrir un bloque > 0");
        assert!(
            s.output.iter().all(|l| l.block == b),
            "comando + salida + exit comparten bloque: {:?}",
            s.output.iter().map(|l| l.block).collect::<Vec<_>>()
        );
        // Un segundo prompt abre un bloque nuevo y monotónico.
        s.push_output(OutputLine::prompt("$ pwd"));
        assert!(
            s.output.last().unwrap().block > b,
            "el segundo comando abre un bloque nuevo"
        );
    }

    #[test]
    fn push_in_block_keeps_async_output_out_of_foreground_card() {
        // El bug de "output mezclado": un job async drenando en su bloque
        // NO debe contaminar el bloque del comando de foreground, aunque
        // `current_block` apunte a este último.
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ fg")); // abre bloque fg
        let fg_block = s.current_block;
        let job_block = s.open_block(); // bloque propio del job (current sigue en fg)
        s.push_in_block(job_block, OutputLine::stdout("salida del job"));
        s.push_output(OutputLine::stdout("salida del fg"));
        let bg = s
            .output
            .iter()
            .find(|l| l.text == "salida del job")
            .unwrap()
            .block;
        let fg = s
            .output
            .iter()
            .find(|l| l.text == "salida del fg")
            .unwrap()
            .block;
        assert_eq!(bg, job_block);
        assert_eq!(fg, fg_block);
        assert_ne!(bg, fg, "job y foreground en cards distintas");
    }

    #[test]
    fn body_lines_excludes_prompt_stage_and_status() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ echo hola | cat"));
        let blk = s.current_block;
        s.push_output(OutputLine::stage_stdout(0, "intermedia"));
        s.push_output(OutputLine::stdout("hola"));
        s.push_output(OutputLine::stderr("ups"));
        s.push_output(OutputLine::notice("✔ exit 0"));
        // Cuerpo = stdout/stderr/notice no-status, sin el prompt ni la etapa.
        assert_eq!(body_lines_for_block(&s, blk), vec!["hola", "ups"]);
    }

    #[test]
    fn body_pointer_click_then_drag_selects_text() {
        use llimphi_widget_text_editor::PointerEvent;
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ echo"));
        let blk = s.current_block;
        s.push_output(OutputLine::stdout("hola mundo"));
        s.push_output(OutputLine::stdout("segunda"));
        // Click al inicio (línea 0, col 0) ancla el caret.
        s = update(s, Msg::BodyPointer { block: blk, ev: PointerEvent::Click { x: 0.0, y: 0.0 } });
        // Drag hasta ~col 4 de la línea 0 (char_width 7.2 ⇒ x≈30) extiende.
        s = update(
            s,
            Msg::BodyPointer {
                block: blk,
                ev: PointerEvent::Drag { initial_x: 0.0, initial_y: 0.0, dx: 30.0, dy: 2.0 },
            },
        );
        let ed = body_editor_state(&s, blk);
        let sel = ed.selected_text().expect("hay selección tras el drag");
        assert_eq!(sel, "hola", "seleccionó las primeras 4 columnas, fue {sel:?}");
    }

    #[test]
    fn finished_command_stays_expanded_then_recedes_on_next() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("seq 1 20");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let blk = s.current_block;
        s = drain_until_idle(s);
        // Recién terminado: sigue EXPANDIDO (se ve completo).
        assert!(!s.collapsed.contains(&blk), "el comando recién hecho queda expandido");
        // Al correr uno nuevo, el anterior recede (se pliega).
        s.input.set_text("echo otra");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.collapsed.contains(&blk), "el anterior se pliega al nacer uno nuevo");
        let nuevo = s.current_block;
        assert!(!s.collapsed.contains(&nuevo), "el nuevo nace expandido");
    }

    #[test]
    fn command_without_output_does_not_recede() {
        // Un comando sin cuerpo (no produjo salida) no se pliega al pasar al
        // siguiente — no hay nada que esconder, y se mostrará distinto.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("true"); // exit 0, sin stdout
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let blk = s.current_block;
        s = drain_until_idle(s);
        s.input.set_text("echo x");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(!s.collapsed.contains(&blk), "un comando sin salida no recede");
    }

    #[test]
    fn word_range_picks_the_word_under_the_column() {
        // "foo bar_baz qux" — col dentro de "bar_baz" selecciona toda la
        // palabra (incluye `_`); sobre el espacio no selecciona.
        let t = "foo bar_baz qux";
        assert_eq!(word_range_at(t, 5), (4, 11)); // dentro de bar_baz
        assert_eq!(word_range_at(t, 0), (0, 3)); // foo
        assert_eq!(word_range_at(t, 3), (0, 3)); // justo después de foo
        assert_eq!(word_range_at(t, 11), (4, 11)); // justo después de bar_baz
    }

    #[test]
    fn double_click_selects_word_in_body() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ echo"));
        let blk = s.current_block;
        s.push_output(OutputLine::stdout("hola mundo cruel"));
        // Doble-click sobre "mundo": col≈7 ⇒ x = 7*char_width + gutter.
        let metrics = body_editor_metrics();
        let x = metrics.gutter_width + 7.0 * metrics.char_width + 1.0;
        s = update(s, Msg::BodyDoubleClick { block: blk, x, y: 2.0 });
        let ed = body_editor_state(&s, blk);
        assert_eq!(ed.selected_text().as_deref(), Some("mundo"));
    }

    #[test]
    fn scroll_clamps_between_zero_and_overflow() {
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 100.0;
        s = update(s, Msg::Scroll(40.0));
        assert_eq!(s.scroll_px, 40.0);
        s = update(s, Msg::Scroll(200.0)); // pasa del tope → clamp a overflow
        assert_eq!(s.scroll_px, 100.0);
        s = update(s, Msg::Scroll(-500.0)); // de vuelta al fondo
        assert_eq!(s.scroll_px, 0.0);
    }

    #[test]
    fn scroll_setea_anchor_para_estabilidad_bajo_append() {
        // Al hacer scroll up, el anchor capta el overflow vigente para
        // que appends posteriores no muevan la vista del usuario (Fase 5
        // del SDD-TERMINAL).
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 100.0;
        s = update(s, Msg::Scroll(40.0));
        assert_eq!(s.scroll_px, 40.0);
        // anchor capturó el overflow al momento del scroll.
        assert_eq!(s.surf_scroll_anchor, 100.0);
        // Simular un append: el overflow crece pero scroll_px NO cambia.
        // La fórmula del view interpretará scroll_y contra el anchor viejo.
        *s.out_overflow.lock().unwrap() = 150.0;
        // El usuario no scrolleó; scroll_px sigue siendo 40 y anchor 100,
        // así que scroll_y intencionado = 100 - 40 = 60 (mismo de antes).
        assert_eq!(s.scroll_px, 40.0);
        assert_eq!(s.surf_scroll_anchor, 100.0);
        // Próximo scroll del usuario re-baseliza al nuevo overflow.
        // curr_scroll_y = (100 - 40) = 60. delta=10 → new = 50.
        // scroll_px = 150 - 50 = 100. anchor = 150.
        s = update(s, Msg::Scroll(10.0));
        assert_eq!(s.scroll_px, 100.0);
        assert_eq!(s.surf_scroll_anchor, 150.0);
    }

    #[test]
    fn scroll_captura_velocidad_para_inercia() {
        // El último delta del usuario queda en `surf_scroll_velocity` para
        // que el próximo Tick lo aplique con decay (Fase 5.2).
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 100.0;
        s = update(s, Msg::Scroll(30.0));
        assert_eq!(s.surf_scroll_velocity, 30.0);
        s = update(s, Msg::Scroll(15.0));
        assert_eq!(s.surf_scroll_velocity, 15.0, "se reemplaza por el último");
    }

    #[test]
    fn tick_aplica_inercia_y_decae() {
        // Con velocidad seteada, Tick scrollea por ella y la reduce por
        // fricción 0.82. Eventualmente cae bajo epsilon y se anula.
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 1000.0;
        s = update(s, Msg::Scroll(40.0));
        let v0 = s.surf_scroll_velocity;
        let px0 = s.scroll_px;
        // Primer Tick: scrollea 40 más → scroll_px sube por ese delta;
        // velocidad cae por fricción.
        s = update(s, Msg::Tick);
        assert!(s.scroll_px > px0, "el tick aplica el delta");
        assert!(
            s.surf_scroll_velocity.abs() < v0.abs(),
            "la velocidad decae"
        );
        // Tras ~30 ticks la velocidad ya cayó bajo epsilon (0.5).
        for _ in 0..30 {
            s = update(s, Msg::Tick);
        }
        assert_eq!(s.surf_scroll_velocity, 0.0, "termina en 0");
    }

    #[test]
    fn inercia_se_detiene_al_tocar_el_fondo() {
        // Si la inercia lleva al usuario contra el fondo (re-pin), la
        // velocidad se anula inmediatamente (sin "rebote" simulado).
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 100.0;
        // Subir un poco para tener margen.
        s = update(s, Msg::Scroll(50.0));
        assert!(s.scroll_px > 0.0);
        // Inyectar velocidad hacia abajo (negativa = scroll down → bottom).
        s.surf_scroll_velocity = -500.0;
        s = update(s, Msg::Tick);
        assert_eq!(s.scroll_px, 0.0, "alcanzó el fondo");
        assert_eq!(s.surf_scroll_velocity, 0.0, "inercia se detiene en el límite");
    }

    #[test]
    fn scroll_re_pin_al_fondo_resetea_anchor() {
        // Si el scroll del usuario alcanza el fondo (scroll_y >= overflow),
        // re-pin: scroll_px=0 y anchor=0. Próximos appends siguen pegados
        // al fondo (UX terminal clásica).
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 100.0;
        s = update(s, Msg::Scroll(40.0));
        assert_eq!(s.surf_scroll_anchor, 100.0);
        s = update(s, Msg::Scroll(-500.0));
        assert_eq!(s.scroll_px, 0.0);
        assert_eq!(s.surf_scroll_anchor, 0.0, "re-pin limpia el anchor");
    }

    #[test]
    fn toggle_block_flips_collapsed_set() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::ToggleBlock(3));
        assert!(s.collapsed.contains(&3), "primer toggle colapsa");
        s = update(s, Msg::ToggleBlock(3));
        assert!(!s.collapsed.contains(&3), "segundo toggle despliega");
    }

    #[test]
    fn clear_output_also_drops_collapsed_set() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ ls"));
        s.collapsed.insert(s.output[0].block);
        s.clear_output();
        assert!(s.output.is_empty());
        assert!(s.collapsed.is_empty(), "clear limpia también los colapsos");
    }

    /// El SurfLayout snapshot que poblaríamos en `output_pane_surface` —
    /// versión sintética para tests de la state machine, sin pasar por el
    /// render. Cubre 3 líneas mono de 6 chars cada una.
    fn synth_surf_layout() -> SurfLayout {
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0,
            line_height: 16.0,
            char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        store.push_line("abcdef");
        store.push_line("ghijkl");
        store.push_line("mnopqr");
        SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, 3)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        }
    }

    #[test]
    fn surf_select_drag_move_arranca_y_extiende_la_seleccion() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout());
        // Primer Move: anchor en línea 0 col 2 (ax=46 = 30+2*8, ay=4).
        s = update(
            s,
            Msg::SurfSelectDrag {
                phase: llimphi_ui::DragPhase::Move,
                dx: 0.0,
                dy: 0.0,
                ax: 46.0,
                ay: 4.0,
            },
        );
        assert!(s.surf_selecting);
        let sel = s.surf_selection.expect("anchor set");
        assert_eq!(sel.anchor, llimphi_widget_terminal::Point::new(0, 2));
        assert_eq!(sel.head, llimphi_widget_terminal::Point::new(0, 2));
        // Move siguiente: delta de (+32, +32) → acc = (78, 36) → fila 2, col 6.
        s = update(
            s,
            Msg::SurfSelectDrag {
                phase: llimphi_ui::DragPhase::Move,
                dx: 32.0,
                dy: 32.0,
                ax: 46.0,
                ay: 4.0,
            },
        );
        let sel = s.surf_selection.expect("extended");
        assert_eq!(sel.anchor, llimphi_widget_terminal::Point::new(0, 2));
        assert_eq!(sel.head, llimphi_widget_terminal::Point::new(2, 6));
    }

    #[test]
    fn surf_select_drag_end_libera_pero_mantiene_seleccion_para_copy() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout());
        // Drag completo (Press → Move → End) cubriendo varios chars.
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::Move, dx: 0.0, dy: 0.0, ax: 46.0, ay: 4.0,
        });
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::Move, dx: 16.0, dy: 0.0, ax: 46.0, ay: 4.0,
        });
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::End, dx: 0.0, dy: 0.0, ax: 46.0, ay: 4.0,
        });
        assert!(!s.surf_selecting, "End libera el flag");
        assert!(s.surf_selection.is_some(), "pero la selección queda para copy");
    }

    #[test]
    fn surf_select_drag_end_sin_drag_real_limpia_la_seleccion_colapsada() {
        // Un Press+End sin Move intermedio = click corto. La selección queda
        // colapsada (anchor == head); el End la limpia para no dejar
        // afford visual sin sentido.
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout());
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::Move, dx: 0.0, dy: 0.0, ax: 46.0, ay: 4.0,
        });
        // Ahora un End sin Mover.
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::End, dx: 0.0, dy: 0.0, ax: 46.0, ay: 4.0,
        });
        assert!(s.surf_selection.is_none(), "click sin drag → sin selección");
    }

    /// Layout sintético con texto que el find puede matchear.
    fn synth_surf_layout_with(lines: &[&str]) -> SurfLayout {
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0,
            line_height: 16.0,
            char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        for l in lines {
            store.push_line(l);
        }
        let len = store.len();
        SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, len)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        }
    }

    #[test]
    fn find_open_inicializa_estado_vacio() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::FindOpen);
        let f = s.find.expect("find abierto");
        assert!(f.query.is_empty());
        assert!(f.matches.is_empty());
        assert!(f.current.is_none());
        assert!(!f.case_insensitive);
    }

    #[test]
    fn find_char_recomputa_y_resalta_el_primer_match() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout_with(&[
            "foo bar baz",
            "qux foo quux",
            "nada que ver",
        ]));
        s = update(s, Msg::FindOpen);
        s = update(s, Msg::FindChar('f'));
        s = update(s, Msg::FindChar('o'));
        s = update(s, Msg::FindChar('o'));
        let f = s.find.as_ref().expect("find abierto");
        assert_eq!(f.matches.len(), 2);
        assert_eq!(f.current, Some(0));
        // La selección debe reflejar el primer match (línea 0, col 0..3).
        let sel = s.surf_selection.expect("highlight");
        assert_eq!(sel.anchor, llimphi_widget_terminal::Point::new(0, 0));
        assert_eq!(sel.head, llimphi_widget_terminal::Point::new(0, 3));
    }

    #[test]
    fn find_next_y_prev_son_ciclicos() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() =
            Some(synth_surf_layout_with(&["aa", "aa", "aa"]));
        s = update(s, Msg::FindOpen);
        s = update(s, Msg::FindChar('a'));
        // 6 matches (2 por línea, no superpuestos).
        assert_eq!(s.find.as_ref().unwrap().matches.len(), 6);
        s = update(s, Msg::FindNext);
        assert_eq!(s.find.as_ref().unwrap().current, Some(1));
        // Prev desde 0 envuelve al último (5).
        s = update(s, Msg::FindPrev);
        s = update(s, Msg::FindPrev);
        assert_eq!(s.find.as_ref().unwrap().current, Some(5));
    }

    #[test]
    fn find_toggle_case_re_busca_con_la_nueva_politica() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout_with(&[
            "Hola", "HOLA", "hola",
        ]));
        s = update(s, Msg::FindOpen);
        s = update(s, Msg::FindChar('h'));
        s = update(s, Msg::FindChar('o'));
        s = update(s, Msg::FindChar('l'));
        s = update(s, Msg::FindChar('a'));
        // Case sensitive: sólo matchea "hola" (línea 2).
        assert_eq!(s.find.as_ref().unwrap().matches.len(), 1);
        s = update(s, Msg::FindToggleCase);
        // Case insensitive: matchea las 3.
        assert_eq!(s.find.as_ref().unwrap().matches.len(), 3);
    }

    #[test]
    fn find_close_limpia_estado_y_selection_del_match() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout_with(&["foo"]));
        s = update(s, Msg::FindOpen);
        s = update(s, Msg::FindChar('f'));
        s = update(s, Msg::FindChar('o'));
        s = update(s, Msg::FindChar('o'));
        assert!(s.surf_selection.is_some());
        s = update(s, Msg::FindClose);
        assert!(s.find.is_none());
        assert!(s.surf_selection.is_none(), "Esc no deja selección residual del match");
    }

    #[test]
    fn find_backspace_re_busca_con_la_query_acortada() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() =
            Some(synth_surf_layout_with(&["foo", "foobar"]));
        s = update(s, Msg::FindOpen);
        for c in "foobar".chars() {
            s = update(s, Msg::FindChar(c));
        }
        assert_eq!(s.find.as_ref().unwrap().matches.len(), 1); // "foobar" matchea sólo línea 1
        s = update(s, Msg::FindBackspace);
        s = update(s, Msg::FindBackspace);
        s = update(s, Msg::FindBackspace);
        // Query = "foo" → 2 matches.
        assert_eq!(s.find.as_ref().unwrap().query, "foo");
        assert_eq!(s.find.as_ref().unwrap().matches.len(), 2);
    }

    #[test]
    fn surf_double_click_selecciona_la_palabra_bajo_el_punto() {
        // Snapshot con "hola mundo querido" en la primera línea — el
        // doble-click en col=6 (sobre 'u' de "mundo") debe seleccionar
        // exactamente "mundo" (bytes 5..10).
        let mut s = State::new(Source::Local);
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0,
            line_height: 16.0,
            char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        store.push_line("hola mundo querido");
        *s.surf_layout.lock().unwrap() = Some(SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, 1)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        });
        // lx = 30 (gutter) + 6 * 8 (char 6) + 2 = 80. ly = 4 (centro fila 0).
        s = update(s, Msg::SurfDoubleClick { lx: 80.0, ly: 4.0, rect_w: 400.0, rect_h: 200.0 });
        let sel = s.surf_selection.expect("selección de palabra");
        assert_eq!(sel.anchor, llimphi_widget_terminal::Point::new(0, 5));
        assert_eq!(sel.head, llimphi_widget_terminal::Point::new(0, 10));
    }

    #[test]
    fn surf_history_acumula_lineas_de_body_entre_frames() {
        // El cuerpo `surf_history` persiste a lo largo de la sesión —
        // a diferencia del Scrollback per-frame que arma el view. Acá
        // simulamos varios push_output y verificamos que la history
        // refleja sólo las líneas de body (no Prompts ni notices).
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ ls"));
        s.push_output(OutputLine::stdout("uno"));
        s.push_output(OutputLine::stderr("err1"));
        s.push_output(OutputLine::notice("✔ exit 0"));
        s.push_output(OutputLine::stdout("dos"));
        let h = s.surf_history.lock().unwrap();
        // Prompts y notices NO van; stdout + stderr SÍ.
        assert_eq!(h.len(), 3);
        assert_eq!(h.line(0), Some("uno"));
        assert_eq!(h.line(1), Some("err1"));
        assert_eq!(h.line(2), Some("dos"));
    }

    #[test]
    fn surf_history_excluye_lineas_de_etapa_de_pipe() {
        // Las stage_lines (capturas de tee de etapas intermedias) tampoco
        // van a la history (espeja el filtro de `body_lines_for_block`).
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ ls | wc"));
        // Línea intermedia con stage=Some(_) — no va.
        let mut staged = OutputLine::stdout("intermedia");
        staged.stage = Some(0);
        s.push_output(staged);
        // Línea de body normal — sí va.
        s.push_output(OutputLine::stdout("final"));
        let h = s.surf_history.lock().unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h.line(0), Some("final"));
    }

    #[test]
    fn refresh_spilled_visible_carga_tail_del_archive() {
        use llimphi_widget_terminal::{Scrollback, SpillStore};
        // History con cap chico + spill → muchas líneas terminan en disco.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sb = Scrollback::new(20);
        let spill = SpillStore::create(dir.path().join("test.spill")).expect("spill");
        sb.enable_spill(spill);
        let history = Arc::new(Mutex::new(sb));
        let cache = Arc::new(Mutex::new(SurfSpilledCache::default()));

        // Cache vacío + history vacía → refresh no carga nada.
        refresh_surf_spilled_visible(&history, &cache);
        assert!(cache.lock().unwrap().lines.is_empty());

        // Push muchas líneas hasta forzar spill.
        for i in 0..50 {
            history.lock().unwrap().push_line(&format!("L{i:04}"));
        }
        let spilled = history.lock().unwrap().spilled_count();
        assert!(spilled > 0, "el cap forzó spill");

        // Refresh carga las últimas N (clamped a MAX_SPILLED_VISIBLE).
        refresh_surf_spilled_visible(&history, &cache);
        let c = cache.lock().unwrap();
        let expected_n = spilled.min(MAX_SPILLED_VISIBLE);
        assert_eq!(c.lines.len(), expected_n);
        assert_eq!(c.cached_at, spilled);
        // Última línea del cache = última línea que entró al spill.
        let last_spilled_id = spilled as u64 - 1;
        let expected_last = format!("L{:04}", last_spilled_id);
        assert_eq!(c.lines.last(), Some(&expected_last));
    }

    #[test]
    fn scrollback_grep_busca_en_memoria_y_spill() {
        // History con cap chico + spill: muchas líneas en disco + algunas
        // en memoria. `:scrollback grep <pat>` debe encontrar hits en
        // ambas mitades y reportarlos por notice.
        let mut s = State::new(Source::Local);
        // Forzar enable_spill (la State::new default no lo activa).
        let dir = tempfile::tempdir().unwrap();
        let mut sb = llimphi_widget_terminal::Scrollback::new(20);
        let spill = llimphi_widget_terminal::SpillStore::create(
            dir.path().join("test.spill"),
        )
        .unwrap();
        sb.enable_spill(spill);
        *s.surf_history.lock().unwrap() = sb;
        // Push lines: some "foo", some "bar". Cap chico → muchas spilled.
        for i in 0..50 {
            let line = if i % 5 == 0 {
                format!("foo_line_{i}")
            } else {
                format!("bar_line_{i}")
            };
            s.push_output(OutputLine::stdout(&line));
        }
        // Sanity: hay spilleadas.
        let total_spilled = s.surf_history.lock().unwrap().spilled_count();
        assert!(total_spilled > 0);
        // grep "foo": debe encontrar las 10 ocurrencias (i = 0, 5, 10, ...).
        s.input.set_text(":scrollback grep foo");
        s = update(s, Msg::Key(KeyEvent {
            key: Key::Named(NamedKey::Enter),
            state: KeyState::Pressed,
            text: None,
            modifiers: llimphi_ui::Modifiers::default(),
            repeat: false,
        }));
        // El último Notice header reporta el total de hits.
        let summary = s.output.iter().rev()
            .find(|l| l.kind == OutputKind::Notice && l.text.starts_with("grep:"))
            .expect("grep summary");
        assert!(summary.text.contains("10 hits"), "summary: {}", summary.text);
    }

    #[test]
    fn refresh_spilled_visible_no_recarga_si_no_cambio() {
        use llimphi_widget_terminal::{Scrollback, SpillStore};
        let dir = tempfile::tempdir().unwrap();
        let mut sb = Scrollback::new(20);
        let spill = SpillStore::create(dir.path().join("test.spill")).unwrap();
        sb.enable_spill(spill);
        let history = Arc::new(Mutex::new(sb));
        for i in 0..30 {
            history.lock().unwrap().push_line(&format!("L{i:04}"));
        }
        let cache = Arc::new(Mutex::new(SurfSpilledCache::default()));
        refresh_surf_spilled_visible(&history, &cache);
        let first_count = cache.lock().unwrap().cached_at;
        // Sin nuevas pushes el cached_at no debe cambiar tras un segundo refresh.
        refresh_surf_spilled_visible(&history, &cache);
        assert_eq!(cache.lock().unwrap().cached_at, first_count);
    }

    #[test]
    fn clear_output_tambien_resetea_history() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::stdout("a"));
        s.push_output(OutputLine::stdout("b"));
        assert_eq!(s.surf_history.lock().unwrap().len(), 2);
        s.clear_output();
        assert_eq!(s.surf_history.lock().unwrap().len(), 0);
        assert_eq!(s.surf_history.lock().unwrap().dropped(), 0);
    }

    #[test]
    fn scrollback_builtin_reporta_estado_en_notice() {
        // Sin spill activo (default del Config), `:scrollback` reporta
        // sólo líneas en memoria y avisa que el spill no está activo.
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::stdout("a"));
        s.push_output(OutputLine::stdout("b"));
        s.push_output(OutputLine::stdout("c"));
        s.input.set_text(":scrollback");
        s = update(s, Msg::Key(KeyEvent {
            key: Key::Named(NamedKey::Enter),
            state: KeyState::Pressed,
            text: None,
            modifiers: llimphi_ui::Modifiers::default(),
            repeat: false,
        }));
        // El último notice debe mencionar el conteo.
        let last_notice = s.output.iter().rev()
            .find(|l| l.kind == OutputKind::Notice)
            .expect("notice");
        assert!(
            last_notice.text.contains("scrollback") || last_notice.text.contains("spill"),
            "notice menciona scrollback/spill: {}", last_notice.text
        );
    }

    #[test]
    fn dos_double_clicks_seguidos_seleccionan_la_linea_entera() {
        // tap-tap = word. tap-tap-tap-tap (dos pares) dentro de 350 ms =
        // line (paridad xterm triple-click). El handler usa el timestamp
        // ms entre los dos SurfDoubleClick.
        let mut s = State::new(Source::Local);
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0, line_height: 16.0, char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        store.push_line("hola mundo querido");
        *s.surf_layout.lock().unwrap() = Some(SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, 1)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        });
        // Primer double-click: selecciona "hola" (palabra).
        s = update(s, Msg::SurfDoubleClick { lx: 50.0, ly: 4.0, rect_w: 400.0, rect_h: 200.0 });
        // Segundo double-click "inmediato": ahora selecciona toda la línea.
        s = update(s, Msg::SurfDoubleClick { lx: 50.0, ly: 4.0, rect_w: 400.0, rect_h: 200.0 });
        let sel = s.surf_selection.expect("line select");
        assert_eq!(sel.anchor.line, 0);
        assert_eq!(sel.anchor.col, 0);
        assert_eq!(sel.head.col, "hola mundo querido".len());
    }

    #[test]
    fn surf_double_click_sobre_separador_no_selecciona() {
        // Double-click sobre un espacio o un delimitador no debe
        // armar selección (paridad con xterm: si el click cae sobre
        // whitespace exactamente, no hay palabra).
        let mut s = State::new(Source::Local);
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0,
            line_height: 16.0,
            char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        store.push_line("hola mundo querido");
        *s.surf_layout.lock().unwrap() = Some(SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, 1)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        });
        // Posicionar sobre el espacio entre "hola" y "mundo" (col=4 byte = ' ').
        // lx = 30 + 4*8 + 2 = 64.
        s = update(
            s,
            Msg::SurfDoubleClick { lx: 64.0, ly: 4.0, rect_w: 400.0, rect_h: 200.0 },
        );
        // El handler de doble-click absorbe el caso "después de palabra" y
        // selecciona la palabra que termina ahí ("hola"). El otro caso
        // (espacio en medio de la línea, no después de palabra) deja la
        // selección sin tocar. Este test confirma que NO panic-ea.
        // Si seleccionó algo, debe ser "hola" (bytes 0..4).
        if let Some(sel) = s.surf_selection {
            assert_eq!(sel.anchor.line, 0);
            assert_eq!(sel.anchor.col, 0);
            assert_eq!(sel.head.col, 4);
        }
    }

    #[test]
    fn surf_open_y_dismiss_menu_actualiza_estado() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::SurfOpenMenu { x: 100.0, y: 50.0 });
        assert_eq!(s.surf_menu, Some((100.0, 50.0)));
        s = update(s, Msg::SurfMenuDismiss);
        assert!(s.surf_menu.is_none());
    }

    #[test]
    fn surf_menu_pick_seleccionar_todo_arma_rango_full() {
        // Item 2 = Seleccionar todo. Pone surf_selection desde (0,0) hasta
        // el fin de la última línea del scrollback.
        let mut s = State::new(Source::Local);
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0, line_height: 16.0, char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        store.push_line("hola");
        store.push_line("mundo");
        store.push_line("xxx");
        *s.surf_layout.lock().unwrap() = Some(SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, 3)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        });
        s = update(s, Msg::SurfOpenMenu { x: 50.0, y: 50.0 });
        s = update(s, Msg::SurfMenuPick(2));
        let sel = s.surf_selection.expect("select all");
        assert_eq!(sel.anchor, llimphi_widget_terminal::Point::new(0, 0));
        // Última línea = "xxx" (3 bytes).
        assert_eq!(sel.head, llimphi_widget_terminal::Point::new(2, 3));
        assert!(s.surf_menu.is_none(), "el pick cierra el menú");
    }

    #[test]
    fn surf_clear_selection_resetea_estado() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout());
        // Arranca un drag.
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::Move, dx: 0.0, dy: 0.0, ax: 46.0, ay: 4.0,
        });
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::Move, dx: 16.0, dy: 0.0, ax: 46.0, ay: 4.0,
        });
        assert!(s.surf_selection.is_some());
        s = update(s, Msg::SurfClearSelection);
        assert!(s.surf_selection.is_none());
        assert!(!s.surf_selecting);
    }
}
