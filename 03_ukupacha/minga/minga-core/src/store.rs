//! Almacén de nodos direccionados por contenido.
//!
//! Cada `SemanticNode` se descompone en `StoredNode`s donde los hijos son
//! referencias por hash, no estructuras inline. Así dos subárboles con la
//! misma estructura se almacenan una sola vez, sin importar en cuántos
//! lugares aparezcan en el repositorio. Esa es la diferencia entre "Git
//! semántico" y "diff de líneas".
//!
//! `NodeStore` es el contrato; `MemStore` es la implementación de
//! referencia, en memoria, agnóstica de IO. Un futuro `SledStore` o
//! `RocksStore` vivirá en otro crate y se enchufará vía este trait sin
//! tocar el resto del núcleo.

use crate::ast::SemanticNode;
use crate::cas::{self, ContentHash};
use std::collections::HashMap;

/// Forma "stored": idéntica a `SemanticNode` excepto que los hijos son
/// hashes en vez de estructuras anidadas. Es el format canónico en
/// reposo y el que permite la deduplicación.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredNode {
    pub kind: String,
    pub field_name: Option<String>,
    pub leaf_text: Option<Vec<u8>>,
    pub children: Vec<ContentHash>,
}

/// Hash de un `StoredNode`, idéntico al `hash_node` del `SemanticNode`
/// equivalente. Permite a un protocolo de wire verificar que el nodo
/// que le entregaron tiene efectivamente el hash que se le anunció,
/// sin necesidad de reconstruir descendientes.
pub fn hash_stored(stored: &StoredNode) -> ContentHash {
    cas::hash_components(
        &stored.kind,
        stored.field_name.as_deref(),
        stored.leaf_text.as_deref(),
        &stored.children,
    )
}

pub trait NodeStore {
    /// Inserta un árbol completo. Recursivamente desempaqueta los hijos
    /// y devuelve el hash de la raíz. Idempotente: insertar el mismo
    /// árbol dos veces no aumenta el tamaño.
    fn put(&mut self, node: &SemanticNode) -> ContentHash;

    /// Inserta un nodo ya troceado por su hash. No recurre en hijos: el
    /// llamador es responsable de garantizar que estarán presentes (lo
    /// hace típicamente un protocolo de sync que va recibiendo nodos en
    /// orden y solicita los faltantes a medida que descubre referencias).
    fn put_chunked(&mut self, hash: ContentHash, stored: StoredNode);

    fn get(&self, h: &ContentHash) -> Option<&StoredNode>;

    fn contains(&self, h: &ContentHash) -> bool {
        self.get(h).is_some()
    }

    /// Reconstruye el `SemanticNode` original a partir de su hash,
    /// resolviendo recursivamente los hijos. `None` si algún hash no se
    /// encuentra (almacén incompleto, inconsistente).
    fn reconstruct(&self, h: &ContentHash) -> Option<SemanticNode>;

    /// Itera todas las parejas `(hash, stored_node)` del store. Sin
    /// orden garantizado. Usado para mergear stores tras una sesión
    /// de sync (un peer recibe los nodos del otro en su sesión, y
    /// luego los volcamos al store compartido).
    fn iter(&self) -> Box<dyn Iterator<Item = (&ContentHash, &StoredNode)> + '_>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Default, Clone)]
pub struct MemStore {
    map: HashMap<ContentHash, StoredNode>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl NodeStore for MemStore {
    fn put(&mut self, node: &SemanticNode) -> ContentHash {
        // Recorrido bottom-up: primero los hijos (devuelven su hash),
        // luego compongo el hash del padre desde sus child_hashes
        // mediante la primitiva canónica de cas. Cada subárbol se
        // hashea exactamente una vez — sin recomputar `hash_node` sobre
        // el árbol entero del padre.
        let mut child_hashes = Vec::with_capacity(node.children.len());
        for c in &node.children {
            child_hashes.push(self.put(c));
        }
        let h = cas::hash_components(
            &node.kind,
            node.field_name.as_deref(),
            node.leaf_text.as_deref(),
            &child_hashes,
        );
        self.map.entry(h).or_insert_with(|| StoredNode {
            kind: node.kind.clone(),
            field_name: node.field_name.clone(),
            leaf_text: node.leaf_text.clone(),
            children: child_hashes,
        });
        h
    }

    fn put_chunked(&mut self, hash: ContentHash, stored: StoredNode) {
        self.map.entry(hash).or_insert(stored);
    }

    fn get(&self, h: &ContentHash) -> Option<&StoredNode> {
        self.map.get(h)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = (&ContentHash, &StoredNode)> + '_> {
        Box::new(self.map.iter())
    }

    fn reconstruct(&self, h: &ContentHash) -> Option<SemanticNode> {
        let s = self.map.get(h)?;
        let mut children = Vec::with_capacity(s.children.len());
        for ch in &s.children {
            children.push(self.reconstruct(ch)?);
        }
        Some(SemanticNode {
            kind: s.kind.clone(),
            field_name: s.field_name.clone(),
            leaf_text: s.leaf_text.clone(),
            children,
        })
    }

    fn len(&self) -> usize {
        self.map.len()
    }
}
