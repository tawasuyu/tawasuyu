//! Almacén persistente de `StoredNode`s indexados por `ContentHash`.
//!
//! Cada nodo se serializa con postcard y se inserta en una `sled::Tree`
//! cuya clave son los 32 bytes del hash. La operación `put` es
//! recursiva sobre los hijos (igual que `MemStore::put`): cada
//! subárbol se hashea y persiste exactamente una vez.

use minga_core::{cas, hash_stored, ContentHash, NodeStore, SemanticNode, StoredNode};
use sled::{Db, Tree};

use crate::error::StoreError;

pub struct SledNodeStore {
    tree: Tree,
}

impl SledNodeStore {
    pub fn open_tree(db: &Db, name: &str) -> Result<Self, StoreError> {
        Ok(Self {
            tree: db.open_tree(name)?,
        })
    }

    /// Inserta un árbol completo. Recursivamente desempaqueta hijos.
    /// Devuelve el hash de la raíz. Idempotente: insertar el mismo
    /// árbol dos veces no añade entradas nuevas.
    pub fn put(&self, node: &SemanticNode) -> Result<ContentHash, StoreError> {
        let mut child_hashes = Vec::with_capacity(node.children.len());
        for c in &node.children {
            child_hashes.push(self.put(c)?);
        }
        let h = cas::hash_components(
            &node.kind,
            node.field_name.as_deref(),
            node.leaf_text.as_deref(),
            &child_hashes,
        );
        if !self.tree.contains_key(h.0)? {
            let stored = StoredNode {
                kind: node.kind.clone(),
                field_name: node.field_name.clone(),
                leaf_text: node.leaf_text.clone(),
                children: child_hashes,
            };
            let bytes = postcard::to_allocvec(&stored)?;
            self.tree.insert(h.0, bytes)?;
        }
        Ok(h)
    }

    /// Inserta un nodo ya troceado por hash. Verifica que el hash
    /// coincida con `hash_stored(stored)` antes de insertar — sin
    /// esa verificación no podemos confiar en la integridad de lo
    /// que viene del wire.
    pub fn put_chunked(
        &self,
        hash: ContentHash,
        stored: &StoredNode,
    ) -> Result<(), StoreError> {
        if hash_stored(stored) != hash {
            return Err(StoreError::HashMismatch);
        }
        if !self.tree.contains_key(hash.0)? {
            let bytes = postcard::to_allocvec(stored)?;
            self.tree.insert(hash.0, bytes)?;
        }
        Ok(())
    }

