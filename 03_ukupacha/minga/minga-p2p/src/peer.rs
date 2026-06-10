//! `MingaPeer`: API de alto nivel para un nodo Minga "always-on".
//!
//! Envuelve `LibP2pNode` con estado compartido (`Mst` + `MemStore` +
//! `AttestationStore` + `Keypair`) protegido por un `Mutex` async, y
//! expone:
//! - `run_passive_accept()`: lanza un bucle que acepta streams de
//!   sync continuamente, procesa cada uno en una task paralela, y
//!   mergea el resultado al estado compartido.
//! - `sync_with(peer_id)`: inicia un sync activo con un peer conocido.
//! - `snapshot()`: instantánea del estado actual.
//!
//! Modelo de concurrencia: cada sync entrante toma un *clone* del
//! estado, ejecuta la sesión sobre la copia, y al terminar mergea las
//! novedades al estado compartido. Múltiples syncs pueden correr en
//! paralelo; el merge final adquiere el lock brevemente. Eventualmente
//! consistente: un sync que empezó antes que un merge terminado puede
//! no ver esas novedades, pero el siguiente sync sí.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use futures::StreamExt;
use libp2p::{Multiaddr, PeerId, Stream};
use tokio::sync::Mutex;
use tokio_util::compat::FuturesAsyncReadCompatExt;

use minga_core::{
    alpha::hash_alpha_with, parse::Dialect, AttestationStore, ContentHash, Keypair, MemStore, Mst,
    NodeStore, RetractionStore, SemanticNode, StoredNode,
};
use minga_dht::DhtKey;
use minga_store::{PersistentRepo, SledNodeStore, StoreError};

use crate::async_driver::{run_sync_async, AsyncSyncError};
use crate::network::{DiscoveredPeer, LibP2pNode, NodeError, SYNC_PROTOCOL};
use crate::session::SyncSession;

