//! `LayoutHost` — orquestador del layout dinámico.
//!
//! Lee un `LayerConfig` (raíz del JSON) y construye un árbol de entidades
//! GPUI dispatch-eando por `kind`:
//!
//! | kind        | factory                                      |
//! |-------------|----------------------------------------------|
//! | "Split"     | `SplitContainer` con dirección + flex        |
//! | "Tree"      | `ManagedTree` con dataset stub               |
//! | "Status"    | `StatusPanel`                                |
//! | (otro)      | placeholder textual con el kind y params     |
//!
//! Cada entidad se memoiza por `NodeId` (string opcional del JSON o el path
//! sintético `root/child/0`). Mientras el `id` no cambie entre rebuilds, la
//! misma instancia se reusa — esto es lo que permite swappear el `kind` de
//! un container (Split → Tabs → Tiled) preservando los hijos sin reset.
//!
//! En Fase 3 el `LayoutHost` solo reconstruye al inicio. La observación del
//! `LayoutModel` (hot-reload del JSON, mutaciones desde la UI) entra en
//! Fase 7. Pero el diseño ya soporta `rebuild()` idempotente — se invoca
//! cuando el modelo cambia y la memoización mantiene los hijos vivos.

use std::collections::HashMap;

use gpui::{
    AnyView, Context, Entity, IntoElement, Render, SharedString, Window, div, prelude::*,
};

use nahual_bus::{AppBus, AppEvent};
use nahual_core::{LayerConfig, LayoutDirection, NodeId};
use nahual_database_explorer::{DatabaseExplorer, DatabaseExplorerEvent};
use nahual_file_explorer::{FileExplorer, FileExplorerEvent};
use nahual_image_viewer::ImageViewer;
use nahual_text_viewer::TextViewer;
use nahual_widget_container_core::ChildSlot;
use nahual_widget_splitter::{SplitContainer, SplitEvent};
use nahual_widget_tabs::TabContainer;
use nahual_widget_tiled::{TiledContainer, TiledEvent};

use crate::layout_model::{LayoutModel, LayoutModelEvent};
use crate::managed_tree::ManagedTree;
use crate::persister::Persister;
use crate::status_panel::StatusPanel;

// =====================================================================
// LayoutHost
// =====================================================================

pub struct LayoutHost {
    /// Modelo observable. Cualquier mutación (set_kind, replace_tree)
    /// dispara un rebuild idempotente que preserva memoización.
    model: Entity<LayoutModel>,

    /// Bus app-level. Lo distribuimos a viewers (que se subscriben para
    /// reaccionar a EntitySelected/Opened) y forwardeamos los eventos
    /// tipados de explorers hacia él.
    bus: Entity<AppBus>,

    /// Persister adoptado — el LayoutHost mantiene su Entity vivo. Sin
    /// strong handle el entity se dropea y la subscripción al model
    /// queda inactiva.
    #[allow(dead_code)]
    persister: Entity<Persister>,

    /// Memoización de instancias por NodeId. Cada slot guarda la entidad
    /// tipada para poder llamarle métodos específicos (set_children en
    /// containers, etc.). Se preserva entre rebuilds — eso es lo que
    /// permite swappear kind del padre sin perder los hijos.
    nodes: HashMap<NodeId, NodeSlot>,

    /// La AnyView raíz, computada por el último `rebuild`. El render solo
    /// pinta este handle.
    root_view: Option<AnyView>,
}

/// Una entidad concreta instanciada en el árbol. Distinguimos por variante
/// porque cada tipo tiene una API distinta para reaccionar a actualizaciones
/// (set_children para Split, etc.). Las factories devuelven `AnyView`
/// directamente (`AnyView::from(entity.clone())`); esta enum solo guarda
/// la handle tipada para llamadas futuras.
enum NodeSlot {
    Split(Entity<SplitContainer>),
    Tabs(Entity<TabContainer>),
    Tiled(Entity<TiledContainer>),
    Tree(Entity<ManagedTree>),
    FileExplorer(Entity<FileExplorer>),
    DatabaseExplorer(Entity<DatabaseExplorer>),
    TextViewer(Entity<TextViewer>),
    ImageViewer(Entity<ImageViewer>),
    Status(Entity<StatusPanel>),
    Placeholder(Entity<PlaceholderView>),
}