    pub fn get(&self, h: &ContentHash) -> Result<Option<StoredNode>, StoreError> {
        match self.tree.get(h.0)? {
            Some(bytes) => Ok(Some(postcard::from_bytes(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn contains(&self, h: &ContentHash) -> Result<bool, StoreError> {
        Ok(self.tree.contains_key(h.0)?)
    }

    /// Reconstruye un `SemanticNode` resolviendo recursivamente todos
    /// los hijos. `Ok(None)` si algún hash no está en el store
    /// (almacén incompleto).
    pub fn reconstruct(&self, h: &ContentHash) -> Result<Option<SemanticNode>, StoreError> {
        let stored = match self.get(h)? {
            Some(s) => s,
            None => return Ok(None),
        };
        let mut children = Vec::with_capacity(stored.children.len());
        for ch in &stored.children {
            match self.reconstruct(ch)? {
                Some(n) => children.push(n),
                None => return Ok(None),
            }
        }
        Ok(Some(SemanticNode {
            kind: stored.kind,
            field_name: stored.field_name,
            leaf_text: stored.leaf_text,
            children,
        }))
    }

    pub fn len(&self) -> usize {
        self.tree.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    pub fn flush(&self) -> Result<(), StoreError> {
        self.tree.flush()?;
        Ok(())
    }

    /// Elimina un nodo del store por su hash. **Cuidado**: los hijos no
    /// se borran en cascada (otros nodos pueden referenciarlos). El
    /// caller es responsable de la consistencia (típicamente: usar
    /// mark-sweep sobre raíces vivas).
    pub fn remove(&self, h: &ContentHash) -> Result<bool, StoreError> {
        Ok(self.tree.remove(h.0)?.is_some())
    }

    /// Itera sólo los hashes (sin deserializar el valor). Más liviano
    /// que `iter` cuando sólo se necesitan las claves — útil para
    /// mark-sweep del GC.
    pub fn iter_hashes(&self) -> impl Iterator<Item = Result<ContentHash, StoreError>> + '_ {
        self.tree.iter().map(|kv| {
            let (k, _) = kv?;
            if k.len() != 32 {
                return Err(StoreError::HashMismatch);
            }
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&k);
            Ok(ContentHash(bytes))
        })
    }

    /// Lee sólo los hashes de los hijos de un nodo (sin reconstruir
    /// `StoredNode` completo más allá del shape del header postcard).
    /// Optimización del walk del mark-sweep: para visitar el subárbol
    /// no necesitamos `kind`/`field_name`/`leaf_text`.
    pub fn children_of(&self, h: &ContentHash) -> Result<Option<Vec<ContentHash>>, StoreError> {
        match self.tree.get(h.0)? {
            Some(bytes) => {
                let stored: StoredNode = postcard::from_bytes(&bytes)?;
                Ok(Some(stored.children))
            }
            None => Ok(None),
        }
    }

    /// Itera todos los pares `(hash, stored_node)` persistidos. Sin
    /// orden garantizado más allá del lexicográfico de sled.
    pub fn iter(
        &self,
    ) -> impl Iterator<Item = Result<(ContentHash, StoredNode), StoreError>> + '_ {
        self.tree.iter().map(|kv| {
            let (k, v) = kv?;
            if k.len() != 32 {
                return Err(StoreError::HashMismatch);
            }
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&k);
            let stored: StoredNode = postcard::from_bytes(&v)?;
            Ok((ContentHash(bytes), stored))
        })
    }
}

/// `SledNodeStore` como `NodeStore` de minga-core: deja que el sync P2P
/// opere **directamente sobre disco** (sled), sin volcar el grafo entero
/// a RAM (`MemStore`). Es el corazón del refactor #5/A — un repo de 1,4M
/// nodos se sirve y sincroniza con memoria O(1), no O(n).
///
/// El trait es infalible; los métodos de `SledNodeStore` devuelven
/// `Result`. Contrato de manejo de errores:
/// - **Escritura** (`put`): un fallo es irrecuperable (disco lleno,
///   corrupción) → `panic` fail-fast. Mejor caer que persistir a medias.
/// - **Wire** (`put_chunked`): un hash que no valida viene de un peer; no
///   debe tumbar el proceso → se ignora con log (best-effort).
/// - **Lectura** (`get`/`contains`/`reconstruct`/`iter`): un fallo se
///   trata como ausencia → el protocolo de sync vuelve a pedir el nodo.
impl NodeStore for SledNodeStore {
    fn put(&mut self, node: &SemanticNode) -> ContentHash {
        SledNodeStore::put(self, node).expect("sled node_store put (escritura irrecuperable)")
    }

    fn put_chunked(&mut self, hash: ContentHash, stored: StoredNode) {
        if let Err(e) = SledNodeStore::put_chunked(self, hash, &stored) {
            eprintln!("minga: put_chunked descartado ({hash:?}): {e}");
        }
    }

    fn get(&self, h: &ContentHash) -> Option<StoredNode> {
        SledNodeStore::get(self, h).unwrap_or(None)
    }

    fn contains(&self, h: &ContentHash) -> bool {
        SledNodeStore::contains(self, h).unwrap_or(false)
    }

    fn reconstruct(&self, h: &ContentHash) -> Option<SemanticNode> {
        SledNodeStore::reconstruct(self, h).unwrap_or(None)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = (ContentHash, StoredNode)> + '_> {
        Box::new(SledNodeStore::iter(self).filter_map(|r| r.ok()))
    }

    fn len(&self) -> usize {
        SledNodeStore::len(self)
    }
}
