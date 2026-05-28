//! `shuma-module-canvas` — el **Lienzo de Contexto** del shell.
//!
//! Tab/panel que dibuja el `SessionGraph` de `shuma-intent` como un
//! grafo visual: cada comando `%cN` es una caja, las dependencias
//! `%pN` son flechas hacia el comando que las produjo. El usuario ve
//! el flujo entero de la sesión y puede saltar atrás (referencia
//! `%c3`) o "tirar de un hilo" para reusarlo.
//!
//! El layout es columnar por profundidad (longest-path): la columna
//! `0` son los comandos sin dependencias, la `N` los que dependen de
//! columnas `<N`. Status de cada nodo se colorea (running ámbar, ok
//! verde, failed rojo). Render directo con `paint_with` + vello — sin
//! depender de `pineal-render` para no arrastrar el backend de
//! dominium al chasis.
//!
//! El módulo es independiente del shell: el host puede sincronizar el
//! grafo enviando `Msg::Record` / `Msg::Complete` después de cada
//! ejecución, pero también funciona standalone con un grafo de demo.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::Theme;
use shuma_intent::{Intention, NodeStatus, SessionGraph};
use shuma_module::{ModuleContributions, ShortcutSpec};

/// `id` canónico del módulo. El shumarc lo referencia.
pub const ID: &str = "canvas";

/// Estado: el grafo de contexto + un offset de scroll vertical.
pub struct State {
    pub graph: SessionGraph,
    pub scroll_y: f32,
}

impl Clone for State {
    fn clone(&self) -> Self {
        Self {
            graph: self.graph.clone(),
            scroll_y: self.scroll_y,
        }
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            graph: SessionGraph::new(),
            scroll_y: 0.0,
        }
    }

    /// Grafo de demo con 4 comandos y 2 dependencias — útil para
    /// probar el render sin tener un shell conectado. Las inyecciones
    /// (`%pN`) son etapas separadas por `|`, como pide `shuma-intent`.
    pub fn demo() -> Self {
        let mut g = SessionGraph::new();
        let c1 = g.record("cat data.json");
        let _ = g.complete(c1, true, 2_400_000); // produce %p1
        let c2 = g.record("%p1 | jq '.users[]'");
        let _ = g.complete(c2, true, 800_000); // produce %p2
        let c3 = g.record("%p2 | grep -c sergio");
        let _ = g.complete(c3, false, 0);
        let _c4 = g.record("%p2 | sort | head"); // running
        Self {
            graph: g,
            scroll_y: 0.0,
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Registrar una intención nueva en el grafo. Devuelve el `%cN`
    /// asignado al caller via el siguiente render (no hay return-msg).
    Record(String),
    /// Marcar un comando como terminado.
    Complete { id: u32, ok: bool, bytes: u64 },
    /// Colapsar todos los nodos exitosos (quietud visual).
    CollapseSucceeded,
    /// Limpiar el grafo entero.
    Reset,
}

pub fn dispatch(action_id: &str) -> Option<Msg> {
    match action_id {
        "canvas.collapse" => Some(Msg::CollapseSucceeded),
        "canvas.reset" => Some(Msg::Reset),
        _ => None,
    }
}

pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Record(line) => {
            s.graph.record(line);
        }
        Msg::Complete { id, ok, bytes } => {
            s.graph.complete(id, ok, bytes);
        }
        Msg::CollapseSucceeded => {
            s.graph.collapse_succeeded();
        }
        Msg::Reset => {
            s.graph = SessionGraph::new();
        }
    }
    s
}

pub fn contributions(_state: &State) -> ModuleContributions {
    ModuleContributions {
        monitors: Vec::new(),
        shortcuts: vec![
            ShortcutSpec::module_action("Collapse", "canvas.collapse")
                .with_hint("Retraer nodos exitosos"),
            ShortcutSpec::module_action("Reset", "canvas.reset")
                .with_hint("Vaciar el grafo"),
        ],
    }
}

/// Caja precomputada para pintar.
#[derive(Clone)]
struct LaidBox {
    id: u32,
    label: String,
    status: NodeStatus,
    collapsed: bool,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Clone)]
struct LaidEdge {
    from: u32,
    to: u32,
}

