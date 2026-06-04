//! `llimphi-widget-navigator` — navegador **data-agnóstico** de nodos en
//! dos modos conmutables: **árbol** (`tree`) y **grafo** (`nodegraph`).
//!
//! Nació para que `pata` muestre las **Mónadas** de nouser y sus archivos
//! en un sidebar, pero el widget no sabe de nouser: recibe un bosque de
//! [`NavNode`]s (id opaco + label + [`NavKind`] + hijos) y emite `Msg`s al
//! interactuar. El caller mapea cada `id` a lo suyo (un `MonadId`, un path)
//! y decide qué hacer al seleccionar/abrir.
//!
//! Igual que el resto de widgets Llimphi, es **render-only y stateless**:
//! el estado (qué nodos están expandidos, cuál está seleccionado, en qué
//! modo está) vive en el `Model` del App; el widget sólo pinta y avisa.
//!
//! - **Árbol** — reusa [`llimphi_widget_tree`]. El navegador aplana el
//!   bosque respetando `is_expanded`, dibuja un icono por [`NavKind`] entre
//!   el chevron y el label, y cablea toggle / select / context por fila.
//! - **Grafo** — reusa [`llimphi_widget_nodegraph`]. Coloca los nodos
//!   visibles en columnas por profundidad, con cables de **contención**
//!   (padre→hijo). El nodo seleccionado se resalta; arrastrar un nodo lo
//!   selecciona; el right-click abre el menú contextual.
//!
//! ```ignore
//! navigator_view(
//!     NavSpec { roots: &model.nodes, mode: model.mode,
//!               selected: model.selected, palette, guides: true },
//!     |id| model.expanded.contains(&id),
//!     Msg::Toggle, Msg::Select, Some(Msg::Open),
//! )
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{DragPhase, View};
use llimphi_theme::Theme;
use llimphi_widget_nodegraph::{
    nodegraph_view_styled, NodeId, NodeSpec, NodeTint, NodegraphMetrics, NodegraphPalette, Wire,
};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

/// Identificador opaco de un nodo. El caller lo asigna y lo recibe de vuelta
/// sin que el widget lo interprete (típicamente un índice a su propio mapa
/// `id → MonadId | PathBuf`).
pub type NavId = u64;

/// Naturaleza de un nodo — sólo para elegir su icono y tinte. El widget no
/// asume semántica de dominio más allá de esto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKind {
    /// Una Mónada (cluster semántico de nouser). Diamante de acento.
    Monad,
    /// Una agrupación intermedia (carpeta lógica, categoría). Cuadrado.
    Group,
    /// Un directorio del filesystem. Cuadrado tenue.
    Dir,
    /// Un archivo hoja. Punto.
    File,
    /// Cualquier otra cosa. Punto tenue.
    Other,
}

/// Un nodo del bosque que el navegador pinta. La jerarquía es explícita
/// (`children`); el navegador la aplana según el estado de expansión.
#[derive(Debug, Clone)]
pub struct NavNode {
    pub id: NavId,
    pub label: String,
    pub kind: NavKind,
    pub children: Vec<NavNode>,
}

impl NavNode {
    /// Un nodo hoja (sin hijos).
    pub fn leaf(id: NavId, label: impl Into<String>, kind: NavKind) -> Self {
        Self {
            id,
            label: label.into(),
            kind,
            children: Vec::new(),
        }
    }

    /// Un nodo con hijos.
    pub fn branch(
        id: NavId,
        label: impl Into<String>,
        kind: NavKind,
        children: Vec<NavNode>,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            kind,
            children,
        }
    }

    /// `true` si tiene al menos un hijo.
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
}

/// Modo de visualización del navegador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavMode {
    /// Árbol indentado con expand/collapse.
    Tree,
    /// Grafo de nodos con cables de contención.
    Graph,
}

