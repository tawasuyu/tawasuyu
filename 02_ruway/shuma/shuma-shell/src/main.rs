//! `shuma-shell` — el shell de brahman, vivo.
//!
//! Tres zonas alrededor de su función principal, el input de abajo:
//!
//! ```text
//!   ┌─ estado ─────────────────────────────────────────┐
//!   │ [RUN]  │   Lienzo de Contexto        │  [SENS]    │
//!   │ macros │   (grafo de intenciones)    │ monitores  │
//!   └─ prompt inteligente ─────────────────────────────┘
//! ```
//!
//! El input no es un campo de texto tonto: `shuma-line` analiza la línea
//! bash mientras se escribe —resaltado por token, autocompletado
//! posicional, descomposición de los pipes—. Los monitores de la derecha
//! grafican CPU y memoria con `shuma-sysmon`. Toda la lógica vive en
//! crates agnósticos; este binario sólo es el frontend GPUI.

use std::panic;
use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, App, Bounds, Context, Element, ElementId, FocusHandle,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyDownEvent, LayoutId, PathBuilder,
    Pixels, Render, SharedString, Style, Window,
};
use nahual_launcher::launch_app;
use nahual_theme::Theme;
use shuma_intent::{Macro, MacroBook, NodeStatus, SessionGraph};
use shuma_line::{CompletionKind, CompletionSource, LineState, TokenKind};
use shuma_shell_render::{layout, LayoutParams};
use shuma_sysmon::{Snapshot, SystemSampler};

/// Cuántas muestras guarda la curva de cada monitor.
const HISTORY: usize = 80;

// =====================================================================
// Fuente de autocompletado — la parte que sí toca el sistema.
// =====================================================================

/// Provee candidatos reales: comandos del `PATH` y rutas del disco.
struct ShellCompletionSource {
    commands: Vec<String>,
}

