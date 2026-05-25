//! llimphi-layout — Física del Espacio.
//!
//! Wrapper sobre `taffy` que resuelve árboles flex/grid y devuelve
//! coordenadas absolutas (no relativas al padre). El consumidor pasa el
//! árbol a `compute(root, viewport)` y obtiene un [`ComputedLayout`] con
//! un rect absoluto por nodo, listo para `llimphi-raster`.

use std::collections::HashMap;

pub use taffy;
pub use taffy::prelude::*;

/// Errores del motor de layout.
#[derive(Debug)]
pub enum LayoutError {
    Taffy(String),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Taffy(s) => write!(f, "taffy: {s}"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Caja absoluta de un nodo (origen en la esquina superior izquierda del viewport).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Resultado de [`LayoutTree::compute`]: rect absoluto por nodo del árbol.
#[derive(Debug, Default)]
pub struct ComputedLayout {
    pub rects: HashMap<NodeId, Rect>,
}

impl ComputedLayout {
    pub fn get(&self, node: NodeId) -> Option<Rect> {
        self.rects.get(&node).copied()
    }
}

/// Árbol de layout. Encapsula la `TaffyTree` y la lógica de absolutización.
pub struct LayoutTree {
    inner: TaffyTree<()>,
}

impl Default for LayoutTree {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutTree {
    pub fn new() -> Self {
        Self {
            inner: TaffyTree::new(),
        }
    }

    /// Crea una hoja (nodo sin hijos).
    pub fn leaf(&mut self, style: Style) -> Result<NodeId, LayoutError> {
        self.inner
            .new_leaf(style)
            .map_err(|e| LayoutError::Taffy(e.to_string()))
    }

    /// Crea un nodo contenedor con hijos.
    pub fn node(&mut self, style: Style, children: &[NodeId]) -> Result<NodeId, LayoutError> {
        self.inner
            .new_with_children(style, children)
            .map_err(|e| LayoutError::Taffy(e.to_string()))
    }

    /// Calcula el layout para `root` con viewport `(w, h)` y devuelve rects absolutos.
    pub fn compute(
        &mut self,
        root: NodeId,
        viewport: (f32, f32),
    ) -> Result<ComputedLayout, LayoutError> {
        self.inner
            .compute_layout(
                root,
                taffy::Size {
                    width: AvailableSpace::Definite(viewport.0),
                    height: AvailableSpace::Definite(viewport.1),
                },
            )
            .map_err(|e| LayoutError::Taffy(e.to_string()))?;
        let mut out = ComputedLayout::default();
        flatten(&self.inner, root, 0.0, 0.0, &mut out.rects)?;
        Ok(out)
    }

    pub fn inner(&self) -> &TaffyTree<()> {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut TaffyTree<()> {
        &mut self.inner
    }
}

fn flatten(
    tree: &TaffyTree<()>,
    node: NodeId,
    ox: f32,
    oy: f32,
    out: &mut HashMap<NodeId, Rect>,
) -> Result<(), LayoutError> {
    let layout = tree
        .layout(node)
        .map_err(|e| LayoutError::Taffy(e.to_string()))?;
    let x = ox + layout.location.x;
    let y = oy + layout.location.y;
    out.insert(
        node,
        Rect {
            x,
            y,
            w: layout.size.width,
            h: layout.size.height,
        },
    );
    let children = tree
        .children(node)
        .map_err(|e| LayoutError::Taffy(e.to_string()))?;
    for child in children {
        flatten(tree, child, x, y, out)?;
    }
    Ok(())
}
