//! `shuma-module-shell` — REPL del shell como módulo enchufable.
//!
//! MVP del shell interactivo. La versión completa GPUI (`shuma-shell`,
//! 3.7k LOC) tiene completion, historial durable, monitores de procesos,
//! grid de runs y transporte remoto; este módulo trae el núcleo
//! funcional: cwd + input + ejecución sincrónica vía `sh -c` + buffer
//! de output. Las features grandes llegarán en bloques aparte
//! (streaming, decoraciones, PTY).
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
//! Builtins implementados: `cd <path>`, `pwd`, `clear`, `exit` (no-op
//! aquí — el chasis maneja la salida).
//!
//! **Limitaciones del MVP**: `update` corre la ejecución sincrónicamente,
//! así que comandos largos congelan la UI. Para `ls`/`pwd`/`echo`/`cat`
//! sobre archivos chicos basta; para `sleep 5` no. La integración con
//! `shuma-exec` para streaming llega cuando el módulo lo necesite.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_theme::Theme;
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use shuma_module::{ModuleContributions, ShortcutSpec, Source};
use std::path::PathBuf;
use std::process::Command;

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

#[derive(Debug, Clone)]
pub struct State {
    pub source: Source,
    pub cwd: PathBuf,
    pub input: TextInputState,
    pub output: Vec<OutputLine>,
    pub focused: bool,
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
        }
    }

    /// Cantidad de líneas en el buffer — alimenta el monitor.
    pub fn output_len(&self) -> usize {
        self.output.len()
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
}

/// Mapea `action_id` de `ShortcutAction::ModuleAction` al `Msg`.
pub fn dispatch(action_id: &str) -> Option<Msg> {
    match action_id {
        "shell.clear" => Some(Msg::Clear),
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
    }
    s
}

fn run_submitted(mut s: State) -> State {
    let line = s.input.text();
    let trimmed = line.trim();
    s.input.clear();
    if trimmed.is_empty() {
        return s;
    }
    push_line(&mut s.output, OutputLine::prompt(format!("$ {trimmed}")));

    // Builtins primero — no spawnean proceso.
    if let Some((cmd, rest)) = split_first_word(trimmed) {
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

    // Comando externo — bloqueante por ahora.
    let report = run_external(trimmed, &s.cwd);
    for line in report.stdout_lines {
        push_line(&mut s.output, OutputLine::stdout(line));
    }
    for line in report.stderr_lines {
        push_line(&mut s.output, OutputLine::stderr(line));
    }
    let notice = match report.outcome {
        RunOutcome::Exited(0) => "✔ exit 0".to_string(),
        RunOutcome::Exited(code) => format!("✘ exit {code}"),
        RunOutcome::Signal => "✘ killed by signal".to_string(),
        RunOutcome::SpawnFailed(e) => format!("✘ no se pudo spawnear: {e}"),
    };
    push_line(&mut s.output, OutputLine::notice(notice));
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

#[derive(Debug, Clone)]
struct RunReport {
    stdout_lines: Vec<String>,
    stderr_lines: Vec<String>,
    outcome: RunOutcome,
}

#[derive(Debug, Clone)]
enum RunOutcome {
    Exited(i32),
    Signal,
    SpawnFailed(String),
}

fn run_external(cmd: &str, cwd: &std::path::Path) -> RunReport {
    let output = Command::new("sh").arg("-c").arg(cmd).current_dir(cwd).output();
    match output {
        Ok(out) => {
            let stdout_lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(String::from)
                .collect();
            let stderr_lines: Vec<String> = String::from_utf8_lossy(&out.stderr)
                .lines()
                .map(String::from)
                .collect();
            let outcome = match out.status.code() {
                Some(code) => RunOutcome::Exited(code),
                None => RunOutcome::Signal,
            };
            RunReport {
                stdout_lines,
                stderr_lines,
                outcome,
            }
        }
        Err(e) => RunReport {
            stdout_lines: Vec::new(),
            stderr_lines: Vec::new(),
            outcome: RunOutcome::SpawnFailed(e.to_string()),
        },
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
    let label = format!(
        "Shell · {} · cwd: {}",
        state.source.label(),
        pretty_path(&state.cwd)
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(label, 12.0, theme.fg_text, Alignment::Start)
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
        assert!(s.output.iter().any(|l| l.text.starts_with("✘ exit")));
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
        assert!(dispatch("desconocido").is_none());
    }

    #[test]
    fn contributions_expose_clear_shortcut() {
        let s = State::new(Source::Local);
        let c = contributions(&s);
        assert!(c.monitors.is_empty());
        assert_eq!(c.shortcuts.len(), 1);
        assert_eq!(c.shortcuts[0].label, "Clear");
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