impl ShellCompletionSource {
    /// Escanea el `PATH` una vez al arrancar.
    fn scan() -> Self {
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
        Self { commands }
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
        let read_from = if dir.is_empty() { "." } else { dir };
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(read_from) {
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
// CurveElement — la "curvita" de un monitor.
// =====================================================================

/// `Element` GPUI que pinta una serie `0..=100` como una curva.
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

struct Shell {
    /// El input inteligente — texto, cursor, análisis.
    line: LineState,
    /// Lienzo: el grafo de intenciones de la sesión.
    session: SessionGraph,
    macros: MacroBook,
    /// Autocompletado vigente y el candidato seleccionado.
    completion: Option<shuma_line::Completion>,
    completion_index: usize,
    show_completion: bool,
    source: ShellCompletionSource,
    /// Muestreo de CPU/memoria.
    sampler: SystemSampler,
    snapshot: Snapshot,
    /// Estado de los paneles laterales.
    left_collapsed: bool,
    right_collapsed: bool,
    focus: FocusHandle,
    focused_once: bool,
}

impl Shell {
    fn new(cx: &mut Context<Self>) -> Self {
        // Datos de ejemplo para que el lienzo no nazca vacío.
        let mut session = SessionGraph::new();
        let c1 = session.record("ssh remote 'cat data.json'");
        session.complete(c1, true, 2_400_000);
        let c2 = session.record("sort | %p1");
        session.complete(c2, true, 2_390_000);

        let mut macros = MacroBook::new();
        macros.insert(Macro::new("build").bind("F1").step("cargo build --release"));
        macros.insert(Macro::new("deploy").bind("F2").step("scp target host:/srv"));
        macros.insert(Macro::new("clean").bind("F3").step("cargo clean"));

        let shell = Self {
            line: LineState::new(),
            session,
            macros,
            completion: None,
            completion_index: 0,
            show_completion: false,
            source: ShellCompletionSource::scan(),
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
            focus: cx.focus_handle(),
            focused_once: false,
        };
        shell.start_sampler(cx);
        shell
    }

    /// Bucle de fondo que refresca los monitores ~1 vez por segundo.
    fn start_sampler(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(1100)).await;
            let alive = this.update(cx, |shell, cx| {
                shell.snapshot = shell.sampler.sample();
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }

    /// Recalcula el autocompletado tras un cambio en la línea.
    fn refresh_completion(&mut self) {
        let comp = self.line.complete(&self.source);
        // El popup se muestra solo si hay una palabra parcial en curso.
        self.show_completion =
            !comp.candidates.is_empty() && comp.replace_end > comp.replace_start;
        self.completion_index = 0;
        self.completion = Some(comp);
    }

    /// Tab: muestra el popup, o aplica el candidato seleccionado si ya
    /// estaba visible.
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

    /// Mueve la selección del popup.
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

    /// Enter: registra la línea como una intención en el lienzo.
    fn submit(&mut self) {
        let cmd = self.line.text().trim().to_string();
        if !cmd.is_empty() {
            let id = self.session.record(&cmd);
            self.session.complete(id, true, 0);
        }
        self.line.clear();
        self.completion = None;
        self.show_completion = false;
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

/// Estado del nodo del lienzo → color de borde.
fn status_rgb(s: NodeStatus) -> Hsla {
    match s {
        NodeStatus::Running => gpui::hsla(45.0 / 360.0, 0.70, 0.55, 1.0),
        NodeStatus::Ok => gpui::hsla(140.0 / 360.0, 0.45, 0.52, 1.0),
        NodeStatus::Failed => gpui::hsla(2.0 / 360.0, 0.65, 0.55, 1.0),
    }
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
        let stage_count = pipeline.stages.iter().filter(|s| s.command.is_some()).count();

        // --- Zona de estado ---
        let pipe_note = if pipeline.is_piped() {
            format!("  ·  ⇄ {stage_count} etapas")
        } else {
            String::new()
        };
        let status = div()
            .h(px(32.))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(14.))
            .bg(panel)
            .text_color(text)
            .child(SharedString::from(format!(
                "● shuma · shell brahman{pipe_note}"
            )))
            .child(
                div()
                    .text_color(dim)
                    .text_size(px(12.))
                    .child(SharedString::from(format!("{} · launcher", self.line.dialect().name()))),
            );

        // --- Panel izquierdo: macros [RUN] ---
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
            let run_items: Vec<_> = self
                .macros
                .all()
                .iter()
                .map(|m| {
                    let key = m.key.clone().unwrap_or_default();
                    div()
                        .px(px(8.))
                        .py(px(6.))
                        .bg(node_bg)
                        .rounded(px(4.))
                        .text_color(text)
                        .text_size(px(13.))
                        .child(SharedString::from(format!("{key}  {}", m.name)))
                })
                .collect();
            div()
                .id("run-panel")
                .w(px(168.))
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
                        .child(div().text_color(dim).text_size(px(12.)).child("[RUN]"))
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
                .children(run_items)
        };

        // --- Lienzo central: grafo de intenciones ---
        let plan = layout(&self.session, &LayoutParams::default());
        let node_els: Vec<_> = plan
            .nodes
            .iter()
            .map(|n| {
                div()
                    .absolute()
                    .left(px(n.rect.x))
                    .top(px(n.rect.y))
                    .w(px(n.rect.w))
                    .h(px(n.rect.h))
                    .p(px(6.))
                    .bg(node_bg)
                    .border_2()
                    .border_color(status_rgb(n.status))
                    .rounded(px(4.))
                    .text_color(text)
                    .text_size(px(12.))
                    .child(SharedString::from(format!("%c{}", n.command_id)))
                    .child(div().text_color(dim).child(SharedString::from(n.label.clone())))
            })
            .collect();
        let canvas = div()
            .flex_1()
            .relative()
            .overflow_hidden()
            .p(px(12.))
            .bg(bg.clone())
            .child(div().text_color(dim).text_size(px(12.)).child("Lienzo de Contexto"))
            .children(node_els);

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
            let mem = self.snapshot.mem_percent;
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
                .w(px(184.))
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
                .child(monitor(
                    "CPU",
                    format!("{cpu:.0} %"),
                    cpu_curve,
                    cpu_color,
                ))
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
                .child(
                    div()
                        .text_color(dim)
                        .text_size(px(10.))
                        .child(SharedString::from(format!("mem {mem:.0} %"))),
                )
        };

        // --- Zona prompt: el input inteligente ---
        let mut input_row: Vec<gpui::Div> = vec![div()
            .flex_none()
            .text_color(accent)
            .child("›  ")];
        let cursor = self.line.cursor();
        let tokens = self.line.tokens();
        let caret = || div().w(px(2.)).h(px(19.)).bg(accent);
        if tokens.is_empty() {
            input_row.push(caret());
            input_row.push(
                div()
                    .text_color(dim)
                    .child("escribe un comando…  (Tab autocompleta)"),
            );
        } else {
            let mut caret_done = false;
            for t in &tokens {
                let color = token_color(t.kind, &theme);
                if !caret_done && cursor >= t.start && cursor < t.end {
                    let local = cursor - t.start;
                    let (left_s, right_s) = t.text.split_at(local);
                    if !left_s.is_empty() {
                        input_row.push(
                            div().flex_none().text_color(color).child(left_s.to_string()),
                        );
                    }
                    input_row.push(caret());
                    input_row.push(
                        div().flex_none().text_color(color).child(right_s.to_string()),
                    );
                    caret_done = true;
                } else {
                    input_row.push(
                        div().flex_none().text_color(color).child(t.text.clone()),
                    );
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

        // --- Popup de autocompletado (flotante sobre el prompt) ---
        let mut popup_layer: Vec<gpui::Div> = Vec::new();
        if self.show_completion {
            if let Some(comp) = &self.completion {
                if !comp.candidates.is_empty() {
                    let kind_label = match comp.kind {
                        CompletionKind::Command => "comando",
                        CompletionKind::Flag => "flag",
                        CompletionKind::Path => "ruta",
                    };
                    // Ventana de 8 candidatos centrada en la selección.
                    let total = comp.candidates.len();
                    let start = self.completion_index.saturating_sub(3).min(total.saturating_sub(8));
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
                                .when(selected, |d| d.bg(accent).text_color(gpui::hsla(0.0, 0.0, 0.1, 1.0)))
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
            .child(status)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .child(left)
                    .child(canvas)
                    .child(right),
            )
            .child(prompt)
            .children(popup_layer)
    }
}

fn main() {
    launch_app("brahman · shuma shell", (1080., 680.), Shell::new);
}
