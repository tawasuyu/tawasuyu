use crate::ast::SemanticNode;
use blake3::Hasher;

/// Hash de 32 bytes que identifica unívocamente un `SemanticNode` por su
/// estructura lógica. Dos nodos con misma estructura → mismo hash, sin
/// importar format, comentarios o posición en el archivo fuente.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in &self.0 {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

/// Hash Merkle de un `SemanticNode`. El hash es función pura de
/// `(kind, field_name, leaf_text, &[child_hash])`. Esquema estricto:
/// los hijos contribuyen como hash, no como bytestream completo. Eso
/// permite verificar un nodo recibido por la red **sin tener** sus
/// hijos: basta con tener los hashes de los hijos (que vienen en el
/// `StoredNode.children`) y reproducir esta función.
pub fn hash_node(node: &SemanticNode) -> ContentHash {
    let child_hashes: Vec<ContentHash> = node.children.iter().map(hash_node).collect();
    hash_components(
        &node.kind,
        node.field_name.as_deref(),
        node.leaf_text.as_deref(),
        &child_hashes,
    )
}

/// Primitiva canónica del hash estructural. Es la única definición
/// authoritativa: cualquier otra función que produzca un hash de
/// contenido debe expresarse encima de ésta. Garantiza que
/// `hash_node(&semantic)` y `hash_stored(&stored)` coincidan bit a bit
/// para representaciones equivalentes del mismo árbol.
pub fn hash_components(
    kind: &str,
    field_name: Option<&str>,
    leaf_text: Option<&[u8]>,
    child_hashes: &[ContentHash],
) -> ContentHash {
    let mut h = Hasher::new();
    write_str(&mut h, kind);
    match field_name {
        Some(f) => {
            h.update(&[1]);
            write_str(&mut h, f);
        }
        None => {
            h.update(&[0]);
        }
    }
    match leaf_text {
        Some(t) => {
            h.update(&[1]);
            h.update(&(t.len() as u64).to_le_bytes());
            h.update(t);
        }
        None => {
            h.update(&[0]);
        }
    }
    h.update(&(child_hashes.len() as u64).to_le_bytes());
    for ch in child_hashes {
        h.update(&ch.0);
    }
    ContentHash(*h.finalize().as_bytes())
}

fn write_str(h: &mut Hasher, s: &str) {
    h.update(&(s.len() as u64).to_le_bytes());
    h.update(s.as_bytes());
}
