//! Máquina de estados de sincronización recursiva sobre la estructura
//! del MST, con verificación criptográfica de cada nodo entregado.
//!
//! La sesión es **pura**: no hace IO, no toca la red, no usa async. El
//! transporte la alimenta vía `handle(msg)` y consume sus salidas como
//! `Vec<Message>`.
//!
//! ## Algoritmo
//!
//! 1. Cada peer construye al inicio un `own_probes: HashMap<ContentHash,
//!    NodeProbe>` que indexa cada nodo interno de su MST por su hash
//!    Merkle de subárbol. Es la tabla con la que respondemos
//!    `ProbeReq`s en `O(1)`.
//!
//! 2. Cada peer envía `Hello` con el hash de su raíz. Si el peer
//!    contrario reconoce ese hash en su propio `own_probes` (o coincide
//!    con su propia raíz, o es la raíz vacía), no hay nada estructural
//!    que descubrir — la rama está ya alineada.
//!
//! 3. Si el hash no se reconoce, el peer emite un `ProbeReq` para
//!    pedirle al otro la estructura de ese subárbol. Cuando llega el
//!    `ProbeRes`, el peer:
//!    - Para cada **clave** del probe que no tiene en su MST, programa
//!      un `Fetch` (la clave entrará al MST cuando llegue su `Deliver`).
//!    - Para cada **child_hash** del probe que no aparece en
//!      `own_probes`, recurre con un nuevo `ProbeReq`. Si el child_hash
//!      ya está en `own_probes`, la rama se poda — toda esa subestructura
//!      es idéntica a la nuestra.
//!
//! 4. Cuando un peer recibe un `Deliver`, verifica que el hash
//!    anunciado coincida con el `hash_stored` real del nodo. Si no,
//!    descarta. Si sí, inserta en el `MemStore` y, si el hash venía de
//!    la raíz del MST del peer (no de un descendiente), también lo
//!    inserta en su MST.
//!
//! 5. Cada `StoredNode` recibido contiene los hashes de sus hijos. Si
//!    el receptor no los tiene, los pide vía `Fetch` (sync transitivo).
//!
//! 6. Un peer envía `Done` cuando: emitió y recibió `Hello`, no tiene
//!    probes pendientes, ni fetches pendientes (raíz o hijo). La sesión
//!    cierra cuando ambos `Done`s han cruzado.

use minga_core::{
    cas::ContentHash, empty_subtree_hash, hash_stored, AttestationStore, Did, Keypair, MemStore,
    Mst, NodeProbe, NodeStore, RetractionStore,
};
use rand::rngs::OsRng;
use rand::RngCore;
use std::collections::{HashMap, HashSet};

use crate::message::Message;

/// Construye el payload firmado del `Hello` con orden fijo:
/// `verifier_nonce(32) || peer_did(32) || root_subtree_hash(32) = 96 bytes`.
/// El `verifier_nonce` es el nonce que emitió el peer que verificará
/// la firma; al firmar sobre él se vincula la firma a esta sesión.
/// Cualquier cambio al format es incompatible al protocolo.
pub(crate) fn hello_payload(
    verifier_nonce: &[u8; 32],
    did: &Did,
    root: &ContentHash,
) -> [u8; 96] {
    let mut p = [0u8; 96];
    p[..32].copy_from_slice(verifier_nonce);
    p[32..64].copy_from_slice(&did.0);
    p[64..].copy_from_slice(&root.0);
    p
}

pub struct SyncSession {
    mst: Mst,
    store: MemStore,
    attestations: AttestationStore,
    retractions: RetractionStore,

    /// Llave del peer local: firma el `Hello` y queda asociada al
    /// `Did` que el peer remoto verá.
    keypair: Keypair,

    /// Identidad del peer remoto, capturada tras verificar la firma
    /// de su `Hello`.
    peer_did: Option<Did>,

    own_probes: HashMap<ContentHash, NodeProbe>,
    own_root_subtree_hash: ContentHash,

    awaited_probes: HashSet<ContentHash>,
    seen_probes: HashSet<ContentHash>,
    awaiting_root: HashSet<ContentHash>,
    awaiting_child: HashSet<ContentHash>,

    rejected_hellos: usize,
    rejected_delivers: usize,
    /// Contador de atestaciones rechazadas: firma rota, llegada antes
    /// de autenticar al peer, o cualquier otra inconsistencia que el
    /// `AttestationStore` rechace.
    rejected_attests: usize,
    /// Contador análogo para retracciones rechazadas.
    rejected_retracts: usize,