impl LayoutHost {
    pub fn new(
        model: Entity<LayoutModel>,
        bus: Entity<AppBus>,
        persister: Entity<Persister>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Subscripción event-filtered: solo rebuildeamos en cambios
        // estructurales (set_kind, replace_tree). FlexChanged proviene de
        // drags de divisor — el splitter ya tiene el flex aplicado en su
        // Vec, rebuildear lo resetearía y rompería el drag.
        cx.subscribe(&model, |this, _, ev: &LayoutModelEvent, cx| match ev {
            LayoutModelEvent::StructureChanged => this.rebuild(cx),
            LayoutModelEvent::FlexChanged => {}
        })
        .detach();

        let mut me = Self {
            model,
            bus,
            persister,
            nodes: HashMap::new(),
            root_view: None,
        };
        me.rebuild(cx);
        me
    }

    /// Rebuild idempotente: walk del árbol del model, instancia (o reusa)
    /// cada nodo, propaga children a los containers.
    pub fn rebuild(&mut self, cx: &mut Context<Self>) {
        // Snapshot del config para no chocar con el borrow al iterar +
        // mutar self.nodes.
        let cfg = self.model.read(cx).tree().clone();
        let used_ids = std::cell::RefCell::new(Vec::new());
        let view = self.build_node(&cfg, "root", &used_ids, cx);

        // GC: tirar nodos cuyo id ya no aparece en el árbol nuevo.
        let used: std::collections::HashSet<NodeId> =
            used_ids.into_inner().into_iter().collect();
        self.nodes.retain(|id, _| used.contains(id));

        self.root_view = Some(view);
        cx.notify();
    }

    /// DFS recursivo. `path` se acumula para los nodos que no traen `id`
    /// propio en el JSON (`root/0/1` etc) — la sintetización vive en
    /// `NodeId::from_layer`.
    fn build_node(
        &mut self,
        cfg: &LayerConfig,
        path: &str,
        used_ids: &std::cell::RefCell<Vec<NodeId>>,
        cx: &mut Context<Self>,
    ) -> AnyView {
        let id = NodeId::from_layer(cfg, path);
        used_ids.borrow_mut().push(id.clone());

        match cfg.kind.as_str() {
            "Split" => self.build_split(id, cfg, path, used_ids, cx),
            "Tabs" => self.build_tabs(id, cfg, path, used_ids, cx),
            "Tiled" => self.build_tiled(id, cfg, path, used_ids, cx),
            "Tree" => self.build_tree(id, cfg, cx),
            "FileExplorer" => self.build_file_explorer(id, cfg, cx),
            "DatabaseExplorer" => self.build_database_explorer(id, cfg, cx),
            "TextViewer" => self.build_text_viewer(id, cx),
            "ImageViewer" => self.build_image_viewer(id, cx),
            "Status" => self.build_status(id, cx),
            _ => self.build_placeholder(id, cfg, cx),
        }
    }

    /// Helper común — construye los `ChildSlot`s de un contenedor haciendo
    /// recursión sobre los hijos del JSON. Usado por Split / Tabs / Tiled.
    fn build_child_slots(
        &mut self,
        cfg: &LayerConfig,
        path: &str,
        used_ids: &std::cell::RefCell<Vec<NodeId>>,
        cx: &mut Context<Self>,
    ) -> Vec<ChildSlot> {
        let mut slots = Vec::with_capacity(cfg.children.len());
        for (i, child) in cfg.children.iter().enumerate() {
            let child_path = format!("{}/{}", path, i);
            let child_view = self.build_node(child, &child_path, used_ids, cx);
            slots.push(ChildSlot {
                id: NodeId::from_layer(child, &child_path),
                flex: child.flex_weight() as f32,
                label: child.get_param("label").cloned(),
                view: child_view,
            });
        }
        slots
    }

    // ------- factories por kind -------