#[derive(Debug, thiserror::Error)]
pub enum PeerSyncError {
    #[error("open stream: {0}")]
    OpenStream(#[from] libp2p_stream::OpenStreamError),

    #[error("sync: {0}")]
    AsyncSync(#[from] AsyncSyncError),
}

#[derive(Debug, thiserror::Error)]
pub enum PeerOpenError {
    #[error("network: {0}")]
    Network(#[from] NodeError),

    #[error("store: {0}")]
    Store(#[from] StoreError),
}

struct PeerState<S: NodeStore> {
    mst: Mst,
    store: S,
    attestations: AttestationStore,
    retractions: RetractionStore,
    /// Tabla local α-hash → (struct-hash, dialect). Se replica al
    /// disco si hay backing persistente (`repo.roots`); también se
    /// empuja al peer remoto durante sync vía `RootDeclaration`.
    roots: HashMap<ContentHash, (ContentHash, Dialect)>,
    keypair: Keypair,
    /// Backing persistente opcional. Si está presente, todo cambio
    /// de estado escribe a disco vía write-through.
    persistent: Option<Arc<PersistentRepo>>,
}

pub struct MingaPeer<S: NodeStore = MemStore> {
    /// El nodo libp2p envuelto en `Arc` para que otros consumidores
    /// (por ejemplo `agora_net_brahman::AgoraNet`) puedan compartir
    /// el mismo `BrahmanNet` — una sola identidad libp2p, una sola
    /// tabla Kademlia, dos protocolos de stream. Ver
    /// [`MingaPeer::brahman_net`] y [`MingaPeer::open_with_node`].
    node: Arc<LibP2pNode>,
    state: Arc<Mutex<PeerState<S>>>,
}

impl MingaPeer<MemStore> {
    pub fn new(
        keypair: Keypair,
        mst: Mst,
        store: MemStore,
        attestations: AttestationStore,
    ) -> Result<Self, NodeError> {
        let node = Arc::new(LibP2pNode::new()?);
        let state = Arc::new(Mutex::new(PeerState {
            mst,
            store,
            attestations,
            retractions: RetractionStore::new(),
            roots: HashMap::new(),
            keypair,
            persistent: None,
        }));
        Ok(Self { node, state })
    }
}

impl MingaPeer<SledNodeStore> {
    /// Abre o crea un peer persistente sobre `path`, construyendo un
    /// `LibP2pNode` propio (efímero). Atajo para procesos que sólo
    /// corren minga; si querés compartir el nodo libp2p con ágora u
    /// otro consumidor, usá [`MingaPeer::open_with_node`].
    pub fn open(keypair: Keypair, path: impl AsRef<Path>) -> Result<Self, PeerOpenError> {
        let node = Arc::new(LibP2pNode::new()?);
        Self::open_with_node(keypair, path, node)
    }

    /// Como [`MingaPeer::open`], pero adopta un `LibP2pNode` ya
    /// existente envuelto en `Arc`. Sirve para compartir un solo nodo
    /// libp2p entre minga y otros consumidores — un solo `PeerId`, una
    /// sola Kademlia, varios protocolos de stream coexistiendo. Esta
    /// es la convergencia que el README de ágora promete: *"agora
    /// corre sobre la red de pares de minga cuando ambos están
    /// activos"*.
    pub fn open_with_node(
        keypair: Keypair,
        path: impl AsRef<Path>,
        node: Arc<LibP2pNode>,
    ) -> Result<Self, PeerOpenError> {
        let repo = Arc::new(PersistentRepo::open(path)?);

        // Cargar MST desde disco.
        let mut mst = Mst::new();
        for r in repo.mst.iter() {
            mst.insert(r?);
        }

        // El store de nodos NO se vuelca a RAM: tomamos un handle
        // compartido al árbol sled de `repo.nodes` (clone barato,
        // `Arc`-backed). El sync opera directo sobre disco. Éste es el
        // corazón del refactor #5/A — un repo de 1,4M nodos arranca con
        // memoria O(1), no O(n).
        let store = repo.nodes.clone();

        // Cargar atestaciones desde disco.
        let mut attestations = AttestationStore::new();
        for r in repo.attestations.iter() {
            let att = r?;
            // `add` re-verifica criptográficamente. Lo persistido ya
            // estaba verificado, pero re-validar es cheap insurance.
            let _ = attestations.add(att);
        }

        // Cargar retracciones desde disco.
        let mut retractions = RetractionStore::new();
        for r in repo.retractions.iter() {
            let r = r?;
            let _ = retractions.add(r);
        }

        // Cargar raíces desde disco. `from_byte` puede devolver None
        // si el dialect persistido es de una versión futura — se
        // descarta esa entrada para no propagarla al wire (no
        // sabríamos verificarla del otro lado tampoco).
        let mut roots: HashMap<ContentHash, (ContentHash, Dialect)> = HashMap::new();
        for r in repo.roots.iter() {
            let (alpha, struct_hash, dialect) = r?;
            if let Some(d) = dialect {
                roots.insert(alpha, (struct_hash, d));
            }
        }

        let state = Arc::new(Mutex::new(PeerState {
            mst,
            store,
            attestations,
            retractions,
            roots,
            keypair,
            persistent: Some(repo),
        }));
        Ok(Self { node, state })
    }
}

impl<S: NodeStore + Clone + Send + 'static> MingaPeer<S> {
    /// Acceso al nodo libp2p subyacente — útil para que otro consumidor
    /// (típicamente `agora_net_brahman::AgoraNet`) abra sus propios
    /// sub-protocolos de stream sobre la misma malla brahman-net. Es
    /// el lado "lectura" de la convergencia; el lado "construcción" es
    /// [`MingaPeer::open_with_node`].
    pub fn brahman_net(&self) -> Arc<LibP2pNode> {
        Arc::clone(&self.node)
    }

    pub fn peer_id(&self) -> PeerId {
        self.node.peer_id
    }

    pub async fn listen(&self, addr: Multiaddr) -> Multiaddr {
        self.node.listen(addr).await
    }

    pub fn dial(&self, addr: Multiaddr) {
        self.node.dial(addr);
    }

    /// Añade un peer al routing table de Kademlia (bootstrap).
    pub fn add_dht_peer(&self, peer: PeerId, addr: Multiaddr) {
        self.node.add_dht_peer(peer, addr);
    }

    /// Consulta DHT por los peers más cercanos al `target`.
    pub async fn find_closest_peers(&self, target: PeerId) -> Vec<DiscoveredPeer> {
        self.node.find_closest_peers(target).await
    }

    /// Anuncia en el DHT que este peer provee el contenido `hash`.
    /// El record viaja con un byte de namespace (`RecordKind::Code`) que
    /// separa el keyspace de minga del de cards/personas sobre la misma
    /// malla Kademlia compartida (`brahman-net`).
    pub fn announce_provider(&self, hash: ContentHash) {
        let key = DhtKey::for_hash(minga_dht::RecordKind::Code, hash.0);
        self.node.start_providing(&key.to_bytes());
    }

    /// Anuncia en el DHT todas las raíces locales del peer (los
    /// α-hashes registrados en `roots`). Útil al arrancar un `listen`:
    /// el peer queda descubrible para cualquier otro que haga
    /// `find_providers(α)` sobre la malla Kademlia compartida. Devuelve
    /// la cantidad de raíces anunciadas — 0 si el repo está vacío o si
    /// nunca se ingirió con `ingest_with_dialect`.
    pub async fn announce_all_roots(&self) -> usize {
        let alphas: Vec<ContentHash> = {
            let s = self.state.lock().await;
            s.roots.keys().copied().collect()
        };
        let n = alphas.len();
        for alpha in alphas {
            let key = DhtKey::for_hash(minga_dht::RecordKind::Code, alpha.0);
            self.node.start_providing(&key.to_bytes());
        }
        n
    }

    /// Consulta el DHT por peers que han anunciado proveer este
    /// contenido. La clave usa el mismo namespace que [`announce_provider`].
    pub async fn find_providers(&self, hash: ContentHash) -> Vec<PeerId> {
        let key = DhtKey::for_hash(minga_dht::RecordKind::Code, hash.0);
        self.node.find_providers(&key.to_bytes()).await
    }

    /// Lanza el bucle de aceptación pasiva. Devuelve un `JoinHandle`
    /// que el caller puede mantener vivo (o ignorar — la task se
    /// aborta al cerrar el runtime).
    ///
    /// Cada stream entrante dispara un sync en una task aislada que
    /// trabaja sobre un clone del estado y mergea al final.
    pub fn run_passive_accept(&self) -> tokio::task::JoinHandle<()> {
        let mut control = self.node.control.clone();
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let mut incoming = control
                .accept(SYNC_PROTOCOL)
                .expect("only one accept handle per protocol");
            while let Some((_peer, stream)) = incoming.next().await {
                let state = Arc::clone(&state);
                tokio::spawn(handle_incoming(stream, state));
            }
        })
    }

    /// Inicia un sync activo con un peer del que ya tenemos conexión
    /// (vía `dial` previo). Toma un snapshot del estado, corre la
    /// sesión, y mergea novedades al volver.
    pub async fn sync_with(&self, peer_id: PeerId) -> Result<(), PeerSyncError> {
        let mut control = self.node.control.clone();
        let stream = control.open_stream(peer_id, SYNC_PROTOCOL).await?;
        let session = self.snapshot_session().await;
        let result = run_sync_async(session, stream.compat()).await?;
        self.merge_back(result).await;
        Ok(())
    }

    async fn snapshot_session(&self) -> SyncSession<S> {
        let s = self.state.lock().await;
        // `s.store.clone()` es O(1) cuando S = SledNodeStore (handle al
        // árbol en disco); O(n) sólo en el peer en memoria (MemStore),
        // donde el grafo entero ya está en RAM de todos modos.
        SyncSession::with_roots(
            s.mst.clone(),
            s.store.clone(),
            s.attestations.clone(),
            s.retractions.clone(),
            s.roots.clone(),
            s.keypair.clone(),
        )
    }

    async fn merge_back(&self, mut session: SyncSession<S>) {
        // Verifica α↔struct de las declaraciones recibidas ANTES de
        // mover la sesión: si el caller no llamó `take_verified_root_decls`
        // explícitamente, las raíces no pasarían a `verified_root_decls`
        // y se perderían en silencio.
        let _verified = session.take_verified_root_decls();
        let delta = session.drain_inserted();
        let (new_mst, new_store, new_atts, new_rets, new_roots) =
            session.into_parts_with_roots();
        let new_nodes = materialize_delta(&new_store, delta);
        let mut s = self.state.lock().await;
        merge_into_state(&mut s, new_mst, new_nodes, new_atts, new_rets, new_roots);
    }

    /// Instantánea del estado actual (mst + store + attestations). El
    /// `store` devuelto es un clone — handle compartido cuando es sled.
    pub async fn snapshot(&self) -> (Mst, S, AttestationStore) {
        let s = self.state.lock().await;
        (s.mst.clone(), s.store.clone(), s.attestations.clone())
    }

    /// Inserta un árbol directamente en el estado del peer (sin sync).
    /// Si el peer está respaldado por disco, también lo persiste.
    /// Anuncia automáticamente al peer como proveedor del contenido en
    /// el DHT — de esa forma cualquier otro peer puede descubrirlo
    /// preguntando "¿quién tiene este hash?".
    /// Devuelve el `ContentHash` raíz del árbol.
    pub async fn ingest(&self, node: &SemanticNode) -> ContentHash {
        let mut s = self.state.lock().await;
        // `s.store.put` persiste por sí mismo cuando S = SledNodeStore
        // (el store ES el árbol en disco); el MST sí necesita su propio
        // write-through al tree `mst` de `repo`.
        let h = s.store.put(node);
        s.mst.insert(h);
        if let Some(repo) = &s.persistent {
            let _ = repo.mst.insert(h);
        }
        drop(s);

        // Anunciamos como proveedores en el DHT con la clave typed
        // (kind = Code) — comparte malla con cards/personas sin colisión.
        let key = DhtKey::for_hash(minga_dht::RecordKind::Code, h.0);
        self.node.start_providing(&key.to_bytes());

        h
    }

    /// Variante de [`ingest`] que conoce el `dialect` del archivo y por
    /// tanto registra la raíz por su **α-hash** (estable bajo
    /// renombrado de variables ligadas), no por su hash estructural.
    /// Devuelve `(alpha_hash, struct_hash)`. Si el peer es persistente,
    /// también actualiza el tree `roots` y los timestamps.
    pub async fn ingest_with_dialect(
        &self,
        node: &SemanticNode,
        dialect: Dialect,
    ) -> (ContentHash, ContentHash) {
        let alpha = hash_alpha_with(dialect, node);
        let mut s = self.state.lock().await;
        let struct_hash = s.store.put(node);
        s.mst.insert(alpha);
        s.roots.insert(alpha, (struct_hash, dialect));
        if let Some(repo) = &s.persistent {
            let _ = repo.mst.insert(alpha);
            let _ = repo.roots.put(alpha, struct_hash, dialect);
        }
        drop(s);

        let key = DhtKey::for_hash(minga_dht::RecordKind::Code, alpha.0);
        self.node.start_providing(&key.to_bytes());

        (alpha, struct_hash)
    }

    /// Inserta una atestación en el peer. Si el peer es persistente,
    /// también la escribe a disco. Falla si la firma no verifica.
    pub async fn ingest_attestation(
        &self,
        att: minga_core::Attestation,
    ) -> Result<(), minga_core::AttestationError> {
        let mut s = self.state.lock().await;
        s.attestations.add(att.clone())?;
        if let Some(repo) = &s.persistent {
            let _ = repo.attestations.add(att);
        }
        Ok(())
    }

    /// Fuerza un flush del backing persistente a disco. No hace nada
    /// si el peer es solo en memoria.
    pub async fn flush(&self) -> Result<(), StoreError> {
        let s = self.state.lock().await;
        if let Some(repo) = &s.persistent {
            repo.flush()?;
        }
        Ok(())
    }
}

async fn handle_incoming<S: NodeStore + Clone + Send + 'static>(
    stream: Stream,
    state: Arc<Mutex<PeerState<S>>>,
) {
    let session = {
        let s = state.lock().await;
        SyncSession::with_roots(
            s.mst.clone(),
            s.store.clone(),
            s.attestations.clone(),
            s.retractions.clone(),
            s.roots.clone(),
            s.keypair.clone(),
        )
    };
    if let Ok(mut result) = run_sync_async(session, stream.compat()).await {
        let _verified = result.take_verified_root_decls();
        let delta = result.drain_inserted();
        let (new_mst, new_store, new_atts, new_rets, new_roots) =
            result.into_parts_with_roots();
        let new_nodes = materialize_delta(&new_store, delta);
        let mut s = state.lock().await;
        merge_into_state(&mut s, new_mst, new_nodes, new_atts, new_rets, new_roots);
    }
    // Errores de sync se ignoran: cada sesión es independiente, una
    // sesión rota no debería tumbar el peer entero. Una iteración
    // futura puede contar errores para telemetría.
}