    /// Nonce aleatorio que **nosotros** emitimos en `Challenge`. La
    /// firma del `Hello` del peer debe ser sobre este nonce.
    self_nonce: [u8; 32],
    /// Nonce que el peer publicó en su `Challenge` — sobre este
    /// nonce firmamos nosotros nuestro `Hello`.
    peer_nonce: Option<[u8; 32]>,

    sent_challenge: bool,
    received_challenge: bool,
    sent_hello: bool,
    received_hello: bool,
    sent_attestations: bool,
    sent_retractions: bool,
    sent_done: bool,
    received_done: bool,
}

impl SyncSession {
    /// Constructor sin retracciones — el chasis lo usa cuando no hay
    /// retracciones que sincronizar (o por compat con tests viejos).
    pub fn new(
        mst: Mst,
        store: MemStore,
        attestations: AttestationStore,
        keypair: Keypair,
    ) -> Self {
        Self::with_retractions(mst, store, attestations, RetractionStore::new(), keypair)
    }

    /// Constructor completo con retracciones. Las retracciones se
    /// empujan al peer junto con las atestaciones (en su propio
    /// mensaje `RetractPush`).
    pub fn with_retractions(
        mst: Mst,
        store: MemStore,
        attestations: AttestationStore,
        retractions: RetractionStore,
        keypair: Keypair,
    ) -> Self {
        let own_probes = mst.build_probe_index();
        let own_root_subtree_hash = mst.root_hash();
        let mut self_nonce = [0u8; 32];
        OsRng.fill_bytes(&mut self_nonce);
        Self {
            mst,
            store,
            attestations,
            retractions,
            keypair,
            peer_did: None,
            own_probes,
            own_root_subtree_hash,
            awaited_probes: HashSet::new(),
            seen_probes: HashSet::new(),
            awaiting_root: HashSet::new(),
            awaiting_child: HashSet::new(),
            rejected_hellos: 0,
            rejected_delivers: 0,
            rejected_attests: 0,
            rejected_retracts: 0,
            self_nonce,
            peer_nonce: None,
            sent_challenge: false,
            received_challenge: false,
            sent_hello: false,
            received_hello: false,
            sent_attestations: false,
            sent_retractions: false,
            sent_done: false,
            received_done: false,
        }
    }

    /// Conveniencia para sesiones sin atestaciones previas. Equivalente
    /// a `new(mst, store, AttestationStore::new(), keypair)`.
    pub fn without_attestations(mst: Mst, store: MemStore, keypair: Keypair) -> Self {
        Self::new(mst, store, AttestationStore::new(), keypair)
    }

    /// Mensaje inicial: `Challenge` con un nonce aleatorio. El `Hello`
    /// y las atestaciones llegarán como respuesta al `Challenge` del
    /// otro peer (cuando lo recibamos, ya tendremos su nonce sobre el
    /// que firmar nuestra identidad).
    pub fn start(&mut self) -> Vec<Message> {
        if self.sent_challenge {
            return Vec::new();
        }
        self.sent_challenge = true;
        let mut out = vec![Message::Challenge {
            nonce: self.self_nonce,
        }];
        out.extend(self.maybe_done());
        out
    }

