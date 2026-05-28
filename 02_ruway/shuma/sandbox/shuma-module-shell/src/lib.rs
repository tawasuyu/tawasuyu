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

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_theme::Theme;
use shuma_exec::{CommandSpec, Exec, Killer, RunEvent, RunHandle, StageSpec};
use shuma_line::{LineState, TokenKind};
use shuma_module::{ModuleContributions, ShortcutSpec, Source};
use std::collections::VecDeque;
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

/// Una línea del buffer de output con su tipo (para coloreado).
#[derive(Debug, Clone)]
pub struct OutputLine {
    pub kind: OutputKind,
    pub text: String,
}

impl OutputLine {
    pub fn prompt(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Prompt,
            text: text.into(),
        }
    }
    pub fn stdout(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Stdout,
            text: text.into(),
        }
    }
    pub fn stderr(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Stderr,
            text: text.into(),
        }
    }
    pub fn notice(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Notice,
            text: text.into(),
        }
    }
}

/// Run vivo: handle de `shuma-exec`, su `Killer` (para SIGTERM desde la
/// UI) y el comando original (para el notice de cierre).
pub struct ActiveRun {
    pub handle: RunHandle,
    pub killer: Killer,
    pub command: String,
    /// Sesión TUI: emulador vt100 + dims del PTY. `Some` cuando el run
    /// arrancó bajo `Exec::Pty` (vim/htop/less/etc.); las teclas van al
    /// stdin del PTY y la pantalla se renderiza como grid de celdas.
    pub tui: Option<TuiSession>,
}

/// Sesión TUI sobre PTY — bufferea el parser vt100 y los dims actuales.
pub struct TuiSession {
    pub parser: vt100::Parser,
    pub rows: u16,
    pub cols: u16,
}

impl TuiSession {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, 0),
            rows,
            cols,
        }
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
    "vi", "vim", "nvim", "nano", "emacs", "helix", "hx", "htop", "btop", "top",
    "less", "more", "man", "claude", "tig", "tui", "watch",
];

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
}

/// Mapea `action_id` de `ShortcutAction::ModuleAction` al `Msg`.
pub fn dispatch(action_id: &str) -> Option<Msg> {
    match action_id {
        "shell.clear" => Some(Msg::Clear),
        "shell.cancel" => Some(Msg::Cancel),
        _ => None,
    }
}

/// Traduce un `KeyEvent` a una llamada sobre `LineState`. Devuelve
/// `true` si tocó el state. No maneja Enter, Tab, Up/Down ni Ctrl-C
/// (esos los intercepta el `update` del módulo).
fn apply_key_to_line(line: &mut LineState, ev: &KeyEvent) -> bool {
    match &ev.key {
        Key::Named(NamedKey::Backspace) => {
            line.backspace();
            true
        }
        Key::Named(NamedKey::Delete) => {
            line.delete();
            true
        }
        Key::Named(NamedKey::ArrowLeft) => {
            if ev.modifiers.ctrl {
                line.move_word_left();
            } else {
                line.move_left();
            }
            true
        }
        Key::Named(NamedKey::ArrowRight) => {
            if ev.modifiers.ctrl {
                line.move_word_right();
            } else {
                line.move_right();
            }
            true
        }
        Key::Named(NamedKey::Home) => {
            line.move_home();
            true
        }
        Key::Named(NamedKey::End) => {
            line.move_end();
            true
        }
        Key::Named(NamedKey::Space) => {
            line.insert(" ");
            true
        }
        _ => {
            if let Some(text) = &ev.text {
                if !text.is_empty() && !text.chars().any(|c| c.is_control()) {
                    line.insert(text);
                    return true;
                }
            }
            false
        }
    }
}

pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Key(ev) => {
            if ev.state != KeyState::Pressed {
                return s;
            }
            // Si hay un TUI activo, las teclas van al stdin del PTY
            // (no al input). El usuario sale tipeando dentro del TUI
            // (`:q` en vim, `q` en less, etc.).
            if is_tui_active(&s) {
                forward_key_to_pty(&s, &ev);
                return s;
            }
            // Si el overlay de búsqueda está abierto, las teclas van ahí.
            if s.history_search.is_some() {
                return handle_search_key(s, &ev);
            }
            // Ctrl-C: si hay run vivo, mandarle SIGTERM y comer la tecla.
            if ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("c"))
            {
                if s.running.is_some() {
                    return cancel_running(s);
                }
            }
            // Ctrl-R: abrir overlay de búsqueda de historial.
            if ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("r"))
            {
                s.history_search = Some(HistorySearch::default());
                return s;
            }
            // Enter: ejecuta.
            if let Key::Named(NamedKey::Enter) = ev.key {
                s.history_cursor = None;
                s = run_submitted(s);
                return s;
            }
            // Tab: completion.
            if let Key::Named(NamedKey::Tab) = ev.key {
                return apply_completion_msg(s);
            }
            // Up/Down: navegación de historial.
            if let Key::Named(NamedKey::ArrowUp) = ev.key {
                return navigate_history(s, shuma_history::Nav::Older);
            }
            if let Key::Named(NamedKey::ArrowDown) = ev.key {
                return navigate_history(s, shuma_history::Nav::Newer);
            }
            // Flecha derecha al final de línea con ghost visible: acepta ghost.
            if let Key::Named(NamedKey::ArrowRight) = ev.key {
                if !ev.modifiers.ctrl && s.input.cursor() == s.input.text().len() {
                    if let Some(suffix) = current_ghost(&s) {
                        if !suffix.is_empty() {
                            s.input.insert(&suffix);
                            return s;
                        }
                    }
                }
            }
            apply_key_to_line(&mut s.input, &ev);
            // Cualquier edición rompe el cursor de navegación de historial.
            s.history_cursor = None;
        }
        Msg::FocusInput => {
            s.focused = true;
        }
        Msg::Clear => {
            s.output.clear();
        }
        Msg::Tick => {
            s = drain_run(s);
        }
        Msg::Cancel => {
            if s.running.is_some() {
                s = cancel_running(s);
            }
        }
    }
    s
}

