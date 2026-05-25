//! `shuma-shell-render` — draw-plan agnóstico del Lienzo de Contexto.
//!
//! Toma un [`SessionGraph`] y computa el layout del grafo de intenciones:
//! cada comando `%cN` es una caja, ubicada en una columna según su
//! profundidad de dependencia (longest-path); cada referencia `%pN`/`%cN`
//! que un comando consume es una arista hacia el comando que la produjo.
//!
//! Agnóstico de UI: el front-end GPUI consume el [`CanvasPlan`] y lo
//! dibuja; [`paint`] ofrece un render directo contra `pineal_render`.

#![forbid(unsafe_code)]

use pineal_render::{Canvas, Color, Point, Rect, StrokeStyle};
use shuma_intent::{Intention, NodeStatus, SessionGraph};

/// Una caja de comando ya posicionada en el lienzo.
#[derive(Debug, Clone)]
pub struct NodeBox {
    pub command_id: u32,
    /// Texto de la intención (el caller lo trunca al dibujar si hace falta).
    pub label: String,
    pub status: NodeStatus,
    pub collapsed: bool,
    pub column: usize,
    pub rect: Rect,
}

/// Una arista del lienzo: flujo de datos entre dos comandos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from_command: u32,
    pub to_command: u32,
    /// `%pN` si el flujo va por un buffer; `None` si es una ref a comando.
    pub buffer_id: Option<u32>,
}

/// El layout completo del lienzo.
#[derive(Debug, Clone, Default)]
pub struct CanvasPlan {
    pub nodes: Vec<NodeBox>,
    pub edges: Vec<Edge>,
}

impl CanvasPlan {
    /// Caja de un comando por su id.
    pub fn node(&self, command_id: u32) -> Option<&NodeBox> {
        self.nodes.iter().find(|n| n.command_id == command_id)
    }
}

/// Parámetros geométricos del layout.
#[derive(Debug, Clone, Copy)]
pub struct LayoutParams {
    pub node_w: f32,
    pub node_h: f32,
    pub collapsed_h: f32,
    pub col_gap: f32,
    pub row_gap: f32,
    pub origin: Point,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            node_w: 160.0,
            node_h: 56.0,
            collapsed_h: 22.0,
            col_gap: 64.0,
            row_gap: 20.0,
            origin: Point::new(16.0, 16.0),
        }
    }
}

