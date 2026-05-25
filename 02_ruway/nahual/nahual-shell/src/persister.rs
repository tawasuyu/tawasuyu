//! `Persister` — escribe el `LayoutModel` a disco en cada cambio.
//!
//! Es una entity sin estado visible (no se renderea). Solo existe para
//! mantener viva la subscripción al `LayoutModel`. Cualquier evento
//! (`StructureChanged` o `FlexChanged`) dispara una escritura sincrónica
//! al `path` configurado.
//!
//! Hoy NO hay debounce — cada drag de divisor emite UN solo `FlexChanged`
//! al final (en DragEnd, no por frame), y los swaps de kind son acción
//! manual del usuario. Si en el futuro las escrituras se vuelven
//! frecuentes, el lugar para sumar debounce es acá: spawn un task que
//! coalesce events dentro de N ms.

use std::path::PathBuf;

use gpui::{Context, Entity};

use crate::layout_model::{LayoutModel, LayoutModelEvent};

pub struct Persister {
    path: PathBuf,
}

impl Persister {
    pub fn new(path: PathBuf, model: Entity<LayoutModel>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&model, |this: &mut Persister, model, _ev: &LayoutModelEvent, cx| {
            this.write(model.read(cx).tree());
        })
        .detach();
        Self { path }
    }

    fn write(&self, tree: &nahual_core::LayerConfig) {
        let json = tree.serialize_json();
        // Anti-loop: si el contenido en disco ya coincide, skip. Esto
        // matters cuando el watcher está corriendo: persister write →
        // notify modify → replace_tree → persister write → ... sin esto
        // sería un loop infinito de syscalls.
        if let Ok(existing) = std::fs::read_to_string(&self.path) {
            if existing == json {
                return;
            }
        }
        if let Err(e) = std::fs::write(&self.path, json) {
            eprintln!(
                "[Persister] error escribiendo {}: {}",
                self.path.display(),
                e
            );
        }
    }
}