    pub fn handle(&mut self, msg: Message) -> Vec<Message> {
        let mut out = Vec::new();
        match msg {
            Message::Challenge { nonce } => {
                if self.received_challenge {
                    // Challenge duplicado: ignoramos. Un peer
                    // legítimo no debería enviar dos.
                    return out;
                }
                self.received_challenge = true;
                self.peer_nonce = Some(nonce);

                // Ahora podemos firmar nuestro Hello sobre el nonce
                // del peer — lo que ata la firma a esta sesión.
                let payload =
                    hello_payload(&nonce, &self.keypair.did(), &self.own_root_subtree_hash);
                let signature = self.keypair.sign(&payload);
                self.sent_hello = true;
                out.push(Message::Hello {
                    peer_did: self.keypair.did(),
                    root_subtree_hash: self.own_root_subtree_hash,
                    signature,
                });

                // Empuje de atestaciones: el peer ya nos verificará
                // como remitente cuando reciba nuestro Hello.
                let atts: Vec<_> = self.attestations.all().cloned().collect();
                if !atts.is_empty() {
                    out.push(Message::AttestPush { attestations: atts });
                }
                self.sent_attestations = true;

                // Y de retracciones: análogo a AttestPush pero con
                // las retracciones que conocemos.
                let rets: Vec<_> = self.retractions.all().cloned().collect();
                if !rets.is_empty() {
                    out.push(Message::RetractPush { retractions: rets });
                }
                self.sent_retractions = true;
            }

            Message::Hello {
                peer_did,
                root_subtree_hash,
                signature,
            } => {
                // ── Autenticación del peer + anti-replay ─────────
                // La firma debe ser sobre nuestro `self_nonce` (que
                // emitimos en nuestro Challenge), atándola a esta
                // sesión. Un Hello capturado de otra sesión tendría
                // un nonce distinto y la verificación fallaría.
                let payload = hello_payload(&self.self_nonce, &peer_did, &root_subtree_hash);
                if !peer_did.verify(&payload, &signature) {
                    self.rejected_hellos += 1;
                    return out;
                }
                self.peer_did = Some(peer_did);
                self.received_hello = true;
                if self.should_probe(&root_subtree_hash) {
                    self.awaited_probes.insert(root_subtree_hash);
                    out.push(Message::ProbeReq {
                        subtree_hash: root_subtree_hash,
                    });
                }
            }

            Message::ProbeReq { subtree_hash } => {
                let probe = self.own_probes.get(&subtree_hash).cloned();
                // Si el subárbol pedido era vacío (o desconocido para
                // nosotros), respondemos con `None` — el peer lo
                // tratará como un punto sin descendientes que descubrir.
                out.push(Message::ProbeRes {
                    subtree_hash,
                    probe,
                });
            }

            Message::ProbeRes {
                subtree_hash,
                probe,
            } => {
                self.awaited_probes.remove(&subtree_hash);
                self.seen_probes.insert(subtree_hash);
                if let Some(probe) = probe {
                    out.extend(self.process_probe(&probe));
                }
            }

            Message::Fetch { hash } => {
                if let Some(stored) = self.store.get(&hash).cloned() {
                    out.push(Message::Deliver { hash, stored });
                }
                // Si no lo tenemos, callamos. El peer no debería estar
                // pidiéndonos algo que no le hayamos anunciado.
            }

            Message::Deliver { hash, stored } => {
                // ── Verificación criptográfica ────────────────────
                // Recomputamos el hash del nodo entregado a partir de
                // sus componentes. Si no coincide con el anunciado,
                // alguien (peer malicioso o ruido en transporte) está
                // intentando colar contenido distinto bajo un hash que
                // no le corresponde. Descartamos silenciosamente y
                // contamos para diagnóstico.
                if hash_stored(&stored) != hash {
                    self.rejected_delivers += 1;
                    // No tocamos awaiting_*: la solicitud sigue
                    // pendiente y el peer (legítimo o no) puede
                    // reintentarla.
                    return out;
                }

                let was_root = self.awaiting_root.remove(&hash);
                self.awaiting_child.remove(&hash);

                // Antes de mover `stored`, descubrimos qué hijos
                // faltan y los pedimos.
                let mut new_fetches = Vec::new();
                for ch in &stored.children {
                    if !self.store.contains(ch)
                        && !self.awaiting_root.contains(ch)
                        && !self.awaiting_child.contains(ch)
                    {
                        self.awaiting_child.insert(*ch);
                        new_fetches.push(*ch);
                    }
                }

                self.store.put_chunked(hash, stored);
                if was_root {
                    self.mst.insert(hash);
                }

                for h in new_fetches {
                    out.push(Message::Fetch { hash: h });
                }
            }

            Message::AttestPush { attestations } => {
                // Antes de procesar atestaciones del peer, exigimos
                // haber autenticado su identidad. Un push antes del
                // `Hello` es protocolo malformado o ataque — todas las
                // atestaciones se cuentan como rechazadas.
                if !self.received_hello {
                    self.rejected_attests += attestations.len();
                    return out;
                }
                for att in attestations {
                    // `AttestationStore::add` re-verifica cada firma.
                    // Una sola atestación corrupta no contamina las
                    // demás del lote.
                    if self.attestations.add(att).is_err() {
                        self.rejected_attests += 1;
                    }
                }
            }

            Message::RetractPush { retractions } => {
                // Mismo contrato que AttestPush: exigimos Hello previo.
                if !self.received_hello {
                    self.rejected_retracts += retractions.len();
                    return out;
                }
                for r in retractions {
                    if self.retractions.add(r).is_err() {
                        self.rejected_retracts += 1;
                    }
                }
            }

            Message::Done => {
                self.received_done = true;
            }
        }
        out.extend(self.maybe_done());
        out
    }

