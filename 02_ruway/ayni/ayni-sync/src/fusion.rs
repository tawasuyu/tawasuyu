//! Fusión verificada de nodos entrantes — con tolerancia a fuera-de-orden.
//!
//! La red no garantiza que un nodo llegue después de sus padres: dos peers
//! pueden reenviarte un hijo antes de que tengas su padre. El [`Fusionador`]
//! resuelve eso con un búfer de PENDIENTES: un nodo cuya firma valida pero cuyo
//! padre aún falta se guarda, y se reintenta cada vez que el grafo crece. Es la
//! semilla local del store-and-forward de P3, hecha barata ya en P1.
//!
//! Toda inserción pasa por el filtro de firma: un nodo cuya firma no valida
//! contra el `autor` que declara se descarta —nunca entra al grafo—.

use ayni_core::{AgoraId, Conversacion, Firma, Hash, MensajeNodo};

use crate::Sobre;

/// Acumula nodos entrantes y los inserta en una [`Conversacion`] en cuanto sus
/// padres están presentes, verificando cada firma antes de admitirlos.
#[derive(Default)]
pub struct Fusionador {
    /// Nodos con firma válida cuyos padres aún no llegaron. Se reintentan tras
    /// cada inserción exitosa.
    pendientes: Vec<MensajeNodo>,
}

impl Fusionador {
    /// Un fusionador sin pendientes.
    pub fn nuevo() -> Self {
        Fusionador::default()
    }

    /// Cuántos nodos esperan a sus padres ahora mismo. Útil para diagnóstico
    /// (un número que no baja delata un grafo incompleto: falta pedir nodos).
    pub fn pendientes(&self) -> usize {
        self.pendientes.len()
    }

    /// Aplica UN nodo entrante. Rechaza el de firma inválida (devuelve vacío sin
    /// tocar nada). Si el padre falta, lo guarda pendiente. Tras insertar,
    /// reintenta los pendientes en cascada. Devuelve los ids REALMENTE añadidos
    /// al grafo —para que el llamador pinte/notifique sólo lo nuevo—.
    pub fn aplicar_nodo(
        &mut self,
        conv: &mut Conversacion,
        nodo: MensajeNodo,
        verificar: impl Fn(&AgoraId, &Hash, &Firma) -> bool + Copy,
    ) -> Vec<Hash> {
        if !nodo.verificar(verificar) {
            return Vec::new();
        }
        // ya presente ⇒ nada que hacer (idempotencia del grafo).
        if conv.contiene(&nodo.id()) {
            return Vec::new();
        }
        self.pendientes.push(nodo);
        self.drenar(conv)
    }

    /// Aplica un LOTE de nodos entrantes —típicamente el volcado que un peer
    /// manda al conectarse ([`ayni_core::Conversacion::instantanea`])—. Verifica
    /// cada uno; admite sólo los de firma válida. No necesita orden: el drenado
    /// alcanza el punto fijo reintentando hasta que ningún padre falte. Devuelve
    /// los ids añadidos.
    pub fn aplicar_lote(
        &mut self,
        conv: &mut Conversacion,
        nodos: impl IntoIterator<Item = MensajeNodo>,
        verificar: impl Fn(&AgoraId, &Hash, &Firma) -> bool + Copy,
    ) -> Vec<Hash> {
        for nodo in nodos {
            if nodo.verificar(verificar) && !conv.contiene(&nodo.id()) {
                self.pendientes.push(nodo);
            }
        }
        self.drenar(conv)
    }

    /// Los ids de PADRES que algún nodo pendiente espera y que aún no están en
    /// el grafo (ni son, ellos mismos, otro pendiente). Son exactamente los
    /// eslabones que hay que PEDIR al peer para que la reconciliación avance.
    pub fn padres_faltantes(&self, conv: &Conversacion) -> Vec<Hash> {
        use std::collections::BTreeSet;
        let en_espera: BTreeSet<Hash> = self.pendientes.iter().map(|n| n.id()).collect();
        let mut faltan = BTreeSet::new();
        for nodo in &self.pendientes {
            for p in nodo.padres() {
                if !conv.contiene(p) && !en_espera.contains(p) {
                    faltan.insert(*p);
                }
            }
        }
        faltan.into_iter().collect()
    }