/// Aplica un Tab: completion en la posición del cursor.
/// - 0 candidatos: no hace nada.
/// - 1 candidato: lo inserta directo.
/// - N candidatos: inserta el prefijo común y deja al usuario tipear más.
fn apply_completion_msg(mut s: State) -> State {
    let comp = s.input.complete(s.completion_source.as_ref());
    if comp.is_empty() {
        return s;
    }
    let candidate: String = if comp.candidates.len() == 1 {
        comp.candidates[0].clone()
    } else {
        common_prefix(&comp.candidates)
    };
    if candidate.is_empty() {
        return s;
    }
    s.input.apply_completion(&comp, &candidate);
    s
}

/// Prefijo común más largo de un slice de strings — usado en completion
/// cuando hay múltiples candidatos.
fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut end = first.len();
    for s in &items[1..] {
        let bytes = s.as_bytes();
        let fbytes = first.as_bytes();
        let mut i = 0;
        while i < end && i < bytes.len() && bytes[i] == fbytes[i] {
            i += 1;
        }
        end = i;
        if end == 0 {
            break;
        }
    }
    // Asegurarse de cortar en límite de carácter UTF-8.
    while end > 0 && !first.is_char_boundary(end) {
        end -= 1;
    }
    first[..end].to_string()
}

/// Navega el historial por Up/Down.
fn navigate_history(mut s: State, dir: shuma_history::Nav) -> State {
    let next = {
        let history = s.history.lock().unwrap();
        history.navigate(s.history_cursor, dir).map(|(i, e)| (i, e.line.clone()))
    };
    if let Some((i, line)) = next {
        s.history_cursor = Some(i);
        s.input.set_text(line);
    } else if matches!(dir, shuma_history::Nav::Newer) {
        // Salir del historial al final: línea vacía.
        s.history_cursor = None;
        s.input.clear();
    }
    s
}

/// Maneja teclas mientras el overlay Ctrl-R está abierto.
fn handle_search_key(mut s: State, ev: &KeyEvent) -> State {
    let Some(mut search) = s.history_search.take() else {
        return s;
    };
    match &ev.key {
        Key::Named(NamedKey::Escape) => {
            // Salida sin aceptar.
            return s;
        }
        Key::Named(NamedKey::Enter) => {
            // Acepta el seleccionado: pasa a la línea (sin ejecutar).
            let pick = {
                let history = s.history.lock().unwrap();
                history
                    .fuzzy_search(&search.query, 50)
                    .get(search.selected)
                    .map(|e| e.line.clone())
            };
            if let Some(line) = pick {
                s.input.set_text(line);
            }
            return s;
        }
        Key::Named(NamedKey::Backspace) => {
            search.query.pop();
            search.selected = 0;
        }
        Key::Named(NamedKey::ArrowDown) => {
            let history = s.history.lock().unwrap();
            let max = history.fuzzy_search(&search.query, 50).len();
            if max > 0 && search.selected + 1 < max {
                search.selected += 1;
            }
        }
        Key::Named(NamedKey::ArrowUp) => {
            search.selected = search.selected.saturating_sub(1);
        }
        _ => {
            if let Some(text) = &ev.text {
                if !text.is_empty() && !text.chars().any(|c| c.is_control()) {
                    search.query.push_str(text);
                    search.selected = 0;
                }
            }
        }
    }
    s.history_search = Some(search);
    s
}

/// `true` si hay un `ActiveRun` en modo TUI (PTY + vt100). Las teclas
/// van al stdin del PTY mientras esto sea cierto.
fn is_tui_active(s: &State) -> bool {
    let Some(arc) = s.running.as_ref() else {
        return false;
    };
    let g = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    g.tui.is_some()
}

