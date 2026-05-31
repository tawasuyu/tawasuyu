//! Adapter [`Source`] sobre el grafo CAS de un repositorio minga (`.minga/`,
//! sled).
//!
//! La cuarta forma de árbol del front universal, distinta a las otras tres:
//! minga guarda código como un **DAG de AST direccionado por contenido**
//! (`StoredNode{kind, leaf_text, children: [hash]}`), donde dos subárboles
//! estructuralmente iguales se almacenan una sola vez. Acá lo navegamos: la
//! raíz lista todos los nodos del store; descender un nodo muestra sus hijos
//! del AST; una hoja (sin hijos) lee su `leaf_text` —el token de código— por
//! el visor. El nombre de fila es el `kind` del nodo (`function_item`,
//! `identifier`, …) + hash corto: navegación etiquetada semánticamente.
//!
//! Puro local (lee el sled del peer, no abre red). Detrás de la feature
//! `minga` para no arrastrar `minga-store`/sled a quien sólo quiere
//! POSIX/wawa.

use std::io;
use std::path::Path;

use minga_core::ContentHash;
use minga_store::PersistentRepo;

use crate::{from_hex, to_hex, Node, NodeId, Source};

/// Id de la raíz sintética que lista todos los nodos del store.
const RAIZ: &str = "@nodos";

/// Fuente que navega el grafo CAS de AST de un repositorio minga.
pub struct MingaSource {
    repo: PersistentRepo,
    etiqueta: String,
}

impl MingaSource {
    /// Abre el repositorio sled en `ruta` (`.minga/` o equivalente). Sled
    /// crea el directorio si no existe — un repo nuevo simplemente queda
    /// vacío. Error si el path no es abrible como base sled.
    pub fn abrir(ruta: impl AsRef<Path>) -> io::Result<Self> {
        let ruta = ruta.as_ref();
        let repo = PersistentRepo::open(ruta).map_err(io::Error::other)?;
        let etiqueta = ruta
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| ruta.to_string_lossy().into_owned());
        Ok(Self { repo, etiqueta })
    }

    fn parse_id(id: &NodeId) -> io::Result<ContentHash> {
        from_hex(id)
            .map(ContentHash)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("id minga inválido: {id}")))
    }

    /// `Node` de un hash: nombre = `kind` + hash corto; contenedor si el
    /// nodo tiene hijos en el AST.
    fn nodo_de(&self, h: &ContentHash) -> Node {
        let hex = to_hex(h.as_bytes());
        let corto: String = hex.chars().take(8).collect();
        match self.repo.nodes.get(h) {
            Ok(Some(stored)) => {
                Node::new(hex, format!("{} · {corto}", stored.kind), !stored.children.is_empty())
            }
            // Referencia colgante (hijo no presente aún) o error de lectura:
            // lo mostramos como hoja anónima en vez de romper la navegación.
            _ => Node::new(hex, format!("? · {corto}"), false),
        }
    }
}

impl Source for MingaSource {
    fn label(&self) -> String {
        self.etiqueta.clone()
    }

    fn root(&self) -> Node {
        Node::new(RAIZ, self.etiqueta.clone(), true)
    }

    fn children(&self, id: &NodeId) -> io::Result<Vec<Node>> {
        if id == RAIZ {
            let mut hashes: Vec<ContentHash> = self
                .repo
                .nodes
                .iter_hashes()
                .filter_map(Result::ok)
                .collect();
            hashes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            return Ok(hashes.iter().map(|h| self.nodo_de(h)).collect());
        }
        let h = Self::parse_id(id)?;
        let hijos = self
            .repo
            .nodes
            .children_of(&h)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("nodo minga inexistente: {id}")))?;
        Ok(hijos.iter().map(|c| self.nodo_de(c)).collect())
    }

    fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        if id == RAIZ {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "la raíz @nodos no tiene contenido leíble",
            ));
        }
        let h = Self::parse_id(id)?;
        let stored = self
            .repo
            .nodes
            .get(&h)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("nodo minga inexistente: {id}")))?;
        // Una hoja lleva su token en `leaf_text`; un nodo interno sin texto
        // (no debería abrirse como hoja, pero por las dudas) cae a su kind.
        Ok(stored.leaf_text.unwrap_or_else(|| stored.kind.into_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::to_hex;
    use minga_core::ast::SemanticNode;
    use minga_store::PersistentRepo;

    fn nodo(kind: &str, leaf: Option<&[u8]>, children: Vec<SemanticNode>) -> SemanticNode {
        SemanticNode {
            kind: kind.into(),
            field_name: None,
            leaf_text: leaf.map(|b| b.to_vec()),
            children,
        }
    }

    /// Abre un repo sled temporal, mete un AST chico (call_expression →
    /// identifier "foo") y devuelve (dir, ruta, hash_raiz). Suelta el handle
    /// sled (drop) para que `MingaSource::abrir` pueda re-abrir el mismo path.
    fn repo_con_ast() -> (tempfile::TempDir, std::path::PathBuf, ContentHash) {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("repo.minga");
        let hash = {
            let repo = PersistentRepo::open(&ruta).unwrap();
            let hoja = nodo("identifier", Some(b"foo"), vec![]);
            let llamada = nodo("call_expression", None, vec![hoja]);
            let h = repo.nodes.put(&llamada).unwrap();
            repo.flush().unwrap();
            h
        };
        (dir, ruta, hash)
    }

    #[test]
    fn navega_dag_de_ast_y_lee_hoja() {
        let (_dir, ruta, raiz) = repo_con_ast();
        let src = MingaSource::abrir(&ruta).unwrap();

        let root = src.root();
        assert_eq!(root.id, RAIZ);
        assert!(root.is_container);

        // La raíz lista todos los nodos; la raíz del AST está entre ellos y
        // es contenedor.
        let nodos = src.children(&root.id).unwrap();
        let raiz_hex = to_hex(raiz.as_bytes());
        let raiz_nodo = nodos.iter().find(|n| n.id == raiz_hex).expect("raíz en el listado");
        assert!(raiz_nodo.name.starts_with("call_expression"));
        assert!(raiz_nodo.is_container);

        // Descender la raíz → su hijo identifier (hoja).
        let hijos = src.children(&raiz_hex).unwrap();
        assert_eq!(hijos.len(), 1);
        assert!(hijos[0].name.starts_with("identifier"));
        assert!(!hijos[0].is_container);

        // Leer la hoja → el token "foo".
        assert_eq!(src.read(&hijos[0].id).unwrap(), b"foo");
    }

    #[test]
    fn repo_vacio_lista_cero_nodos() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("vacio.minga");
        {
            let _repo = PersistentRepo::open(&ruta).unwrap();
        }
        let src = MingaSource::abrir(&ruta).unwrap();
        assert!(src.children(&RAIZ.to_string()).unwrap().is_empty());
        assert!(src.read(&RAIZ.to_string()).is_err());
    }

    #[test]
    fn id_basura_es_error() {
        let (_dir, ruta, _raiz) = repo_con_ast();
        let src = MingaSource::abrir(&ruta).unwrap();
        assert!(src.children(&"no-es-hex".to_string()).is_err());
        assert!(src.read(&"no-es-hex".to_string()).is_err());
    }
}
