//! Confianza y membresía — la dimensión social del grafo (P7).
//!
//! Tres hechos sociales viajan como cargas firmadas más (ver [`crate::Carga`]),
//! y de ellos se DERIVAN, plegando el DAG, tres lecturas sin autoridad central:
//!
//!   * **Membresía firmada** ([`Conversacion::membresia`]). Quién está en la
//!     conversación no es una lista que un servidor guarda: se calcula plegando
//!     los nodos [`Carga::Membresia`] en orden topológico, con una regla de
//!     autoridad simple —sólo un miembro vigente admite o expulsa— y un ancla
//!     —el autor del primer nodo es el fundador, miembro implícito e
//!     inexpulsable—. Dos pares con el mismo grafo calculan la MISMA membresía.
//!   * **Grafo de confianza agora** ([`Conversacion::confianza_desde`]). Cada
//!     [`Carga::Atestacion`] es una arista firmada `autor → sujeto`. La confianza
//!     se propaga por caminos: desde un observador, un recorrido en anchura
//!     alcanza a quienes él atestigua (a un salto), a quienes ellos atestiguan
//!     (a dos saltos)… La fe es fractal y revocable (`nivel = 0` borra la arista).
//!   * **Recibos simétricos** ([`Conversacion::recibos`]). Quién vio qué se lee
//!     de los nodos [`Carga::Recibo`]. Que sean simétricos —no se dan a quien no
//!     los da— es política de la UX; el núcleo sólo expone el hecho verificable.
//!
//! Todo esto es modelo PURO `no_std`: viaja a wawa con el resto de `ayni-core`.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use format::{AgoraId, Firma, Hash};

use crate::conversacion::Conversacion;
use crate::error::ErrorAyni;
use crate::nodo::{AccionMembresia, Atestacion, CambioMembresia, Carga, Recibo};

/// La membresía VIGENTE de una conversación, derivada del grafo.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Membresia {
    /// El fundador: el autor del primer nodo en orden topológico. Miembro
    /// implícito y ancla de autoridad —no puede ser expulsado—. `None` sólo en
    /// una conversación vacía.
    pub fundador: Option<AgoraId>,
    /// El conjunto de identidades agora que son miembros ahora mismo. Incluye al
    /// fundador. Orden determinista por id (`BTreeSet`).
    pub miembros: BTreeSet<AgoraId>,
}

impl Membresia {
    /// ¿Es `id` miembro vigente?
    pub fn contiene(&self, id: &AgoraId) -> bool {
        self.miembros.contains(id)
    }

    /// Cuántos miembros hay.
    pub fn len(&self) -> usize {
        self.miembros.len()
    }

    /// ¿No hay nadie? (conversación vacía).
    pub fn esta_vacia(&self) -> bool {
        self.miembros.is_empty()
    }
}

impl Conversacion {
    /// El orden en que se pliegan los hechos sociales: topológico (los padres
    /// antes que los hijos) y determinista. Cae a orden de id sólo ante el
    /// imposible ciclo. Helper privado compartido por las derivaciones de P7.
    fn orden_para_plegado(&self) -> Vec<Hash> {
        self.orden_topologico()
            .unwrap_or_else(|| self.nodos().map(|(id, _)| *id).collect())
    }

    /// Deriva la MEMBRESÍA vigente plegando los nodos [`Carga::Membresia`] en
    /// orden topológico. El autor del primer nodo funda la conversación (miembro
    /// implícito); de ahí en adelante, sólo un miembro vigente puede dar de alta
    /// o de baja a alguien —un alta de un no-miembro se ignora en silencio—, y el
    /// fundador no puede ser expulsado. Es un cálculo puro sobre el grafo: dos
    /// pares con los mismos nodos obtienen la misma membresía, sin servidor.
    pub fn membresia(&self) -> Membresia {
        let mut miembros: BTreeSet<AgoraId> = BTreeSet::new();
        let mut fundador: Option<AgoraId> = None;

        for id in self.orden_para_plegado() {
            let Some(nodo) = self.obtener(&id) else {
                continue;
            };
            let autor = *nodo.autor();

            // El primer nodo del orden establece al fundador: miembro implícito.
            if fundador.is_none() {
                fundador = Some(autor);
                miembros.insert(autor);
            }

            if let Carga::Membresia(cm) = &nodo.contenido.carga {
                // Autoridad: sólo un miembro vigente cambia la membresía.
                if !miembros.contains(&autor) {
                    continue;
                }
                match cm.accion {
                    AccionMembresia::Alta => {
                        miembros.insert(cm.sujeto);
                    }
                    AccionMembresia::Baja => {
                        // El fundador es el ancla de autoridad: inexpulsable.
                        if Some(cm.sujeto) != fundador {
                            miembros.remove(&cm.sujeto);
                        }
                    }
                }
            }
        }

        Membresia { fundador, miembros }
    }

