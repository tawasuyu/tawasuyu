//! Adapter [`Source`] sobre los objetos content-addressed de una imagen
//! wawa (`.img`).
//!
//! Reusa `wawa-explorer-core::Disco` (lectura pura, sin red ni daemon: carga
//! el log entero en memoria y sirve lookups O(1) por hash). Navega el DAG
//! BLAKE3: el [`NodeId`] de un objeto es su hash en hex; los hijos son
//! `Objeto::hijos`; la hoja se lee de `Objeto::datos`.
//!
//! Los objetos wawa son **anónimos** (su identidad ES el hash), así que el
//! nombre de fila es el hash corto — feo pero navegable. La raíz se toma del
//! ancla del superbloque (`raiz`, si no `manifiesto`); si la imagen no tiene
//! anclas, se sintetiza una raíz `@imagen` que lista todos los objetos.

use std::io;

use wawa_explorer_core::Disco;

use crate::{from_hex, to_hex, Node, NodeId, Source};

/// Id sintético de la raíz cuando el superbloque no tiene anclas — su único
/// rol es contener "todos los objetos del grafo" para que la imagen siga
/// siendo navegable.
const RAIZ_SINTETICA: &str = "@imagen";

/// Fuente que navega el DAG de una imagen wawa cargada en memoria.
pub struct WawaImgSource {
    disco: Disco,
    etiqueta: String,
}

impl WawaImgSource {
    /// Abre y carga la imagen `.img` en `ruta`. Error de I/O o de formato si
    /// la imagen está corrupta o no es una imagen wawa.
    pub fn abrir(ruta: impl AsRef<std::path::Path>) -> io::Result<Self> {
        let ruta = ruta.as_ref();
        let disco = Disco::abrir(ruta).map_err(io::Error::other)?;
        let etiqueta = ruta
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| ruta.to_string_lossy().into_owned());
        Ok(Self { disco, etiqueta })
    }

    /// Construye el `Node` de un objeto del grafo. El nombre es el hash
    /// corto; es contenedor si tiene hijos.
    fn nodo_de(&self, hash: &[u8; 32], nombre: Option<String>) -> Node {
        let hex = to_hex(hash);
        let nombre = nombre.unwrap_or_else(|| hex.chars().take(12).collect());
        let es_contenedor = self
            .disco
            .hijos(hash)
            .map(|h| !h.is_empty())
            .unwrap_or(false);
        Node::new(hex, nombre, es_contenedor)
    }

    fn parse_id(id: &NodeId) -> io::Result<[u8; 32]> {
        from_hex(id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("id wawa inválido: {id}"))
        })
    }
}

impl Source for WawaImgSource {
    fn label(&self) -> String {
        self.etiqueta.clone()
    }

    fn root(&self) -> Node {
        let sb = self.disco.superbloque();
        if let Some(raiz) = sb.raiz {
            self.nodo_de(&raiz, Some("raíz".into()))
        } else if let Some(man) = sb.manifiesto {
            self.nodo_de(&man, Some("manifiesto".into()))
        } else {
            // Sin anclas: raíz sintética que contiene todo el grafo.
            Node::new(RAIZ_SINTETICA, self.etiqueta.clone(), true)
        }
    }

