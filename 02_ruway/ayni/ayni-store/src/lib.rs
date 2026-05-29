// =============================================================================
//  ayni :: ayni-store — la conversación en disco
// -----------------------------------------------------------------------------
//  Cada nodo del DAG se guarda con su id (BLAKE3) como clave y su forma postcard
//  como valor, en un `sled::Db`. Como el id ES el hash del contenido, guardar
//  dos veces el mismo nodo es idempotente y nunca hay claves en conflicto. Al
//  cargar, se leen todos los nodos y se reconstruye la `Conversacion` con
//  `desde_nodos` (que tolera cualquier orden de inserción).
//
//  No verifica firmas al cargar: confía en su propio disco. Los nodos que
//  llegan por la RED se verifican antes de tocar el grafo (en el `Fusionador`
//  de `ayni-sync`); lo que aquí se relee ya pasó ese filtro al guardarse.
// =============================================================================

use ayni_core::{Conversacion, Hash, MensajeNodo};

/// Falla de una operación del store.
#[derive(Debug, thiserror::Error)]
pub enum ErrorStore {
    /// Error del motor sled (E/S, corrupción de página, lock del directorio).
    #[error("ayni-store :: sled: {0}")]
    Sled(#[from] sled::Error),
    /// Un valor en disco no decodifica como `MensajeNodo` — formato viejo o
    /// corrupción. Se reporta en lugar de envenenar el grafo en silencio.
    #[error("ayni-store :: nodo en disco ilegible (postcard)")]
    Decodificacion,
    /// Un blob leído no hashea a su propia clave — corrupción en disco. El
    /// direccionamiento por contenido la detecta sin ambigüedad.
    #[error("ayni-store :: blob corrupto (el hash no coincide con su clave)")]
    BlobCorrupto,
}

/// El almacén en disco de una conversación.
pub struct AlmacenAyni {
    db: sled::Db,
    /// Árbol aparte para los blobs de adjuntos (P5), direccionados por su hash
    /// BLAKE3. Separado de los nodos para que recorrer la conversación no
    /// arrastre contenidos pesados.
    blobs: sled::Tree,
}

impl AlmacenAyni {
    /// Abre (o crea) el almacén en `ruta`.
    pub fn abrir(ruta: impl AsRef<std::path::Path>) -> Result<Self, ErrorStore> {
        let db = sled::open(ruta)?;
        let blobs = db.open_tree("blobs")?;
        Ok(AlmacenAyni { db, blobs })
    }

    /// Guarda (o reemplaza, que es lo mismo: el id es el hash) un nodo.
    pub fn guardar(&self, nodo: &MensajeNodo) -> Result<(), ErrorStore> {
        let bytes = postcard::to_allocvec(nodo).map_err(|_| ErrorStore::Decodificacion)?;
        self.db.insert(nodo.id(), bytes)?;
        Ok(())
    }

    /// Guarda TODOS los nodos de una conversación. Útil para un volcado inicial;
    /// en caliente conviene [`guardar`](Self::guardar) por nodo nuevo.
    pub fn guardar_todos(&self, conv: &Conversacion) -> Result<(), ErrorStore> {
        for (_, nodo) in conv.nodos() {
            self.guardar(nodo)?;
        }
        Ok(())
    }

    /// ¿Está este nodo en disco?
    pub fn contiene(&self, id: &Hash) -> Result<bool, ErrorStore> {
        Ok(self.db.contains_key(id)?)
    }

    /// Cuántos nodos hay persistidos.
    pub fn len(&self) -> usize {
        self.db.len()
    }

    /// ¿Almacén vacío?
    pub fn esta_vacio(&self) -> bool {
        self.db.is_empty()
    }

    /// Reconstruye la conversación entera desde disco. Lee todos los nodos y los
    /// re-cablea con [`Conversacion::desde_nodos`] (tolera cualquier orden).
    pub fn cargar(&self) -> Result<Conversacion, ErrorStore> {
        let mut nodos = Vec::with_capacity(self.db.len());
        for entrada in self.db.iter() {
            let (_, valor) = entrada?;
            let nodo: MensajeNodo =
                postcard::from_bytes(&valor).map_err(|_| ErrorStore::Decodificacion)?;
            nodos.push(nodo);
        }
        Ok(Conversacion::desde_nodos(nodos))
    }

    /// Fuerza el volcado a disco (sled ya persiste, pero esto espera al fsync).
    pub fn sincronizar(&self) -> Result<(), ErrorStore> {
        self.db.flush()?;
        Ok(())
    }

    // --- blobs de adjuntos (P5) ---------------------------------------------