/// Traduce una tecla a su secuencia de bytes para el PTY (xterm-compat).
/// Las TUIs esperan estos códigos.
fn key_to_pty_bytes(ev: &KeyEvent) -> Vec<u8> {
    match &ev.key {
        Key::Named(NamedKey::Enter) => b"\r".to_vec(),
        Key::Named(NamedKey::Tab) => b"\t".to_vec(),
        Key::Named(NamedKey::Backspace) => b"\x7f".to_vec(),
        Key::Named(NamedKey::Escape) => b"\x1b".to_vec(),
        Key::Named(NamedKey::ArrowUp) => b"\x1b[A".to_vec(),
        Key::Named(NamedKey::ArrowDown) => b"\x1b[B".to_vec(),
        Key::Named(NamedKey::ArrowRight) => b"\x1b[C".to_vec(),
        Key::Named(NamedKey::ArrowLeft) => b"\x1b[D".to_vec(),
        Key::Named(NamedKey::Home) => b"\x1b[H".to_vec(),
        Key::Named(NamedKey::End) => b"\x1b[F".to_vec(),
        Key::Named(NamedKey::PageUp) => b"\x1b[5~".to_vec(),
        Key::Named(NamedKey::PageDown) => b"\x1b[6~".to_vec(),
        Key::Named(NamedKey::Delete) => b"\x1b[3~".to_vec(),
        Key::Named(NamedKey::Space) => b" ".to_vec(),
        _ => {
            // Ctrl-<x>: codifica el byte 0x01..0x1a para letras.
            if ev.modifiers.ctrl {
                if let Key::Character(c) = &ev.key {
                    if let Some(ch) = c.chars().next() {
                        let lo = ch.to_ascii_lowercase();
                        if ('a'..='z').contains(&lo) {
                            return vec![(lo as u8) - b'a' + 1];
                        }
                    }
                }
            }
            ev.text.as_deref().unwrap_or("").as_bytes().to_vec()
        }
    }
}

/// Manda los bytes de la tecla al PTY del run activo. No-op si no hay
/// tui activo.
fn forward_key_to_pty(s: &State, ev: &KeyEvent) {
    let Some(arc) = s.running.as_ref() else {
        return;
    };
    let bytes = key_to_pty_bytes(ev);
    if bytes.is_empty() {
        return;
    }
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.handle.write_input(bytes);
}

/// Sugerencia "ghost" para la línea actual — el prefijo histórico más
/// reciente que extiende el texto que ya está tipeado.
fn current_ghost(s: &State) -> Option<String> {
    let text = s.input.text();
    if text.is_empty() || s.input.cursor() != text.len() {
        return None;
    }
    let history = s.history.lock().ok()?;
    let corpus: Vec<String> = history.entries().iter().rev().map(|e| e.line.clone()).collect();
    shuma_line::ghost_suggestion(text, &corpus)
}

fn run_submitted(mut s: State) -> State {
    let line = s.input.text().to_string();
    let trimmed = line.trim().to_string();
    s.input.clear();
    if trimmed.is_empty() {
        return s;
    }
    push_line(&mut s.output, OutputLine::prompt(format!("$ {trimmed}")));

    // Append al historial — todo lo que el usuario Enter-eó queda
    // registrado, builtins incluidos (para que `cd ../foo` reaparezca
    // por Up). `IgnoreConsecutive` evita ráfagas iguales.
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = shuma_history::Entry::new(
            trimmed.clone(),
            s.cwd.display().to_string(),
            now,
        );
        if let Ok(mut h) = s.history.lock() {
            let _ = h.append(entry);
        }
    }

    // Builtins primero — no spawnean proceso, corren aunque haya run vivo.
    if let Some((cmd, rest)) = split_first_word(&trimmed) {
        match cmd {
            "cd" => {
                return apply_cd(s, rest);
            }
            "pwd" => {
                let cwd_str = s.cwd.display().to_string();
                push_line(&mut s.output, OutputLine::stdout(cwd_str));
                return s;
            }
            "clear" => {
                s.output.clear();
                return s;
            }
            "exit" => {
                push_line(
                    &mut s.output,
                    OutputLine::notice("exit: el chasis maneja la salida (F12 para cerrar)"),
                );
                return s;
            }
            _ => {}
        }
    }

    // Comando externo. Si ya hay uno corriendo, lo encolamos; si no,
    // arrancamos ahora mismo.
    if s.running.is_some() {
        s.queue.push_back(trimmed);
        push_line(
            &mut s.output,
            OutputLine::notice("⌛ en cola — esperando a que el comando actual termine"),
        );
        return s;
    }
    start_run(s, trimmed)
}

fn start_run(mut s: State, line: String) -> State {
    let cwd_str = s.cwd.display().to_string();
    let (spec, tui) = build_spec(&line, &cwd_str);
    let handle = shuma_exec::run(&spec);
    let killer = handle.killer();
    let active = ActiveRun {
        handle,
        killer,
        command: line,
        tui,
    };
    s.running = Some(Arc::new(Mutex::new(active)));
    s
}

