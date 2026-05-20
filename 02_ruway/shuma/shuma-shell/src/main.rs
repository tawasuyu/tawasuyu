//! `shuma-shell` — el shell de brahman, ejecutando de verdad.
//!
//! El shell trabaja *dentro de una sesión* ([`shuma_session::WorkSession`]):
//! un directorio actual —que es además el identificador de aislamiento—,
//! el historial de comandos ejecutados y los grupos reutilizables.
//!
//! ```text
//!   ┌─ estado · cwd · aislamiento ──────────────────────┐
//!   │ [RUN]   │   comandos ejecutados + su salida   │ [SENS] │
//!   │ grupos  │   (streaming en vivo)               │ monit. │
//!   └─ prompt inteligente ─────────────────────────────┘
//! ```
//!
//! El input se analiza con `shuma-line` (resaltado + autocompletado);
//! al ejecutar, `shuma-exec` lanza el comando y transmite su salida
//! línea a línea, que se vuelca en el panel central. El cerebro vive en
//! crates agnósticos — este binario sólo es el frontend GPUI.

use std::panic;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{
    div, point, prelude::*, px, App, Bounds, Context, CursorStyle, Element, ElementId, FocusHandle,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyDownEvent, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Render, SharedString, Style,
    Window,
};
use nahual_launcher::launch_app;
use nahual_theme::Theme;
use shuma_exec::{run as exec_run, CommandSpec, RunEvent, RunHandle};
use shuma_line::{CompletionKind, CompletionSource, LineState, TokenKind};
use shuma_session::{CommandRun, RunId, RunStatus, WorkSession};
use shuma_sysmon::{Snapshot, SystemSampler};

/// Cuántas muestras guarda la curva de cada monitor.
const HISTORY: usize = 80;
/// Líneas de salida visibles por comando (modo launcher liviano).
const OUTPUT_LINES: usize = 16;

/// Segundo Unix actual.
fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// =====================================================================
// Fuente de autocompletado.
// =====================================================================

struct ShellCompletionSource {
    commands: Vec<String>,
    cwd: String,
}

impl ShellCompletionSource {
    fn scan(cwd: String) -> Self {
        let mut commands = Vec::new();
        if let Ok(path) = std::env::var("PATH") {
            for dir in path.split(':') {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for e in entries.flatten() {
                        if let Some(name) = e.file_name().to_str() {
                            commands.push(name.to_string());
                        }
                    }
                }
            }
        }
        commands.sort();
        commands.dedup();
        Self { commands, cwd }
    }
}

impl CompletionSource for ShellCompletionSource {
    fn commands(&self) -> Vec<String> {
        self.commands.clone()
    }

    fn paths(&self, prefix: &str) -> Vec<String> {
        let (dir, partial) = match prefix.rfind('/') {
            Some(i) => (&prefix[..=i], &prefix[i + 1..]),
            None => ("", prefix),
        };
        // Una ruta relativa se resuelve contra el cwd de la sesión.
        let base = if dir.starts_with('/') {
            dir.to_string()
        } else if dir.is_empty() {
            self.cwd.clone()
        } else {
            format!("{}/{}", self.cwd, dir)
        };
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&base) {
            for e in entries.flatten() {
                if let Some(name) = e.file_name().to_str() {
                    if name.starts_with(partial) {
                        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        let slash = if is_dir { "/" } else { "" };
                        out.push(format!("{dir}{name}{slash}"));
                    }
                }
            }
        }
        out.sort();
        out
    }
}

// =====================================================================
// CurveElement — la curva de un monitor.
// =====================================================================

struct CurveElement {
    values: Vec<f32>,
    color: Hsla,
}

impl CurveElement {
    fn new(values: Vec<f32>, color: Hsla) -> Self {
        Self { values, color }
    }
}