    /// Guarda un blob direccionado por su hash BLAKE3 y devuelve ese hash. La
    /// dedup es automática: dos blobs idénticos comparten clave y se guardan una
    /// sola vez. El hash se calcula con la misma función del grafo (`format`),
    /// así que coincide con el `Adjunto::hash` que lo referencia.
    pub fn guardar_blob(&self, bytes: &[u8]) -> Result<Hash, ErrorStore> {
        let hash = ayni_core::hash(bytes);
        self.blobs.insert(hash, bytes)?;
        Ok(hash)
    }

    /// ¿Está este blob en disco?
    pub fn tiene_blob(&self, hash: &Hash) -> Result<bool, ErrorStore> {
        Ok(self.blobs.contains_key(hash)?)
    }

    /// Lee un blob por su hash, VERIFICANDO su integridad: recalcula el hash de
    /// los bytes leídos y rechaza si no coincide (corrupción en disco). Devuelve
    /// `None` si no está.
    pub fn cargar_blob(&self, hash: &Hash) -> Result<Option<Vec<u8>>, ErrorStore> {
        match self.blobs.get(hash)? {
            None => Ok(None),
            Some(bytes) => {
                let bytes = bytes.to_vec();
                if ayni_core::hash(&bytes) != *hash {
                    return Err(ErrorStore::BlobCorrupto);
                }
                Ok(Some(bytes))
            }
        }
    }

    /// Cuántos blobs hay.
    pub fn num_blobs(&self) -> usize {
        self.blobs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{Carga, Conversacion};
    use ayni_crypto::{verificar_firma, Identidad};

    #[test]
    fn guardar_y_recargar_conserva_el_hilo() {
        let dir = tempfile::tempdir().unwrap();
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");

        // construir un hilo de 3 mensajes y persistirlo nodo a nodo.
        let mut conv = Conversacion::nueva();
        {
            let almacen = AlmacenAyni::abrir(dir.path()).unwrap();
            for i in 0..3 {
                let nodo = conv.redactar(alicia.agora_id(), Carga::Texto(format!("m{i}")), i, |id| {
                    alicia.firmar(id)
                });
                conv.agregar(nodo.clone()).unwrap();
                almacen.guardar(&nodo).unwrap();
            }
            almacen.sincronizar().unwrap();
            assert_eq!(almacen.len(), 3);
        }

        // reabrir desde cero y reconstruir.
        let almacen = AlmacenAyni::abrir(dir.path()).unwrap();
        let recargada = almacen.cargar().unwrap();
        assert_eq!(recargada.len(), 3);
        assert_eq!(
            recargada.orden_topologico(),
            conv.orden_topologico(),
            "el hilo se reconstruye idéntico"
        );
        assert!(
            recargada.verificar_firmas(verificar_firma).is_ok(),
            "las firmas sobreviven al disco"
        );
    }

    #[test]
    fn blobs_con_dedup_y_verificacion() {
        use ayni_core::Adjunto;
        let dir = tempfile::tempdir().unwrap();
        let almacen = AlmacenAyni::abrir(dir.path()).unwrap();

        let contenido = b"# nota de khipu\ncuerpo del documento adjunto";
        let adj = Adjunto::de_bytes("khipu", "text/markdown", "nota.md", contenido);

        // guardar el blob: su hash coincide con el del adjunto.
        let h = almacen.guardar_blob(contenido).unwrap();
        assert_eq!(h, adj.hash, "el blob se direcciona por el mismo hash que el adjunto");
        assert!(almacen.tiene_blob(&adj.hash).unwrap());

        // dedup: re-guardar el mismo contenido no agrega otro blob.
        almacen.guardar_blob(contenido).unwrap();
        assert_eq!(almacen.num_blobs(), 1, "dedup por contenido");

        // recargar verifica integridad y los bytes vuelven intactos.
        let leido = almacen.cargar_blob(&adj.hash).unwrap().unwrap();
        assert_eq!(leido, contenido);
        assert!(adj.verifica(&leido), "el blob recuperado satisface la referencia");

        // un hash que no existe → None.
        assert!(almacen.cargar_blob(&[0u8; 32]).unwrap().is_none());
    }

    #[test]
    fn guardar_es_idempotente_por_contenido() {
        let dir = tempfile::tempdir().unwrap();
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let conv = Conversacion::nueva();
        let nodo = conv.redactar(alicia.agora_id(), Carga::Texto("uno".into()), 1, |id| {
            alicia.firmar(id)
        });
        let almacen = AlmacenAyni::abrir(dir.path()).unwrap();
        almacen.guardar(&nodo).unwrap();
        almacen.guardar(&nodo).unwrap(); // de nuevo: mismo id, no duplica.
        assert_eq!(almacen.len(), 1);
        assert!(almacen.contiene(&nodo.id()).unwrap());
    }
}