/// Computa el layout del grafo de intenciones.
pub fn layout(graph: &SessionGraph, p: &LayoutParams) -> CanvasPlan {
    let cmds = graph.commands();
    let mut edges = Vec::new();
    // Profundidad de cada comando por su id (los comandos sólo refieren
    // resultados previos, así que recorrer en orden de id basta).
    let mut depth: Vec<(u32, usize)> = Vec::with_capacity(cmds.len());

    for c in cmds {
        let refs = Intention::parse(&c.intention).refs();
        let mut d = 0usize;
        for r in refs {
            if let Some(producer) = graph.resolve(r) {
                edges.push(Edge {
                    from_command: producer.id,
                    to_command: c.id,
                    buffer_id: producer.output_buffer,
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

    // Posiciona: columna = profundidad, fila = orden de llegada en la columna.
    let mut rows_in_col: Vec<usize> = Vec::new();
    let mut nodes = Vec::with_capacity(cmds.len());
    for (c, &(_, col)) in cmds.iter().zip(&depth) {
        while rows_in_col.len() <= col {
            rows_in_col.push(0);
        }
        let row = rows_in_col[col];
        rows_in_col[col] += 1;

        let h = if c.collapsed { p.collapsed_h } else { p.node_h };
        let x = p.origin.x + col as f32 * (p.node_w + p.col_gap);
        let y = p.origin.y + row as f32 * (p.node_h + p.row_gap);
        nodes.push(NodeBox {
            command_id: c.id,
            label: c.intention.clone(),
            status: c.status,
            collapsed: c.collapsed,
            column: col,
            rect: Rect::new(x, y, p.node_w, h),
        });
    }
    CanvasPlan { nodes, edges }
}

/// Color de borde según el estado del nodo.
fn status_color(s: NodeStatus) -> Color {
    match s {
        NodeStatus::Running => Color::from_hex(0xe0b341), // ámbar
        NodeStatus::Ok => Color::from_hex(0x4caf6a),      // verde
        NodeStatus::Failed => Color::from_hex(0xd0463b),  // rojo
    }
}

/// Dibuja el plan contra un `Canvas`: aristas primero (al fondo), luego
/// las cajas de comando. El texto lo dibuja el caller si quiere control
/// de truncado/fuente.
pub fn paint(plan: &CanvasPlan, canvas: &mut dyn Canvas) {
    let edge_stroke = StrokeStyle::new(1.5, Color::from_hex(0x6b7280));
    for e in &plan.edges {
        let (Some(a), Some(b)) = (plan.node(e.from_command), plan.node(e.to_command))
        else {
            continue;
        };
        let from = Point::new(a.rect.right(), a.rect.y + a.rect.h / 2.0);
        let to = Point::new(b.rect.x, b.rect.y + b.rect.h / 2.0);
        canvas.stroke_line(from, to, edge_stroke);
    }
    for n in &plan.nodes {
        canvas.fill_rect(n.rect, Color::from_hex(0x1c2128));
        canvas.stroke_rect(n.rect, StrokeStyle::new(2.0, status_color(n.status)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sesión: c1 produce %p1; c2 lo consume.
    fn chained_session() -> SessionGraph {
        let mut g = SessionGraph::new();
        let c1 = g.record("cat data.json");
        g.complete(c1, true, 2400); // → %p1
        let _c2 = g.record("sort | %p1");
        g
    }

    #[test]
    fn layout_places_dependent_in_a_later_column() {
        let g = chained_session();
        let plan = layout(&g, &LayoutParams::default());
        assert_eq!(plan.nodes.len(), 2);
        let c1 = plan.node(1).unwrap();
        let c2 = plan.node(2).unwrap();
        assert_eq!(c1.column, 0);
        assert_eq!(c2.column, 1, "el que consume %p1 va una columna después");
        assert!(c2.rect.x > c1.rect.x);
    }

    #[test]
    fn layout_creates_an_edge_for_the_buffer_flow() {
        let g = chained_session();
        let plan = layout(&g, &LayoutParams::default());
        assert_eq!(plan.edges.len(), 1);
        assert_eq!(plan.edges[0].from_command, 1);
        assert_eq!(plan.edges[0].to_command, 2);
        assert_eq!(plan.edges[0].buffer_id, Some(1));
    }

    #[test]
    fn independent_commands_share_column_zero() {
        let mut g = SessionGraph::new();
        g.record("ls");
        g.record("pwd");
        let plan = layout(&g, &LayoutParams::default());
        assert!(plan.nodes.iter().all(|n| n.column == 0));
        assert!(plan.edges.is_empty());
        // Apiladas en filas distintas.
        assert_ne!(plan.nodes[0].rect.y, plan.nodes[1].rect.y);
    }

    #[test]
    fn paint_emits_commands_to_a_recorder() {
        use pineal_render::{PlanRecorder, RenderCmd};
        let g = chained_session();
        let plan = layout(&g, &LayoutParams::default());
        let mut rec = PlanRecorder::new();
        paint(&plan, &mut rec);
        let cmds = rec.into_plan().cmds;
        // 1 línea (la arista) + 2 fill + 2 stroke (las cajas).
        assert!(cmds.iter().any(|c| matches!(c, RenderCmd::StrokeLine { .. })));
        assert_eq!(
            cmds.iter().filter(|c| matches!(c, RenderCmd::FillRect { .. })).count(),
            2
        );
    }
}
