//! `ManagedTree` — wrapper de `TreeView` que aporta su propio modelo de
//! datos + estado de expansión. En Fase 3 sirve como stub data-driven (los
//! datasets se eligen vía param `dataset` del JSON). En Fase 4 este patrón
//! se concretiza en `FileExplorer` (TreeView + FsProvider) y
//! `DatabaseExplorer` (TreeView + SqliteProvider).
//!
//! La entidad es completamente Render-able y se entrega al LayoutHost como
//! un AnyView. Los TreeView events (RowClicked, ChevronToggled, …) se
//! traducen acá en cambios al estado de expansión y luego se re-emiten
//! como `ManagedTreeEvent` para que el host (LayoutHost o un App-bus
//! futuro) los consuma.

use std::collections::HashSet;

use gpui::{
    Context, Entity, EventEmitter, IntoElement, Render, SharedString, Window, div, prelude::*,
};

use nahual_widget_tree::{RowId, RowKind, TreeEvent, TreeRow, TreeView};

// =====================================================================
// Datasets stub (Fase 3). En Fase 4 los reemplazan los providers reales.
// =====================================================================

#[derive(Clone)]
pub struct DemoNode {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub children: Vec<DemoNode>,
}

impl DemoNode {
    fn leaf(id: &'static str, label: &'static str, icon: &'static str) -> Self {
        Self { id, label, icon, children: vec![] }
    }
    fn branch(id: &'static str, label: &'static str, children: Vec<Self>) -> Self {
        Self { id, label, icon: "📁", children }
    }
}

/// Resuelve el dataset por `key` (proveniente del param `dataset` del JSON).
/// Cualquier key desconocida cae al stub vacío para no romper el render.
pub fn dataset_for(key: &str) -> DemoNode {
    match key {
        "sources" => yahweh_sources_tree(),
        "deps" => yahweh_deps_tree(),
        _ => DemoNode {
            id: "unknown",
            label: "(dataset desconocido)",
            icon: "❓",
            children: vec![],
        },
    }
}

fn yahweh_sources_tree() -> DemoNode {
    DemoNode::branch(
        "src-root",
        "nahual (src)",
        vec![
            DemoNode::branch(
                "shell",
                "shell",
                vec![DemoNode::leaf("shell/main.rs", "main.rs", "📄")],
            ),
            DemoNode::branch(
                "widgets",
                "widgets",
                vec![
                    DemoNode::branch(
                        "widgets/tree",
                        "tree",
                        vec![DemoNode::leaf("widgets/tree/lib.rs", "lib.rs", "📄")],
                    ),
                    DemoNode::branch(
                        "widgets/splitter",
                        "splitter",
                        vec![DemoNode::leaf("widgets/splitter/lib.rs", "lib.rs", "📄")],
                    ),
                ],
            ),
            DemoNode::branch(
                "libs",
                "libs",
                vec![
                    DemoNode::branch(
                        "libs/core",
                        "core",
                        vec![DemoNode::leaf("libs/core/lib.rs", "lib.rs", "📄")],
                    ),
                    DemoNode::branch(
                        "libs/theme",
                        "theme",
                        vec![DemoNode::leaf("libs/theme/lib.rs", "lib.rs", "📄")],
                    ),
                    DemoNode::branch(
                        "libs/providers",
                        "providers",
                        vec![
                            DemoNode::leaf("libs/providers/fs.rs", "fs.rs", "📄"),
                            DemoNode::leaf("libs/providers/sqlite.rs", "sqlite.rs", "📄"),
                        ],
                    ),
                ],
            ),
        ],
    )
}

fn yahweh_deps_tree() -> DemoNode {
    fn branch(id: &'static str, label: &'static str, children: Vec<DemoNode>) -> DemoNode {
        DemoNode { id, label, icon: "📦", children }
    }
    let leaf = DemoNode::leaf;
    branch(
        "deps-root",
        "deps",
        vec![
            branch(
                "ui",
                "ui",
                vec![
                    leaf("dep:gpui", "gpui 0.2.2", "🧊"),
                    leaf("dep:gpui-macros", "gpui-macros 0.2.2", "🧊"),
                ],
            ),
            branch(
                "async",
                "async",
                vec![
                    leaf("dep:tokio", "tokio 1.x", "🌀"),
                    leaf("dep:async-trait", "async-trait 0.1", "🌀"),
                ],
            ),
            branch(
                "data",
                "data",
                vec![
                    leaf("dep:serde", "serde 1", "🧬"),
                    leaf("dep:serde_json", "serde_json 1", "🧬"),
                    leaf("dep:rusqlite", "rusqlite 0.31", "🗃️"),
                ],
            ),
            leaf("dep:notify", "notify 6.1", "👂"),
            leaf("dep:uuid", "uuid 1", "🔗"),
        ],
    )
}