/// Decide cómo lanzar `line`: si el primer token está en la allowlist
/// TUI (o el usuario lo prefijó con `:tui`), abre un PTY; si no, va por
/// el shell normal (streaming Stdout/Stderr).
fn build_spec(line: &str, cwd: &str) -> (CommandSpec, Option<TuiSession>) {
    // Prefijo explícito `:tui <comando>`.
    let (cmd_line, force_tui) = match line.strip_prefix(":tui ") {
        Some(rest) => (rest.trim(), true),
        None => (line, false),
    };
    let first_word = cmd_line.split_whitespace().next().unwrap_or("");
    let is_tui = force_tui || TUI_ALLOWLIST.contains(&first_word);
    if !is_tui {
        return (CommandSpec::shell(line, cwd), None);
    }
    // Bajo PTY: parseamos en stages básicos por whitespace. No soporta
    // pipes ni redirecciones — un TUI fullscreen no los usa.
    let parts: Vec<String> = cmd_line.split_whitespace().map(String::from).collect();
    if parts.is_empty() {
        return (CommandSpec::shell(line, cwd), None);
    }
    let program = parts[0].clone();
    let args = parts[1..].to_vec();
    let spec = CommandSpec {
        exec: Exec::Pty {
            program,
            args,
            cols: PTY_COLS,
            rows: PTY_ROWS,
        },
        cwd: cwd.to_string(),
        capture_limit: 0,
        spill_path: None,
        stdin_data: None,
    };
    // Stage marker — usamos `parts` para sintaxis, no para ejecutar; el
    // Exec::Pty arma el spawn directo. La conversión a `StageSpec`
    // queda como guía visual del tooltip si después la queremos
    // exponer (hoy `Exec::Pty` no usa stages).
    let _ = StageSpec {
        program: parts[0].clone(),
        args: parts[1..].to_vec(),
    };
    (spec, Some(TuiSession::new(PTY_ROWS, PTY_COLS)))
}

fn drain_run(mut s: State) -> State {
    let Some(active_arc) = s.running.clone() else {
        return s;
    };
    let mut finished_with: Option<RunEvent> = None;
    {
        let mut guard = match active_arc.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let events = guard.handle.try_events();
        for ev in events {
            match ev {
                RunEvent::Stdout(line) => push_line(&mut s.output, OutputLine::stdout(line)),
                RunEvent::Stderr(line) => push_line(&mut s.output, OutputLine::stderr(line)),
                RunEvent::Truncated => push_line(
                    &mut s.output,
                    OutputLine::notice("… (salida truncada por límite de captura)"),
                ),
                RunEvent::Spilled(path) => push_line(
                    &mut s.output,
                    OutputLine::notice(format!("… (resto volcado a {path})")),
                ),
                RunEvent::Bytes(bytes) => {
                    if let Some(tui) = guard.tui.as_mut() {
                        tui.parser.process(&bytes);
                    }
                }
                ev @ (RunEvent::Exited(_) | RunEvent::Failed(_)) => {
                    finished_with = Some(ev);
                }
            }
        }
    }
    if let Some(ev) = finished_with {
        let notice = match ev {
            RunEvent::Exited(0) => "✔ exit 0".to_string(),
            RunEvent::Exited(code) => format!("✘ exit {code}"),
            RunEvent::Failed(e) => format!("✘ no se pudo spawnear: {e}"),
            _ => unreachable!(),
        };
        push_line(&mut s.output, OutputLine::notice(notice));
        s.running = None;
        // Si quedó algo en cola, arrancarlo ya — sin esperar otro Tick.
        if let Some(next) = s.queue.pop_front() {
            s = start_run(s, next);
        }
    }
    s
}

fn cancel_running(mut s: State) -> State {
    if let Some(arc) = s.running.as_ref() {
        let guard = match arc.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        // SIGKILL al grupo entero — la UI cancela "ya". SIGTERM educado
        // sirve cuando se quiere darle tiempo al proceso a limpiar; en
        // un shell interactivo, Ctrl-C debe doler.
        guard.killer.kill();
        // El próximo Tick observará `RunEvent::Exited` y limpiará el handle.
    }
    push_line(&mut s.output, OutputLine::notice("⏹ cancel (SIGKILL enviado)"));
    s
}

fn apply_cd(mut s: State, rest: &str) -> State {
    let target = if rest.trim().is_empty() {
        // `cd` sin args → HOME (convención bash/zsh).
        match std::env::var("HOME") {
            Ok(h) => PathBuf::from(h),
            Err(_) => {
                push_line(
                    &mut s.output,
                    OutputLine::notice("cd: HOME no está definido"),
                );
                return s;
            }
        }
    } else {
        let trimmed = rest.trim();
        let p = PathBuf::from(trimmed);
        if p.is_absolute() {
            p
        } else {
            s.cwd.join(p)
        }
    };
    match std::fs::canonicalize(&target) {
        Ok(canonical) => {
            if canonical.is_dir() {
                s.cwd = canonical;
            } else {
                push_line(
                    &mut s.output,
                    OutputLine::notice(format!(
                        "cd: no es un directorio: {}",
                        target.display()
                    )),
                );
            }
        }
        Err(e) => {
            push_line(
                &mut s.output,
                OutputLine::notice(format!("cd: {}: {e}", target.display())),
            );
        }
    }
    s
}

fn split_first_word(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    if line.is_empty() {
        return None;
    }
    match line.find(char::is_whitespace) {
        Some(i) => Some((&line[..i], &line[i + 1..])),
        None => Some((line, "")),
    }
}

fn push_line(buf: &mut Vec<OutputLine>, line: OutputLine) {
    buf.push(line);
    let len = buf.len();
    if len > MAX_OUTPUT_LINES {
        buf.drain(0..len - MAX_OUTPUT_LINES);
    }
}

pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let header = shell_header(state, theme);
    let main_panel: View<HostMsg> = if is_tui_active(state) {
        tui_panel::<HostMsg>(state, theme)
    } else {
        output_pane::<HostMsg>(state, theme)
    };
    let input = shell_input_view(state, theme, lift.clone());

    let mut children = vec![header, main_panel, input];
    if state.history_search.is_some() {
        children.push(history_search_panel::<HostMsg>(state, theme));
    }

    View::new(Style {
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
    .fill(theme.bg_app)
    .children(children)
}

/// Color por `TokenKind` — paleta diseñada para que el comando salte y
/// los flags/strings tengan su propio tono.
fn token_color(kind: TokenKind, theme: &Theme) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    match kind {
        TokenKind::Command => theme.accent,
        TokenKind::Argument => theme.fg_text,
        TokenKind::Flag => Color::from_rgba8(220, 200, 120, 255), // amarillo
        TokenKind::StringLit => Color::from_rgba8(160, 210, 140, 255), // verde
        TokenKind::Variable => Color::from_rgba8(200, 160, 220, 255), // violeta
        TokenKind::Pipe | TokenKind::Redirect | TokenKind::Operator => theme.accent,
        TokenKind::Comment | TokenKind::Whitespace => theme.fg_muted,
        TokenKind::Unknown => theme.fg_destructive,
    }
}

/// Renderiza la línea de entrada con tokens coloreados, cursor visible
/// y ghost suggestion. El layout es un nodo único con `paint_with` —
/// medimos cada token con el typesetter en el closure para alinear el
/// cursor al carácter exacto.
fn shell_input_view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let bg = if state.focused {
        theme.bg_input_focus
    } else {
        theme.bg_input
    };
    let border = if state.focused {
        theme.border_focus
    } else {
        theme.border
    };

    let text = state.input.text().to_string();
    let cursor = state.input.cursor();
    let tokens = state.input.tokens();
    let ghost = current_ghost(state);
    let placeholder = if text.is_empty() && ghost.is_none() {
        Some("tipeá un comando…".to_string())
    } else {
        None
    };
    let theme_clone = *theme;
    let focused = state.focused;

    let painter = move |scene: &mut vello::Scene,
                        ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        use llimphi_ui::llimphi_text::{
            draw_layout, layout_block, measurement, Alignment as TAlign, TextBlock,
        };
        let pad_x = 10.0;
        let baseline_y = rect.y as f64 + 8.0;
        let mut x = rect.x as f64 + pad_x;
        let mut cursor_x: f64 = x;

        if let Some(ph) = &placeholder {
            let block = TextBlock {
                text: ph,
                size_px: 13.0,
                color: theme_clone.fg_placeholder,
                origin: (x, baseline_y),
                max_width: None,
                alignment: TAlign::Start,
                line_height: 1.2,
            };
            let layout = layout_block(ts, &block);
            draw_layout(scene, &layout, theme_clone.fg_placeholder, (x, baseline_y));
        }

        // Pinta cada token con su color; al pasar por el cursor, capturá la x.
        let mut last_end = 0usize;
        for tok in &tokens {
            let color = token_color(tok.kind, &theme_clone);
            let segment = &text[tok.start..tok.end];
            let block = TextBlock {
                text: segment,
                size_px: 13.0,
                color,
                origin: (x, baseline_y),
                max_width: None,
                alignment: TAlign::Start,
                line_height: 1.2,
            };
            let layout = layout_block(ts, &block);
            let m = measurement(&layout);
            draw_layout(scene, &layout, color, (x, baseline_y));
            if tok.start < cursor && cursor <= tok.end {
                // Cursor cae en este token: medir prefijo hasta el cursor.
                let prefix = &text[tok.start..cursor];
                if prefix.is_empty() {
                    cursor_x = x;
                } else {
                    let pblock = TextBlock {
                        text: prefix,
                        size_px: 13.0,
                        color,
                        origin: (x, baseline_y),
                        max_width: None,
                        alignment: TAlign::Start,
                        line_height: 1.2,
                    };
                    let plat = layout_block(ts, &pblock);
                    cursor_x = x + measurement(&plat).width as f64;
                }
            }
            x += m.width as f64;
            last_end = tok.end;
        }
        if cursor == last_end || (cursor == 0 && tokens.is_empty()) {
            cursor_x = x;
        }

        // Ghost suggestion: pinta el sufijo en color muted detrás del cursor.
        if let Some(suffix) = &ghost {
            if !suffix.is_empty() {
                let block = TextBlock {
                    text: suffix,
                    size_px: 13.0,
                    color: theme_clone.fg_placeholder,
                    origin: (x, baseline_y),
                    max_width: None,
                    alignment: TAlign::Start,
                    line_height: 1.2,
                };
                let layout = layout_block(ts, &block);
                draw_layout(
                    scene,
                    &layout,
                    theme_clone.fg_placeholder,
                    (x, baseline_y),
                );
            }
        }

        // Cursor — barra vertical de 2 px, color accent si focado.
        if focused {
            use llimphi_ui::llimphi_raster::kurbo::Rect as KurboRect;
            use llimphi_ui::llimphi_raster::peniko::Fill;
            let cursor_rect = KurboRect::new(
                cursor_x,
                baseline_y + 2.0,
                cursor_x + 2.0,
                baseline_y + 18.0,
            );
            scene.fill(
                Fill::NonZero,
                vello::kurbo::Affine::IDENTITY,
                Color::from_rgba8(214, 222, 232, 220),
                None,
                &cursor_rect,
            );
        }
    };

    let inner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .paint_with(painter);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
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
    .on_click(lift(Msg::FocusInput))
    .children(vec![inner])
}

