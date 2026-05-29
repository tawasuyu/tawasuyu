//! La conversación — el DAG de nodos firmados y sus operaciones.
//!
//! Una `Conversacion` es un conjunto de [`MensajeNodo`] indexado por su id. No
//! impone un orden lineal: el orden lo derivan los `padres`. Las operaciones
//! clave son:
//!
//!   * [`Conversacion::cabezas`] — la FRONTERA del grafo: los nodos que nadie
//!     toma como padre. Un mensaje nuevo nace tomando las cabezas actuales como
//!     padres; así "responder" cose la conversación sin un servidor que asigne
//!     números de secuencia.
//!   * [`Conversacion::orden_topologico`] — una linealización determinista para
//!     pintar el hilo: cada nodo aparece DESPUÉS de todos sus padres, y los
//!     empates se rompen por `(ts, id)` —estable y legible—.
//!   * [`Conversacion::verificar_firmas`] — recorre todo el grafo validando la
//!     firma de cada nodo con el closure que da quien tenga la cripto.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use format::{AgoraId, Firma, Hash};

use crate::error::ErrorAyni;
use crate::nodo::{Carga, Contenido, MensajeNodo, VERSION_NODO};

/// El grafo de una conversación: nodos direccionados por su id.
///
/// `BTreeMap` (no `HashMap`) a propósito: es `no_std`-friendly y su iteración
/// es DETERMINISTA por id, lo que hace reproducibles las salidas de
/// [`Conversacion::cabezas`] y [`Conversacion::raices`] sin un paso de orden
/// extra.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conversacion {
    nodos: BTreeMap<Hash, MensajeNodo>,
}

impl Conversacion {
    /// Una conversación vacía.
    pub fn nueva() -> Self {
        Conversacion {
            nodos: BTreeMap::new(),
        }
    }

    /// Inserta un nodo en el grafo tras validar su INTEGRIDAD ESTRUCTURAL:
    /// versión conocida y todos sus padres ya presentes. NO valida la firma
    /// —eso es [`Conversacion::verificar_firmas`], que necesita cripto que este
    /// núcleo no tiene—. Idempotente: reinsertar un id ya presente es un no-op
    /// que devuelve el id (el direccionamiento por contenido garantiza que dos
    /// nodos con el mismo id son bit-idénticos).
    ///
    /// Devuelve el id bajo el que quedó indexado.
    pub fn agregar(&mut self, nodo: MensajeNodo) -> Result<Hash, ErrorAyni> {
        if nodo.contenido.version != VERSION_NODO {
            return Err(ErrorAyni::VersionDesconocida);
        }
        for padre in nodo.padres() {
            if !self.nodos.contains_key(padre) {
                return Err(ErrorAyni::PadreAusente);
            }
        }
        let id = nodo.id();
        self.nodos.entry(id).or_insert(nodo);
        Ok(id)
    }

    /// Construye y sella un mensaje nuevo que CONTINÚA la conversación: toma las
    /// cabezas actuales como padres, compone el contenido y lo firma con el
    /// closure dado. NO lo inserta —el llamador decide cuándo, y normalmente lo
    /// hace tras la firma para poder difundirlo—. En una conversación vacía el
    /// nodo resultante es una raíz (sin padres).
    pub fn redactar(
        &self,
        autor: AgoraId,
        carga: Carga,
        ts: u64,
        firmar: impl FnOnce(&Hash) -> Firma,
    ) -> MensajeNodo {
        let contenido = Contenido::nuevo(autor, self.cabezas(), carga, ts);
        MensajeNodo::sellar(contenido, firmar)
    }

    /// Conveniencia: [`redactar`](Self::redactar) un texto y [`agregar`](Self::agregar)lo
    /// en un solo paso, devolviendo su id. Útil para construir el grafo local
    /// del autor cuando no hace falta interceptar el nodo antes de insertarlo.
    pub fn publicar_texto(
        &mut self,
        autor: AgoraId,
        texto: impl Into<alloc::string::String>,
        ts: u64,
        firmar: impl FnOnce(&Hash) -> Firma,
    ) -> Result<Hash, ErrorAyni> {
        let nodo = self.redactar(autor, Carga::Texto(texto.into()), ts, firmar);
        self.agregar(nodo)
    }