/// Materializa el *delta* de hashes de la sesión a pares `(hash, nodo)`
/// leyéndolos del store de la sesión. O(delta), no O(n): nunca recorre el
/// store entero — el punto del refactor #5/A cuando el backend es sled.
fn materialize_delta<S: NodeStore>(
    store: &S,
    delta: Vec<ContentHash>,
) -> Vec<(ContentHash, StoredNode)> {
    delta
        .into_iter()
        .filter_map(|h| store.get(&h).map(|n| (h, n)))
        .collect()
}

fn merge_into_state<S: NodeStore>(
    state: &mut PeerState<S>,
    new_mst: Mst,
    new_nodes: Vec<(ContentHash, StoredNode)>,
    new_atts: AttestationStore,
    new_rets: RetractionStore,
    new_roots: HashMap<ContentHash, (ContentHash, Dialect)>,
) {
    // Write-through: cada inserción en memoria también va al backing
    // persistente si existe. Errores de IO se ignoran (best-effort);
    // el estado en memoria sigue siendo la fuente de verdad inmediata
    // y un siguiente sync re-popula lo que se haya perdido.
    for h in new_mst.iter() {
        state.mst.insert(*h);
        if let Some(repo) = &state.persistent {
            let _ = repo.mst.insert(*h);
        }
    }
    // Sólo el *delta* de nodos nuevos del sync (no el store entero).
    // `state.store.put_chunked` persiste por sí mismo cuando es sled (el
    // store ES el árbol en disco — mismo handle que `repo.nodes`); en el
    // peer en memoria simplemente copia el nodo al MemStore. En ningún
    // caso se recorre O(n) el grafo completo.
    for (h, node) in new_nodes {
        state.store.put_chunked(h, node);
    }
    for att in new_atts.all() {
        if state.attestations.add(att.clone()).is_ok() {
            // Solo persistimos las que pasaron verificación en memoria.
            if let Some(repo) = &state.persistent {
                let _ = repo.attestations.add(att.clone());
            }
        }
    }
    for r in new_rets.all() {
        if state.retractions.add(r.clone()).is_ok() {
            if let Some(repo) = &state.persistent {
                let _ = repo.retractions.add(r.clone());
            }
        }
    }
    // Raíces ya verificadas (α↔struct↔dialect): la fuente local es
    // autoritativa, así que sólo insertamos las que no conocemos
    // todavía. La verificación criptográfica ya pasó en la sesión.
    for (alpha, (struct_hash, dialect)) in new_roots {
        if state.roots.contains_key(&alpha) {
            continue;
        }
        state.roots.insert(alpha, (struct_hash, dialect));
        if let Some(repo) = &state.persistent {
            let _ = repo.roots.put(alpha, struct_hash, dialect);
        }
    }
}
