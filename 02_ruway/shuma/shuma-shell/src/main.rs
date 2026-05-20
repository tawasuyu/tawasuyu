//! `shuma-shell` — el shell de brahman, en tres zonas.
//!
//! Layout fijo de la spec:
//! ```text
//!   ┌─ status ─────────────────────────────────────────┐
//!   │ [RUN]  │   Lienzo de Contexto        │  [SENS]    │
//!   │ macros │   (grafo de intenciones)    │ telemetría │
//!   └─ prompt fijo ────────────────────────────────────┘
//! ```
//!
//! La lógica vive en `shuma-intent` (parser + grafo + macros) y
//! `shuma-shell-render` (layout del lienzo); la ejecución real la hace
//! `sandokan`. Esta v1 renderiza la estructura con datos de ejemplo —
//! el cableado interactivo (typing en el prompt, F-keys) es el paso
//! siguiente.

use gpui::{div, prelude::*, px, Context, IntoElement, Render, SharedString, Window};
use nahual_launcher::launch_app;
use nahual_theme::Theme;
use shuma_intent::{Macro, MacroBook, NodeStatus, SessionGraph};
use shuma_shell_render::{layout, LayoutParams};

/// Estado del shell.
struct Shell {
    session: SessionGraph,
    macros: MacroBook,
    prompt: String,
}

impl Shell {
    fn new(_cx: &mut Context<Self>) -> Self {
        // --- Datos de ejemplo para ver la estructura poblada ---
        let mut session = SessionGraph::new();
        let c1 = session.record("ssh remote 'cat data.json'");
        session.complete(c1, true, 2_400_000);
        let c2 = session.record("sort | %p1");
        session.complete(c2, true, 2_390_000);
        let c3 = session.record("wc -l | %p2");
        session.complete(c3, false, 0);
        session.record("grep ERROR | %p1");

        let mut macros = MacroBook::new();
        macros.insert(Macro::new("build").bind("F1").step("cargo build --release"));
        macros.insert(Macro::new("deploy").bind("F2").step("scp target host:/srv"));
        macros.insert(Macro::new("clean").bind("F3").step("cargo clean"));

        Self {
            session,
            macros,
            prompt: "ssh remote 'cat data.json' | %p1 | sort".to_string(),
        }
    }
}

/// Color de borde según el estado de un nodo del lienzo.
fn status_rgb(s: NodeStatus) -> gpui::Rgba {
    match s {
        NodeStatus::Running => gpui::rgb(0xe0b341),
        NodeStatus::Ok => gpui::rgb(0x4caf6a),
        NodeStatus::Failed => gpui::rgb(0xd0463b),
    }
}

impl Render for Shell {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let bg = theme.bg_app;
        let panel = gpui::rgb(0x161b22);
        let node_bg = gpui::rgb(0x1c2128);
        let text = theme.fg_text;
        let dim = theme.fg_muted;
        let accent = gpui::rgb(0x88c0d0);

        // --- Zona status (arriba) ---
        let status = div()
            .h(px(34.))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(14.))
            .bg(panel)
            .text_color(text)
            .child("● sandokan UP   ·   brahman shell")
            .child(div().text_color(dim).child("shuma 0.1"));

        // --- Zona [RUN] — macros ---
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
                    .child(SharedString::from(format!("{key}  {}", m.name)))
            })
            .collect();
        let run = div()
            .w(px(160.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(10.))
            .bg(panel)
            .child(div().text_color(dim).child("[RUN]"))
            .children(run_items);

        // --- Zona lienzo central — grafo de intenciones ---
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
                    .child(SharedString::from(format!("%c{}", n.command_id)))
                    .child(
                        div()
                            .text_color(dim)
                            .child(SharedString::from(n.label.clone())),
                    )
            })
            .collect();
        let canvas = div()
            .flex_1()
            .relative()
            .overflow_hidden()
            .p(px(12.))
            .bg(bg)
            .child(div().text_color(dim).child("Lienzo de Contexto"))
            .children(node_els);

        // --- Zona [SENS] — telemetría ---
        let sens = div()
            .w(px(180.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(10.))
            .bg(panel)
            .text_color(text)
            .child(div().text_color(dim).child("[SENS]"))
            .child(
                div()
                    .p(px(8.))
                    .bg(node_bg)
                    .rounded(px(4.))
                    .child("CPU")
                    .child(div().text_color(accent).child("— °C")),
            )
            .child(
                div()
                    .p(px(8.))
                    .bg(node_bg)
                    .rounded(px(4.))
                    .child("MEM")
                    .child(div().text_color(accent).child("— G")),
            );

        // --- Zona prompt (abajo) ---
        let prompt = div()
            .h(px(40.))
            .flex()
            .items_center()
            .px(px(14.))
            .bg(panel)
            .text_color(text)
            .child(SharedString::from(format!("›  {}", self.prompt)));

        // --- Composición ---
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .child(status)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .child(run)
                    .child(canvas)
                    .child(sens),
            )
            .child(prompt)
    }
}

fn main() {
    launch_app("brahman · shuma shell", (1040., 660.), Shell::new);
}
