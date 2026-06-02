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
use std::collections::{HashSet, VecDeque};
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
            // Remote no soporta PTY → write_input no aplica.
            BackendHandle::Remote(_) => false,
        }
    }
    pub fn resize(&self, rows: u16, cols: u16) -> bool {
        match self {
            BackendHandle::Local(h) => h.resize(rows, cols),
            BackendHandle::Remote(_) => false,
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
    /// Etapas de pipe desplegadas — `(block, stage)`. Click en un chip de
    /// etapa alterna la pertenencia; al estar presente se muestran sus
    /// líneas capturadas en vivo (tee) bajo la fila de etapas.
    pub expanded_stages: HashSet<(u64, usize)>,
    /// Patrones de comandos inferidos del historial (`shuma-infer`). Se
    /// recalculan al cerrar cada comando y alimentan el ghost con la
    /// secuencia predicha (no sólo el historial reciente). Vacío al
    /// arrancar y hasta tener suficiente historial.
    pub patterns: Vec<shuma_infer::EmergingPattern>,
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
}

/// Estado del overlay de búsqueda Ctrl-R.
#[derive(Debug, Clone, Default)]
pub struct HistorySearch {
    pub query: String,
    pub selected: usize,
}

impl State {
    pub fn new(source: Source) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let completion_source = Arc::new(ShellSource::new(&cwd));
        let history = Arc::new(Mutex::new(open_history()));
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
            expanded_stages: HashSet::new(),
            patterns: Vec::new(),
            scroll_px: 0.0,
            out_viewport_h: Arc::new(Mutex::new(0.0)),
            out_overflow: Arc::new(Mutex::new(0.0)),
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
        }
        line.block = self.current_block;
        push_line(&mut self.output, line);
    }

    /// Reserva un bloque nuevo sin tocar `current_block` — para runs que
    /// drenan asíncronos (foreground lento, jobs de fondo) y necesitan su
    /// propia card aunque otros comandos se intercalen mientras tanto.
    pub(crate) fn open_block(&mut self) -> u64 {
        self.block_seq += 1;
        self.block_seq
    }

    /// Empuja una línea en un bloque explícito (no en `current_block`).
    /// La usa el drenado de runs async para que su salida quede en SU
    /// card y no en la del comando que el usuario tipeó mientras tanto.
    pub(crate) fn push_in_block(&mut self, block: u64, mut line: OutputLine) {
        line.block = block;
        push_line(&mut self.output, line);
    }

    /// Vacía el buffer y el set de colapsos. No resetea `block_seq` —
    /// mantener ids monotónicos es inofensivo y evita reusos.
    pub(crate) fn clear_output(&mut self) {
        self.output.clear();
        self.collapsed.clear();
        self.expanded_stages.clear();
        self.scroll_px = 0.0;
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
}

mod update;
mod view;

pub use update::*;
pub use view::*;

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
    fn common_prefix_returns_longest_shared_start() {
        let xs: Vec<String> = vec!["cargo".into(), "cargo-edit".into(), "cargot".into()];
        assert_eq!(common_prefix(&xs), "cargo");
        let ys: Vec<String> = vec!["abc".into(), "xyz".into()];
        assert_eq!(common_prefix(&ys), "");
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
        let text = "abrí https://gioser.net y mirá";
        let url_start = text.find("https").unwrap();
        let url_end = url_start + "https://gioser.net".len();
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
}