    fn children(&self, id: &NodeId) -> io::Result<Vec<Node>> {
        if id == RAIZ_SINTETICA {
            // Todos los objetos del grafo, en orden estable por hash.
            let mut hashes: Vec<[u8; 32]> = self.disco.hashes().copied().collect();
            hashes.sort_unstable();
            return Ok(hashes.iter().map(|h| self.nodo_de(h, None)).collect());
        }
        let hash = Self::parse_id(id)?;
        let hijos = self.disco.hijos(&hash).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("objeto wawa inexistente: {id}"))
        })?;
        Ok(hijos.to_vec().iter().map(|h| self.nodo_de(h, None)).collect())
    }

    fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        if id == RAIZ_SINTETICA {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "la raíz sintética @imagen no tiene contenido leíble",
            ));
        }
        let hash = Self::parse_id(id)?;
        self.disco
            .objeto(&hash)
            .map(|o| o.datos.clone())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("objeto wawa inexistente: {id}"))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::to_hex;
    use format::{componer_registro, Objeto, SuperBloque, MAGIA, TAM_SECTOR, VERSION_SUPERBLOQUE};
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;

    /// Construye un `.img` sintético: dos hojas y un objeto-directorio que
    /// las referencia. `anclar` decide si el superbloque apunta su `raiz` al
    /// directorio (true) o queda sin anclas (false). Devuelve además los
    /// hashes del directorio y de la primera hoja para las aserciones.
    fn img_sintetico(anclar: bool) -> (tempfile::TempDir, PathBuf, [u8; 32], [u8; 32]) {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("t.img");
        let mut f = File::create(&ruta).unwrap();

        let hoja_a = Objeto { datos: b"hola wawa".to_vec(), hijos: vec![] };
        let pa = hoja_a.serializar().unwrap();
        let ha = format::hash(&pa);

        let hoja_b = Objeto { datos: vec![0xFF; 4], hijos: vec![] };
        let pb = hoja_b.serializar().unwrap();
        let hb = format::hash(&pb);

        let dir_obj = Objeto { datos: b"raiz".to_vec(), hijos: vec![ha, hb] };
        let pd = dir_obj.serializar().unwrap();
        let hd = format::hash(&pd);

        let reg_a = componer_registro(&pa);
        let reg_b = componer_registro(&pb);
        let reg_d = componer_registro(&pd);
        let sectores = (reg_a.len() + reg_b.len() + reg_d.len()) / TAM_SECTOR;

        let sb = SuperBloque {
            magia: MAGIA,
            version: VERSION_SUPERBLOQUE,
            log_inicio: 1,
            cursor: 1 + sectores as u64,
            raiz: if anclar { Some(hd) } else { None },
            manifiesto: None,
        };
        let sbb = sb.serializar().unwrap();
        let mut sbs = vec![0u8; TAM_SECTOR];
        sbs[..sbb.len()].copy_from_slice(&sbb);

        f.write_all(&sbs).unwrap();
        f.write_all(&reg_a).unwrap();
        f.write_all(&reg_b).unwrap();
        f.write_all(&reg_d).unwrap();
        f.sync_all().unwrap();

        (dir, ruta, hd, ha)
    }

    #[test]
    fn raiz_anclada_navega_a_las_hojas() {
        let (_dir, ruta, hd, ha) = img_sintetico(true);
        let src = WawaImgSource::abrir(&ruta).unwrap();

        let root = src.root();
        assert_eq!(root.id, to_hex(&hd));
        assert_eq!(root.name, "raíz");
        assert!(root.is_container);

        let kids = src.children(&root.id).unwrap();
        assert_eq!(kids.len(), 2);
        let hoja = kids.iter().find(|n| n.id == to_hex(&ha)).expect("hoja A");
        assert!(!hoja.is_container);
        assert_eq!(src.read(&hoja.id).unwrap(), b"hola wawa");
    }

    #[test]
    fn sin_anclas_usa_raiz_sintetica() {
        let (_dir, ruta, _hd, ha) = img_sintetico(false);
        let src = WawaImgSource::abrir(&ruta).unwrap();

        let root = src.root();
        assert_eq!(root.id, RAIZ_SINTETICA);
        assert!(root.is_container);
        // Lista los 3 objetos del grafo.
        let todos = src.children(&root.id).unwrap();
        assert_eq!(todos.len(), 3);
        // La hoja A sigue siendo leíble por su hash.
        assert_eq!(src.read(&to_hex(&ha)).unwrap(), b"hola wawa");
        // La raíz sintética no tiene contenido.
        assert!(src.read(&RAIZ_SINTETICA.to_string()).is_err());
    }

    #[test]
    fn id_basura_es_error() {
        let (_dir, ruta, _hd, _ha) = img_sintetico(true);
        let src = WawaImgSource::abrir(&ruta).unwrap();
        assert!(src.children(&"no-es-hex".to_string()).is_err());
        assert!(src.read(&"no-es-hex".to_string()).is_err());
    }

    #[test]
    fn abrir_archivo_no_wawa_es_error() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("basura.img");
        std::fs::write(&ruta, b"esto no es una imagen wawa").unwrap();
        assert!(WawaImgSource::abrir(&ruta).is_err());
    }
}