impl NavMode {
    /// Etiquetas para un control segmentado (en el mismo orden que
    /// [`NavMode::index`] / [`NavMode::from_index`]).
    pub const LABELS: [&'static str; 2] = ["Árbol", "Grafo"];

    /// El otro modo (para un botón de toggle simple).
    pub fn toggled(self) -> Self {
        match self {
            NavMode::Tree => NavMode::Graph,
            NavMode::Graph => NavMode::Tree,
        }
    }

    /// Índice 0/1 — para alimentar un control segmentado.
    pub fn index(self) -> usize {
        match self {
            NavMode::Tree => 0,
            NavMode::Graph => 1,
        }
    }

    /// Recupera el modo desde un índice de control segmentado (≥1 = grafo).
    pub fn from_index(i: usize) -> Self {
        if i == 0 {
            NavMode::Tree
        } else {
            NavMode::Graph
        }
    }
}

/// Paleta del navegador: hereda las de tree y nodegraph del [`Theme`]
/// semántico, más los tintes por [`NavKind`] para los iconos.
#[derive(Debug, Clone, Copy)]
pub struct NavPalette {
    pub tree: TreePalette,
    pub graph: NodegraphPalette,
    pub accent: Color,
    pub monad: Color,
    pub group: Color,
    pub dir: Color,
    pub file: Color,
    pub other: Color,
}

impl Default for NavPalette {
    fn default() -> Self {
        Self::from_theme(&Theme::dark())
    }
}

impl NavPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            tree: TreePalette::from_theme(t),
            graph: NodegraphPalette::from_theme(t),
            accent: t.accent,
            monad: t.accent,
            group: t.fg_text,
            dir: t.fg_muted,
            file: t.fg_text,
            other: t.fg_muted,
        }
    }

    /// El color del icono de un nodo según su clase.
    pub fn kind_color(&self, kind: NavKind) -> Color {
        match kind {
            NavKind::Monad => self.monad,
            NavKind::Group => self.group,
            NavKind::Dir => self.dir,
            NavKind::File => self.file,
            NavKind::Other => self.other,
        }
    }
}

/// Lo que el navegador necesita saber para pintar, sin los callbacks.
pub struct NavSpec<'a> {
    /// Las raíces del bosque a mostrar.
    pub roots: &'a [NavNode],
    /// Modo activo.
    pub mode: NavMode,
    /// Nodo seleccionado (resaltado en ambos modos). `None` = ninguno.
    pub selected: Option<NavId>,
    /// Paleta.
    pub palette: NavPalette,
    /// Dibujar líneas guía de indentación en el árbol.
    pub guides: bool,
}

/// Alto de fila del árbol / paso vertical del grafo.
const ROW_H: f32 = 24.0;
/// Tamaño del icono de clase (px).
const ICON_PX: f32 = 14.0;