/// Layout columnar por profundidad. Equivalente a
/// `shuma_shell_render::layout` pero in-tree para no arrastrar
/// `pineal-render` al chasis.
fn layout(graph: &SessionGraph) -> (Vec<LaidBox>, Vec<LaidEdge>) {
    const NODE_W: f32 = 160.0;
    const NODE_H: f32 = 56.0;
    const COLLAPSED_H: f32 = 22.0;
    const COL_GAP: f32 = 64.0;
    const ROW_GAP: f32 = 20.0;
    const ORIGIN_X: f32 = 16.0;
    const ORIGIN_Y: f32 = 16.0;

    let cmds = graph.commands();
    let mut edges: Vec<LaidEdge> = Vec::new();
    let mut depth: Vec<(u32, usize)> = Vec::with_capacity(cmds.len());
    for c in cmds {
        let refs = Intention::parse(&c.intention).refs();
        let mut d = 0usize;
        for r in refs {
            if let Some(producer) = graph.resolve(r) {
                edges.push(LaidEdge {
                    from: producer.id,
                    to: c.id,
                });
                let pd = depth
                    .iter()
                    .find(|(id, _)| *id == producer.id)
                    .map(|(_, d)| *d)
                    .unwrap_or(0);
                d = d.max(pd + 1);
            }
        }
        depth.push((c.id, d));
    }
    let mut rows_in_col: Vec<usize> = Vec::new();
    let mut boxes: Vec<LaidBox> = Vec::with_capacity(cmds.len());
    for (c, &(_, col)) in cmds.iter().zip(&depth) {
        while rows_in_col.len() <= col {
            rows_in_col.push(0);
        }
        let row = rows_in_col[col];
        rows_in_col[col] += 1;
        let h = if c.collapsed { COLLAPSED_H } else { NODE_H };
        let x = ORIGIN_X + col as f32 * (NODE_W + COL_GAP);
        let y = ORIGIN_Y + row as f32 * (NODE_H + ROW_GAP);
        boxes.push(LaidBox {
            id: c.id,
            label: c.intention.clone(),
            status: c.status,
            collapsed: c.collapsed,
            x,
            y,
            w: NODE_W,
            h,
        });
    }
    (boxes, edges)
}

pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    _lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let (boxes, edges) = layout(&state.graph);
    let theme_clone = *theme;
    let scroll_y = state.scroll_y as f64;
    let header_label = format!(
        "Lienzo de contexto · {} comandos · {} aristas",
        boxes.len(),
        edges.len()
    );

    let painter = move |scene: &mut vello::Scene,
                        ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::{BezPath, PathEl, Point as KPt, Rect as KRect};
        use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
        use llimphi_ui::llimphi_text::{draw_layout, layout_block, Alignment as TAlign, TextBlock};
        // Aristas primero (al fondo) — Bezier suaves entre el borde
        // derecho del producer y el borde izquierdo del consumer.
        let stroke_color = Color::from_rgba8(107, 114, 128, 200);
        for e in &edges {
            let from = boxes.iter().find(|b| b.id == e.from);
            let to = boxes.iter().find(|b| b.id == e.to);
            let (Some(a), Some(b)) = (from, to) else { continue };
            let ax = rect.x as f64 + a.x as f64 + a.w as f64;
            let ay = rect.y as f64 + a.y as f64 + a.h as f64 * 0.5 - scroll_y;
            let bx = rect.x as f64 + b.x as f64;
            let by = rect.y as f64 + b.y as f64 + b.h as f64 * 0.5 - scroll_y;
            let dx = (bx - ax) * 0.5;
            let path = BezPath::from_vec(vec![
                PathEl::MoveTo(KPt::new(ax, ay)),
                PathEl::CurveTo(
                    KPt::new(ax + dx, ay),
                    KPt::new(bx - dx, by),
                    KPt::new(bx, by),
                ),
            ]);
            scene.stroke(
                &llimphi_ui::llimphi_raster::kurbo::Stroke::new(1.5),
                vello::kurbo::Affine::IDENTITY,
                stroke_color,
                None,
                &path,
            );
        }
        // Cajas: fill + stroke por status + label.
        for b in &boxes {
            let x0 = rect.x as f64 + b.x as f64;
            let y0 = rect.y as f64 + b.y as f64 - scroll_y;
            let krect = KRect::new(x0, y0, x0 + b.w as f64, y0 + b.h as f64);
            scene.fill(
                Fill::NonZero,
                vello::kurbo::Affine::IDENTITY,
                theme_clone.bg_panel,
                None,
                &krect,
            );
            let status_color = match b.status {
                NodeStatus::Running => Color::from_rgba8(0xe0, 0xb3, 0x41, 255),
                NodeStatus::Ok => Color::from_rgba8(0x4c, 0xaf, 0x6a, 255),
                NodeStatus::Failed => Color::from_rgba8(0xd0, 0x46, 0x3b, 255),
            };
            scene.stroke(
                &llimphi_ui::llimphi_raster::kurbo::Stroke::new(2.0),
                vello::kurbo::Affine::IDENTITY,
                status_color,
                None,
                &krect,
            );
            // Label: `%cN  <intention>` (trunca si no entra).
            let label = format!("%c{}  {}", b.id, b.label);
            let truncated = truncate_to_fit(&label, b.w as usize / 7);
            let block = TextBlock {
                text: &truncated,
                size_px: 11.0,
                color: theme_clone.fg_text,
                origin: (x0 + 8.0, y0 + 6.0),
                max_width: Some(b.w - 16.0),
                alignment: TAlign::Start,
                line_height: 1.2,
                italic: false,
                font_family: None,
            };
            let layout = layout_block(ts, &block);
            draw_layout(scene, &layout, theme_clone.fg_text, (x0 + 8.0, y0 + 6.0));
            // Status text (más chico, abajo) — solo si no está colapsado.
            if !b.collapsed {
                let status_str = match b.status {
                    NodeStatus::Running => "● running",
                    NodeStatus::Ok => "✔ ok",
                    NodeStatus::Failed => "✘ failed",
                };
                let sblock = TextBlock {
                    text: status_str,
                    size_px: 10.0,
                    color: status_color,
                    origin: (x0 + 8.0, y0 + b.h as f64 - 18.0),
                    max_width: Some(b.w - 16.0),
                    alignment: TAlign::Start,
                    line_height: 1.0,
                    italic: false,
                    font_family: None,
                };
                let slayout = layout_block(ts, &sblock);
                draw_layout(
                    scene,
                    &slayout,
                    status_color,
                    (x0 + 8.0, y0 + b.h as f64 - 18.0),
                );
            }
        }
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(header_label, 12.0, theme.fg_muted, Alignment::Start);

    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(3.0)
    .paint_with(painter);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![header, canvas])
}