impl IntoElement for CurveElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CurveElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _layout: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) {
        let n = self.values.len();
        if n < 2 {
            return;
        }
        let ox: f32 = bounds.origin.x.into();
        let oy: f32 = bounds.origin.y.into();
        let bw: f32 = bounds.size.width.into();
        let bh: f32 = bounds.size.height.into();
        let mut pb = PathBuilder::stroke(px(1.6));
        for (i, v) in self.values.iter().enumerate() {
            let x = ox + bw * (i as f32 / (n - 1) as f32);
            let y = oy + bh - (v.clamp(0.0, 100.0) / 100.0) * bh;
            let p = point(px(x), px(y));
            if i == 0 {
                pb.move_to(p);
            } else {
                pb.line_to(p);
            }
        }
        if let Ok(path) = pb.build() {
            window.paint_path(path, self.color);
        }
    }
}

// =====================================================================
// El shell.
// =====================================================================

/// Qué panel lateral está redimensionando un drag activo.
#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

/// Estado de un arrastre de divisor en curso.
struct Drag {
    side: Side,
    /// Posición X del cursor al iniciar el arrastre.
    start_x: f32,
    /// Ancho del panel al iniciar el arrastre.
    start_w: f32,
}

struct Shell {
    line: LineState,
    /// La sesión de trabajo: cwd, historial y grupos.
    session: WorkSession,
    /// Comandos en curso, con su canal de salida.
    active: Vec<(RunId, RunHandle)>,
    completion: Option<shuma_line::Completion>,
    completion_index: usize,
    show_completion: bool,
    source: ShellCompletionSource,
    sampler: SystemSampler,
    snapshot: Snapshot,
    left_collapsed: bool,
    right_collapsed: bool,
    /// Anchos de los paneles laterales (los divisores los ajustan).
    left_width: f32,
    right_width: f32,
    /// Arrastre de divisor en curso, si lo hay.
    drag: Option<Drag>,
    focus: FocusHandle,
    focused_once: bool,
}