/// Panel de TUI: pinta la pantalla del PTY como grid monoespaciado.
/// Se invoca cuando `is_tui_active(state)` es `true`.
fn tui_panel<HostMsg: Clone + 'static>(state: &State, theme: &Theme) -> View<HostMsg> {
    // Tomar un snapshot del estado actual del screen para que la
    // closure de paint pueda ser `Send + Sync` (no captura el Mutex).
    let snapshot: Option<TuiSnapshot> = state
        .running
        .as_ref()
        .and_then(|arc| arc.lock().ok().and_then(|g| capture_tui(&g)));
    let theme_clone = *theme;

    let painter = move |scene: &mut vello::Scene,
                        ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::Rect as KurboRect;
        use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
        use llimphi_ui::llimphi_text::{
            draw_layout, layout_block, Alignment as TAlign, TextBlock,
        };
        let Some(snap) = &snapshot else { return };
        // Tamaño de la celda derivado del rect disponible. Monoespacio,
        // ancho/alto fijos por celda. Si el panel es chico el grid
        // se recorta abajo/derecha (no scrolleamos por ahora).
        let pad = 6.0_f64;
        let avail_w = (rect.w as f64 - pad * 2.0).max(0.0);
        let avail_h = (rect.h as f64 - pad * 2.0).max(0.0);
        let cell_w = (avail_w / snap.cols as f64).max(1.0);
        let cell_h = (avail_h / snap.rows as f64).max(1.0);
        let font_size = (cell_h * 0.75).clamp(8.0, 18.0) as f32;
        let origin_x = rect.x as f64 + pad;
        let origin_y = rect.y as f64 + pad;

        // Backgrounds primero (en bloques rect), texto encima.
        for (r, row) in snap.cells.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let bg = vt_color(cell.bg, theme_clone, true);
                if bg.components[3] > 0.0 {
                    let x0 = origin_x + c as f64 * cell_w;
                    let y0 = origin_y + r as f64 * cell_h;
                    let rect = KurboRect::new(x0, y0, x0 + cell_w, y0 + cell_h);
                    scene.fill(Fill::NonZero, vello::kurbo::Affine::IDENTITY, bg, None, &rect);
                }
            }
        }
        // Texto por celda. Para reducir shaping, agrupamos runs con
        // mismo color contiguo en la misma fila.
        for (r, row) in snap.cells.iter().enumerate() {
            let mut c = 0usize;
            while c < row.len() {
                let fg = vt_color(row[c].fg, theme_clone, false);
                let mut end = c + 1;
                let mut buf = String::new();
                buf.push_str(&row[c].ch);
                while end < row.len() && row[end].fg == row[c].fg {
                    buf.push_str(&row[end].ch);
                    end += 1;
                }
                if !buf.trim().is_empty() {
                    let x0 = origin_x + c as f64 * cell_w;
                    let y0 = origin_y + r as f64 * cell_h;
                    let block = TextBlock {
                        text: &buf,
                        size_px: font_size,
                        color: fg,
                        origin: (x0, y0),
                        max_width: None,
                        alignment: TAlign::Start,
                        line_height: 1.0,
                    };
                    let layout = layout_block(ts, &block);
                    draw_layout(scene, &layout, fg, (x0, y0));
                }
                c = end;
            }
        }
        // Cursor: barra vertical en (cursor_r, cursor_c).
        if !snap.hide_cursor {
            let x0 = origin_x + snap.cursor_c as f64 * cell_w;
            let y0 = origin_y + snap.cursor_r as f64 * cell_h;
            let rect = KurboRect::new(x0, y0 + 2.0, x0 + 2.0, y0 + cell_h);
            scene.fill(
                Fill::NonZero,
                vello::kurbo::Affine::IDENTITY,
                Color::from_rgba8(214, 222, 232, 220),
                None,
                &rect,
            );
        }
    };

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .paint_with(painter)
}

/// Snapshot copiable del Screen para enviar a una closure `paint_with`.
struct TuiSnapshot {
    cells: Vec<Vec<TuiCell>>,
    rows: u16,
    cols: u16,
    cursor_r: u16,
    cursor_c: u16,
    hide_cursor: bool,
}

#[derive(Clone)]
struct TuiCell {
    ch: String,
    fg: vt100::Color,
    bg: vt100::Color,
}

/// Copia el screen actual de un `ActiveRun` PTY a un snapshot
/// `Send`-able. Devuelve `None` si el run no es TUI.
fn capture_tui(active: &std::sync::MutexGuard<'_, ActiveRun>) -> Option<TuiSnapshot> {
    let tui = active.tui.as_ref()?;
    let screen = tui.parser.screen();
    let (rows, cols) = screen.size();
    let mut cells: Vec<Vec<TuiCell>> = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut row: Vec<TuiCell> = Vec::with_capacity(cols as usize);
        for c in 0..cols {
            let (ch, fg, bg) = match screen.cell(r, c) {
                Some(cell) => (
                    if cell.has_contents() {
                        cell.contents().to_string()
                    } else {
                        " ".to_string()
                    },
                    cell.fgcolor(),
                    cell.bgcolor(),
                ),
                None => (" ".into(), vt100::Color::Default, vt100::Color::Default),
            };
            row.push(TuiCell { ch, fg, bg });
        }
        cells.push(row);
    }
    let (cursor_r, cursor_c) = screen.cursor_position();
    Some(TuiSnapshot {
        cells,
        rows,
        cols,
        cursor_r,
        cursor_c,
        hide_cursor: screen.hide_cursor(),
    })
}