    /// Procesa un [`Sobre`] de anti-entropía contra la conversación y devuelve
    /// `(ids_nuevos, respuestas)`: los nodos que entraron al grafo (para pintar)
    /// y los sobres a devolver AL MISMO peer para que la reconciliación siga.
    /// El `Sobre::Hola` no es asunto del fusionador (lo maneja la app, que tiene
    /// la cripto) — aquí se ignora.
    ///
    /// El baile completo: A anuncia `Cabezas`; B pide las que le faltan
    /// (`Pedir`); A las entrega (`Entrega`); al insertarlas, B descubre padres
    /// ausentes y los pide; A entrega… hasta que B tiene todo el cono causal.
    /// Sólo viajan los nodos que de verdad faltaban.
    pub fn procesar(
        &mut self,
        conv: &mut Conversacion,
        sobre: Sobre,
        verificar: impl Fn(&AgoraId, &Hash, &Firma) -> bool + Copy,
    ) -> (Vec<Hash>, Vec<Sobre>) {
        match sobre {
            Sobre::Hola { .. } => (Vec::new(), Vec::new()),
            Sobre::Cabezas(ids) => {
                let faltan: Vec<Hash> = ids.into_iter().filter(|h| !conv.contiene(h)).collect();
                let resp = if faltan.is_empty() {
                    Vec::new()
                } else {
                    vec![Sobre::Pedir(faltan)]
                };
                (Vec::new(), resp)
            }
            Sobre::Pedir(ids) => {
                let nodos: Vec<MensajeNodo> =
                    ids.iter().filter_map(|h| conv.obtener(h).cloned()).collect();
                let resp = if nodos.is_empty() {
                    Vec::new()
                } else {
                    vec![Sobre::Entrega(nodos)]
                };
                (Vec::new(), resp)
            }
            Sobre::Entrega(nodos) => {
                let nuevos = self.aplicar_lote(conv, nodos, verificar);
                (nuevos, self.pedir_padres(conv))
            }
            Sobre::Nodo(nodo) => {
                let nuevos = self.aplicar_nodo(conv, nodo, verificar);
                (nuevos, self.pedir_padres(conv))
            }
        }
    }

    /// Envuelve [`padres_faltantes`](Self::padres_faltantes) en un `Pedir`, o nada.
    fn pedir_padres(&self, conv: &Conversacion) -> Vec<Sobre> {
        let faltan = self.padres_faltantes(conv);
        if faltan.is_empty() {
            Vec::new()
        } else {
            vec![Sobre::Pedir(faltan)]
        }
    }

    /// Reintenta insertar los pendientes hasta que una pasada completa no añada
    /// ninguno (punto fijo). Devuelve todos los ids añadidos en el proceso.
    fn drenar(&mut self, conv: &mut Conversacion) -> Vec<Hash> {
        let mut anadidos = Vec::new();
        loop {
            let mut progreso = false;
            let mut quedan = Vec::with_capacity(self.pendientes.len());
            for nodo in core::mem::take(&mut self.pendientes) {
                // ¿todos los padres ya en el grafo?
                let listo = nodo.padres().iter().all(|p| conv.contiene(p));
                if listo {
                    match conv.agregar(nodo) {
                        Ok(id) => {
                            if !anadidos.contains(&id) {
                                anadidos.push(id);
                            }
                            progreso = true;
                        }
                        Err(_) => { /* no debería: padres verificados arriba */ }
                    }
                } else {
                    quedan.push(nodo);
                }
            }
            self.pendientes = quedan;
            if !progreso {
                break;
            }
        }
        anadidos
    }
}