    fn build_split(
        &mut self,
        id: NodeId,
        cfg: &LayerConfig,
        path: &str,
        used_ids: &std::cell::RefCell<Vec<NodeId>>,
        cx: &mut Context<Self>,
    ) -> AnyView {
        let direction = match cfg.layout_direction() {
            LayoutDirection::Vertical => LayoutDirection::Vertical,
            LayoutDirection::Horizontal => LayoutDirection::Horizontal,
            LayoutDirection::Overlay => LayoutDirection::Vertical, // fallback
        };

        // Get-or-create — si ya existe del rebuild anterior y es Split, lo
        // reusamos. Si era de otro tipo, lo descartamos.
        let entity = match self.nodes.get(&id) {
            Some(NodeSlot::Split(e)) => e.clone(),
            _ => {
                let e = cx.new(|cx| SplitContainer::new(direction, cx));
                // Suscripción a DragEnd para persistir flex al model.
                // Usamos el id del Split como ancla para resolver children
                // por id en el LayerConfig.
                let model = self.model.clone();
                let split_node_id = id.clone();
                cx.subscribe(&e, move |_, split_entity, ev: &SplitEvent, cx| {
                    if !matches!(ev, SplitEvent::DragEnd) {
                        return;
                    }
                    // Snapshot de los flex actuales del splitter.
                    let snapshots: Vec<(NodeId, f32)> = split_entity
                        .read(cx)
                        .children()
                        .iter()
                        .map(|c| (c.id.clone(), c.flex))
                        .collect();
                    let _ = split_node_id; // (queda disponible si en
                                            // futuro queremos targetear
                                            // el padre directamente).
                    model.update(cx, |m, cx| {
                        for (child_id, flex) in snapshots {
                            m.set_flex(&child_id, flex, cx);
                        }
                    });
                })
                .detach();
                self.nodes.insert(id.clone(), NodeSlot::Split(e.clone()));
                e
            }
        };

        // Sincronizamos la dirección por si el JSON cambió.
        entity.update(cx, |s, cx| s.set_direction(direction, cx));

        let slots = self.build_child_slots(cfg, path, used_ids, cx);
        entity.update(cx, |s, cx| s.set_children(slots, cx));

        AnyView::from(entity)
    }

    fn build_tabs(
        &mut self,
        id: NodeId,
        cfg: &LayerConfig,
        path: &str,
        used_ids: &std::cell::RefCell<Vec<NodeId>>,
        cx: &mut Context<Self>,
    ) -> AnyView {
        let entity = match self.nodes.get(&id) {
            Some(NodeSlot::Tabs(e)) => e.clone(),
            _ => {
                let e = cx.new(|cx| TabContainer::new(cx));
                self.nodes.insert(id.clone(), NodeSlot::Tabs(e.clone()));
                e
            }
        };

        let slots = self.build_child_slots(cfg, path, used_ids, cx);
        entity.update(cx, |s, cx| s.set_children(slots, cx));

        AnyView::from(entity)
    }

    fn build_tiled(
        &mut self,
        id: NodeId,
        cfg: &LayerConfig,
        path: &str,
        used_ids: &std::cell::RefCell<Vec<NodeId>>,
        cx: &mut Context<Self>,
    ) -> AnyView {
        let entity = match self.nodes.get(&id) {
            Some(NodeSlot::Tiled(e)) => e.clone(),
            _ => {
                let e = cx.new(|cx| TiledContainer::new(cx));
                // Drag-to-swap: el TiledContainer emite Reordered cuando
                // un drag termina sobre otro tile. Lo commiteamos al
                // model swappeando children del padre — el rebuild
                // posterior aplicará el nuevo orden preservando los
                // entities por NodeId.
                let model = self.model.clone();
                let parent_id = id.clone();
                cx.subscribe(&e, move |_, _, ev: &TiledEvent, cx| match ev {
                    TiledEvent::Reordered {
                        from_index,
                        to_index,
                        ..
                    } => {
                        let from = *from_index;
                        let to = *to_index;
                        let parent = parent_id.clone();
                        model.update(cx, |m, cx| m.swap_children(&parent, from, to, cx));
                    }
                })
                .detach();
                self.nodes.insert(id.clone(), NodeSlot::Tiled(e.clone()));
                e
            }
        };

        let slots = self.build_child_slots(cfg, path, used_ids, cx);
        entity.update(cx, |s, cx| s.set_children(slots, cx));

        AnyView::from(entity)
    }