/// Convierte un `vt100::Color` a un `peniko::Color`, respetando el tema
/// del shell (los 16 índices ANSI se mapean a una paleta consistente).
fn vt_color(
    c: vt100::Color,
    theme: Theme,
    is_bg: bool,
) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    match c {
        vt100::Color::Default => {
            if is_bg {
                // Transparent — el panel ya tiene su propio fill.
                Color::from_rgba8(0, 0, 0, 0)
            } else {
                theme.fg_text
            }
        }
        vt100::Color::Rgb(r, g, b) => Color::from_rgba8(r, g, b, 255),
        vt100::Color::Idx(i) => ansi_idx_to_color(i),
    }
}

/// Mapeo 256 → RGB usando la paleta xterm estándar. Cubre los 16
/// básicos, el cubo 6×6×6 y la rampa de grises.
fn ansi_idx_to_color(i: u8) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    const BASIC: [[u8; 3]; 16] = [
        [0, 0, 0], [205, 49, 49], [13, 188, 121], [229, 229, 16],
        [36, 114, 200], [188, 63, 188], [17, 168, 205], [229, 229, 229],
        [102, 102, 102], [241, 76, 76], [35, 209, 139], [245, 245, 67],
        [59, 142, 234], [214, 112, 214], [41, 184, 219], [255, 255, 255],
    ];
    if i < 16 {
        let [r, g, b] = BASIC[i as usize];
        return Color::from_rgba8(r, g, b, 255);
    }
    if i >= 232 {
        let v = 8 + (i - 232) * 10;
        return Color::from_rgba8(v, v, v, 255);
    }
    let i = i - 16;
    let r = i / 36;
    let g = (i / 6) % 6;
    let b = i % 6;
    let to_byte = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
    Color::from_rgba8(to_byte(r), to_byte(g), to_byte(b), 255)
}

/// Overlay de búsqueda Ctrl-R. Vive como hijo extra del root cuando
/// `state.history_search` está activo; un input + lista de matches.
fn history_search_panel<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
) -> View<HostMsg> {
    let search = state.history_search.as_ref().expect("panel sólo se construye con search activo");
    let matches: Vec<String> = {
        let history = state.history.lock().unwrap();
        history
            .fuzzy_search(&search.query, 50)
            .into_iter()
            .map(|e| e.line.clone())
            .collect()
    };
    let label = format!("Ctrl-R › {}", search.query);
    let mut children: Vec<View<HostMsg>> = vec![View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(label, 12.0, theme.accent, Alignment::Start)];

    for (i, m) in matches.iter().enumerate().take(8) {
        let color = if i == search.selected {
            theme.accent
        } else {
            theme.fg_text
        };
        let bg = if i == search.selected {
            theme.bg_selected
        } else {
            theme.bg_panel
        };
        children.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(18.0_f32),
                },
                ..Default::default()
            })
            .fill(bg)
            .text_aligned(m.clone(), 12.0, color, Alignment::Start),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .children(children)
}

fn shell_header<HostMsg: Clone + 'static>(state: &State, theme: &Theme) -> View<HostMsg> {
    let status = if let Some(arc) = state.running.as_ref() {
        let cmd = match arc.lock() {
            Ok(g) => g.command.clone(),
            Err(p) => p.into_inner().command.clone(),
        };
        let queued = state.queue.len();
        if queued > 0 {
            format!(" · ⟳ {cmd} (+{queued} en cola)")
        } else {
            format!(" · ⟳ {cmd}")
        }
    } else {
        String::new()
    };
    let label = format!(
        "Shell · {} · cwd: {}{}",
        state.source.label(),
        pretty_path(&state.cwd),
        status,
    );
    let color = if state.is_running() {
        theme.accent
    } else {
        theme.fg_text
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(label, 12.0, color, Alignment::Start)
}

fn output_pane<HostMsg: Clone + 'static>(state: &State, theme: &Theme) -> View<HostMsg> {
    // Tomamos las últimas N líneas que caben — sin scroll real todavía
    // (el panel asume altura fija; el chasis lo recorta con flex).
    const MAX_VISIBLE: usize = 200;
    let start = state.output.len().saturating_sub(MAX_VISIBLE);
    let visible = &state.output[start..];

    let mut children: Vec<View<HostMsg>> = Vec::with_capacity(visible.len());
    for line in visible {
        children.push(render_output_line::<HostMsg>(line, &state.cwd, theme));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        align_items: Some(AlignItems::Stretch),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .children(children)
}

/// Un segmento de una línea con su color resuelto. `start..end` es el
/// rango de bytes en el texto crudo.
#[derive(Debug, Clone)]
struct Span {
    start: usize,
    end: usize,
    color: llimphi_ui::llimphi_raster::peniko::Color,
}

