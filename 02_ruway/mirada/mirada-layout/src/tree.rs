//! Árbol fractal de teselado: un nodo es una **app** o un **sub-espacio**.
//!
//! El [`Workspace`](crate::Workspace) plano reparte una pantalla entre ventanas.
//! Este módulo añade el primitivo recursivo: un [`SpaceNode`] reparte su espacio
//! entre [`LayoutNode`]s, y un nodo puede ser una ventana (hoja) **o** otro
//! sub-espacio entero. Para el espacio padre, ese sub-espacio ocupa un solo
//! hueco del teselado; dentro, reparte su propio espacio con sus propias reglas.
//!
//! Es el cimiento del zoom semántico (entrar/salir del árbol de procesos) y de
//! las sub-pantallas con sus propias zonas. La resolución
//! ([`SpaceNode::resolve`]) **aplana** el árbol a píxeles absolutos: el
//! `mirada-protocol` sigue recibiendo una lista plana de ventanas+rect, así que
//! el Cuerpo no se entera de la recursión.
//!
//! Modela sólo el **anidamiento del teselado** — no foco ni flotantes, que son
//! asuntos del [`Workspace`](crate::Workspace) plano. Un `SpaceNode` de un solo
//! nivel (todas hojas) resuelve idéntico a [`Workspace::layout`] sin flotantes
//! (ver tests).

use alloc::boxed::Box;
use alloc::vec::Vec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::geometry::Rect;
use crate::layout::{tile, LayoutParams};
use crate::workspace::WindowId;

/// Un nodo del árbol fractal: una **app** (hoja) o un **sub-espacio** anidado.
///
/// Las variantes nuevas se añaden **al final** para no mover los índices con
/// que `postcard`/serde las serializan.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LayoutNode {
    /// Una ventana concreta.
    Leaf(WindowId),
    /// Un sub-escritorio entero — la recursión.
    Space(Box<SpaceNode>),
}

impl LayoutNode {
    /// Cuántas ventanas (hojas) cuelgan de este nodo, recursivamente.
    pub fn leaf_count(&self) -> usize {
        match self {
            LayoutNode::Leaf(_) => 1,
            LayoutNode::Space(space) => space.leaf_count(),
        }
    }
}

/// Un espacio de teselado: parámetros propios + sus nodos hijos. El análogo
/// recursivo de [`Workspace`](crate::Workspace), pero modelando sólo el
/// anidamiento (sin foco ni flotantes).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SpaceNode {
    /// Cómo reparte este espacio su superficie entre sus hijos.
    pub params: LayoutParams,
    /// Los nodos hijos, en orden de teselado (y de pintado).
    pub children: Vec<LayoutNode>,
}

impl SpaceNode {
    /// Un espacio vacío con los parámetros dados.
    pub fn new(params: LayoutParams) -> Self {
        Self { params, children: Vec::new() }
    }

    /// Cuántas ventanas (hojas) cuelgan de este espacio, recursivamente.
    pub fn leaf_count(&self) -> usize {
        self.children.iter().map(LayoutNode::leaf_count).sum()
    }

    /// Aplana el árbol a píxeles absolutos: el rect de cada ventana dentro de
    /// `screen`. Reparte `screen` entre los hijos con [`tile`] según `params`;
    /// una hoja toma su hueco tal cual, un sub-espacio recurre dentro del suyo.
    /// El orden de salida es el recorrido en profundidad de los hijos — el mismo
    /// orden de pintado (atrás→adelante) que usa el resto del motor.
    pub fn resolve(&self, screen: Rect) -> Vec<(WindowId, Rect)> {
        let slots = tile(screen, self.children.len(), &self.params);
        let mut out = Vec::with_capacity(self.children.len());
        for (child, slot) in self.children.iter().zip(slots) {
            match child {
                LayoutNode::Leaf(id) => out.push((*id, slot)),
                LayoutNode::Space(space) => out.extend(space.resolve(slot)),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::LayoutMode;
    use crate::workspace::Workspace;

    const SCREEN: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };

    fn flat(params: LayoutParams, ids: &[WindowId]) -> SpaceNode {
        SpaceNode {
            params,
            children: ids.iter().map(|&id| LayoutNode::Leaf(id)).collect(),
        }
    }

    #[test]
    fn a_flat_tree_matches_the_plain_workspace_layout() {
        // El criterio de la Fase 2: aplanar un árbol de un nivel coincide con
        // el layout plano actual, para cada modo de teselado.
        for mode in LayoutMode::ALL {
            let params = LayoutParams { mode, ..LayoutParams::default() };
            let ids = [10, 20, 30, 40];
            let mut ws = Workspace::new(params);
            for &id in &ids {
                ws.add(id);
            }
            assert_eq!(
                flat(params, &ids).resolve(SCREEN),
                ws.layout(SCREEN),
                "modo {mode:?}"
            );
        }
    }

    #[test]
    fn an_empty_space_resolves_to_nothing() {
        assert!(SpaceNode::new(LayoutParams::default()).resolve(SCREEN).is_empty());
    }

    #[test]
    fn a_nested_space_subdivides_within_its_slot() {
        // Padre en dos columnas: izquierda una app, derecha un sub-espacio con
        // dos apps apiladas (rows).
        let params = LayoutParams { mode: LayoutMode::Columns, gap: 0, ..LayoutParams::default() };
        let inner = SpaceNode {
            params: LayoutParams { mode: LayoutMode::Rows, gap: 0, ..LayoutParams::default() },
            children: alloc::vec![LayoutNode::Leaf(2), LayoutNode::Leaf(3)],
        };
        let root = SpaceNode {
            params,
            children: alloc::vec![LayoutNode::Leaf(1), LayoutNode::Space(Box::new(inner))],
        };
        let placed = root.resolve(SCREEN);
        // Tres ventanas en total.
        assert_eq!(placed.len(), 3);
        assert_eq!(root.leaf_count(), 3);
        // La 1 ocupa la columna izquierda entera.
        let r1 = placed.iter().find(|(id, _)| *id == 1).unwrap().1;
        assert_eq!(r1, Rect::new(0, 0, 960, 1080));
        // La 2 y la 3 parten la columna derecha por alto.
        let r2 = placed.iter().find(|(id, _)| *id == 2).unwrap().1;
        let r3 = placed.iter().find(|(id, _)| *id == 3).unwrap().1;
        assert_eq!(r2, Rect::new(960, 0, 960, 540));
        assert_eq!(r3, Rect::new(960, 540, 960, 540));
        // Cubren la pantalla sin solaparse.
        let total: i64 = placed.iter().map(|(_, r)| r.area()).sum();
        assert_eq!(total, SCREEN.area());
    }

    #[test]
    fn resolution_is_deterministic() {
        let s = flat(LayoutParams::default(), &[1, 2, 3, 4, 5]);
        assert_eq!(s.resolve(SCREEN), s.resolve(SCREEN));
    }

    #[test]
    fn leaf_count_walks_the_whole_tree() {
        let deep = SpaceNode {
            params: LayoutParams::default(),
            children: alloc::vec![
                LayoutNode::Leaf(1),
                LayoutNode::Space(Box::new(SpaceNode {
                    params: LayoutParams::default(),
                    children: alloc::vec![
                        LayoutNode::Leaf(2),
                        LayoutNode::Space(Box::new(flat(LayoutParams::default(), &[3, 4]))),
                    ],
                })),
            ],
        };
        assert_eq!(deep.leaf_count(), 4);
        assert_eq!(deep.resolve(SCREEN).len(), 4);
    }
}
