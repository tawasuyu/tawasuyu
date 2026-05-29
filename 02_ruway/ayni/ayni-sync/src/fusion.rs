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