    /// El GRAFO DE CONFIANZA visto desde `observador`: a quién alcanza por
    /// caminos de atestaciones y a cuántos saltos (la distancia MÍNIMA). Una
    /// arista `autor → sujeto` existe si el autor atestiguó al sujeto con
    /// `nivel ≥ 1`; el último valor en orden topológico gana, y `nivel = 0`
    /// revoca (borra la arista). El `observador` no aparece en el resultado;
    /// cada sujeto alcanzable sí, con su número de saltos (`1` = directo).
    ///
    /// Es un recorrido en anchura: la confianza propagada es transitiva pero la
    /// distancia la conserva, de modo que la UX puede decidir cuánto pesa "a un
    /// salto" frente a "a tres". No hay autoridad: cada observador calcula SU
    /// grafo desde sus propias atestaciones.
    pub fn confianza_desde(&self, observador: &AgoraId) -> BTreeMap<AgoraId, u32> {
        // 1. Aristas dirigidas con el nivel vigente (último en topo-orden gana).
        let mut aristas: BTreeMap<AgoraId, BTreeMap<AgoraId, u8>> = BTreeMap::new();
        for id in self.orden_para_plegado() {
            let Some(nodo) = self.obtener(&id) else {
                continue;
            };
            if let Carga::Atestacion(at) = &nodo.contenido.carga {
                let salientes = aristas.entry(*nodo.autor()).or_default();
                if at.nivel == 0 {
                    salientes.remove(&at.sujeto);
                } else {
                    salientes.insert(at.sujeto, at.nivel);
                }
            }
        }

        // 2. BFS desde el observador, registrando la distancia mínima.
        let mut hops: BTreeMap<AgoraId, u32> = BTreeMap::new();
        let mut vistos: BTreeSet<AgoraId> = BTreeSet::new();
        vistos.insert(*observador);
        let mut frontera: Vec<AgoraId> = alloc::vec![*observador];
        let mut salto = 0u32;
        while !frontera.is_empty() {
            salto += 1;
            let mut siguiente: Vec<AgoraId> = Vec::new();
            for actual in frontera {
                if let Some(salientes) = aristas.get(&actual) {
                    for sujeto in salientes.keys() {
                        if vistos.insert(*sujeto) {
                            hops.insert(*sujeto, salto);
                            siguiente.push(*sujeto);
                        }
                    }
                }
            }
            frontera = siguiente;
        }
        hops
    }

    /// Quién ha ACUSADO RECIBO de qué: un mapa `id de mensaje → conjunto de
    /// autores que declararon haberlo visto`. Se lee de los nodos
    /// [`Carga::Recibo`]. Cada acuse va firmado, así que "visto por fulano" es
    /// verificable. La simetría (no acusar a quien no acusa) es política de la UX.
    pub fn recibos(&self) -> BTreeMap<Hash, BTreeSet<AgoraId>> {
        let mut mapa: BTreeMap<Hash, BTreeSet<AgoraId>> = BTreeMap::new();
        for (_, nodo) in self.nodos() {
            if let Carga::Recibo(r) = &nodo.contenido.carga {
                let autor = *nodo.autor();
                for visto in &r.vistos {
                    mapa.entry(*visto).or_default().insert(autor);
                }
            }
        }
        mapa
    }

    /// El conjunto de nodos cuyo recibo ha dado `autor` —lo que esa identidad ha
    /// declarado haber visto—. Útil para que la UX sepa qué resaltar como "leído
    /// por X" y para hacer cumplir la reciprocidad de los recibos.
    pub fn acuses_de(&self, autor: &AgoraId) -> BTreeSet<Hash> {
        let mut vistos: BTreeSet<Hash> = BTreeSet::new();
        for (_, nodo) in self.nodos() {
            if nodo.autor() == autor {
                if let Carga::Recibo(r) = &nodo.contenido.carga {
                    for v in &r.vistos {
                        vistos.insert(*v);
                    }
                }
            }
        }
        vistos
    }

    // --- Constructores de conveniencia (redactar + agregar), como
    //     `publicar_texto`. Toman las cabezas actuales como padres, sellan con el
    //     closure firmante y dejan el nodo en el grafo, devolviendo su id. ---

    /// Da de ALTA a `sujeto` como miembro. La autoridad la valida después
    /// [`membresia`](Self::membresia) —este método sólo redacta el hecho firmado—.
    pub fn admitir(
        &mut self,
        autor: AgoraId,
        sujeto: AgoraId,
        ts: u64,
        firmar: impl FnOnce(&Hash) -> Firma,
    ) -> Result<Hash, ErrorAyni> {
        let carga = Carga::Membresia(CambioMembresia {
            accion: AccionMembresia::Alta,
            sujeto,
        });
        let nodo = self.redactar(autor, carga, ts, firmar);
        self.agregar(nodo)
    }

    /// Da de BAJA a `sujeto`. (El fundador es inexpulsable: la baja se ignorará
    /// al derivar la membresía aunque el nodo exista.)
    pub fn expulsar(
        &mut self,
        autor: AgoraId,
        sujeto: AgoraId,
        ts: u64,
        firmar: impl FnOnce(&Hash) -> Firma,
    ) -> Result<Hash, ErrorAyni> {
        let carga = Carga::Membresia(CambioMembresia {
            accion: AccionMembresia::Baja,
            sujeto,
        });
        let nodo = self.redactar(autor, carga, ts, firmar);
        self.agregar(nodo)
    }

    /// ATESTIGUA a `sujeto` con `nivel` (`1..=255`; `0` revoca la atestación
    /// previa del mismo autor). Añade una arista firmada al grafo de confianza.
    pub fn atestar(
        &mut self,
        autor: AgoraId,
        sujeto: AgoraId,
        nivel: u8,
        ts: u64,
        firmar: impl FnOnce(&Hash) -> Firma,
    ) -> Result<Hash, ErrorAyni> {
        let nodo = self.redactar(autor, Carga::Atestacion(Atestacion { sujeto, nivel }), ts, firmar);
        self.agregar(nodo)
    }

    /// ACUSA RECIBO de `vistos` — declara firmadamente haber visto esos nodos.
    pub fn acusar_recibo(
        &mut self,
        autor: AgoraId,
        vistos: Vec<Hash>,
        ts: u64,
        firmar: impl FnOnce(&Hash) -> Firma,
    ) -> Result<Hash, ErrorAyni> {
        let nodo = self.redactar(autor, Carga::Recibo(Recibo { vistos }), ts, firmar);
        self.agregar(nodo)
    }
}
