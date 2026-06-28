use super::*;

/// Tipo de cada línea del buffer — define el color que la `view` usa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OutputKind {
    /// El comando tal como lo tipeó el usuario (precede a su output).
    Prompt,
    /// stdout del comando.
    Stdout,
    /// stderr del comando.
    Stderr,
    /// Mensaje del shell mismo (cd, error de spawn, exit status, etc.).
    Notice,
    /// Respuesta del LLM (`:explica`/`:resume`/`:filtra`/`:hacé` cuando produce
    /// texto). Se trata como **salida de primera clase**: es parte del cuerpo
    /// del bloque, se tiñe distinto y la recogen `gather_block_text` + los
    /// redireccionadores (`%cN`, `:write`, `:yank`, `:filtra`) — así una
    /// respuesta de IA se puede volver a filtrar, guardar o encadenar.
    Ai,
}

/// Una línea del buffer de output con su tipo (para coloreado) y el
/// bloque de comando al que pertenece. El render agrupa las líneas con
/// el mismo `block` en una *card* desplegable (un `$ cmd` + su salida +
/// su exit status). `block == 0` = líneas sueltas sin comando dueño.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    /// Línea de respuesta del LLM (`OutputKind::Ai`) — cuerpo de primera clase,
    /// redireccionable y re-filtrable como cualquier stdout.
    pub fn ai(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Ai,
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
    /// Como [`try_events`], pero limitado a `max` eventos por tick. El resto
    /// queda en la cola del backend para el próximo llamado. Necesario para
    /// no pasmar el render con ráfagas grandes (`ls -alR`, builds verbose).
    pub fn try_events_limit(&mut self, max: usize) -> Vec<RunEvent> {
        match self {
            BackendHandle::Local(h) => h.try_events_limit(max),
            // El backend remoto todavía drena todo de una; cuando soporte
            // límite, encadenamos. Mientras tanto, el limit es un techo
            // suave (no rompe nada, solo no rinde igual con un remoto
            // que escupe rápido).
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

/// Estado de actividad de una sesión/panel — alimenta el aviso visual
/// (color del LED en el diente y en la tab). Tres signos que el usuario pidió:
/// quieto, con movimiento (algo corriendo / saliendo output) y claude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// Sin comando en curso ni novedad — prompt ocioso.
    Idle,
    /// Hay un comando corriendo (foreground): movimiento.
    Busy,
    /// La sesión corre `claude` (TUI) — color propio para distinguirla.
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
pub(crate) const PTY_ROWS: u16 = 24;
pub(crate) const PTY_COLS: u16 = 80;

/// Tabla de comandos que pedimos PTY automáticamente. Otros pueden
/// pedirlo con el prefijo `:tui ...`.
pub(crate) const TUI_ALLOWLIST: &[&str] = &[
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
    /// lines.len() - 1`. Cap a [`MAX_SPILLED_LOADED`].
    pub lines: Vec<String>,
    /// Global id de la primera línea cacheada (la más vieja del cache).
    pub first_id: u64,
    /// `spilled_count` al momento del último refresh — para detectar staleness.
    pub cached_at: usize,
    /// Inicio deseado de la ventana del archive (Fase 5.12 — paginado al
    /// scrollear hacia arriba). `None` = ventana "cola" automática (las
    /// últimas [`MAX_SPILLED_VISIBLE`], liviana, sigue el final cuando spillea
    /// más). `Some(id)` = el usuario paginó hacia atrás: la ventana arranca en
    /// `id` (clampeado a no más de [`MAX_SPILLED_LOADED`] desde el final) y se
    /// congela ahí hasta que vuelva al fondo. Lo mueve `apply_scroll_delta`.
    pub window_start: Option<u64>,
}

/// Tope de líneas spilleadas que la ventana "cola" muestra de entrada
/// (pegadas al buffer vivo). Más atrás se carga paginando al scrollear.
/// ~30 KB para líneas típicas de 150 chars.
pub const MAX_SPILLED_VISIBLE: usize = 200;

/// Tope duro de líneas spilleadas cargadas a la vez al paginar hacia atrás
/// (acota memoria + tiempo de refresh). Más viejo que esto → `:scrollback
/// open`. ~300 KB para líneas típicas.
pub const MAX_SPILLED_LOADED: usize = 2000;

/// Cuántas líneas más viejas carga cada paginación al tocar el tope.
pub const SPILL_PAGE: usize = 200;

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
    /// E1 — libro de macros parametrizables (`:macro`). Cargado de
    /// `~/.config/shuma/macros.toml` al arrancar; cada `:macro save`/`rm`
    /// lo reescribe. Las macros se instancian sustituyendo `%1..%9` por los
    /// argumentos de `:macro run`.
    pub macro_book: shuma_intent::MacroBook,
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
    /// Factor de zoom del texto del shell (1.0 = default). Ctrl+rueda lo
    /// ajusta. Aplicado al `font_size`, `row_h` y `char_width` de la
    /// superficie de output. Bounded [0.5, 3.0] al renderizar.
    pub font_zoom: f32,
    /// Offset horizontal del scroll del shell en px (≥ 0). Útil cuando
    /// el zoom-in hace que las líneas excedan el viewport — Shift+rueda
    /// mueve este valor. El gutter queda fijo; el texto se desplaza.
    pub surf_scroll_x: f32,
    /// A qué recibe el Enter de la línea: `None` = arrancar un comando
    /// nuevo (la "línea"); `Some(block)` = mandar la línea por stdin al
    /// comando vivo de ese bloque. Permite responder prompts de varios
    /// comandos en paralelo, alternando con click/hover sobre su card. Se
    /// fija al arrancar un comando, al hacer click/hover en su card o en la
    /// línea, y se limpia cuando ese comando cierra.
    pub input_focus: Option<u64>,
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
    /// A1 — firmas de coreografías que el usuario **descartó** (chip «descartar»):
    /// no se vuelven a ofrecer como grupo en esta sesión. Sólo en memoria.
    pub dismissed_choreo: std::collections::HashSet<Vec<String>>,
    /// A2 — líneas largas para las que el usuario **descartó** el alias ofrecido:
    /// no se vuelven a ofrecer en esta sesión. Sólo en memoria (el aceptado, en
    /// cambio, queda aprendido al shumarc).
    pub dismissed_alias: std::collections::HashSet<String>,
    /// A4 — corrección «¿quisiste decir…?» por bloque: cuando un comando falla
    /// con `command not found`, el binario más cercano (Levenshtein) sobre la
    /// línea original. Un notice clickeable bajo el bloque la lleva al input.
    /// Sólo en memoria.
    pub did_you_mean: std::collections::HashMap<u64, String>,
    /// A6 — cuántos comandos largos (≥ `[rules].on_long_command_secs`)
    /// terminaron **sin que el usuario los acuse**. El chasis lo lee para pintar
    /// la badge en el diente de la sesión cuando no está activa, y lo pone en
    /// cero al volver a ella ([`State::ack_long_alerts`]). `0` = nada pendiente.
    /// Sólo en memoria.
    pub long_alerts: usize,
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
    /// Primer bloque «marcado» para cotejar de un clic (el chip ⇄ del header).
    /// Con otro bloque ya marcado, el segundo clic dispara `:compara %cA %cB`
    /// entre ambos y vuelve a `None`. `None` = sin ancla de comparación.
    pub compare_anchor: Option<u64>,
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
    /// Momento de creación de cada bloque (unix secs) — alimenta el badge
    /// de "hace N minutos" en vez del crudo "exit N". Lo setea
    /// [`State::push_output`] (Prompt) y [`State::open_block`].
    pub block_started: std::collections::HashMap<u64, u64>,
    /// Momento de cierre de cada bloque (unix secs) — lo setea el cierre del
    /// run (notice `✔/✘`). Con [`block_started`] da la duración que alimenta
    /// el titular semáforo del header colapsado. Sólo vive en memoria: no se
    /// persiste (una sesión restaurada no muestra duración en sus bloques).
    pub block_ended: std::collections::HashMap<u64, u64>,
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
    /// E3 — guarda de re-entrada de `[rules].on_exit_nonzero`: se arma en
    /// cada submit del usuario y se desarma al disparar la regla, para que
    /// el propio comando de la regla (si también falla) no la re-dispare.
    pub exit_rule_fired: bool,
    /// E3 — guarda de re-entrada de `[rules].on_enter_cwd`: evita que el
    /// comando de una regla de cwd que a su vez haga `cd` re-dispare reglas.
    pub in_cwd_rule: bool,
    /// E5 — petición al LLM pendiente que el host (chasis) debe cumplir:
    /// el módulo no habla con la red, sólo expresa la intención (`:?`/
    /// `:explica`/`:resume`). El chasis la toma con [`State::take_llm_request`],
    /// corre `pluma-llm` en un thread y devuelve `Msg::LlmResult`. `None`
    /// salvo entre la invocación y su resultado.
    pub llm_request: Option<LlmRequest>,
    /// `true` mientras una petición LLM está en vuelo (la tomó el host) —
    /// evita que el host la re-dispare en cada tick.
    pub llm_inflight: bool,
    /// Header del bloque donde aterrizará la respuesta de un `LlmKind::Text`
    /// (`:explica`/`:resume`/`:filtra`): la respuesta abre su **propio bloque**
    /// referenciable (`%cM`) en vez de mezclarse con el bloque actual, para que
    /// se pueda volver a filtrar/guardar/encadenar. `None` para `:?`/`:hacé`
    /// (que van al input, sin abrir bloque). Lo arma el builtin y lo consume
    /// `Msg::LlmResult`.
    pub llm_block_label: Option<String>,
    /// Búsqueda semántica pendiente (`:buscar`). Mismo patrón que `llm_request`:
    /// el módulo expresa la intención, el chasis embebe y devuelve
    /// `Msg::SemanticResult`. `None` salvo entre la invocación y su resultado.
    pub semantic_request: Option<SemanticRequest>,
    /// `true` mientras una búsqueda semántica está en vuelo (la tomó el host).
    pub semantic_inflight: bool,
}

/// E5 — qué hacer con la respuesta del LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmKind {
    /// `:?` — la respuesta es una línea de comando: va al input para que el
    /// usuario la revise y ejecute (NUNCA se auto-ejecuta).
    Command,
    /// `:explica`/`:resume` — la respuesta es texto informativo: va al
    /// output del bloque.
    Text,
    /// `:hacé` — la respuesta es una **invocación de control en JSON**
    /// (`{"id":…,"args":{…}}` o `nada`): el módulo la resuelve con `atipay` a un
    /// plan validado y pone la línea exacta en el input, etiquetada por peligro
    /// (NUNCA se auto-ejecuta). Evita que el modelo invente flags inexistentes.
    Atipay,
}

/// E5 — una invocación al LLM que el host debe cumplir. Campos públicos para
/// que el chasis los lea y arme el `ChatRequest` (system + prompt + tope).
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub kind: LlmKind,
    pub system: String,
    pub prompt: String,
    pub max_tokens: u32,
    /// Backend a usar (de la config global del SO `wawa.ai.llm`). Si `backend`
    /// está vacío, el chasis cae a `from_env`. Configurable desde wawa-panel.
    pub llm: wawa_config::LlmSettings,
}

