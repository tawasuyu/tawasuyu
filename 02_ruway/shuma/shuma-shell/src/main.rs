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

use std::collections::HashMap;

use gpui::{
    div, point, prelude::*, px, App, Bounds, Context, CursorStyle, Element, ElementId, FocusHandle,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyDownEvent, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Render, ScrollHandle,
    SharedString, Style, Window,
};
use nahual_launcher::launch_app;
use nahual_theme::Theme;
use shuma_exec::{run as exec_run, CommandSpec, Exec, RunEvent, RunHandle, StageSpec};
use shuma_line::{CompletionKind, CompletionSource, LineState, TokenKind};
use shuma_session::{CommandRun, RunId, RunStatus, Stream, WorkSession};
use shuma_sysmon::{Snapshot, SystemSampler};

/// Cuántas muestras guarda la curva de cada monitor.
const HISTORY: usize = 80;
/// Archivos/directorios que delatan la estructura de un proyecto.
const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "go.mod",
    "Makefile",
    "pyproject.toml",
    "pom.xml",
    "build.gradle",
];

/// Marcadores de proyecto presentes en `dir`.
fn markers_in(dir: &str) -> Vec<String> {
    let base = std::path::Path::new(dir);
    PROJECT_MARKERS
        .iter()
        .filter(|m| base.join(m).exists())
        .map(|m| m.to_string())
        .collect()
}

/// Segundo Unix actual.
fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Índice de grupo (0-based) de una tecla de función `f1`..`f8`.
fn fkey_index(key: &str) -> Option<usize> {
    let n: usize = key.strip_prefix('f')?.parse().ok()?;
    (1..=8).contains(&n).then_some(n - 1)
}

/// Quita las comillas exteriores de un argumento (`"hola"` → `hola`).
fn unquote(arg: &str) -> String {
    let b = arg.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        let inner = &arg[1..arg.len() - 1];
        if b[0] == b'"' {
            inner.replace("\\\"", "\"").replace("\\\\", "\\")
        } else {
            inner.to_string()
        }
    } else {
        arg.to_string()
    }
}