// =====================================================================
// Eventos re-emitidos
// =====================================================================

/// Re-emitidos por ManagedTree después de procesar el evento bruto del
/// TreeView interno. Contienen el `dataset_key` para que el host distinga
/// entre múltiples ManagedTrees.
///
/// Los campos están marcados `dead_code`-ok porque en Fase 3 nadie se
/// suscribe — el LayoutHost los va a consumir en Fase 4 vía un AppBus que
/// reenvíe estos eventos a los viewers (TextViewer / ImageViewer / etc).
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum ManagedTreeEvent {
    RowClicked { dataset: SharedString, id: String },
    RowDoubleClicked { dataset: SharedString, id: String },
    ContextMenu { dataset: SharedString, id: Option<String> },
}

// =====================================================================
// Widget
// =====================================================================

pub struct ManagedTree {
    view: Entity<TreeView>,
    data: DemoNode,
    expanded: HashSet<String>,
    dataset_key: SharedString,
}

impl EventEmitter<ManagedTreeEvent> for ManagedTree {}

impl ManagedTree {
    pub fn new(
        list_id: SharedString,
        dataset_key: SharedString,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = cx.new(|cx| TreeView::new(list_id, cx));
        cx.subscribe(&view, |this: &mut ManagedTree, _, ev, cx| {
            this.on_tree_event(ev, cx);
        })
        .detach();

        let data = dataset_for(&dataset_key);
        let mut expanded = HashSet::new();
        expanded.insert(data.id.to_string());

        let me = Self {
            view,
            data,
            expanded,
            dataset_key,
        };
        me.push_rows(cx);
        me
    }

    fn push_rows(&self, cx: &mut Context<Self>) {
        let mut rows = Vec::new();
        flatten(&self.data, 0, &self.expanded, &mut rows);
        self.view.update(cx, |tree, cx| tree.set_rows(rows, cx));
    }

    fn on_tree_event(&mut self, event: &TreeEvent, cx: &mut Context<Self>) {
        match event {
            TreeEvent::ChevronToggled(id) => {
                let key = id.as_str().to_string();
                if !self.expanded.remove(&key) {
                    self.expanded.insert(key);
                }
                self.push_rows(cx);
            }
            TreeEvent::RowClicked(id) => {
                cx.emit(ManagedTreeEvent::RowClicked {
                    dataset: self.dataset_key.clone(),
                    id: id.to_string(),
                });
            }
            TreeEvent::RowDoubleClicked(id) => {
                cx.emit(ManagedTreeEvent::RowDoubleClicked {
                    dataset: self.dataset_key.clone(),
                    id: id.to_string(),
                });
            }
            TreeEvent::ContextMenuRequested { id, .. } => {
                cx.emit(ManagedTreeEvent::ContextMenu {
                    dataset: self.dataset_key.clone(),
                    id: id.as_ref().map(|i| i.to_string()),
                });
            }
            TreeEvent::ActiveChanged(_) => {}
        }
    }

    /// Reservado para Fase 4: el LayoutHost lo va a consultar al
    /// re-emitir eventos al bus.
    #[allow(dead_code)]
    pub fn dataset_key(&self) -> &str {
        &self.dataset_key
    }
}

impl Render for ManagedTree {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.view.clone())
    }
}

// -------- helpers --------

fn flatten(
    node: &DemoNode,
    depth: u32,
    expanded: &HashSet<String>,
    out: &mut Vec<TreeRow>,
) {
    let kind = if node.children.is_empty() {
        RowKind::Leaf
    } else {
        RowKind::Branch
    };
    let is_expanded = expanded.contains(node.id);
    out.push(TreeRow {
        id: RowId::new(node.id),
        label: node.label.to_string(),
        depth,
        kind,
        expanded: is_expanded,
        icon: Some(node.icon.to_string()),
    });
    if is_expanded {
        for child in &node.children {
            flatten(child, depth + 1, expanded, out);
        }
    }
}