    fn build_tree(&mut self, id: NodeId, cfg: &LayerConfig, cx: &mut Context<Self>) -> AnyView {
        // Param `dataset` selecciona el stub. Default: "sources".
        let dataset = cfg
            .get_param("dataset")
            .cloned()
            .unwrap_or_else(|| "sources".to_string());

        let entity = match self.nodes.get(&id) {
            Some(NodeSlot::Tree(e)) => e.clone(),
            _ => {
                let list_id = SharedString::from(format!("tree-{}", id));
                let dataset_key = SharedString::from(dataset);
                let e = cx.new(|cx| ManagedTree::new(list_id, dataset_key, cx));
                self.nodes.insert(id.clone(), NodeSlot::Tree(e.clone()));
                e
            }
        };

        AnyView::from(entity)
    }

    fn build_file_explorer(
        &mut self,
        id: NodeId,
        cfg: &LayerConfig,
        cx: &mut Context<Self>,
    ) -> AnyView {
        // Param `root` define el path inicial. Default: "." (cwd).
        let root = cfg
            .get_param("root")
            .cloned()
            .unwrap_or_else(|| ".".to_string());

        let entity = match self.nodes.get(&id) {
            Some(NodeSlot::FileExplorer(e)) => e.clone(),
            _ => {
                let e = cx.new(|cx| FileExplorer::new(root, cx));
                // Forwarder: cuando el explorer emite eventos tipados, los
                // traducimos al format agnóstico del AppBus.
                let bus = self.bus.clone();
                cx.subscribe(&e, move |_, _, ev: &FileExplorerEvent, cx| {
                    let app_ev = match ev {
                        FileExplorerEvent::FileSelected { path } => {
                            Some(AppEvent::EntitySelected {
                                provider: "local_fs".to_string(),
                                provider_path: None,
                                id: path.clone(),
                            })
                        }
                        FileExplorerEvent::FileOpened { path } => {
                            Some(AppEvent::EntityOpened {
                                provider: "local_fs".to_string(),
                                provider_path: None,
                                id: path.clone(),
                            })
                        }
                        FileExplorerEvent::RootChanged { .. } => None,
                    };
                    if let Some(ev) = app_ev {
                        bus.update(cx, |_, cx| cx.emit(ev));
                    }
                })
                .detach();
                self.nodes
                    .insert(id.clone(), NodeSlot::FileExplorer(e.clone()));
                e
            }
        };
        AnyView::from(entity)
    }

    fn build_database_explorer(
        &mut self,
        id: NodeId,
        cfg: &LayerConfig,
        cx: &mut Context<Self>,
    ) -> AnyView {
        // Param `path` define el .sqlite. Default: "nahual.db" en cwd.
        let path = cfg
            .get_param("path")
            .cloned()
            .unwrap_or_else(|| "nahual.db".to_string());

        let entity = match self.nodes.get(&id) {
            Some(NodeSlot::DatabaseExplorer(e)) => e.clone(),
            _ => {
                let e = cx.new(|cx| DatabaseExplorer::new(path.clone(), cx));
                // Forwarder al bus; el `provider_path` lleva el path del
                // .sqlite para que el TextViewer pueda construir su propio
                // SqliteDataProvider de la misma DB.
                let bus = self.bus.clone();
                let db_path = path.clone();
                cx.subscribe(&e, move |_, _, ev: &DatabaseExplorerEvent, cx| {
                    let app_ev = match ev {
                        DatabaseExplorerEvent::EntitySelected { id } => {
                            Some(AppEvent::EntitySelected {
                                provider: "sqlite_db".to_string(),
                                provider_path: Some(db_path.clone()),
                                id: id.clone(),
                            })
                        }
                        DatabaseExplorerEvent::EntityOpened { id } => {
                            Some(AppEvent::EntityOpened {
                                provider: "sqlite_db".to_string(),
                                provider_path: Some(db_path.clone()),
                                id: id.clone(),
                            })
                        }
                    };
                    if let Some(ev) = app_ev {
                        bus.update(cx, |_, cx| cx.emit(ev));
                    }
                })
                .detach();
                self.nodes
                    .insert(id.clone(), NodeSlot::DatabaseExplorer(e.clone()));
                e
            }
        };
        AnyView::from(entity)
    }

    fn build_text_viewer(&mut self, id: NodeId, cx: &mut Context<Self>) -> AnyView {
        let entity = match self.nodes.get(&id) {
            Some(NodeSlot::TextViewer(e)) => e.clone(),
            _ => {
                let bus = self.bus.clone();
                let e = cx.new(|cx| TextViewer::new(bus, cx));
                self.nodes
                    .insert(id.clone(), NodeSlot::TextViewer(e.clone()));
                e
            }
        };
        AnyView::from(entity)
    }