impl Shell {
    fn new(cx: &mut Context<Self>) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "/".to_string());

        let mut session = WorkSession::new("sesión", &cwd);
        // Grupos de ejemplo — recetas reutilizables.
        session.save_group("estado git", vec!["git status --short".into()]);
        session.save_group("build", vec!["cargo build --release".into()]);

        let shell = Self {
            line: LineState::new(),
            session,
            active: Vec::new(),
            completion: None,
            completion_index: 0,
            show_completion: false,
            source: ShellCompletionSource::scan(cwd),
            sampler: SystemSampler::new(HISTORY),
            snapshot: Snapshot {
                cpu_percent: 0.0,
                mem_percent: 0.0,
                mem_used_mb: 0,
                mem_total_mb: 0,
                valid: false,
            },
            left_collapsed: false,
            right_collapsed: false,
            left_width: 176.0,
            right_width: 188.0,
            drag: None,
            focus: cx.focus_handle(),
            focused_once: false,
        };
        shell.start_loop(cx);
        shell
    }

    /// Bucle de fondo: drena la salida de los comandos (~9/s) y refresca
    /// los monitores (~1/s).
    fn start_loop(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut tick: u32 = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(110)).await;
                tick += 1;
                let sysmon = tick % 10 == 0;
                let alive = this.update(cx, |shell, cx| {
                    let mut changed = shell.drain_exec();
                    if sysmon {
                        shell.snapshot = shell.sampler.sample();
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Vuelca la salida disponible de los comandos en curso al historial.
    fn drain_exec(&mut self) -> bool {
        let now = unix_now();
        let mut changed = false;
        for (id, handle) in &mut self.active {
            for ev in handle.try_events() {
                changed = true;
                match ev {
                    RunEvent::Stdout(l) | RunEvent::Stderr(l) => {
                        self.session.append_output(*id, l)
                    }
                    RunEvent::Exited(code) => self.session.finish_run(*id, code, now),
                    RunEvent::Failed(msg) => {
                        self.session
                            .append_output(*id, format!("✗ no se pudo lanzar: {msg}"));
                        self.session.finish_run(*id, -1, now);
                    }
                }
            }
        }
        self.active.retain(|(_, h)| !h.is_finished());
        changed
    }

    /// Resuelve el destino de un `cd` contra el cwd de la sesión.
    fn resolve_cd(&self, arg: &str) -> Result<String, String> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let target = if arg.is_empty() || arg == "~" {
            home
        } else if let Some(rest) = arg.strip_prefix("~/") {
            format!("{home}/{rest}")
        } else if arg.starts_with('/') {
            arg.to_string()
        } else {
            format!("{}/{}", self.session.cwd(), arg)
        };
        match std::fs::canonicalize(&target) {
            Ok(p) if p.is_dir() => Ok(p.to_string_lossy().into_owned()),
            Ok(_) => Err(format!("cd: no es un directorio: {target}")),
            Err(e) => Err(format!("cd: {target}: {e}")),
        }
    }

    /// Ejecuta una línea: `cd` se maneja internamente (cambia el cwd y,
    /// con él, el aislamiento); el resto se lanza con `shuma-exec` y su
    /// salida se transmite al panel central.
    fn run_command(&mut self, line: String) {
        let line = line.trim().to_string();
        if line.is_empty() {
            return;
        }
        let now = unix_now();

        // `cd` interno — un subproceso no podría cambiar nuestro cwd.
        if line == "cd" || line.starts_with("cd ") {
            let arg = line.strip_prefix("cd").unwrap_or("").trim();
            let id = self.session.begin_run(&line, now);
            match self.resolve_cd(arg) {
                Ok(new_cwd) => {
                    self.session.set_cwd(new_cwd.clone());
                    self.source.cwd = new_cwd.clone();
                    self.session.append_output(id, format!("→ {new_cwd}"));
                    self.session.finish_run(id, 0, now);
                }
                Err(e) => {
                    self.session.append_output(id, e);
                    self.session.finish_run(id, 1, now);
                }
            }
            return;
        }

        let id = self.session.begin_run(&line, now);
        let spec = CommandSpec::bash(&line, self.session.cwd());
        self.active.push((id, exec_run(&spec)));
    }

    fn refresh_completion(&mut self) {
        let comp = self.line.complete(&self.source);
        self.show_completion =
            !comp.candidates.is_empty() && comp.replace_end > comp.replace_start;
        self.completion_index = 0;
        self.completion = Some(comp);
    }

    fn on_tab(&mut self) {
        let comp = self.line.complete(&self.source);
        if comp.candidates.is_empty() {
            return;
        }
        if self.show_completion {
            let idx = self.completion_index.min(comp.candidates.len() - 1);
            let candidate = comp.candidates[idx].clone();
            self.line.apply_completion(&comp, &candidate);
            self.show_completion = false;
            self.completion = None;
        } else {
            self.completion_index = 0;
            self.completion = Some(comp);
            self.show_completion = true;
        }
    }

    fn cycle_completion(&mut self, delta: i32) {
        if !self.show_completion {
            return;
        }
        if let Some(comp) = &self.completion {
            let n = comp.candidates.len();
            if n > 0 {
                let i = self.completion_index as i32 + delta;
                self.completion_index = i.rem_euclid(n as i32) as usize;
            }
        }
    }

    /// Enter — ejecuta el contenido del input.
    fn submit(&mut self) {
        let line = self.line.text().to_string();
        self.line.clear();
        self.completion = None;
        self.show_completion = false;
        self.run_command(line);
    }

    fn handle_key(&mut self, event: &KeyDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let key = ks.key.as_str();
        let ctrl = ks.modifiers.control;
        let mut changed = false;

        match key {
            "enter" => {
                self.submit();
                cx.notify();
                return;
            }
            "escape" => {
                self.show_completion = false;
                cx.notify();
                return;
            }
            "tab" => {
                self.on_tab();
                cx.notify();
                return;
            }
            "up" => {
                self.cycle_completion(-1);
                cx.notify();
                return;
            }
            "down" => {
                self.cycle_completion(1);
                cx.notify();
                return;
            }
            "backspace" => {
                self.line.backspace();
                changed = true;
            }
            "delete" => {
                self.line.delete();
                changed = true;
            }
            "left" => {
                if ctrl {
                    self.line.move_word_left();
                } else {
                    self.line.move_left();
                }
            }
            "right" => {
                if ctrl {
                    self.line.move_word_right();
                } else {
                    self.line.move_right();
                }
            }
            "home" => self.line.move_home(),
            "end" => self.line.move_end(),
            "a" if ctrl => self.line.move_home(),
            "e" if ctrl => self.line.move_end(),
            "u" if ctrl => {
                self.line.clear();
                changed = true;
            }
            _ => {
                if !ctrl {
                    if let Some(ch) = ks.key_char.as_deref() {
                        if !ch.chars().any(|c| c.is_control()) {
                            self.line.insert(ch);
                            changed = true;
                        }
                    }
                }
            }
        }

        if changed {
            self.refresh_completion();
        }
        cx.notify();
    }
}

/// Color de resaltado de cada clase de token.
fn token_color(kind: TokenKind, theme: &Theme) -> Hsla {
    match kind {
        TokenKind::Command => gpui::hsla(190.0 / 360.0, 0.65, 0.62, 1.0),
        TokenKind::Argument => theme.fg_text,
        TokenKind::Flag => gpui::hsla(38.0 / 360.0, 0.80, 0.62, 1.0),
        TokenKind::StringLit => gpui::hsla(95.0 / 360.0, 0.42, 0.60, 1.0),
        TokenKind::Variable => gpui::hsla(280.0 / 360.0, 0.55, 0.72, 1.0),
        TokenKind::Pipe => gpui::hsla(190.0 / 360.0, 0.90, 0.72, 1.0),
        TokenKind::Redirect => gpui::hsla(20.0 / 360.0, 0.78, 0.62, 1.0),
        TokenKind::Operator => gpui::hsla(0.0, 0.66, 0.66, 1.0),
        TokenKind::Comment => theme.fg_muted,
        TokenKind::Whitespace => theme.fg_text,
        TokenKind::Unknown => gpui::hsla(0.0, 0.70, 0.60, 1.0),
    }
}

/// Resalta un texto estático (la línea de un comando del historial).
fn highlight(text: &str, theme: &Theme) -> Vec<gpui::Div> {
    shuma_line::tokenize(text, shuma_line::Dialect::Bash)
        .into_iter()
        .map(|t| {
            div()
                .flex_none()
                .text_color(token_color(t.kind, theme))
                .child(t.text)
        })
        .collect()
}

/// Acorta el cwd: el `$HOME` se muestra como `~`.
fn pretty_cwd(cwd: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if cwd == home => "~".to_string(),
        Ok(home) if cwd.starts_with(&format!("{home}/")) => format!("~{}", &cwd[home.len()..]),
        _ => cwd.to_string(),
    }
}