    /// Fusiona OTRA conversación en ésta, insertando los nodos que falten. Como
    /// dos pares con el mismo grafo calculan el mismo conjunto de ids, fusionar
    /// es conmutativo e idempotente —la base de la convergencia sin servidor—.
    /// Recorre `otra` en orden topológico para que los padres precedan a sus
    /// hijos; devuelve los ids REALMENTE añadidos (los que esta conversación no
    /// tenía), para que el llamador sepa qué hay de nuevo que pintar o notificar.
    /// Un nodo de `otra` cuyo padre no esté ni aquí ni allí se omite en silencio
    /// (grafo de origen incoherente) — el camino normal nunca lo produce.
    pub fn fusionar(&mut self, otra: &Conversacion) -> Vec<Hash> {
        let orden = otra
            .orden_topologico()
            .unwrap_or_else(|| otra.nodos.keys().copied().collect());
        let mut nuevos = Vec::new();
        for id in orden {
            if let Some(nodo) = otra.nodos.get(&id) {
                if !self.nodos.contains_key(&id) {
                    if let Ok(id_ins) = self.agregar(nodo.clone()) {
                        nuevos.push(id_ins);
                    }
                }
            }
        }
        nuevos
    }

    /// El nodo de un id, si está presente.
    pub fn obtener(&self, id: &Hash) -> Option<&MensajeNodo> {
        self.nodos.get(id)
    }

    /// ¿Está este id en el grafo?
    pub fn contiene(&self, id: &Hash) -> bool {
        self.nodos.contains_key(id)
    }

    /// Cuántos nodos hay.
    pub fn len(&self) -> usize {
        self.nodos.len()
    }

    /// ¿Grafo vacío?
    pub fn esta_vacia(&self) -> bool {
        self.nodos.is_empty()
    }

    /// Itera los nodos del grafo (en orden de id, por el `BTreeMap`).
    pub fn nodos(&self) -> impl Iterator<Item = (&Hash, &MensajeNodo)> {
        self.nodos.iter()
    }

    /// Una INSTANTÁNEA del grafo como lista de nodos en orden topológico —los
    /// padres antes que sus hijos—. Es la forma que viaja por la red cuando un
    /// peer vuelca su conversación entera a otro recién conectado, y la que
    /// reconstruye un grafo idéntico al re-insertarla.
    pub fn instantanea(&self) -> Vec<MensajeNodo> {
        let orden = self
            .orden_topologico()
            .unwrap_or_else(|| self.nodos.keys().copied().collect());
        orden
            .iter()
            .filter_map(|id| self.nodos.get(id).cloned())
            .collect()
    }

    /// Las RAÍCES: nodos sin padres. En una conversación de un solo origen hay
    /// una; con varios dispositivos/participantes que arrancan en paralelo
    /// puede haber varias hasta que un nodo posterior las una. Orden por id.
    pub fn raices(&self) -> Vec<Hash> {
        self.nodos
            .iter()
            .filter(|(_, n)| n.padres().is_empty())
            .map(|(id, _)| *id)
            .collect()
    }

    /// Las CABEZAS (tips): nodos que ningún otro toma como padre — la frontera
    /// viva de la conversación. Un mensaje nuevo las toma como padres. Orden por
    /// id (determinista, vía `BTreeSet`).
    pub fn cabezas(&self) -> Vec<Hash> {
        let referidos: BTreeSet<Hash> = self
            .nodos
            .values()
            .flat_map(|n| n.padres().iter().copied())
            .collect();
        self.nodos
            .keys()
            .filter(|id| !referidos.contains(*id))
            .copied()
            .collect()
    }