    fn build_image_viewer(&mut self, id: NodeId, cx: &mut Context<Self>) -> AnyView {
        let entity = match self.nodes.get(&id) {
            Some(NodeSlot::ImageViewer(e)) => e.clone(),
            _ => {
                let bus = self.bus.clone();
                let e = cx.new(|cx| ImageViewer::new(bus, cx));
                self.nodes
                    .insert(id.clone(), NodeSlot::ImageViewer(e.clone()));
                e
            }
        };
        AnyView::from(entity)
    }

    fn build_status(&mut self, id: NodeId, cx: &mut Context<Self>) -> AnyView {
        let entity = match self.nodes.get(&id) {
            Some(NodeSlot::Status(e)) => e.clone(),
            _ => {
                let model = self.model.clone();
                let e = cx.new(|cx| StatusPanel::new(model, cx));
                self.nodes.insert(id.clone(), NodeSlot::Status(e.clone()));
                e
            }
        };
        AnyView::from(entity)
    }

    fn build_placeholder(
        &mut self,
        id: NodeId,
        cfg: &LayerConfig,
        cx: &mut Context<Self>,
    ) -> AnyView {
        // Si ya hay placeholder con esta id pero el kind cambió, lo
        // recreamos para reflejar el nuevo `kind` en su mensaje.
        let want_kind = cfg.kind.clone();
        let create_new = match self.nodes.get(&id) {
            Some(NodeSlot::Placeholder(e)) => {
                let same_kind = e.read(cx).kind == want_kind;
                !same_kind
            }
            _ => true,
        };

        if create_new {
            let kind_clone = cfg.kind.clone();
            let e = cx.new(|cx| PlaceholderView::new(kind_clone, cx));
            self.nodes
                .insert(id.clone(), NodeSlot::Placeholder(e.clone()));
            return AnyView::from(e);
        }

        // Reuso.
        if let Some(NodeSlot::Placeholder(e)) = self.nodes.get(&id) {
            return AnyView::from(e.clone());
        }
        // Imposible llegar acá si la lógica de arriba está bien, pero
        // mantenemos el fallback para no panicar en debug builds.
        let kind_clone = cfg.kind.clone();
        let e = cx.new(|cx| PlaceholderView::new(kind_clone, cx));
        self.nodes.insert(id, NodeSlot::Placeholder(e.clone()));
        AnyView::from(e)
    }
}

impl Render for LayoutHost {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // En Fase 3 el árbol se construyó en `new` y queda fijo. Si
        // root_view es None (ej. config vacío + GC barrió todo), pintamos
        // un placeholder neutro.
        match self.root_view.clone() {
            Some(v) => div().size_full().child(v),
            None => div()
                .size_full()
                .child("(layout vacío — revisar layout.json)"),
        }
    }
}

// =====================================================================
// PlaceholderView — kind no reconocido
// =====================================================================

/// View neutra que se instancia para cualquier `kind` que el LayoutHost no
/// sepa construir. Renderea el `kind` y los params para que sea evidente
/// qué falta implementar — útil mientras se desarrollan kinds nuevos
/// (FileExplorer, Tabs, Tiled, etc.).
pub struct PlaceholderView {
    kind: String,
}

impl PlaceholderView {
    pub fn new(kind: String, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<nahual_theme::Theme>(|_, cx| cx.notify())
            .detach();
        Self { kind }
    }
}

impl Render for PlaceholderView {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = nahual_theme::Theme::global(cx).clone();
        div()
            .size_full()
            .bg(theme.bg_panel.clone())
            .p(gpui::px(16.0))
            .flex()
            .flex_col()
            .gap(gpui::px(6.0))
            .child(
                div()
                    .text_color(theme.accent)
                    .text_size(gpui::px(14.0))
                    .child(SharedString::from(format!("⟨ kind: {} ⟩", self.kind))),
            )
            .child(
                div()
                    .text_color(theme.fg_muted)
                    .text_size(gpui::px(11.0))
                    .child("(placeholder — kind no implementado todavía)"),
            )
    }
}