/// Panel de un comando ejecutado: cabecera resaltada + su salida.
fn run_panel(r: &CommandRun, theme: &Theme, node_bg: Hsla) -> impl IntoElement {
    let (glyph, gcolor) = match r.status {
        RunStatus::Running => ("▷", gpui::hsla(45.0 / 360.0, 0.75, 0.60, 1.0)),
        RunStatus::Ok => ("✓", gpui::hsla(140.0 / 360.0, 0.48, 0.55, 1.0)),
        RunStatus::Failed => ("✗", gpui::hsla(2.0 / 360.0, 0.68, 0.60, 1.0)),
    };
    let dim = theme.fg_muted;

    let total = r.output.len();
    let skipped = total.saturating_sub(OUTPUT_LINES);
    let mut body: Vec<gpui::Div> = Vec::new();
    if skipped > 0 {
        body.push(
            div()
                .text_size(px(11.))
                .text_color(dim)
                .child(SharedString::from(format!("… {skipped} líneas antes"))),
        );
    }
    for line in r.output.iter().skip(skipped) {
        body.push(
            div()
                .text_size(px(12.))
                .text_color(theme.fg_text)
                .child(SharedString::from(line.clone())),
        );
    }

    let exit_note = match r.exit_code {
        Some(0) => String::new(),
        Some(c) => format!("salió {c}"),
        None => String::new(),
    };

    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .p(px(8.))
        .bg(node_bg)
        .border_l_2()
        .border_color(gcolor)
        .rounded(px(5.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .child(div().flex_none().text_color(gcolor).child(glyph))
                .children(highlight(&r.line, theme))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(dim)
                        .child(SharedString::from(exit_note)),
                ),
        )
        .children(body)
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            window.focus(&self.focus);
            self.focused_once = true;
        }
        let theme = Theme::global(cx).clone();
        let bg = theme.bg_app.clone();
        let panel = gpui::hsla(220.0 / 360.0, 0.16, 0.11, 1.0);
        let node_bg = gpui::hsla(220.0 / 360.0, 0.14, 0.16, 1.0);
        let accent = gpui::hsla(190.0 / 360.0, 0.70, 0.62, 1.0);
        let text = theme.fg_text;
        let dim = theme.fg_muted;

        let pipeline = self.line.pipeline();
        let piped = pipeline.stages.iter().filter(|s| s.command.is_some()).count();

        // --- Barra de estado: cwd + identificador de aislamiento ---
        let status = div()
            .h(px(32.))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(14.))
            .bg(panel)
            .text_color(text)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(10.))
                    .items_baseline()
                    .child("● shuma")
                    .child(div().text_color(accent).child(SharedString::from(format!(
                        "📁 {}",
                        pretty_cwd(self.session.cwd())
                    ))))
                    .child(div().text_color(dim).text_size(px(11.)).child(SharedString::from(
                        format!("aisl:{}", self.session.isolation_id()),
                    ))),
            )
            .child(
                div().text_color(dim).text_size(px(12.)).child(SharedString::from(
                    if piped > 1 {
                        format!("⇄ {piped} etapas · {} en curso", self.active.len())
                    } else {
                        format!("{} en curso", self.active.len())
                    },
                )),
            );

        // --- Panel izquierdo: grupos reutilizables [RUN] ---
        let left = if self.left_collapsed {
            div()
                .id("expand-left")
                .w(px(26.))
                .flex()
                .flex_col()
                .items_center()
                .pt(px(8.))
                .bg(panel)
                .text_color(dim)
                .cursor_pointer()
                .hover(|s| s.bg(node_bg))
                .child("»")
                .on_click(cx.listener(|s, _, _, cx| {
                    s.left_collapsed = false;
                    cx.notify();
                }))
        } else {
            let groups: Vec<_> = self
                .session
                .groups()
                .iter()
                .map(|g| {
                    let joined = g.lines.join(" && ");
                    let count = g.lines.len();
                    div()
                        .id(SharedString::from(format!("group-{}", g.name)))
                        .px(px(8.))
                        .py(px(6.))
                        .bg(node_bg)
                        .rounded(px(4.))
                        .text_color(text)
                        .text_size(px(13.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.bg_row_hover))
                        .child(SharedString::from(format!("▸ {}  ·{count}", g.name)))
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.run_command(joined.clone());
                            cx.notify();
                        }))
                })
                .collect();
            div()
                .id("run-panel")
                .w(px(self.left_width))
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(10.))
                .bg(panel)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(div().text_color(dim).text_size(px(12.)).child("[RUN] grupos"))
                        .child(
                            div()
                                .id("collapse-left")
                                .px(px(5.))
                                .text_color(dim)
                                .cursor_pointer()
                                .hover(|s| s.text_color(accent))
                                .child("«")
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.left_collapsed = true;
                                    cx.notify();
                                })),
                        ),
                )
                .children(groups)
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(dim)
                        .child("clic para ejecutar el grupo"),
                )
        };

        // --- Lienzo central: comandos ejecutados + su salida ---
        let runs: Vec<_> = self
            .session
            .history()
            .iter()
            .rev()
            .take(40)
            .map(|r| run_panel(r, &theme, node_bg))
            .collect();
        let runs_empty = runs.is_empty();
        let canvas = div()
            .id("runs")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(10.))
            .bg(bg.clone())
            .when(runs_empty, |d| {
                d.child(div().text_color(dim).child(
                    "Escribe un comando abajo y presiona Enter — su salida aparece aquí.",
                ))
            })
            .children(runs);

        // --- Panel derecho: monitores [SENS] ---
        let right = if self.right_collapsed {
            div()
                .id("expand-right")
                .w(px(26.))
                .flex()
                .flex_col()
                .items_center()
                .pt(px(8.))
                .bg(panel)
                .text_color(dim)
                .cursor_pointer()
                .hover(|s| s.bg(node_bg))
                .child("«")
                .on_click(cx.listener(|s, _, _, cx| {
                    s.right_collapsed = false;
                    cx.notify();
                }))
        } else {
            let cpu = self.snapshot.cpu_percent;
            let cpu_curve = self.sampler.cpu_history().values();
            let mem_curve = self.sampler.mem_history().values();
            let cpu_color = gpui::hsla(190.0 / 360.0, 0.72, 0.62, 1.0);
            let mem_color = gpui::hsla(265.0 / 360.0, 0.55, 0.70, 1.0);

            let monitor = |title: &str, value: String, curve: Vec<f32>, color: Hsla| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(8.))
                    .bg(node_bg)
                    .rounded(px(5.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_baseline()
                            .child(div().text_color(dim).text_size(px(11.)).child(title.to_string()))
                            .child(div().text_color(color).child(SharedString::from(value))),
                    )
                    .child(div().h(px(44.)).child(CurveElement::new(curve, color)))
            };

            div()
                .id("sens-panel")
                .w(px(self.right_width))
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(10.))
                .bg(panel)
                .text_color(text)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(div().text_color(dim).text_size(px(12.)).child("[SENS]"))
                        .child(
                            div()
                                .id("collapse-right")
                                .px(px(5.))
                                .text_color(dim)
                                .cursor_pointer()
                                .hover(|s| s.text_color(accent))
                                .child("»")
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.right_collapsed = true;
                                    cx.notify();
                                })),
                        ),
                )
                .child(monitor("CPU", format!("{cpu:.0} %"), cpu_curve, cpu_color))
                .child(monitor(
                    "MEM",
                    if self.snapshot.valid {
                        format!(
                            "{:.1}/{:.0} GB",
                            self.snapshot.mem_used_mb as f32 / 1024.0,
                            self.snapshot.mem_total_mb as f32 / 1024.0
                        )
                    } else {
                        "— GB".to_string()
                    },
                    mem_curve,
                    mem_color,
                ))
        };

        // --- Zona prompt: el input inteligente ---
        let mut input_row: Vec<gpui::Div> = vec![div().flex_none().text_color(accent).child("›  ")];
        let cursor = self.line.cursor();
        let tokens = self.line.tokens();
        let caret = || div().w(px(2.)).h(px(19.)).bg(accent);
        if tokens.is_empty() {
            input_row.push(caret());
            input_row.push(
                div()
                    .text_color(dim)
                    .child("escribe un comando…  (Tab autocompleta · Enter ejecuta)"),
            );
        } else {
            let mut caret_done = false;
            for t in &tokens {
                let color = token_color(t.kind, &theme);
                if !caret_done && cursor >= t.start && cursor < t.end {
                    let local = cursor - t.start;
                    let (left_s, right_s) = t.text.split_at(local);
                    if !left_s.is_empty() {
                        input_row
                            .push(div().flex_none().text_color(color).child(left_s.to_string()));
                    }
                    input_row.push(caret());
                    input_row
                        .push(div().flex_none().text_color(color).child(right_s.to_string()));
                    caret_done = true;
                } else {
                    input_row.push(div().flex_none().text_color(color).child(t.text.clone()));
                }
            }
            if !caret_done {
                input_row.push(caret());
            }
        }
        let prompt = div()
            .h(px(46.))
            .flex()
            .flex_row()
            .items_center()
            .px(px(14.))
            .bg(panel)
            .text_color(text)
            .text_size(px(14.))
            .children(input_row);

        // --- Popup de autocompletado ---
        let mut popup_layer: Vec<gpui::Div> = Vec::new();
        if self.show_completion {
            if let Some(comp) = &self.completion {
                if !comp.candidates.is_empty() {
                    let kind_label = match comp.kind {
                        CompletionKind::Command => "comando",
                        CompletionKind::Flag => "flag",
                        CompletionKind::Path => "ruta",
                    };
                    let total = comp.candidates.len();
                    let start = self
                        .completion_index
                        .saturating_sub(3)
                        .min(total.saturating_sub(8));
                    let rows: Vec<_> = comp
                        .candidates
                        .iter()
                        .enumerate()
                        .skip(start)
                        .take(8)
                        .map(|(i, cand)| {
                            let selected = i == self.completion_index;
                            div()
                                .px(px(8.))
                                .py(px(3.))
                                .when(selected, |d| {
                                    d.bg(accent).text_color(gpui::hsla(0.0, 0.0, 0.1, 1.0))
                                })
                                .when(!selected, |d| d.text_color(text))
                                .child(SharedString::from(cand.clone()))
                        })
                        .collect();
                    popup_layer.push(
                        div()
                            .absolute()
                            .left(px(28.))
                            .bottom(px(52.))
                            .w(px(320.))
                            .flex()
                            .flex_col()
                            .bg(node_bg)
                            .border_1()
                            .border_color(accent)
                            .rounded(px(5.))
                            .text_size(px(13.))
                            .child(
                                div()
                                    .px(px(8.))
                                    .py(px(3.))
                                    .text_color(dim)
                                    .text_size(px(11.))
                                    .child(SharedString::from(format!(
                                        "{kind_label} · {total} · ↑↓ Tab"
                                    ))),
                            )
                            .children(rows),
                    );
                }
            }
        }

        // --- Divisores arrastrables ---
        let divider = |side: Side, cx: &mut Context<Self>| {
            div()
                .w(px(5.))
                .bg(node_bg)
                .cursor(CursorStyle::ResizeLeftRight)
                .hover(|s| s.bg(accent))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |shell, ev: &MouseDownEvent, _w, cx| {
                        let start_w = match side {
                            Side::Left => shell.left_width,
                            Side::Right => shell.right_width,
                        };
                        shell.drag = Some(Drag {
                            side,
                            start_x: ev.position.x.into(),
                            start_w,
                        });
                        cx.notify();
                    }),
                )
        };

        let mut middle = div().flex().flex_row().flex_1().overflow_hidden().child(left);
        if !self.left_collapsed {
            middle = middle.child(divider(Side::Left, cx));
        }
        middle = middle.child(canvas);
        if !self.right_collapsed {
            middle = middle.child(divider(Side::Right, cx));
        }
        middle = middle.child(right);

        // --- Composición ---
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(bg)
            .track_focus(&self.focus)
            .key_context("ShumaShell")
            .on_key_down(cx.listener(Self::handle_key))
            .on_mouse_move(cx.listener(|shell, ev: &MouseMoveEvent, _w, cx| {
                if let Some(drag) = &shell.drag {
                    let cur: f32 = ev.position.x.into();
                    let delta = cur - drag.start_x;
                    match drag.side {
                        // El panel izquierdo crece al arrastrar a la derecha.
                        Side::Left => {
                            shell.left_width = (drag.start_w + delta).clamp(130.0, 420.0)
                        }
                        // El derecho crece al arrastrar a la izquierda.
                        Side::Right => {
                            shell.right_width = (drag.start_w - delta).clamp(130.0, 420.0)
                        }
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|shell, _ev: &MouseUpEvent, _w, cx| {
                    if shell.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(status)
            .child(middle)
            .child(prompt)
            .children(popup_layer)
    }
}

fn main() {
    launch_app("brahman · shuma shell", (1100., 700.), Shell::new);
}