/// Trunca `s` a `max_chars` chars (caracteres, no bytes), agregando `…`.
fn truncate_to_fit(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let cut: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "canvas");
    }

    #[test]
    fn demo_state_has_four_nodes() {
        let s = State::demo();
        assert_eq!(s.graph.commands().len(), 4);
    }

    #[test]
    fn record_msg_adds_a_node() {
        let s = State::new();
        let s = update(s, Msg::Record("ls -la".into()));
        assert_eq!(s.graph.commands().len(), 1);
        assert_eq!(s.graph.commands()[0].intention, "ls -la");
    }

    #[test]
    fn complete_msg_assigns_status_and_buffer() {
        let mut s = State::new();
        let id = s.graph.record("echo hola");
        let s = update(
            s,
            Msg::Complete {
                id,
                ok: true,
                bytes: 5,
            },
        );
        let node = &s.graph.commands()[0];
        assert_eq!(node.status, NodeStatus::Ok);
        assert!(node.output_buffer.is_some());
        assert_eq!(node.output_bytes, 5);
    }

    #[test]
    fn collapse_msg_retracts_successful_nodes() {
        let s = State::demo();
        let s = update(s, Msg::CollapseSucceeded);
        let ok_nodes: Vec<_> = s
            .graph
            .commands()
            .iter()
            .filter(|c| c.status == NodeStatus::Ok)
            .collect();
        assert!(!ok_nodes.is_empty());
        assert!(ok_nodes.iter().all(|c| c.collapsed));
    }

    #[test]
    fn reset_msg_empties_graph() {
        let s = State::demo();
        let s = update(s, Msg::Reset);
        assert!(s.graph.is_empty());
    }

    #[test]
    fn layout_places_dependents_in_higher_columns() {
        let s = State::demo();
        let (boxes, _) = layout(&s.graph);
        // c1 (cat data.json) está en col 0; c2 (jq … %p1) en col 1;
        // c3 (grep … %p2) en col 2; c4 (sort %p2 | head) también en col 2.
        let c1 = boxes.iter().find(|b| b.id == 1).unwrap();
        let c2 = boxes.iter().find(|b| b.id == 2).unwrap();
        let c3 = boxes.iter().find(|b| b.id == 3).unwrap();
        assert!(c1.x < c2.x);
        assert!(c2.x < c3.x);
    }

    #[test]
    fn truncate_to_fit_respects_limit() {
        assert_eq!(truncate_to_fit("hola", 10), "hola");
        let t = truncate_to_fit("comando muy largo aquí", 8);
        assert_eq!(t.chars().count(), 8);
        assert!(t.ends_with('…'));
    }
}
