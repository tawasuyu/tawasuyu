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
}

/// El almacén en disco de una conversación.
pub struct AlmacenAyni {
    db: sled::Db,
}

impl AlmacenAyni {
    /// Abre (o crea) el almacén en `ruta`.
    pub fn abrir(ruta: impl AsRef<std::path::Path>) -> Result<Self, ErrorStore> {
        Ok(AlmacenAyni {
            db: sled::open(ruta)?,
        })
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