/// Decide cómo ejecutar una línea. Si es un pipe «simple» —sólo comandos,
/// argumentos y `|`, sin `$`, redirecciones, operadores ni globs— brahman
/// la ejecuta **directo**, conectando los procesos él mismo. Si tiene
/// sintaxis que el modo directo aún no absorbe, cae a `bash -c`: bash
/// queda como un parser de sintaxis, no como el ejecutor por defecto.
fn plan_exec(line: &str) -> Exec {
    use shuma_line::TokenKind::*;
    let tokens = shuma_line::tokenize(line, shuma_line::Dialect::Bash);
    let simple = !tokens.is_empty()
        && tokens.iter().all(|t| {
            matches!(t.kind, Command | Argument | Flag | StringLit | Pipe | Whitespace)
                && !t.text.contains(['*', '?', '[', ']', '{', '}'])
                && !t.text.starts_with('~')
        });
    if simple {
        let pipeline = shuma_line::split_pipeline(&tokens);
        let mut stages = Vec::new();
        for st in &pipeline.stages {
            match &st.command {
                Some(cmd) => stages.push(StageSpec {
                    program: cmd.clone(),
                    args: st.args.iter().map(|a| unquote(a)).collect(),
                }),
                // Una etapa sin comando (línea incompleta) → al shell.
                None => return Exec::Shell { line: line.into(), program: "bash".into() },
            }
        }
        if !stages.is_empty() {
            return Exec::Direct { stages };
        }
    }
    Exec::Shell { line: line.into(), program: "bash".into() }
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

/// Estado de presentación de la tarjeta de un comando.
#[derive(Default, Clone, Copy)]
struct RunUi {
    /// Acordeón cerrado — sólo se ve la cabecera.
    collapsed: bool,
    /// El filtro muestra stderr en vez de stdout.
    show_stderr: bool,
    /// El usuario tocó el acordeón a mano — ya no se autocolapsa.
    user_touched: bool,
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
    /// Estado de presentación por comando (acordeón, filtro stderr).
    run_ui: HashMap<RunId, RunUi>,
    /// Largo del historial en el último `:save` — define qué comandos
    /// entran al próximo grupo guardado.
    group_anchor: usize,
    /// Patrones detectados por el motor de inferencia (cache).
    patterns: Vec<shuma_infer::EmergingPattern>,
    /// Si está activo, el próximo comando reprocesa la salida de este run.
    reprocess_source: Option<RunId>,
    /// Scroll del feed central — sigue al comando más reciente.
    scroll: ScrollHandle,
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
            run_ui: HashMap::new(),
            group_anchor: 0,
            patterns: Vec::new(),
            reprocess_source: None,
            scroll: ScrollHandle::new(),
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
                    RunEvent::Stdout(l) => {
                        self.session.append_output(*id, Stream::Stdout, l)
                    }
                    RunEvent::Stderr(l) => {
                        self.session.append_output(*id, Stream::Stderr, l)
                    }
                    RunEvent::Truncated => self.session.mark_truncated(*id),
                    RunEvent::Spilled(path) => {
                        self.session.mark_truncated(*id);
                        self.session.append_output(
                            *id,
                            Stream::Stdout,
                            format!("↡ salida excedente volcada a {path}"),
                        );
                    }
                    RunEvent::Exited(code) => self.session.finish_run(*id, code, now),
                    RunEvent::Failed(msg) => {
                        self.session.append_output(
                            *id,
                            Stream::Stderr,
                            format!("✗ no se pudo lanzar: {msg}"),
                        );
                        self.session.finish_run(*id, -1, now);
                    }
                }
            }
        }
        let finished = self.active.iter().any(|(_, h)| h.is_finished());
        self.active.retain(|(_, h)| !h.is_finished());
        if finished {
            // Al cerrarse un comando, el motor de inferencia revisa si
            // emergió un patrón repetido y lo promueve a un grupo.
            self.infer_patterns();
        }
        if changed {
            self.scroll.scroll_to_bottom();
        }
        changed
    }

    /// Comandos del historial reducidos a registros de inferencia.
    fn infer_records(&self) -> Vec<shuma_infer::CommandRecord> {
        self.session
            .history()
            .iter()
            .map(|r| {
                shuma_infer::CommandRecord::parse(&r.line, &r.cwd, r.status == RunStatus::Ok)
            })
            .collect()
    }

    /// Corre el motor de inferencia, cachea los patrones y promueve el
    /// más fuerte a un grupo reutilizable (rehidratación).
    fn infer_patterns(&mut self) {
        let records = self.infer_records();
        self.patterns =
            shuma_infer::detect_patterns(&records, &shuma_infer::InferConfig::default());
        if let Some(top) = self.patterns.first() {
            let name = format!("✨ {}", top.suggested_name());
            if self.session.group(&name).is_none() {
                self.session.save_group(name, top.example.clone());
            }
        }
    }

    /// Condición de disparo de un patrón: los marcadores de proyecto
    /// comunes a todos los directorios donde corrió.
    fn pattern_trigger(&self, p: &shuma_infer::EmergingPattern) -> Vec<String> {
        let mut dirs = p.directories.iter();
        let Some(first) = dirs.next() else {
            return Vec::new();
        };
        let mut common = markers_in(first);
        for d in dirs {
            let here = markers_in(d);
            common.retain(|m| here.contains(m));
        }
        common
    }

    /// La secuencia que el motor predice como continuación, si la hay.
    fn predicted_sequence(&self) -> Option<String> {
        if self.patterns.is_empty() {
            return None;
        }
        let records = self.infer_records();
        let tail = &records[records.len().saturating_sub(6)..];
        let (pi, next) = shuma_infer::predict_next(tail, &self.patterns)?;
        if next.is_empty() {
            return None;
        }
        // Disparo por estructura: no anticipar un patrón en un directorio
        // que no comparte su forma (no sugerir `cargo` sin `Cargo.toml`).
        let trigger = self.pattern_trigger(&self.patterns[pi]);
        if !trigger.is_empty() {
            let here = markers_in(self.session.cwd());
            if !trigger.iter().all(|m| here.contains(m)) {
                return None;
            }
        }
        Some(next.join(" && "))
    }

    /// Calcula el sufijo fantasma del prompt: el resto de la línea que el
    /// shell predice. Sólo con el cursor al final.
    fn compute_ghost(&self) -> Option<String> {
        let line = self.line.text();
        if line.is_empty() || self.line.cursor() != line.len() {
            return None;
        }
        // Corpus por prioridad: secuencia predicha, luego historial
        // reciente.
        let mut corpus: Vec<String> = Vec::new();
        if let Some(seq) = self.predicted_sequence() {
            corpus.push(seq);
        }
        for r in self.session.history().iter().rev() {
            corpus.push(r.line.clone());
        }
        shuma_line::ghost_suggestion(line, &corpus)
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
    /// Guarda como grupo los comandos ejecutados desde el último `:save`.
    fn save_group(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let n = self.session.history().len().saturating_sub(self.group_anchor);
        if n > 0 {
            self.session.save_recent_as_group(name, n);
            self.group_anchor = self.session.history().len();
        }
    }

    fn run_command(&mut self, line: String) {
        let line = line.trim().to_string();
        if line.is_empty() {
            return;
        }
        let now = unix_now();

        // Meta-comandos del shell — configuran la sesión, no se ejecutan.
        if let Some(name) = line.strip_prefix(":save ") {
            self.save_group(name);
            return;
        }
        if let Some(arg) = line.strip_prefix(":limit ") {
            // `:limit <MB>` — tope de captura de la sesión; 0 = sin tope.
            if let Ok(mb) = arg.trim().parse::<usize>() {
                self.session.set_capture_limit(mb * 1024 * 1024);
            }
            return;
        }
        if let Some(arg) = line.strip_prefix(":spill ") {
            // `:spill on|off` — volcar a disco la salida excedente.
            self.session
                .set_spill(matches!(arg.trim(), "on" | "si" | "sí" | "1" | "true"));
            return;
        }

        // Los comandos anteriores que el usuario no fijó se autocolapsan
        // al aparecer uno nuevo abajo — orden de terminal tradicional.
        for ui in self.run_ui.values_mut() {
            if !ui.user_touched {
                ui.collapsed = true;
            }
        }

        // `cd` interno — un subproceso no podría cambiar nuestro cwd.
        if line == "cd" || line.starts_with("cd ") {
            let arg = line.strip_prefix("cd").unwrap_or("").trim();
            let id = self.session.begin_run(&line, now);
            self.run_ui.insert(id, RunUi::default());
            match self.resolve_cd(arg) {
                Ok(new_cwd) => {
                    self.session.set_cwd(new_cwd.clone());
                    self.source.cwd = new_cwd.clone();
                    self.session
                        .append_output(id, Stream::Stdout, format!("→ {new_cwd}"));
                    self.session.finish_run(id, 0, now);
                }
                Err(e) => {
                    self.session.append_output(id, Stream::Stderr, e);
                    self.session.finish_run(id, 1, now);
                }
            }
            self.scroll.scroll_to_bottom();
            return;
        }

        let id = self.session.begin_run(&line, now);
        self.run_ui.insert(id, RunUi::default());
        let spec = self.build_spec(&line, None, id);
        self.active.push((id, exec_run(&spec)));
        self.scroll.scroll_to_bottom();
    }

    /// Arma la `CommandSpec` de una línea: decide directo vs shell y
    /// aplica la política de captura de la sesión.
    fn build_spec(&self, line: &str, stdin: Option<String>, run_id: RunId) -> CommandSpec {
        let policy = self.session.capture();
        let spill_path = (policy.spill && policy.limit_bytes > 0).then(|| {
            std::env::temp_dir()
                .join(format!("shuma-spill-{}-{run_id}.log", std::process::id()))
        });
        CommandSpec {
            exec: plan_exec(line),
            cwd: self.session.cwd().to_string(),
            capture_limit: policy.limit_bytes,
            spill_path,
            stdin_data: stdin,
        }
    }

    /// Reprocesa la salida capturada del comando `source`: ejecuta `line`
    /// alimentándole esa salida por stdin, sin volver a correr el
    /// original. Así un resultado se filtra con distintas herramientas.
    fn run_reprocess(&mut self, line: String, source: RunId) {
        let line = line.trim().to_string();
        if line.is_empty() {
            return;
        }
        let data: String = self
            .session
            .run(source)
            .map(|r| r.lines_of(Stream::Stdout).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        for ui in self.run_ui.values_mut() {
            if !ui.user_touched {
                ui.collapsed = true;
            }
        }
        let now = unix_now();
        let id = self.session.begin_run(&line, now);
        self.run_ui.insert(id, RunUi::default());
        let spec = self.build_spec(&line, Some(data), id);
        self.active.push((id, exec_run(&spec)));
        self.scroll.scroll_to_bottom();
    }

    /// Mata el proceso de un comando en curso.
    fn kill_run(&self, id: RunId) {
        if let Some((_, handle)) = self.active.iter().find(|(rid, _)| *rid == id) {
            handle.kill();
        }
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

    /// Enter — ejecuta el contenido del input, o reprocesa una salida
    /// previa si hay un origen de reproceso activo.
    fn submit(&mut self) {
        let line = self.line.text().to_string();
        self.line.clear();
        self.completion = None;
        self.show_completion = false;
        if let Some(source) = self.reprocess_source.take() {
            self.run_reprocess(line, source);
        } else {
            self.run_command(line);
        }
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
                self.reprocess_source = None;
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
                } else if self.line.cursor() == self.line.text().len() {
                    // En el extremo, la flecha derecha acepta el fantasma.
                    if let Some(g) = self.compute_ghost() {
                        self.line.insert(&g);
                        changed = true;
                    }
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
            "space" if ctrl => {
                // Ctrl+Space también acepta el fantasma predicho.
                if let Some(g) = self.compute_ghost() {
                    self.line.insert(&g);
                    changed = true;
                }
            }
            _ if fkey_index(key).is_some() => {
                // F1..F8 ejecutan el grupo de esa posición en [RUN].
                let idx = fkey_index(key).unwrap();
                let joined =
                    self.session.groups().get(idx).map(|g| g.lines.join(" && "));
                if let Some(j) = joined {
                    self.run_command(j);
                }
                cx.notify();
                return;
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

/// Renderiza la tarjeta de un comando ejecutado: cabecera-acordeón +
/// filtro stdout/stderr + cuerpo de salida.
fn render_run(
    r: &CommandRun,
    ui: RunUi,
    theme: &Theme,
    node_bg: Hsla,
    cx: &mut Context<Shell>,
) -> impl IntoElement {
    let id = r.id;
    let dim = theme.fg_muted;
    let (glyph, gcolor) = match r.status {
        RunStatus::Running => ("▷", gpui::hsla(45.0 / 360.0, 0.75, 0.60, 1.0)),
        RunStatus::Ok => ("✓", gpui::hsla(140.0 / 360.0, 0.48, 0.55, 1.0)),
        RunStatus::Failed => ("✗", gpui::hsla(2.0 / 360.0, 0.68, 0.60, 1.0)),
    };
    let stderr_color = gpui::hsla(8.0 / 360.0, 0.62, 0.66, 1.0);
    let accent = gpui::hsla(190.0 / 360.0, 0.60, 0.62, 1.0);

    // Nota a la derecha: salida no-cero, truncado, y conteo si colapsada.
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = r.exit_code {
        if c != 0 {
            parts.push(format!("salió {c}"));
        }
    }
    if r.truncated {
        parts.push("⚠ truncado".to_string());
    }
    if ui.collapsed {
        let n = r.count_of(Stream::Stdout);
        if n > 0 {
            parts.push(format!("{n} líneas"));
        }
    }
    let note = parts.join(" · ");

    // Cabecera-acordeón: un clic colapsa/expande.
    let caret = if ui.collapsed { "▸" } else { "▾" };
    let header_left = div()
        .id(SharedString::from(format!("hdr-{id}")))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .flex_1()
        .cursor_pointer()
        .child(div().flex_none().text_color(dim).child(caret))
        .child(div().flex_none().text_color(gcolor).child(glyph))
        .children(highlight(&r.line, theme))
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(dim)
                .child(SharedString::from(note)),
        )
        .on_click(cx.listener(move |shell, _, _, cx| {
            let e = shell.run_ui.entry(id).or_default();
            e.collapsed = !e.collapsed;
            e.user_touched = true;
            cx.notify();
        }));

    // Filtro de stderr — sólo aparece si el comando emitió errores.
    let stderr_chip = if r.has_stderr() {
        let n = r.count_of(Stream::Stderr);
        Some(
            div()
                .id(SharedString::from(format!("err-{id}")))
                .flex_none()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(3.))
                .text_size(px(11.))
                .cursor_pointer()
                .when(ui.show_stderr, |d| {
                    d.bg(stderr_color).text_color(gpui::hsla(0.0, 0.0, 0.12, 1.0))
                })
                .when(!ui.show_stderr, |d| d.text_color(stderr_color))
                .child(SharedString::from(format!("⚠ {n}")))
                .on_click(cx.listener(move |shell, _, _, cx| {
                    let e = shell.run_ui.entry(id).or_default();
                    e.show_stderr = !e.show_stderr;
                    cx.notify();
                })),
        )
    } else {
        None
    };

    // Botón de matar — sólo mientras el comando sigue corriendo.
    let kill_chip = if r.status == RunStatus::Running {
        Some(
            div()
                .id(SharedString::from(format!("kill-{id}")))
                .flex_none()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(3.))
                .text_size(px(11.))
                .text_color(gpui::hsla(2.0 / 360.0, 0.66, 0.64, 1.0))
                .cursor_pointer()
                .hover(|s| s.bg(gpui::hsla(2.0 / 360.0, 0.55, 0.28, 1.0)))
                .child("✕ matar")
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.kill_run(id);
                    cx.notify();
                })),
        )
    } else {
        None
    };

    // Reprocesar — sólo si el comando dejó algo en stdout que filtrar.
    let reprocess_chip = if r.count_of(Stream::Stdout) > 0 {
        Some(
            div()
                .id(SharedString::from(format!("repro-{id}")))
                .flex_none()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(3.))
                .text_size(px(11.))
                .text_color(accent)
                .cursor_pointer()
                .hover(|s| s.text_color(gpui::hsla(0.0, 0.0, 0.95, 1.0)))
                .child("⤳ reprocesar")
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.reprocess_source = Some(id);
                    cx.notify();
                })),
        )
    } else {
        None
    };

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .child(header_left)
        .children(stderr_chip)
        .children(reprocess_chip)
        .children(kill_chip);

    // Cuerpo: sólo con el acordeón abierto. El filtro elige el flujo.
    let mut body: Vec<gpui::Div> = Vec::new();
    if !ui.collapsed {
        // Etapas del pipe: un clic re-ejecuta la línea hasta esa etapa,
        // como un comando nuevo — así se inspeccionan los intermedios.
        if r.line.contains('|') {
            let toks = shuma_line::tokenize(&r.line, shuma_line::Dialect::Bash);
            let pipe = shuma_line::split_pipeline(&toks);
            if pipe.stages.len() >= 2 {
                let chip_bg = gpui::hsla(220.0 / 360.0, 0.18, 0.24, 1.0);
                let accent = gpui::hsla(190.0 / 360.0, 0.62, 0.62, 1.0);
                let mut chips: Vec<gpui::AnyElement> = vec![div()
                    .flex_none()
                    .text_size(px(10.))
                    .text_color(dim)
                    .child("⇢ etapas")
                    .into_any_element()];
                for (i, st) in pipe.stages.iter().enumerate() {
                    let end = st.tokens.last().map(|t| t.end).unwrap_or(r.line.len());
                    let prefix = r.line[..end].trim().to_string();
                    let name =
                        st.command.clone().unwrap_or_else(|| format!("{}", i + 1));
                    chips.push(
                        div()
                            .id(SharedString::from(format!("stage-{id}-{i}")))
                            .flex_none()
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(3.))
                            .bg(chip_bg)
                            .text_size(px(11.))
                            .text_color(accent)
                            .cursor_pointer()
                            .hover(|s| s.text_color(gpui::hsla(0.0, 0.0, 0.95, 1.0)))
                            .child(SharedString::from(name))
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                shell.run_command(prefix.clone());
                                cx.notify();
                            }))
                            .into_any_element(),
                    );
                }
                body.push(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(4.))
                        .items_center()
                        .children(chips),
                );
            }
        }

        let stream = if ui.show_stderr { Stream::Stderr } else { Stream::Stdout };
        let lines: Vec<&str> = r.lines_of(stream).collect();
        let color = if ui.show_stderr { stderr_color } else { theme.fg_text };
        if lines.is_empty() {
            body.push(div().text_size(px(11.)).text_color(dim).child(
                if ui.show_stderr { "sin errores" } else { "sin salida" },
            ));
        } else {
            // Sin truncar: si hay contenido, se muestra entero.
            for l in &lines {
                body.push(
                    div()
                        .text_size(px(12.))
                        .text_color(color)
                        .child(SharedString::from(l.to_string())),
                );
            }
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .p(px(8.))
        .bg(node_bg)
        .border_l_2()
        .border_color(gcolor)
        .rounded(px(5.))
        .child(header)
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
                div().text_color(dim).text_size(px(12.)).child(SharedString::from({
                    // Política de captura de la sesión: tope + volcado.
                    let pol = self.session.capture();
                    let cap = if pol.limit_bytes == 0 {
                        "cap ∞".to_string()
                    } else {
                        format!(
                            "cap {}M{}",
                            pol.limit_bytes / (1024 * 1024),
                            if pol.spill { "↡" } else { "" }
                        )
                    };
                    let running = if piped > 1 {
                        format!("⇄ {piped} etapas · {} en curso", self.active.len())
                    } else {
                        format!("{} en curso", self.active.len())
                    };
                    format!("{cap}  ·  {running}")
                })),
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
                .enumerate()
                .map(|(idx, g)| {
                    let joined = g.lines.join(" && ");
                    let count = g.lines.len();
                    // Los 8 primeros grupos llevan atajo dinámico F1..F8.
                    let label = if idx < 8 {
                        format!("F{}  ▸ {}  ·{count}", idx + 1, g.name)
                    } else {
                        format!("▸ {}  ·{count}", g.name)
                    };
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
                        .child(SharedString::from(label))
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
                                .flex()
                                .flex_row()
                                .gap(px(4.))
                                .items_center()
                                .child(
                                    div()
                                        .id("save-group")
                                        .px(px(5.))
                                        .text_color(dim)
                                        .cursor_pointer()
                                        .hover(|s| s.text_color(accent))
                                        .child("＋")
                                        .on_click(cx.listener(|s, _, _, cx| {
                                            s.line.set_text(":save ");
                                            cx.notify();
                                        })),
                                )
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
                        ),
                )
                .children(groups)
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(dim)
                        .child("clic ejecuta · ＋ guarda lo último"),
                )
        };

        // --- Lienzo central: comandos ejecutados + su salida ---
        // Orden de terminal: los más viejos arriba, los nuevos abajo.
        let hist = self.session.history();
        let start = hist.len().saturating_sub(40);
        let runs: Vec<_> = hist[start..]
            .iter()
            .map(|r| {
                let ui = self.run_ui.get(&r.id).copied().unwrap_or_default();
                render_run(r, ui, &theme, node_bg, cx)
            })
            .collect();
        let runs_empty = runs.is_empty();
        let canvas = div()
            .id("runs")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
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
        // Sugerencia fantasma — el resto que el shell predice, en gris.
        if let Some(ghost) = self.compute_ghost() {
            input_row.push(
                div()
                    .flex_none()
                    .text_color(theme.fg_disabled)
                    .child(SharedString::from(ghost)),
            );
        }
        let input_bar = div()
            .h(px(46.))
            .flex()
            .flex_row()
            .items_center()
            .px(px(14.))
            .text_color(text)
            .text_size(px(14.))
            .children(input_row);
        // Banner del modo reproceso — escribí un filtro para la salida.
        let banner = self.reprocess_source.map(|src| {
            div()
                .px(px(14.))
                .py(px(3.))
                .bg(gpui::hsla(190.0 / 360.0, 0.30, 0.22, 1.0))
                .text_size(px(11.))
                .text_color(accent)
                .child(SharedString::from(format!(
                    "⤳ reprocesando la salida de #{src} — escribí un filtro · Esc cancela"
                )))
        });
        let prompt = div()
            .flex()
            .flex_col()
            .bg(panel)
            .children(banner)
            .child(input_bar);

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

impl Drop for Shell {
    /// Al cerrar la sesión, limpia sus archivos de volcado temporales.
    fn drop(&mut self) {
        let prefix = format!("shuma-spill-{}-", std::process::id());
        if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

fn main() {
    launch_app("brahman · shuma shell", (1100., 700.), Shell::new);
}
