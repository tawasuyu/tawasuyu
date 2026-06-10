//! El contrato [`NodeSource`] —lo mínimo que el VFS necesita de un
//! repositorio Minga— y sus dos backends. Agnóstico de `fuser`.
//!
//! El VFS no quiere conocer `sled` ni la estructura interna del store:
//! sólo necesita (a) enumerar las raíces del MST y (b) resolver un nodo
//! por hash. Eso es [`NodeSource`]. [`RepoSource`] lo implementa sobre
//! el [`PersistentRepo`] en disco; [`MemSource`] sobre un `MemStore` en
//! RAM (tests, índices efímeros recién sincronizados).

use minga_core::{ContentHash, SemanticNode, StoredNode};
use minga_store::PersistentRepo;

/// Lo que el VFS necesita de un repositorio para proyectarlo.
pub trait NodeSource {
    /// Hashes raíz: el conjunto de claves del MST, un elemento por
    /// archivo ingerido. Es lo que se lista bajo `roots/`.
    fn roots(&self) -> Vec<ContentHash>;

    /// Resuelve un único nodo (un eslabón del grafo) por su hash.
    /// `None` si no está en el almacén.
    fn get(&self, hash: &ContentHash) -> Option<StoredNode>;
}

/// Reconstruye el `SemanticNode` completo de un hash, resolviendo
/// recursivamente sus hijos contra `source`.
///
/// Devuelve `None` si el almacén está incompleto: o el propio `hash`
/// falta, o lo hace algún descendiente (puede ocurrir en un repo a
/// medio sincronizar).
pub fn reconstruct<S>(source: &S, hash: &ContentHash) -> Option<SemanticNode>
where
    S: NodeSource + ?Sized,
{
    let stored = source.get(hash)?;
    let mut children = Vec::with_capacity(stored.children.len());
    for child in &stored.children {
        children.push(reconstruct(source, child)?);
    }
    Some(SemanticNode {
        kind: stored.kind,
        field_name: stored.field_name,
        leaf_text: stored.leaf_text,
        children,
    })
}

/// [`NodeSource`] respaldado por un [`PersistentRepo`] de `minga-store`
/// (almacén `sled` en disco). Es la fuente que usa `minga mount`.
pub struct RepoSource {
    repo: PersistentRepo,
}

impl RepoSource {
    /// Envuelve un repo ya abierto. La propiedad pasa al `RepoSource`:
    /// el repo se cierra cuando éste se dropea.
    pub fn new(repo: PersistentRepo) -> Self {
        Self { repo }
    }
}

impl NodeSource for RepoSource {
    fn roots(&self) -> Vec<ContentHash> {
        // Las claves del MST corruptas (si las hubiera) se descartan en
        // silencio: un par de entradas ilegibles no deben tirar el `ls`.
        // Esto devuelve **α-hashes**: la identidad estable de los
        // archivos ingeridos, no su hash estructural.
        self.repo.mst.iter().filter_map(Result::ok).collect()
    }

    fn get(&self, hash: &ContentHash) -> Option<StoredNode> {
        // Primero intentamos resolver `hash` como α-hash de una raíz:
        // si lo es, redirigimos al struct-hash que apunta al `StoredNode`
        // real dentro del grafo CAS. Si no es raíz, asumimos que es un
        // hash estructural y lo buscamos directo (esto cubre la
        // navegación `cas/<hash>` de cualquier nodo interno).
        if let Ok(Some((struct_hash, _dialect))) = self.repo.roots.get(hash) {
            return self.repo.nodes.get(&struct_hash).ok().flatten();
        }
        self.repo.nodes.get(hash).ok().flatten()
    }
}

/// [`NodeSource`] en memoria: un `MemStore` más un conjunto explícito
/// de raíces. Para tests y para montar índices que viven sólo en RAM.
#[derive(Default)]
pub struct MemSource {
    store: minga_core::MemStore,
    roots: Vec<ContentHash>,
}

impl MemSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserta un árbol como raíz (un "archivo") y devuelve su hash.
    /// Idempotente: ingerir dos veces el mismo árbol no lo duplica.
    pub fn add_root(&mut self, node: &SemanticNode) -> ContentHash {
        use minga_core::NodeStore;
        let hash = self.store.put(node);
        if !self.roots.contains(&hash) {
            self.roots.push(hash);
        }
        hash
    }
}

impl NodeSource for MemSource {
    fn roots(&self) -> Vec<ContentHash> {
        self.roots.clone()
    }

    fn get(&self, hash: &ContentHash) -> Option<StoredNode> {
        use minga_core::NodeStore;
        // `NodeStore::get` ya devuelve owned (trait por valor desde #5/A).
        self.store.get(hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minga_core::ast::SemanticNode;

    fn leaf(kind: &str, text: &str) -> SemanticNode {
        SemanticNode {
            kind: kind.to_string(),
            field_name: None,
            leaf_text: Some(text.as_bytes().to_vec()),
            children: Vec::new(),
        }
    }

    #[test]
    fn mem_source_reconstructs_what_it_stored() {
        let tree = SemanticNode {
            kind: "root".to_string(),
            field_name: None,
            leaf_text: None,
            children: vec![leaf("a", "1"), leaf("b", "2")],
        };
        let mut src = MemSource::new();
        let hash = src.add_root(&tree);

        assert_eq!(src.roots(), vec![hash]);
        let back = reconstruct(&src, &hash).expect("debe reconstruir");
        assert_eq!(back, tree);
    }

    #[test]
    fn add_root_is_idempotent() {
        let tree = leaf("only", "x");
        let mut src = MemSource::new();
        let h1 = src.add_root(&tree);
        let h2 = src.add_root(&tree);
        assert_eq!(h1, h2);
        assert_eq!(src.roots().len(), 1);
    }

    #[test]
    fn unknown_hash_reconstructs_to_none() {
        let src = MemSource::new();
        assert!(reconstruct(&src, &ContentHash([0u8; 32])).is_none());
    }
}
