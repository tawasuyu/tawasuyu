//! `pluma-notebook-graph-llimphi` — vista de nodos sobre un
//! [`pluma_notebook_core::Notebook`].
//!
//! Conecta el DAG ya existente del notebook con
//! [`llimphi_widget_nodegraph`] para que las celdas y sus dependencias
//! sean editables visualmente:
//!
//! - cada [`Cell`] es un nodo cuyo título resume el `CellKind` + el
//!   primer fragmento del `source` (sin entrar en el contenido);
//! - cada celda tiene un único pin de entrada (`in`) y un único pin de
//!   salida cuyo nombre es el `port_kind` del último output (`text`,
//!   `scalar`, `table`, `image`, `geometry`, `none` o `out` si no se
//!   ejecutó nunca);
//! - cada arista `cell.depends_on` se materializa como un [`Wire`]
//!   `(dep, 0) → (cell, 0)`.
//!
//! Layout: si la celda tiene `Position::Some` la respeta tal cual; si
//! no, la coloca por **rank topológico** (columna = profundidad en el
//! DAG, fila = orden dentro de la columna). El layout automático es
//! perezoso — basta una sola vez por frame del caller.
//!
//! Edición: dos funciones helper aplican los `Msg`s del widget al
//! notebook directamente, sin que el caller toque la API de
//! `Notebook`:
//!
//! - [`apply_drag`] suma el delta a la `Position` de la celda
//!   (creándola si no existía).
//! - [`apply_connect`] llama a `notebook.add_dependency` y propaga
//!   staleness — los rechazos por ciclo se devuelven como `false`.
//!
//! El crate **no** mantiene estado propio: el caller pasa `&Notebook`
//! a [`notebook_graph_view`] y muta el notebook desde su `update` con
//! [`apply_drag`] / [`apply_connect`].

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};

use llimphi_ui::{DragPhase, View};
use llimphi_widget_nodegraph::{
    nodegraph_view, NodeId, NodeSpec, NodegraphMetrics, NodegraphPalette, PinIdx, Wire,
};
use pluma_notebook_core::{Cell, CellId, CellKind, Notebook, Position};

/// Parámetros visuales del layout automático.
#[derive(Debug, Clone, Copy)]
pub struct AutoLayout {
    /// Espacio horizontal entre columnas (centro a centro).
    pub col_step: f32,
    /// Espacio vertical entre filas (centro a centro).
    pub row_step: f32,
    /// Origen del lienzo (esquina superior izquierda del primer nodo).
    pub origin_x: f32,
    pub origin_y: f32,
}

impl Default for AutoLayout {
    fn default() -> Self {
        Self {
            col_step: 230.0,
            row_step: 110.0,
            origin_x: 40.0,
            origin_y: 40.0,
        }
    }
}

/// `(CellId, Position)` resuelto por celda en el frame actual. El
/// caller no necesita inspeccionarlo, pero queda público porque
/// permite que un test verifique el layout sin pintar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedPosition {
    pub cell: CellId,
    pub x: f32,
    pub y: f32,
}