    fn process_probe(&mut self, probe: &NodeProbe) -> Vec<Message> {
        let mut out = Vec::new();

        // Cada clave del probe que no tenemos pasa a `awaiting_root` y
        // generamos un Fetch. Si ya está en el store (sin estar aún en
        // el MST), simplemente la promovemos al MST sin pedirla.
        for k in &probe.keys {
            if self.mst.contains(k) {
                continue;
            }
            if self.store.contains(k) {
                self.mst.insert(*k);
                continue;
            }
            if self.awaiting_root.contains(k) {
                continue;
            }
            self.awaiting_root.insert(*k);
            out.push(Message::Fetch { hash: *k });
        }

        // Para cada subárbol hijo, decidimos si recurrir o podar:
        //   - el vacío se reconoce por hash sin red,
        //   - los que ya tenemos en `own_probes` (igualdad de hash =
        //     subestructura idéntica) se podan,
        //   - los ya vistos o solicitados no se duplican,
        //   - el resto dispara un `ProbeReq` recursivo.
        for ch in &probe.child_hashes {
            if self.should_probe(ch) {
                self.awaited_probes.insert(*ch);
                out.push(Message::ProbeReq { subtree_hash: *ch });
            }
        }

        out
    }

    /// Decide si vale la pena solicitar un probe sobre `h`. Cuatro
    /// razones para NO pedirlo:
    /// - es el subárbol vacío (lo conocemos por convención),
    /// - coincide con nuestra propia raíz (igualdad estructural),
    /// - aparece en `own_probes` (ya tenemos un subárbol idéntico),
    /// - ya lo solicitamos o ya lo recibimos.
    fn should_probe(&self, h: &ContentHash) -> bool {
        if *h == empty_subtree_hash() {
            return false;
        }
        if *h == self.own_root_subtree_hash {
            return false;
        }
        if self.own_probes.contains_key(h) {
            return false;
        }
        if self.awaited_probes.contains(h) || self.seen_probes.contains(h) {
            return false;
        }
        true
    }

    fn maybe_done(&mut self) -> Vec<Message> {
        if self.sent_done {
            return Vec::new();
        }
        if !self.sent_challenge || !self.received_challenge {
            return Vec::new();
        }
        if !self.sent_hello || !self.received_hello {
            return Vec::new();
        }
        if !self.sent_attestations || !self.sent_retractions {
            return Vec::new();
        }
        if !self.awaited_probes.is_empty() {
            return Vec::new();
        }
        if !self.awaiting_root.is_empty() || !self.awaiting_child.is_empty() {
            return Vec::new();
        }
        self.sent_done = true;
        vec![Message::Done]
    }

    pub fn is_done(&self) -> bool {
        self.sent_done && self.received_done
    }

    pub fn rejected_delivers(&self) -> usize {
        self.rejected_delivers
    }

    pub fn rejected_hellos(&self) -> usize {
        self.rejected_hellos
    }

    pub fn rejected_attests(&self) -> usize {
        self.rejected_attests
    }

    pub fn rejected_retracts(&self) -> usize {
        self.rejected_retracts
    }

    /// `true` si la sesión ya verificó el `Hello` del peer remoto.
    /// Útil para tests que necesitan saber cuándo es seguro inyectar
    /// `AttestPush`/`RetractPush` (que requieren `received_hello`).
    pub fn received_hello(&self) -> bool {
        self.received_hello
    }

    pub fn attestations(&self) -> &AttestationStore {
        &self.attestations
    }

    pub fn retractions(&self) -> &RetractionStore {
        &self.retractions
    }

    /// Identidad del peer remoto, capturada tras verificar su `Hello`.
    /// `None` si todavía no llegó un `Hello` válido.
    pub fn peer_did(&self) -> Option<Did> {
        self.peer_did
    }

    pub fn local_did(&self) -> Did {
        self.keypair.did()
    }

    /// Nonce aleatorio que esta sesión emitió en su `Challenge`.
    /// Expuesto principalmente para tests y debugging — el nonce
    /// viaja en claro por el wire y no es secreto.
    pub fn self_nonce(&self) -> [u8; 32] {
        self.self_nonce
    }

    pub fn mst(&self) -> &Mst {
        &self.mst
    }

    pub fn store(&self) -> &MemStore {
        &self.store
    }

    pub fn into_parts(self) -> (Mst, MemStore, AttestationStore) {
        (self.mst, self.store, self.attestations)
    }

    /// Variante de [`into_parts`] que también devuelve las retracciones.
    /// Pensada para callers que necesitan mezclar `RetractPush`es
    /// recibidos en su estado persistente.
    pub fn into_parts_with_retractions(
        self,
    ) -> (Mst, MemStore, AttestationStore, RetractionStore) {
        (self.mst, self.store, self.attestations, self.retractions)
    }
}