/// Petición de **búsqueda semántica** que el chasis debe cumplir: embebe `query`
/// + `candidates` con el daemon de embeddings (o mock), contra un índice
/// persistido por `scope`, y devuelve los más parecidos. Campos públicos para
/// que el host arme la búsqueda.
#[derive(Debug, Clone)]
pub struct SemanticRequest {
    /// Espacio del índice persistido: `"history"` (comandos) · `"files"`
    /// (archivos). Cada scope tiene su archivo de índice en disco.
    pub scope: String,
    /// Lo que el usuario busca por significado.
    pub query: String,
    /// Corpus a rankear como pares `(clave, texto_a_embeber)`. La **clave** es
    /// estable e identifica la entrada en el índice (y es lo que se muestra); el
    /// **texto** es lo que se embebe (puede traer más contexto que la clave).
    /// Para comandos clave==texto; para archivos clave incluye el mtime (para
    /// re-embeber al cambiar) y el texto es ruta + fragmento del contenido.
    pub candidates: Vec<(String, String)>,
    /// Socket del daemon de embeddings (`""` = por defecto).
    pub socket: String,
    /// Dimensión del fallback mock si no hay daemon.
    pub dim: usize,
}

/// Snapshot serializable del output de una sesión — lo que el chasis
/// persiste a disco cuando la sesión tiene el flag «persistir» y rehidrata
/// al reabrir la app. Sólo datos: las asas vivas (runs, locks) no viajan.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct OutputSnapshot {
    pub lines: Vec<OutputLine>,
    /// Comando por bloque (headers que sobreviven al recorte del buffer).
    pub block_command: std::collections::HashMap<u64, String>,
    /// Momento de apertura por bloque (unix secs) — los "hace N min".
    pub block_started: std::collections::HashMap<u64, u64>,
    /// Contador monotónico al momento del snapshot.
    pub block_seq: u64,
}