/// Construye el `View<Msg>` que pinta el notebook como lienzo de
/// nodos.
///
/// - `on_drag_cell(cell_id, phase, dx, dy) -> Option<Msg>` se llama
///   cada vez que el usuario arrastra la title bar de un nodo. El
///   caller materializa el `Msg` y desde el `update` invoca
///   [`apply_drag`] sobre el notebook.
/// - `on_connect(from_cell, to_cell) -> Option<Msg>` se llama al
///   soltar un cable de un pin de salida en un pin de entrada. El
///   caller invoca [`apply_connect`] desde el `update`.
pub fn notebook_graph_view<Msg, FDrag, FConnect>(
    notebook: &Notebook,
    layout: AutoLayout,
    palette: &NodegraphPalette,
    metrics: &NodegraphMetrics,
    on_drag_cell: FDrag,
    on_connect: FConnect,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FDrag: Fn(CellId, DragPhase, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    FConnect: Fn(CellId, CellId) -> Option<Msg> + Send + Sync + 'static,
{
    let (nodes, wires, _id_map) = build_nodegraph(notebook, layout);
    nodegraph_view(
        &nodes,
        &wires,
        palette,
        metrics,
        move |node_id, phase, dx, dy| {
            on_drag_cell(node_id as CellId, phase, dx, dy)
        },
        move |from_node, _from_pin: PinIdx, to_node, _to_pin: PinIdx| {
            on_connect(from_node as CellId, to_node as CellId)
        },
    )
}

/// Aplica el delta de un drag a la `Position` de una celda. Si la
/// celda no tenía posición, se la crea en `(layout.origin_x,
/// layout.origin_y)` antes de sumar el delta (de modo que el primer
/// drag manual la fija visualmente en lugar de "saltar" hacia el
/// auto-layout). Devuelve `false` si la celda no existe.
///
/// El `phase` se ignora — el delta llega ya integrado por evento.
pub fn apply_drag(
    notebook: &mut Notebook,
    layout: AutoLayout,
    cell: CellId,
    _phase: DragPhase,
    dx: f32,
    dy: f32,
) -> bool {
    if notebook.cell(cell).is_none() {
        return false;
    }
    let cur = notebook.position(cell).unwrap_or(Position::new(
        layout.origin_x,
        layout.origin_y,
    ));
    let new_x = (cur.x + dx).max(0.0);
    let new_y = (cur.y + dy).max(0.0);
    notebook.set_position(cell, Some(Position::new(new_x, new_y)))
}

/// Añade `from → to` al notebook y propaga staleness desde `to`. La
/// propia `to` queda `Stale` (su último output fue producido sin la
/// nueva dependencia), y desde ahí se propaga el cono.
///
/// Devuelve `false` si la celda no existe o si la arista cerraría un
/// ciclo (lo decide `Notebook::add_dependency`).
pub fn apply_connect(notebook: &mut Notebook, from: CellId, to: CellId) -> bool {
    if from == to {
        return false;
    }
    if !notebook.add_dependency(to, from) {
        return false;
    }
    notebook.set_state(to, pluma_notebook_core::CellState::Stale);
    notebook.propagate_stale(to);
    true
}

/// Convierte el notebook en `(nodos, cables, mapa de posiciones)`.
/// Visible para tests y para callers que quieran integrar
/// `nodegraph_view` con un patrón distinto al de
/// [`notebook_graph_view`].
pub fn build_nodegraph(
    notebook: &Notebook,
    layout: AutoLayout,
) -> (Vec<NodeSpec>, Vec<Wire>, Vec<ResolvedPosition>) {
    let order = notebook.execution_order().unwrap_or_default();
    let positions = resolve_positions(notebook, &order, layout);

    let mut nodes: Vec<NodeSpec> = Vec::with_capacity(notebook.cells().len());
    let pos_lookup: HashMap<CellId, (f32, f32)> = positions
        .iter()
        .map(|p| (p.cell, (p.x, p.y)))
        .collect();

    for cell in notebook.cells() {
        let (x, y) = pos_lookup
            .get(&cell.id)
            .copied()
            .unwrap_or((layout.origin_x, layout.origin_y));
        nodes.push(NodeSpec {
            id: cell.id as NodeId,
            label: node_label(cell),
            x,
            y,
            inputs: vec!["in".into()],
            outputs: vec![output_port_label(cell).to_string()],
        });
    }

    let mut wires: Vec<Wire> = Vec::new();
    for cell in notebook.cells() {
        for &dep in &cell.depends_on {
            wires.push(Wire {
                from_node: dep as NodeId,
                from_output: 0,
                to_node: cell.id as NodeId,
                to_input: 0,
            });
        }
    }

    (nodes, wires, positions)
}

/// Resuelve `(x, y)` por celda combinando la `Position` declarada en
/// el notebook con el rank topológico para las celdas sin posición.
fn resolve_positions(
    notebook: &Notebook,
    exec_order: &[CellId],
    layout: AutoLayout,
) -> Vec<ResolvedPosition> {
    // Profundidad por celda: cell con 0 deps → rank 0; cell con deps →
    // 1 + max(rank de deps). Se calcula sobre el orden topológico
    // (garantiza que las deps ya tienen rank cuando llegamos a la
    // celda).
    let mut rank: BTreeMap<CellId, u32> = BTreeMap::new();
    for &id in exec_order {
        let cell = notebook.cell(id).expect("del orden");
        let r = cell
            .depends_on
            .iter()
            .filter_map(|d| rank.get(d).copied())
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        rank.insert(id, r);
    }
    // Si exec_order quedó vacío (notebook con ciclo), caemos a "todas
    // las celdas en rank 0" para no perder nodos.
    if rank.is_empty() {
        for c in notebook.cells() {
            rank.insert(c.id, 0);
        }
    }

    // Agrupar por columna: cuántas celdas hay en cada rank y qué
    // fila les toca (orden estable por id para que el layout sea
    // determinista entre frames).
    let mut by_rank: BTreeMap<u32, Vec<CellId>> = BTreeMap::new();
    let mut ids_sorted: Vec<CellId> = notebook.cells().iter().map(|c| c.id).collect();
    ids_sorted.sort_unstable();
    for id in &ids_sorted {
        let r = *rank.get(id).unwrap_or(&0);
        by_rank.entry(r).or_default().push(*id);
    }

    let mut out: Vec<ResolvedPosition> = Vec::with_capacity(notebook.cells().len());
    for cell in notebook.cells() {
        // Manual position gana sobre el auto-layout.
        if let Some(p) = notebook.position(cell.id) {
            out.push(ResolvedPosition {
                cell: cell.id,
                x: p.x,
                y: p.y,
            });
            continue;
        }
        let r = *rank.get(&cell.id).unwrap_or(&0);
        let col = by_rank.get(&r).expect("rank declarado");
        let row_idx = col
            .iter()
            .position(|&id| id == cell.id)
            .expect("celda en su rank") as f32;
        let x = layout.origin_x + (r as f32) * layout.col_step;
        let y = layout.origin_y + row_idx * layout.row_step;
        out.push(ResolvedPosition {
            cell: cell.id,
            x,
            y,
        });
    }
    out
}

fn node_label(cell: &Cell) -> String {
    let snippet = first_line_summary(&cell.source, 28);
    let prefix = match &cell.kind {
        CellKind::Markdown => "md".to_string(),
        CellKind::Code { language } => format!("code:{language}"),
        CellKind::Embed { module } => format!("embed:{module}"),
    };
    if snippet.is_empty() {
        prefix
    } else {
        format!("{prefix} · {snippet}")
    }
}

fn first_line_summary(source: &str, max_len: usize) -> String {
    let line = source
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() <= max_len {
        line.to_string()
    } else {
        let truncated: String = line.chars().take(max_len.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

fn output_port_label(cell: &Cell) -> &'static str {
    match &cell.last_output {
        Some(o) => o.payload.port_kind(),
        None => "out",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pluma_notebook_core::{CellKind, CellOutput, OutputPayload};

    fn chain() -> (Notebook, CellId, CellId, CellId) {
        let mut nb = Notebook::new();
        let a = nb.push(CellKind::Code { language: "rust".into() }, "let x = 1;");
        let b = nb.push(
            CellKind::Code { language: "rust".into() },
            "let y = x + 1;",
        );
        let c = nb.push(
            CellKind::Code { language: "rust".into() },
            "println!(\"{y}\");",
        );
        assert!(nb.add_dependency(b, a));
        assert!(nb.add_dependency(c, b));
        (nb, a, b, c)
    }

    #[test]
    fn build_produces_one_node_per_cell_and_one_wire_per_edge() {
        let (nb, _a, _b, _c) = chain();
        let layout = AutoLayout::default();
        let (nodes, wires, _pos) = build_nodegraph(&nb, layout);
        assert_eq!(nodes.len(), 3);
        assert_eq!(wires.len(), 2);
    }

    #[test]
    fn auto_layout_ranks_by_dag_depth() {
        let (nb, a, b, c) = chain();
        let layout = AutoLayout::default();
        let (_nodes, _wires, positions) = build_nodegraph(&nb, layout);
        let lookup: HashMap<CellId, (f32, f32)> =
            positions.iter().map(|p| (p.cell, (p.x, p.y))).collect();
        // a en columna 0, b en columna 1, c en columna 2.
        let xa = lookup.get(&a).unwrap().0;
        let xb = lookup.get(&b).unwrap().0;
        let xc = lookup.get(&c).unwrap().0;
        assert!(xa < xb && xb < xc, "columnas crecen con el rank");
    }

    #[test]
    fn manual_position_overrides_auto_layout() {
        let (mut nb, a, _b, _c) = chain();
        nb.set_position(a, Some(Position::new(999.0, 123.0)));
        let layout = AutoLayout::default();
        let (_nodes, _wires, positions) = build_nodegraph(&nb, layout);
        let pa = positions.iter().find(|p| p.cell == a).unwrap();
        assert!((pa.x - 999.0).abs() < 1e-3);
        assert!((pa.y - 123.0).abs() < 1e-3);
    }

    #[test]
    fn apply_drag_creates_position_if_missing() {
        let (mut nb, a, _b, _c) = chain();
        assert_eq!(nb.position(a), None);
        let layout = AutoLayout::default();
        assert!(apply_drag(&mut nb, layout, a, DragPhase::Move, 50.0, 30.0));
        let p = nb.position(a).unwrap();
        assert!((p.x - (layout.origin_x + 50.0)).abs() < 1e-3);
        assert!((p.y - (layout.origin_y + 30.0)).abs() < 1e-3);
    }

    #[test]
    fn apply_drag_accumulates_deltas() {
        let (mut nb, a, _b, _c) = chain();
        let layout = AutoLayout::default();
        apply_drag(&mut nb, layout, a, DragPhase::Move, 10.0, 0.0);
        apply_drag(&mut nb, layout, a, DragPhase::Move, 5.0, 5.0);
        let p = nb.position(a).unwrap();
        assert!((p.x - (layout.origin_x + 15.0)).abs() < 1e-3);
        assert!((p.y - (layout.origin_y + 5.0)).abs() < 1e-3);
    }

    #[test]
    fn apply_drag_clamps_negative_coordinates() {
        let (mut nb, a, _b, _c) = chain();
        let layout = AutoLayout::default();
        // Movés muy a la izquierda — el clamp deja la coord en 0, no en
        // negativo (los nodos nunca salen del lienzo por encima/izq).
        apply_drag(&mut nb, layout, a, DragPhase::Move, -10_000.0, -10_000.0);
        let p = nb.position(a).unwrap();
        assert!(p.x >= 0.0);
        assert!(p.y >= 0.0);
    }

    #[test]
    fn apply_connect_adds_dependency_and_propagates_stale() {
        // a, b sueltas + c hijo de a; conecto b → c, todos deben
        // quedar Stale por debajo de b.
        let mut nb = Notebook::new();
        let a = nb.push(CellKind::Code { language: "rust".into() }, "let x = 1;");
        let b = nb.push(CellKind::Code { language: "rust".into() }, "let y = 2;");
        let c = nb.push(
            CellKind::Code { language: "rust".into() },
            "let z = x + y;",
        );
        nb.add_dependency(c, a);
        // Marco todo Fresh — apply_connect debe re-staltear el cono.
        for id in [a, b, c] {
            nb.set_state(id, pluma_notebook_core::CellState::Fresh);
        }

        assert!(apply_connect(&mut nb, b, c));
        // c quedó Stale (es el destino, su dep cambió).
        assert_eq!(
            nb.cell(c).unwrap().state,
            pluma_notebook_core::CellState::Stale
        );
        // El cable se materializó.
        assert!(nb.cell(c).unwrap().depends_on.contains(&b));
    }

    #[test]
    fn apply_connect_refuses_cycles() {
        let (mut nb, a, _b, c) = chain();
        // a depender de c cerraría a→b→c→a.
        assert!(!apply_connect(&mut nb, c, a));
        // El notebook quedó sano (sin la arista nueva).
        assert!(!nb.cell(a).unwrap().depends_on.contains(&c));
    }

    #[test]
    fn apply_connect_refuses_self_loops() {
        let (mut nb, a, _b, _c) = chain();
        assert!(!apply_connect(&mut nb, a, a));
    }

    #[test]
    fn output_port_reflects_last_output() {
        let mut nb = Notebook::new();
        let a = nb.push(CellKind::Code { language: "rust".into() }, "1 + 1");
        // Sin ejecutar → port "out".
        let (nodes_pre, _, _) = build_nodegraph(&nb, AutoLayout::default());
        assert_eq!(nodes_pre[0].outputs[0], "out");
        // Con scalar → port "scalar".
        nb.set_last_output(
            a,
            Some(CellOutput {
                stdout: String::new(),
                value: Some("2".into()),
                payload: OutputPayload::Scalar(2.0),
            }),
        );
        let (nodes_post, _, _) = build_nodegraph(&nb, AutoLayout::default());
        assert_eq!(nodes_post[0].outputs[0], "scalar");
    }

    #[test]
    fn label_includes_kind_prefix_and_snippet() {
        let (nb, a, _b, _c) = chain();
        let (nodes, _, _) = build_nodegraph(&nb, AutoLayout::default());
        let node_a = nodes.iter().find(|n| n.id == a as NodeId).unwrap();
        // Debe arrancar con "code:rust · " y luego un fragmento del source.
        assert!(node_a.label.starts_with("code:rust"));
        assert!(node_a.label.contains("let x"));
    }
}