/// Compone el navegador. Los callbacks se identifican por [`NavId`]:
/// - `is_expanded(id)` → si un nodo rama está abierto (sólo árbol);
/// - `on_toggle(id)` → al click en el chevron (sólo árbol);
/// - `on_select(id)` → al click en la fila (árbol) o al arrastrar el nodo
///   (grafo);
/// - `on_context(id)` → al right-click (ambos modos); `None` = sin menú.
pub fn navigator_view<Msg, FExp, FTog, FSel, FCtx>(
    spec: NavSpec,
    is_expanded: FExp,
    on_toggle: FTog,
    on_select: FSel,
    on_context: Option<FCtx>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FExp: Fn(NavId) -> bool,
    FTog: Fn(NavId) -> Msg,
    FSel: Fn(NavId) -> Msg + Send + Sync + 'static,
    FCtx: Fn(NavId) -> Msg,
{
    match spec.mode {
        NavMode::Tree => tree_mode(spec, is_expanded, on_toggle, on_select, on_context),
        NavMode::Graph => graph_mode(spec, is_expanded, on_select, on_context),
    }
}

// =====================================================================
// Árbol
// =====================================================================

fn tree_mode<Msg, FExp, FTog, FSel, FCtx>(
    spec: NavSpec,
    is_expanded: FExp,
    on_toggle: FTog,
    on_select: FSel,
    on_context: Option<FCtx>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FExp: Fn(NavId) -> bool,
    FTog: Fn(NavId) -> Msg,
    FSel: Fn(NavId) -> Msg,
    FCtx: Fn(NavId) -> Msg,
{
    let mut rows: Vec<TreeRow<Msg>> = Vec::new();
    for root in spec.roots {
        push_rows(
            root,
            0,
            &spec,
            &is_expanded,
            &on_toggle,
            &on_select,
            &on_context,
            &mut rows,
        );
    }
    tree_view(TreeSpec {
        rows,
        row_height: ROW_H,
        indent_px: 14.0,
        palette: spec.palette.tree,
        guides: spec.guides,
    })
}

#[allow(clippy::too_many_arguments)]
fn push_rows<Msg, FExp, FTog, FSel, FCtx>(
    node: &NavNode,
    depth: usize,
    spec: &NavSpec,
    is_expanded: &FExp,
    on_toggle: &FTog,
    on_select: &FSel,
    on_context: &Option<FCtx>,
    out: &mut Vec<TreeRow<Msg>>,
) where
    Msg: Clone + Send + Sync + 'static,
    FExp: Fn(NavId) -> bool,
    FTog: Fn(NavId) -> Msg,
    FSel: Fn(NavId) -> Msg,
    FCtx: Fn(NavId) -> Msg,
{
    let has_children = node.has_children();
    let expanded = has_children && is_expanded(node.id);
    let icon = kind_icon_view::<Msg>(node.kind, spec.palette.kind_color(node.kind));
    let mut row = TreeRow::new(
        node.label.clone(),
        depth,
        has_children,
        expanded,
        spec.selected == Some(node.id),
        on_toggle(node.id),
        on_select(node.id),
    )
    .with_icon(icon);
    if let Some(ctx) = on_context.as_ref().map(|f| f(node.id)) {
        row = row.with_context(ctx);
    }
    out.push(row);

    if expanded {
        for child in &node.children {
            push_rows(
                child, depth + 1, spec, is_expanded, on_toggle, on_select, on_context, out,
            );
        }
    }
}

// =====================================================================
// Grafo
// =====================================================================

/// Un nodo visible aplanado para el grafo: su id, su label/kind y la posición
/// (índice) de su padre en la lista (`None` = raíz).
struct FlatNode {
    id: NavId,
    label: String,
    kind: NavKind,
    depth: usize,
    parent: Option<usize>,
    has_children: bool,
}

fn flatten_for_graph<FExp: Fn(NavId) -> bool>(
    roots: &[NavNode],
    is_expanded: &FExp,
) -> Vec<FlatNode> {
    let mut out = Vec::new();
    for root in roots {
        walk_graph(root, 0, None, is_expanded, &mut out);
    }
    out
}

fn walk_graph<FExp: Fn(NavId) -> bool>(
    node: &NavNode,
    depth: usize,
    parent: Option<usize>,
    is_expanded: &FExp,
    out: &mut Vec<FlatNode>,
) {
    let has_children = node.has_children();
    let me = out.len();
    out.push(FlatNode {
        id: node.id,
        label: node.label.clone(),
        kind: node.kind,
        depth,
        parent,
        has_children,
    });
    if has_children && is_expanded(node.id) {
        for child in &node.children {
            walk_graph(child, depth + 1, Some(me), is_expanded, out);
        }
    }
}

fn graph_mode<Msg, FExp, FSel, FCtx>(
    spec: NavSpec,
    is_expanded: FExp,
    on_select: FSel,
    on_context: Option<FCtx>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FExp: Fn(NavId) -> bool,
    FSel: Fn(NavId) -> Msg + Send + Sync + 'static,
    FCtx: Fn(NavId) -> Msg,
{
    let flat = flatten_for_graph(spec.roots, &is_expanded);
    let metrics = NodegraphMetrics {
        node_width: 150.0,
        ..NodegraphMetrics::default()
    };

    // Layout: columna por profundidad, una fila por nodo visible.
    const MARGIN: f32 = 24.0;
    const COL_GAP: f32 = 36.0;
    const ROW_GAP: f32 = 12.0;
    let node_h = metrics.node_height(1, 1);
    let col_w = metrics.node_width + COL_GAP;

    let mut nodes: Vec<NodeSpec> = Vec::with_capacity(flat.len());
    let mut wires: Vec<Wire> = Vec::new();
    let ids: Vec<NavId> = flat.iter().map(|f| f.id).collect();

    for (i, f) in flat.iter().enumerate() {
        let inputs = if f.parent.is_some() {
            vec![String::new()]
        } else {
            Vec::new()
        };
        let outputs = if f.has_children {
            vec![String::new()]
        } else {
            Vec::new()
        };
        // Prefijo del icono en el label (el nodegraph no tiene slot de icono;
        // un glifo simple por clase basta para distinguirlos de un vistazo).
        let label = format!("{} {}", kind_glyph(f.kind), f.label);
        nodes.push(NodeSpec {
            id: i as NodeId,
            label,
            x: MARGIN + f.depth as f32 * col_w,
            y: MARGIN + i as f32 * (node_h + ROW_GAP),
            inputs,
            outputs,
        });
        if let Some(p) = f.parent {
            wires.push(Wire {
                from_node: p as NodeId,
                from_output: 0,
                to_node: i as NodeId,
                to_input: 0,
            });
        }
    }

    // Arrastrar un nodo lo selecciona (al soltar). El grafo no reposiciona
    // por arrastre — el layout es derivado, no editable.
    let drag_ids = ids.clone();
    let on_drag = move |id: NodeId, phase: DragPhase, _dx: f32, _dy: f32| match phase {
        DragPhase::End => drag_ids
            .get(id as usize)
            .map(|nav_id| on_select(*nav_id)),
        DragPhase::Move => None,
    };
    // Sin conexiones: la contención es fija.
    let on_connect = |_: NodeId, _: u16, _: NodeId, _: u16| None;

    // Right-click → menú contextual (evaluado en build, por nodo).
    let ctx_ids = &ids;
    let on_right: Option<Box<dyn Fn(NodeId) -> Option<Msg>>> = on_context.map(|f| {
        let f = move |id: NodeId| ctx_ids.get(id as usize).map(|nav_id| f(*nav_id));
        Box::new(f) as Box<dyn Fn(NodeId) -> Option<Msg>>
    });

    // Resaltado del nodo seleccionado.
    let sel_idx = spec
        .selected
        .and_then(|sid| ids.iter().position(|id| *id == sid));
    let accent = spec.palette.accent;
    let tint = move |id: NodeId| -> Option<NodeTint> {
        if Some(id as usize) == sel_idx {
            Some(NodeTint {
                bg_title: Some(accent),
                ..NodeTint::default()
            })
        } else {
            None
        }
    };

    nodegraph_view_styled(
        &nodes,
        &wires,
        &spec.palette.graph,
        &metrics,
        on_drag,
        on_connect,
        on_right,
        Some(&tint as &dyn Fn(NodeId) -> Option<NodeTint>),
        None,
    )
}

/// Glifo ASCII-ish por clase para el label del grafo.
fn kind_glyph(kind: NavKind) -> &'static str {
    match kind {
        NavKind::Monad => "◈",
        NavKind::Group => "▣",
        NavKind::Dir => "▸",
        NavKind::File => "·",
        NavKind::Other => "·",
    }
}

// =====================================================================
// Icono vectorial por clase (para el árbol)
// =====================================================================

/// Un mini-canvas con el icono de la clase, tinte `color`. Diamante para
/// Mónada, cuadrado para grupo/dir, círculo para archivo.
fn kind_icon_view<Msg: Clone + 'static>(kind: NavKind, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::length(ICON_PX),
            height: llimphi_ui::llimphi_layout::taffy::prelude::length(ICON_PX),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Point, RoundedRect};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let r = (rect.w.min(rect.h) as f64 * 0.34).max(1.5);
        match kind {
            NavKind::Monad => {
                // Diamante (cuadrado a 45°).
                let mut p = BezPath::new();
                p.move_to(Point::new(cx, cy - r));
                p.line_to(Point::new(cx + r, cy));
                p.line_to(Point::new(cx, cy + r));
                p.line_to(Point::new(cx - r, cy));
                p.close_path();
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &p);
            }
            NavKind::Group | NavKind::Dir => {
                let sq = RoundedRect::new(cx - r, cy - r, cx + r, cy + r, 2.0);
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &sq);
            }
            NavKind::File | NavKind::Other => {
                let dot = (rect.w.min(rect.h) as f64 * 0.22).max(1.0);
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Circle::new((cx, cy), dot),
                );
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    enum Msg {
        Toggle(NavId),
        Select(NavId),
        Open(NavId),
    }

    fn forest() -> Vec<NavNode> {
        vec![NavNode::branch(
            1,
            "Mónada src",
            NavKind::Monad,
            vec![
                NavNode::leaf(11, "lib.rs", NavKind::File),
                NavNode::leaf(12, "main.rs", NavKind::File),
            ],
        )]
    }

    #[test]
    fn navmode_toggle_e_indices() {
        assert_eq!(NavMode::Tree.toggled(), NavMode::Graph);
        assert_eq!(NavMode::Graph.toggled(), NavMode::Tree);
        assert_eq!(NavMode::Tree.index(), 0);
        assert_eq!(NavMode::from_index(1), NavMode::Graph);
        assert_eq!(NavMode::from_index(0), NavMode::Tree);
    }

    #[test]
    fn navnode_constructores() {
        let n = NavNode::leaf(1, "x", NavKind::File);
        assert!(!n.has_children());
        let b = NavNode::branch(2, "y", NavKind::Monad, vec![n]);
        assert!(b.has_children());
        assert_eq!(b.children.len(), 1);
    }

    #[test]
    fn flatten_grafo_respeta_expansion() {
        let roots = forest();
        // Colapsado: sólo la raíz.
        let collapsed = flatten_for_graph(&roots, &|_| false);
        assert_eq!(collapsed.len(), 1);
        assert_eq!(collapsed[0].id, 1);
        assert!(collapsed[0].parent.is_none());
        assert!(collapsed[0].has_children);
        // Expandido: raíz + 2 hijos, con parent = índice 0.
        let expanded = flatten_for_graph(&roots, &|id| id == 1);
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[1].parent, Some(0));
        assert_eq!(expanded[2].parent, Some(0));
        assert_eq!(expanded[1].depth, 1);
    }

    #[test]
    fn navigator_view_construye_en_ambos_modos() {
        // No paniquea construyendo el View en cada modo (smoke).
        let roots = forest();
        let palette = NavPalette::default();
        for mode in [NavMode::Tree, NavMode::Graph] {
            let _v: View<Msg> = navigator_view(
                NavSpec {
                    roots: &roots,
                    mode,
                    selected: Some(1),
                    palette,
                    guides: true,
                },
                |id| id == 1,
                Msg::Toggle,
                Msg::Select,
                Some(Msg::Open),
            );
        }
    }
}
