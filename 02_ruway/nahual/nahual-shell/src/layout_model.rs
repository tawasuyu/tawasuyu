//! `LayoutModel` — fuente de verdad mutable del árbol de layout.
//!
//! Distinguimos dos clases de cambio:
//!
//! - [`LayoutModelEvent::StructureChanged`]: cambios que requieren rebuild
//!   del árbol de entidades (kind, children, params relevantes). Estos
//!   también invocan `cx.notify()` para que los `cx.observe` (StatusPanel,
//!   etc.) refresquen.
//! - [`LayoutModelEvent::FlexChanged`]: actualización de `flex` de un
//!   nodo (típicamente proviene de un drag de divisor). NO requiere
//!   rebuild — el SplitContainer ya tiene el flex aplicado en su Vec; solo
//!   nos importa para persistir. Por eso no llamamos `cx.notify()`: solo
//!   emitimos el evento, así los `cx.observe` (que rebuilden) se mantienen
//!   silenciosos durante el drag.
//!
//! El `Persister` (ver `shell/persister.rs`) se subscribe vía
//! `cx.subscribe` y reacciona a los dos.

use gpui::{Context, EventEmitter};

use nahual_core::{LayerConfig, NodeId};

#[derive(Clone, Debug)]
pub enum LayoutModelEvent {
    /// Estructural — kind / children / params. Triggerea rebuild en
    /// `LayoutHost` y persist.
    StructureChanged,
    /// Solo flex de un nodo. Triggerea persist; NO rebuild.
    FlexChanged,
}

pub struct LayoutModel {
    tree: LayerConfig,
}

impl EventEmitter<LayoutModelEvent> for LayoutModel {}

impl LayoutModel {
    pub fn new(tree: LayerConfig) -> Self {
        Self { tree }
    }

    pub fn tree(&self) -> &LayerConfig {
        &self.tree
    }

    /// Reemplazo completo del árbol — para hot-reload del JSON.
    pub fn replace_tree(&mut self, tree: LayerConfig, cx: &mut Context<Self>) {
        self.tree = tree;
        cx.emit(LayoutModelEvent::StructureChanged);
        cx.notify();
    }

    /// Cambia el `kind` del nodo cuyo id JSON coincide con `target_id`.
    pub fn set_kind(
        &mut self,
        target_id: &NodeId,
        new_kind: &str,
        cx: &mut Context<Self>,
    ) {
        let changed = mutate_node(&mut self.tree, target_id, &mut |node| {
            if node.kind != new_kind {
                node.kind = new_kind.to_string();
                true
            } else {
                false
            }
        });
        if changed {
            cx.emit(LayoutModelEvent::StructureChanged);
            cx.notify();
        }
    }

    /// Setea el flex de un nodo. Solo emite `FlexChanged` (no notify) —
    /// usado al final de un drag de divisor para persistir sin
    /// triggerear rebuild.
    pub fn set_flex(&mut self, target_id: &NodeId, flex: f32, cx: &mut Context<Self>) {
        let new_val = Some(flex as f64);
        let changed = mutate_node(&mut self.tree, target_id, &mut |node| {
            if node.flex != new_val {
                node.flex = new_val;
                true
            } else {
                false
            }
        });
        if changed {
            cx.emit(LayoutModelEvent::FlexChanged);
        }
    }

    /// Intercambia dos children del nodo `parent_id`. Triggerea
    /// `StructureChanged` (rebuild + persist), porque cambia el orden de
    /// instanciación. Si los índices son iguales o están out-of-bounds,
    /// es no-op.
    pub fn swap_children(
        &mut self,
        parent_id: &NodeId,
        idx_a: usize,
        idx_b: usize,
        cx: &mut Context<Self>,
    ) {
        if idx_a == idx_b {
            return;
        }
        let mut did_swap = false;
        mutate_node(&mut self.tree, parent_id, &mut |node| {
            if idx_a < node.children.len() && idx_b < node.children.len() {
                node.children.swap(idx_a, idx_b);
                did_swap = true;
                true
            } else {
                false
            }
        });
        if did_swap {
            cx.emit(LayoutModelEvent::StructureChanged);
            cx.notify();
        }
    }
}

fn mutate_node(
    node: &mut LayerConfig,
    target: &NodeId,
    f: &mut impl FnMut(&mut LayerConfig) -> bool,
) -> bool {
    if let Some(id) = &node.id {
        if id == target.as_str() {
            return f(node);
        }
    }
    for child in node.children.iter_mut() {
        if mutate_node(child, target, f) {
            return true;
        }
    }
    false
}
