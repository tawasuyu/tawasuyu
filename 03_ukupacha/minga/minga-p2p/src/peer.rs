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

use std::path::Path;
use std::sync::Arc;

use futures::StreamExt;
use libp2p::{Multiaddr, PeerId, Stream};
use tokio::sync::Mutex;
use tokio_util::compat::FuturesAsyncReadCompatExt;

use minga_core::{AttestationStore, ContentHash, Keypair, MemStore, Mst, NodeStore, SemanticNode};
use minga_store::{PersistentRepo, StoreError};

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

struct PeerState {
    mst: Mst,
    store: MemStore,
    attestations: AttestationStore,
    keypair: Keypair,
    /// Backing persistente opcional. Si está presente, todo cambio
    /// de estado escribe a disco vía write-through.
    persistent: Option<Arc<PersistentRepo>>,
}

pub struct MingaPeer {
    node: LibP2pNode,
    state: Arc<Mutex<PeerState>>,
}

impl MingaPeer {
    pub fn new(
        keypair: Keypair,
        mst: Mst,
        store: MemStore,
        attestations: AttestationStore,
    ) -> Result<Self, NodeError> {
        let node = LibP2pNode::new()?;
        let state = Arc::new(Mutex::new(PeerState {
            mst,
            store,
            attestations,
            keypair,
            persistent: None,
        }));
        Ok(Self { node, state })
    }

    /// Abre o crea un peer persistente sobre `path`. Si el directorio
    /// no contiene un repo, se crea vacío. Si lo contiene, se carga
    /// el estado completo (MST, nodos, atestaciones) en memoria.
    /// Cualquier cambio posterior se escribe a disco vía write-through.
    pub fn open(keypair: Keypair, path: impl AsRef<Path>) -> Result<Self, PeerOpenError> {
        let repo = Arc::new(PersistentRepo::open(path)?);

        // Cargar MST desde disco.
        let mut mst = Mst::new();
        for r in repo.mst.iter() {
            mst.insert(r?);
        }

        // Cargar nodos desde disco.
        let mut store = MemStore::new();
        for r in repo.nodes.iter() {
            let (h, node) = r?;
            store.put_chunked(h, node);
        }

        // Cargar atestaciones desde disco.
        let mut attestations = AttestationStore::new();
        for r in repo.attestations.iter() {
            let att = r?;
            // `add` re-verifica criptográficamente. Lo persistido ya
            // estaba verificado, pero re-validar es cheap insurance.
            let _ = attestations.add(att);
        }

        let node = LibP2pNode::new()?;
        let state = Arc::new(Mutex::new(PeerState {
            mst,
            store,
            attestations,
            keypair,
            persistent: Some(repo),
        }));
        Ok(Self { node, state })
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
    /// Otros peers podrán descubrirlo vía `find_providers(hash)`.
    pub fn announce_provider(&self, hash: ContentHash) {
        self.node.start_providing(&hash.0);
    }

    /// Consulta el DHT por peers que han anunciado proveer este
    /// contenido. La unión de los `PeerId`s permite a quien busque
    /// `hash` decidir a quién dial directamente para sincronizar.
    pub async fn find_providers(&self, hash: ContentHash) -> Vec<PeerId> {
        self.node.find_providers(&hash.0).await
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

    async fn snapshot_session(&self) -> SyncSession {
        let s = self.state.lock().await;
        SyncSession::new(
            s.mst.clone(),
            s.store.clone(),
            s.attestations.clone(),
            s.keypair.clone(),
        )
    }

    async fn merge_back(&self, session: SyncSession) {
        let (new_mst, new_store, new_atts) = session.into_parts();
        let mut s = self.state.lock().await;
        merge_into_state(&mut s, new_mst, new_store, new_atts);
    }

    /// Instantánea del estado actual (mst + store + attestations).
    pub async fn snapshot(&self) -> (Mst, MemStore, AttestationStore) {
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
        let h = s.store.put(node);
        s.mst.insert(h);
        if let Some(repo) = &s.persistent {
            let _ = repo.nodes.put(node);
            let _ = repo.mst.insert(h);
        }
        drop(s);

        // Anunciamos como proveedores en el DHT. Best-effort: si no
        // hay peers cercanos para replicar, el record vive local hasta
        // que llegue una conexión.
        self.node.start_providing(&h.0);

        h
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

async fn handle_incoming(stream: Stream, state: Arc<Mutex<PeerState>>) {
    let session = {
        let s = state.lock().await;
        SyncSession::new(
            s.mst.clone(),
            s.store.clone(),
            s.attestations.clone(),
            s.keypair.clone(),
        )
    };
    if let Ok(result) = run_sync_async(session, stream.compat()).await {
        let (new_mst, new_store, new_atts) = result.into_parts();
        let mut s = state.lock().await;
        merge_into_state(&mut s, new_mst, new_store, new_atts);
    }
    // Errores de sync se ignoran: cada sesión es independiente, una
    // sesión rota no debería tumbar el peer entero. Una iteración
    // futura puede contar errores para telemetría.
}

fn merge_into_state(
    state: &mut PeerState,
    new_mst: Mst,
    new_store: MemStore,
    new_atts: AttestationStore,
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
    for (h, node) in new_store.iter() {
        state.store.put_chunked(*h, node.clone());
        if let Some(repo) = &state.persistent {
            let _ = repo.nodes.put_chunked(*h, node);
        }
    }
    for att in new_atts.all() {
        if state.attestations.add(att.clone()).is_ok() {
            // Solo persistimos las que pasaron verificación en memoria.
            if let Some(repo) = &state.persistent {
                let _ = repo.attestations.add(att.clone());
            }
        }
    }
}