/// Convierte las decoraciones de `shuma-line` en spans coloreados para
/// el typesetter. Las decoraciones reclaman rangos; los huecos quedan
/// con el color base de la línea.
fn build_spans(
    text: &str,
    decorations: &[shuma_line::Decoration],
    base: llimphi_ui::llimphi_raster::peniko::Color,
    theme: &Theme,
) -> Vec<Span> {
    use shuma_line::DecorationKind as Dk;
    let mut spans: Vec<Span> = Vec::new();
    let mut cursor = 0usize;
    for d in decorations {
        if d.start < cursor || d.end > text.len() || d.start >= d.end {
            continue;
        }
        if d.start > cursor {
            spans.push(Span {
                start: cursor,
                end: d.start,
                color: base,
            });
        }
        let color = match d.kind {
            // Paths reales y grep refs caen al accent — el "azul de path"
            // tradicional de un terminal. Sin underline (Llimphi no lo
            // soporta hoy); el cambio de color basta para distinguirlos.
            Dk::Path { .. } | Dk::GrepRef { .. } => theme.accent,
            // URLs en accent también (el ojo busca el mismo tono).
            Dk::Url(_) => theme.accent,
            // SHAs en muted (color "etiqueta") — destacan sin gritar.
            Dk::GitSha(_) => theme.fg_muted,
            // Issue refs en accent (atajos pendientes a links).
            Dk::IssueRef(_) => theme.accent,
            // Box-drawing en accent — para que las "cajas" calcen visualmente.
            Dk::BoxDraw => theme.accent,
        };
        spans.push(Span {
            start: d.start,
            end: d.end,
            color,
        });
        cursor = d.end;
    }
    if cursor < text.len() {
        spans.push(Span {
            start: cursor,
            end: text.len(),
            color: base,
        });
    }
    spans
}

/// Pinta una línea del output. Para Stdout/Stderr aplica
/// `shuma_line::decorate_line` y dibuja los spans con colores
/// distintos vía `paint_with`. Para Prompt/Notice usa el atajo
/// `text_aligned` plano.
fn render_output_line<HostMsg: Clone + 'static>(
    line: &OutputLine,
    cwd: &std::path::Path,
    theme: &Theme,
) -> View<HostMsg> {
    let style = Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    };

    match line.kind {
        OutputKind::Prompt => View::new(style).text_aligned(
            line.text.clone(),
            12.0,
            theme.accent,
            Alignment::Start,
        ),
        OutputKind::Notice => View::new(style).text_aligned(
            line.text.clone(),
            12.0,
            theme.fg_muted,
            Alignment::Start,
        ),
        OutputKind::Stdout | OutputKind::Stderr => {
            let base = if matches!(line.kind, OutputKind::Stderr) {
                theme.fg_destructive
            } else {
                theme.fg_text
            };
            let decorations = shuma_line::decorate_line(&line.text, cwd);
            // Atajo: si no hubo decoraciones, no abrimos un paint custom
            // (más eficiente y menos código en el hot path).
            if decorations.is_empty() {
                return View::new(style).text_aligned(
                    line.text.clone(),
                    12.0,
                    base,
                    Alignment::Start,
                );
            }
            let spans = build_spans(&line.text, &decorations, base, theme);
            let text = line.text.clone();
            View::new(style).paint_with(move |scene, ts, rect| {
                use llimphi_ui::llimphi_text::{
                    draw_layout, layout_block, measurement, Alignment, TextBlock,
                };
                let mut x = rect.x as f64;
                let y = rect.y as f64;
                for s in &spans {
                    let segment = &text[s.start..s.end];
                    let block = TextBlock {
                        text: segment,
                        size_px: 12.0,
                        color: s.color,
                        origin: (x, y),
                        max_width: None,
                        alignment: Alignment::Start,
                        line_height: 1.2,
                    };
                    let layout = layout_block(ts, &block);
                    let m = measurement(&layout);
                    draw_layout(scene, &layout, s.color, (x, y));
                    x += m.width as f64;
                }
            })
        }
    }
}

fn pretty_path(p: &std::path::Path) -> String {
    let full = p.display().to_string();
    if let Ok(home) = std::env::var("HOME") {
        if full == home {
            return "~".into();
        }
        if let Some(rest) = full.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    full
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
            let has_pid = !arc.lock().unwrap().killer.pids().is_empty();
            if has_pid {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            !arc.lock().unwrap().killer.pids().is_empty(),
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
    fn build_spans_segments_a_line_with_a_url() {
        use shuma_line::{Decoration, DecorationKind};
        let theme = Theme::dark();
        let text = "abrí https://gioser.net y mirá";
        // Hardcodea la decoración para no depender del FS.
        let url_start = text.find("https").unwrap();
        let url_end = url_start + "https://gioser.net".len();
        let decs = vec![Decoration {
            start: url_start,
            end: url_end,
            kind: DecorationKind::Url(text[url_start..url_end].to_string()),
        }];
        let spans = build_spans(text, &decs, theme.fg_text, &theme);
        assert_eq!(spans.len(), 3, "pre, url, post: {spans:?}");
        assert_eq!(spans[0].color, theme.fg_text);
        assert_eq!(spans[1].color, theme.accent);
        assert_eq!(spans[2].color, theme.fg_text);
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
}