    /// Una linealización DETERMINISTA del grafo donde cada nodo aparece después
    /// de todos sus padres (orden topológico de Kahn). Los empates —nodos cuyos
    /// padres ya están todos emitidos— se resuelven por `(ts, id)`: cronológico
    /// primero, id como desempate estable. Esto da un hilo legible y, lo más
    /// importante, IGUAL en todos los pares que tengan el mismo grafo.
    ///
    /// Devuelve `None` sólo si el grafo tuviera un ciclo —imposible por
    /// construcción (un padre es el hash de un contenido preexistente), pero la
    /// detección se mantiene como red de seguridad ante datos corruptos
    /// introducidos saltándose [`agregar`](Self::agregar).
    pub fn orden_topologico(&self) -> Option<Vec<Hash>> {
        // hijos[p] = nodos que listan a `p` como padre; grado[n] = padres de `n`
        // aún no emitidos. Kahn clásico, pero la cola de "listos" es un BTreeSet
        // ordenado por (ts, id) para que la salida sea determinista.
        let mut grado: BTreeMap<Hash, usize> = BTreeMap::new();
        let mut hijos: BTreeMap<Hash, Vec<Hash>> = BTreeMap::new();
        for (id, nodo) in &self.nodos {
            grado.insert(*id, nodo.padres().len());
            for padre in nodo.padres() {
                hijos.entry(*padre).or_default().push(*id);
            }
        }

        // clave de orden de un nodo: (ts declarado, id) — id rompe empates.
        let clave = |id: &Hash| -> (u64, Hash) {
            let ts = self.nodos.get(id).map(|n| n.contenido.ts).unwrap_or(0);
            (ts, *id)
        };

        let mut listos: BTreeSet<(u64, Hash)> = grado
            .iter()
            .filter(|(_, g)| **g == 0)
            .map(|(id, _)| clave(id))
            .collect();

        let mut orden = Vec::with_capacity(self.nodos.len());
        while let Some(&menor) = listos.iter().next() {
            listos.remove(&menor);
            let (_, id) = menor;
            orden.push(id);
            if let Some(hs) = hijos.get(&id) {
                for h in hs {
                    if let Some(g) = grado.get_mut(h) {
                        *g -= 1;
                        if *g == 0 {
                            listos.insert(clave(h));
                        }
                    }
                }
            }
        }

        if orden.len() == self.nodos.len() {
            Some(orden)
        } else {
            None // ciclo: imposible por construcción, pero no mentimos si lo hay
        }
    }

    /// Recorre todo el grafo verificando la firma de cada nodo con el closure
    /// `(autor, id, firma) -> bool`. Devuelve `Ok(())` si todas validan, o
    /// `Err(id)` con el id del PRIMER nodo cuya firma no valida (en orden de id,
    /// por el `BTreeMap`) — útil para señalar exactamente qué nodo rechazar.
    pub fn verificar_firmas(
        &self,
        mut verificar: impl FnMut(&AgoraId, &Hash, &Firma) -> bool,
    ) -> Result<(), Hash> {
        for (id, nodo) in &self.nodos {
            if !nodo.verificar(&mut verificar) {
                return Err(*id);
            }
        }
        Ok(())
    }

    /// Serializa la conversación entera a `postcard`: la lista de nodos en orden
    /// topológico (determinista). Forma de transporte/persistencia de todo el
    /// grafo de una vez —el diff de Merkle que sólo manda lo que falta es trabajo
    /// de la capa de sincronización (P3); aquí va el grafo completo—.
    pub fn serializar(&self) -> Vec<u8> {
        let orden = self.orden_topologico().unwrap_or_else(|| self.nodos.keys().copied().collect());
        let lista: Vec<&MensajeNodo> = orden.iter().filter_map(|id| self.nodos.get(id)).collect();
        postcard::to_allocvec(&lista).expect("ayni :: postcard alloc no falla para la conversación")
    }

    /// Reconstruye una conversación desde su forma `postcard`. Reinserta cada
    /// nodo vía [`agregar`](Self::agregar) —respetando la validación estructural—;
    /// como [`serializar`](Self::serializar) emite en orden topológico, los
    /// padres siempre preceden a sus hijos y la reinserción no tropieza.
    pub fn deserializar(bytes: &[u8]) -> Result<Self, ErrorAyni> {
        let lista: Vec<MensajeNodo> =
            postcard::from_bytes(bytes).map_err(|_| ErrorAyni::Deserializacion)?;
        let mut conv = Conversacion::nueva();
        for nodo in lista {
            conv.agregar(nodo)?;
        }
        Ok(conv)
    }
}
