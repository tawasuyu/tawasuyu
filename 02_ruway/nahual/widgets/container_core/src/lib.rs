//! `nahual_widget_container_core` — tipos compartidos por todos los
//! contenedores (Splitter, Tabs, Tiled, futuros).
//!
//! La pieza más relevante es [`ChildSlot`]: el "paquete" con que la Shell
//! le entrega a un contenedor un hijo ya instanciado. La identidad
//! estable (`id: NodeId`) es lo que permite **swappear el kind del
//! contenedor sin perder los hijos**: cuando el JSON cambia
//! `kind: "Split"` por `kind: "Tabs"`, el LayoutHost descarta el viejo
//! contenedor pero pasa los mismos `ChildSlot` (con los mismos AnyView ya
//! con estado) al contenedor nuevo. Esa preservación es la promesa
//! arquitectónica de la app.
//!
//! `flex` y `label` son metadatos opcionales que cada contenedor
//! interpreta a su gusto:
//! - Splitter: usa `flex` para repartir; ignora `label`.
//! - Tabs: usa `label` para el título de la pestaña; ignora `flex`.
//! - Tiled: usa ambos opcionalmente (peso de tile, label hover).

use gpui::AnyView;
use nahual_core::NodeId;

/// Slot de un hijo entregado a un contenedor. La Shell construye el
/// `Vec<ChildSlot>` haciendo DFS sobre el `LayerConfig` del JSON.
#[derive(Clone)]
pub struct ChildSlot {
    /// Identidad estable (proviene del campo `id` del JSON, o se
    /// sintetiza desde el path estructural).
    pub id: NodeId,
    /// Peso flex relativo entre hermanos. Útil para Splitter / Tiled;
    /// los contenedores que no lo usan lo ignoran.
    pub flex: f32,
    /// Texto opcional para decoración (título de tab, label de tile, etc).
    /// Si `None`, los contenedores que lo necesiten caen al `id` como
    /// fallback razonable.
    pub label: Option<String>,
    /// El widget instanciado, listo para colgar del árbol de render.
    pub view: AnyView,
}