impl State {
    /// Captura el output vigente como [`OutputSnapshot`], limitado a las
    /// últimas `max_lines` líneas (cortar al medio de un bloque es válido:
    /// el render rearma el header desde `block_command`).
    pub fn output_snapshot(&self, max_lines: usize) -> OutputSnapshot {
        let start = self.output.len().saturating_sub(max_lines);
        let lines: Vec<OutputLine> = self.output[start..].to_vec();
        // Sólo la metadata de los bloques presentes en el recorte.
        let presentes: HashSet<u64> = lines.iter().map(|l| l.block).collect();
        OutputSnapshot {
            block_command: self
                .block_command
                .iter()
                .filter(|(b, _)| presentes.contains(b))
                .map(|(b, c)| (*b, c.clone()))
                .collect(),
            block_started: self
                .block_started
                .iter()
                .filter(|(b, _)| presentes.contains(b))
                .map(|(b, t)| (*b, *t))
                .collect(),
            block_seq: self.block_seq,
            lines,
        }
    }

    /// Rehidrata un snapshot al frente del buffer (pensado para el arranque,
    /// con el buffer todavía vacío). Los bloques restaurados quedan
    /// **plegados** (menos el último) para que la sesión abra compacta, y
    /// un notice separador marca la costura.
    pub fn restore_output(&mut self, snap: OutputSnapshot) {
        if snap.lines.is_empty() {
            return;
        }
        let ultimo = snap.lines.iter().map(|l| l.block).max().unwrap_or(0);
        for l in &snap.lines {
            if l.block != 0 && l.block != ultimo {
                self.collapsed.insert(l.block);
            }
        }
        self.block_command.extend(snap.block_command);
        self.block_started.extend(snap.block_started);
        self.block_seq = self.block_seq.max(snap.block_seq);
        let mut restauradas = snap.lines;
        let n = restauradas.len();
        restauradas.push(OutputLine::notice(format!(
            "— sesión restaurada ({n} líneas) —"
        )));
        restauradas.extend(std::mem::take(&mut self.output));
        self.output = restauradas;
        // Las líneas nuevas siguen en bloques nuevos, nunca en los viejos.
        self.current_block = 0;
    }
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
        // Un contenedor arranca en SU interior (`/root`, el home del root con
        // el que entramos), no en el cwd del host: tras el chroot el path del
        // host no existe adentro y `pwd`/`ls`/el prompt se contradecían.
        let cwd = match &source {
            // Contenedor: arranca en su interior (`/root`).
            Source::Container { .. } => PathBuf::from("/root"),
            // Contenedor remoto: idem, su interior (`/root`) en el host remoto.
            Source::RemoteContainer { .. } => PathBuf::from("/root"),
            // Remoto: arranca en el `$HOME` remoto (`~`); el `cd` lo trackea.
            Source::Remote { .. } => PathBuf::from("~"),
            _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
        };
        let completion_source = completion_source_for(&source, &cwd);
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
            macro_book: load_macro_book(),
            current_run_node: None,
            current_run_bytes: 0,
            vim_sel: None,
            block_seq: 0,
            current_block: 0,
            collapsed: HashSet::new(),
            section_collapsed: HashSet::new(),
            section_sort: HashMap::new(),
            font_zoom: 1.0,
            surf_scroll_x: 0.0,
            input_focus: None,
            expanded_stages: HashSet::new(),
            patterns: Vec::new(),
            dismissed_choreo: std::collections::HashSet::new(),
            dismissed_alias: std::collections::HashSet::new(),
            did_you_mean: std::collections::HashMap::new(),
            long_alerts: 0,
            // Política de captura inicial desde el rc (los builtins `:limit` /
            // `:spill` la sobreescriben en vivo). `0` MiB = sin tope.
            capture_limit_bytes: config.capture.limit_mb.saturating_mul(1024 * 1024),
            spill: config.capture.spill,
            reprocess_source: None,
            compare_anchor: None,
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
            block_started: std::collections::HashMap::new(),
            block_ended: std::collections::HashMap::new(),
            block_command: std::collections::HashMap::new(),
            input_edit_at_ms: now_unix_millis(),
            config,
            exit_rule_fired: false,
            in_cwd_rule: false,
            llm_request: None,
            llm_inflight: false,
            llm_block_label: None,
            semantic_request: None,
            semantic_inflight: false,
        }
    }

    /// E5 — el host toma la petición LLM pendiente (si la hay y no hay otra
    /// en vuelo), marcándola en vuelo. Devuelve `None` si no hay nada que
    /// hacer. El host la corre y responde con `Msg::LlmResult`.
    pub fn take_llm_request(&mut self) -> Option<LlmRequest> {
        if self.llm_inflight {
            return None;
        }
        let req = self.llm_request.take()?;
        self.llm_inflight = true;
        Some(req)
    }

    /// El host toma la búsqueda semántica pendiente (si la hay y no hay otra en
    /// vuelo), marcándola en vuelo. El host la corre y responde con
    /// `Msg::SemanticResult`.
    pub fn take_semantic_request(&mut self) -> Option<SemanticRequest> {
        if self.semantic_inflight {
            return None;
        }
        let req = self.semantic_request.take()?;
        self.semantic_inflight = true;
        Some(req)
    }

    /// Empuja una línea al buffer asignándole bloque. Cada `Prompt` abre
    /// un bloque nuevo (id monotónico); las demás líneas heredan el
    /// bloque abierto. El render usa esto para agrupar cada comando con
    /// Empuja una **nota** (línea informativa del shell) al buffer. Público para
    /// que el host (chasis) deje avisos en el output —p.ej. el resultado de una
    /// búsqueda de archivos que se pintó en el panel del Explorer.
    pub fn push_notice(&mut self, text: impl Into<String>) {
        self.push_output(OutputLine::notice(text.into()));
    }

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
        self.compare_anchor = None;
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

    /// Estado de actividad para el aviso visual (color del LED): claude tiene
    /// prioridad (aunque "corra", queremos su color propio), luego comando en
    /// curso = movimiento, si no quieto.
    pub fn activity(&self) -> Activity {
        if crate::update::pty::running_skin(self) == Some(AppSkin::Claude) {
            Activity::Claude
        } else if self.is_running() {
            Activity::Busy
        } else {
            Activity::Idle
        }
    }

    /// `true` si el PTY vivo está en **alternate screen** — una TUI de pantalla
    /// completa (vim, htop, less, man…) que necesita capturar Esc. El chasis lo
    /// consulta para decidir si Esc cierra el drawer Quake (no hay TUI) o se
    /// reenvía al programa (sí la hay).
    pub fn is_fullscreen_tui(&self) -> bool {
        crate::update::pty::is_tui_fullscreen(self)
    }

    /// A6 — comandos largos terminados pendientes de acuse (los que el chasis
    /// badgea en el diente de la sesión cuando no está activa).
    pub fn long_alerts(&self) -> usize {
        self.long_alerts
    }

    /// A6 — el usuario volvió a esta sesión: limpia la badge de comando largo.
    /// Lo llama el chasis al activar la sesión (y por Tick mientras es la activa).
    pub fn ack_long_alerts(&mut self) {
        self.long_alerts = 0;
    }

    /// Devuelve el `ActiveRun` (foreground o background) cuyo bloque es
    /// `block`, si existe — sin importar si sigue vivo. Permite dirigir el
    /// stdin del input a CUALQUIER comando en curso, no sólo al foreground.
    pub(crate) fn job_by_block(&self, block: u64) -> Option<Arc<Mutex<ActiveRun>>> {
        if let Some(r) = self.running.as_ref() {
            if r.lock().map(|g| g.block == block).unwrap_or(false) {
                return Some(r.clone());
            }
        }
        self.bg_jobs
            .iter()
            .find(|j| j.lock().map(|g| g.block == block).unwrap_or(false))
            .cloned()
    }

    /// `true` si `block` pertenece a un comando que sigue corriendo (no ha
    /// cerrado). Lo usa el render para no plegar las ejecuciones vivas y el
    /// `run_submitted` para no hacerlas recede al arrancar otro comando.
    pub(crate) fn block_has_live_job(&self, block: u64) -> bool {
        match self.job_by_block(block) {
            Some(arc) => arc
                .lock()
                .map(|g| !g.handle.is_finished())
                .unwrap_or(false),
            None => false,
        }
    }

    /// Snapshot del grafo de intenciones — el chasis lo lee cada tick
    /// y lo sincroniza al `shuma-module-canvas` activo.
    pub fn intent_graph(&self) -> &SessionGraph {
        &self.intent_graph
    }
}
