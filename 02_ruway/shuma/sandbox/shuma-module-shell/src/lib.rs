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
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_theme::Theme;
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use shuma_exec::{CommandSpec, Killer, RunEvent, RunHandle};
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
}

impl std::fmt::Debug for ActiveRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveRun")
            .field("command", &self.command)
            .field("finished", &self.handle.is_finished())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct State {
    pub source: Source,
    pub cwd: PathBuf,
    pub input: TextInputState,
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
}

impl State {
    pub fn new(source: Source) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        Self {
            source,
            cwd,
            input: TextInputState::new(),
            output: Vec::new(),
            focused: true,
            running: None,
            queue: VecDeque::new(),
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

#[derive(Debug, Clone)]
pub enum Msg {
    /// Tecla recibida desde el chasis. Si es Enter, dispara la
    /// ejecución; si no, se forwardea al `TextInputState`.
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

pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Key(ev) => {
            if ev.state != KeyState::Pressed {
                return s;
            }
            // Ctrl-C: si hay run vivo, mandarle SIGTERM y comer la tecla.
            // Si no hay run vivo, dejar que el TextInputState lo procese
            // (el widget no usa Ctrl-C hoy, pero queda abierto).
            if ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("c"))
            {
                if s.running.is_some() {
                    return cancel_running(s);
                }
            }
            if let Key::Named(NamedKey::Enter) = ev.key {
                s = run_submitted(s);
                return s;
            }
            s.input.apply_key(&ev);
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

fn run_submitted(mut s: State) -> State {
    let line = s.input.text();
    let trimmed = line.trim().to_string();
    s.input.clear();
    if trimmed.is_empty() {
        return s;
    }
    push_line(&mut s.output, OutputLine::prompt(format!("$ {trimmed}")));

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
    let spec = CommandSpec::shell(&line, cwd_str);
    let handle = shuma_exec::run(&spec);
    let killer = handle.killer();
    let active = ActiveRun {
        handle,
        killer,
        command: line,
    };
    s.running = Some(Arc::new(Mutex::new(active)));
    s
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
                RunEvent::Bytes(_) => {
                    // PTY no se usa todavía desde acá.
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

    let output_pane = output_pane(state, theme);
    let lift_focus = lift.clone();
    let input_palette = TextInputPalette::from_theme(theme);
    let input = text_input_view(
        &state.input,
        "tipeá un comando…",
        state.focused,
        &input_palette,
        lift_focus(Msg::FocusInput),
    );

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
    .children(vec![header, output_pane, input])
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
        let color = match line.kind {
            OutputKind::Prompt => theme.accent,
            OutputKind::Stdout => theme.fg_text,
            OutputKind::Stderr => theme.fg_destructive,
            OutputKind::Notice => theme.fg_muted,
        };
        children.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(16.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(line.text.clone(), 12.0, color, Alignment::Start),
        );
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
