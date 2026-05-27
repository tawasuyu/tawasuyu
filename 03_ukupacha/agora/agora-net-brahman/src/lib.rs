//! `agora-net-brahman` — bridge entre [`agora_gossip`] y la malla
//! brahman-net.
//!
//! Cierra la promesa del README de ágora: *"agora corre sobre la red de
//! pares de minga cuando ambos están activos"*. Hace dos cosas:
//!
//! 1. Registra el sub-protocolo de stream `/agora/gossip/1.0.0` sobre
//!    [`card_net::BrahmanNet`]. El mismo nodo libp2p que `MingaPeer`
//!    abre para `/minga/sync/1.0.0` sirve también para gossip de ágora
//!    — un solo socket, un solo DHT, dos protocolos.
//! 2. Hace la cinta transportadora entre los `Message`s puros de
//!    `agora-gossip` (Announce/Request/Bundle) y bytes length-prefixed
//!    en la stream libp2p (`u32 LE len || postcard`, idéntico al
//!    framing de Minga).
//!
//! ## Modelo
//!
//! [`AgoraNet`] envuelve un `BrahmanNet` y un `TrustGraph` compartido
//! detrás de un `Mutex` async. La crate ofrece dos constructores:
//!
//! - [`AgoraNet::standalone`] crea un `BrahmanNet` dedicado — útil para
//!   procesos que sólo corren ágora.
//! - [`AgoraNet::sharing`] reutiliza un `BrahmanNet` ya abierto por
//!   otro consumidor (típicamente la `MingaPeer`). Es la convergencia
//!   propiamente dicha: una sola identidad libp2p, una sola tabla
//!   Kademlia, dos protocolos hablando con los mismos pares.
//!
//! ## Protocolo en el cable
//!
//! Una ronda de gossip es **PUSH desde el iniciador**:
//!
//! 1. A abre stream, escribe `Announce(haves_A)`.
//! 2. B lee el announce, computa `haves_A - haves_B` (lo que B no
//!    tiene), escribe `Request(missing)`. Si no falta nada, no
//!    responde — la stream se cierra.
//! 3. A lee `Request`, busca esas atestaciones, escribe `Bundle(...)`.
//! 4. B lee `Bundle`, mergea al grafo (cada atestación re-verifica
//!    firma en `TrustGraph::add_attestation` — bundles falsos se
//!    cuentan en `bundles_recibidos_rechazados`).
//!
//! Para sync bidireccional cada lado ejecuta su propio `gossip_with`.
//! El protocolo no acopla los sentidos — más simple, más auditable, y
//! permite que sólo uno de los dos tenga conectividad saliente.

#![forbid(unsafe_code)]

use std::sync::Arc;

use agora_core::Attestation;
use agora_gossip::{al_recibir_announce, al_recibir_bundle, al_recibir_request, Digest, GossipStats, Message};
use agora_graph::TrustGraph;
use card_net::{BrahmanNet, Multiaddr, NodeError, PeerId};
use futures::StreamExt;
use libp2p::{Stream, StreamProtocol};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_util::compat::FuturesAsyncReadCompatExt;

/// Sub-protocolo libp2p para gossip de ágora sobre brahman-net.
/// Convive en el mismo nodo con `/minga/sync/1.0.0` sin colisión.
pub const GOSSIP_PROTOCOL: StreamProtocol = StreamProtocol::new("/agora/gossip/1.0.0");

/// Cota dura del tamaño de un frame (16 MB) — protege al receptor
/// contra peers maliciosos o bugs que intenten allocar gigas. Igual
/// al `MAX_FRAME_SIZE` de minga.
const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum AgoraNetError {
    #[error("network: {0}")]
    Network(#[from] NodeError),

    #[error("open stream: {0}")]
    OpenStream(#[from] libp2p_stream::OpenStreamError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("postcard decode: {0}")]
    Postcard(#[from] postcard::Error),

    #[error("frame demasiado grande: {0} bytes")]
    FrameTooLarge(u32),

    #[error("el par cerró antes del primer mensaje")]
    StreamClosedEarly,
}

/// Resultado de una ronda de gossip activo desde la perspectiva del
/// iniciador (que está **mandando** novedades al peer).
#[derive(Debug, Clone, Copy, Default)]
pub struct PushStats {
    /// Atestaciones que el peer pidió y se le mandaron.
    pub bundles_enviados: usize,
    /// Atestaciones que el peer pidió y NO teníamos (raro: el peer
    /// pidió hashes que estaban en nuestro propio Announce — bug del
    /// peer o race con un GC local). Las contamos para tracing.
    pub requests_sin_match: usize,
    /// `true` si el peer no pidió nada (ya estaba al día o más
    /// adelantado). En ese caso `bundles_enviados == 0`.
    pub peer_al_dia: bool,
}

impl From<GossipStats> for PushStats {
    fn from(g: GossipStats) -> Self {
        Self {
            bundles_enviados: g.requests_atendidos,
            requests_sin_match: g.requests_sin_match,
            peer_al_dia: g.requests_atendidos == 0 && g.requests_sin_match == 0,
        }
    }
}

/// Bridge entre `agora-gossip` y `BrahmanNet`.
pub struct AgoraNet {
    net: Arc<BrahmanNet>,
    graph: Arc<Mutex<TrustGraph>>,
}

impl AgoraNet {
    /// Crea un `AgoraNet` que comparte un `BrahmanNet` ya existente —
    /// típicamente el devuelto por [`MingaPeer::brahman_net`], para
    /// que minga y ágora hablen con el mismo nodo libp2p (un sólo
    /// `PeerId`, una sola Kademlia, dos sub-protocolos de stream).
    /// Esta es la convergencia propiamente dicha del README.
    ///
    /// El `graph` queda envuelto en un `Arc<Mutex<...>>` interno;
    /// usá [`AgoraNet::snapshot`] para leerlo desde otros hilos.
    pub fn sharing(net: Arc<BrahmanNet>, graph: TrustGraph) -> Self {
        Self {
            net,
            graph: Arc::new(Mutex::new(graph)),
        }
    }

    /// Crea un `AgoraNet` con `BrahmanNet` propio (no compartido).
    /// Útil cuando el proceso sólo corre ágora.
    pub fn standalone(graph: TrustGraph) -> Result<Self, AgoraNetError> {
        let net = Arc::new(BrahmanNet::new()?);
        Ok(Self::sharing(net, graph))
    }

    pub fn peer_id(&self) -> PeerId {
        self.net.peer_id
    }

    /// Adopta una multiaddr a escuchar. Devuelve la dirección final
    /// (con puerto resuelto si pediste `/tcp/0`).
    pub async fn listen(&self, addr: Multiaddr) -> Multiaddr {
        self.net.listen(addr).await
    }

    /// Dispara un dial al peer; la conexión sucede en la swarm task.
    pub fn dial(&self, addr: Multiaddr) {
        self.net.dial(addr);
    }

    /// Agrega un peer al routing table de Kademlia.
    pub fn add_dht_peer(&self, peer: PeerId, addr: Multiaddr) {
        self.net.add_dht_peer(peer, addr);
    }

    /// Snapshot inmutable del grafo local. Útil para UIs que pintan
    /// el estado actual sin bloquear el accept loop.
    pub async fn snapshot(&self) -> TrustGraph {
        self.graph.lock().await.clone()
    }

    /// Acceso al grafo compartido — el caller que quiera mutarlo
    /// directamente (registrar identidades, agregar atestaciones
    /// firmadas localmente) puede tomar este `Arc` y hacer
    /// `graph.lock().await`.
    pub fn graph(&self) -> Arc<Mutex<TrustGraph>> {
        Arc::clone(&self.graph)
    }

    /// Lanza una task que acepta streams del protocolo gossip y
    /// atiende cada uno en paralelo. La task vive hasta que el
    /// `BrahmanNet` se apague — devolvé el `JoinHandle` para
    /// cancelarla en shutdown explícito si hace falta.
    pub fn run_passive_accept(&self) -> tokio::task::JoinHandle<()> {
        let mut control = self.net.control.clone();
        let graph = Arc::clone(&self.graph);
        tokio::spawn(async move {
            let mut incoming = control
                .accept(GOSSIP_PROTOCOL)
                .expect("only one accept handle per protocol");
            while let Some((_peer, stream)) = incoming.next().await {
                let graph = Arc::clone(&graph);
                tokio::spawn(async move {
                    // Errores de stream son normales (peer se cae,
                    // protocolo violado) — no propagamos, sólo se
                    // descarta la sesión.
                    let _ = handle_incoming(stream, graph).await;
                });
            }
        })
    }

    /// Ronda activa de gossip: abre stream, ANUNCIA nuestro digest,
    /// y si el peer pide atestaciones, se las manda.
    ///
    /// Este es el lado **emisor**: termina con `bundles_enviados`
    /// atestaciones empujadas al peer (que las verifica e integra
    /// en su lado). Para recibir novedades nosotros del peer, el peer
    /// debe llamar `gossip_with` en sentido inverso.
    pub async fn gossip_with(&self, peer_id: PeerId) -> Result<PushStats, AgoraNetError> {
        let mut control = self.net.control.clone();
        let stream = control.open_stream(peer_id, GOSSIP_PROTOCOL).await?;
        let mut compat = stream.compat();

        // 1) Anunciamos nuestro digest.
        let announce = {
            let g = self.graph.lock().await;
            Message::Announce(Digest::from_graph(&g))
        };
        send_frame(&mut compat, &announce).await?;

        // 2) Escuchamos respuesta. Si el peer cerró (no necesita
        // nada), terminamos con `peer_al_dia = true`. Si pidió,
        // computamos el bundle y se lo mandamos.
        let mut stats = GossipStats::default();
        match read_frame_optional(&mut compat).await? {
            Some(Message::Request(hashes)) => {
                let bundle: Vec<Attestation> = {
                    let g = self.graph.lock().await;
                    al_recibir_request(&g, &hashes, &mut stats)
                };
                if !bundle.is_empty() {
                    send_frame(&mut compat, &Message::Bundle(bundle)).await?;
                }
            }
            // El peer no respondió Request: o ya estaba al día o
            // mandó algo distinto (lo descartamos — protocolo viola).
            _ => {}
        }

        Ok(PushStats::from(stats))
    }
}

// =============================================================================
//  Handler del lado pasivo
// =============================================================================

async fn handle_incoming(stream: Stream, graph: Arc<Mutex<TrustGraph>>) -> Result<(), AgoraNetError> {
    let mut compat = stream.compat();

    // 1) Esperamos el Announce del peer.
    let msg = match read_frame_optional(&mut compat).await? {
        Some(m) => m,
        None => return Ok(()), // peer abrió y cerró sin decir nada
    };
    let announce = match msg {
        Message::Announce(d) => d,
        // Si el peer mandó otra cosa, no es protocolo válido —
        // cerramos sin replicar.
        _ => return Ok(()),
    };

    // 2) Computamos lo que nos falta.
    let faltantes = {
        let g = graph.lock().await;
        al_recibir_announce(&g, &announce)
    };
    if faltantes.is_empty() {
        // Estamos al día — no pedimos nada. Cierre limpio.
        return Ok(());
    }

    // 3) Pedimos.
    send_frame(&mut compat, &Message::Request(faltantes)).await?;

    // 4) Recibimos Bundle y mergeamos. Cada atestación pasa por
    // `TrustGraph::add_attestation` que re-verifica firma — un peer
    // malicioso no puede inyectar evidencia falsa por gossip.
    let mut stats = GossipStats::default();
    if let Some(Message::Bundle(b)) = read_frame_optional(&mut compat).await? {
        let mut g = graph.lock().await;
        al_recibir_bundle(&mut g, b, &mut stats);
    }

    Ok(())
}

// =============================================================================
//  Wire framing — length-prefixed postcard, idéntico al de minga
// =============================================================================

async fn send_frame<S>(stream: &mut S, msg: &Message) -> Result<(), AgoraNetError>
where
    S: AsyncWrite + Unpin,
{
    let bytes = postcard::to_allocvec(msg)?;
    let len = bytes.len() as u32;
    if len > MAX_FRAME_SIZE {
        return Err(AgoraNetError::FrameTooLarge(len));
    }
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

/// Lee un frame. `Ok(None)` cuando el peer cerró el stream limpiamente
/// antes del primer byte de longitud. Cualquier otro EOF (mitad de
/// longitud, mitad de cuerpo) es error real.
async fn read_frame_optional<S>(stream: &mut S) -> Result<Option<Message>, AgoraNetError>
where
    S: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(AgoraNetError::Io(e)),
    }
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_SIZE {
        return Err(AgoraNetError::FrameTooLarge(len));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(Some(postcard::from_bytes(&buf)?))
}

// =============================================================================
//  Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::{Attestation, Claim, IdentityKind, Keypair};
    use agora_graph::TrustGraph;
    use std::time::Duration;

    async fn wait_for_dial(net: &BrahmanNet, peer: PeerId) {
        for _ in 0..100 {
            let peers = net.find_closest_peers(peer).await;
            if peers.iter().any(|p| p.peer_id == peer) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn make_attestation(by: &Keypair, subject: &Keypair, pred: &str, val: &str) -> Attestation {
        Attestation::create(
            by,
            Claim::new(subject.identity_id(), pred, val, 1_700_000_000),
        )
    }

    #[tokio::test]
    async fn gossip_push_converges_two_graphs() {
        // Alice tiene 2 atestaciones, Bob tiene 0. Tras un
        // `gossip_with` Bob ← Alice, Bob debería tener las 2.
        let alice_yumaira = Keypair::from_seed([20; 32]);
        let alice_venezuela = Keypair::from_seed([10; 32]);
        let alice_comunidad = Keypair::from_seed([30; 32]);

        let mut g_alice = TrustGraph::new();
        g_alice.register(alice_yumaira.identity(IdentityKind::Person, "Yumaira"));
        g_alice.register(alice_venezuela.identity(IdentityKind::Institution, "Venezuela"));
        g_alice.register(alice_comunidad.identity(IdentityKind::Community, "Vecinos del Valle"));
        g_alice
            .add_attestation(make_attestation(&alice_venezuela, &alice_yumaira, "nacionalidad", "venezolana"))
            .unwrap();
        g_alice
            .add_attestation(make_attestation(&alice_comunidad, &alice_yumaira, "miembro-de", "El Valle"))
            .unwrap();
        assert_eq!(g_alice.attestation_count(), 2);

        // Bob conoce las mismas identidades pero no tiene atestaciones.
        let mut g_bob = TrustGraph::new();
        g_bob.register(alice_yumaira.identity(IdentityKind::Person, "Yumaira"));
        g_bob.register(alice_venezuela.identity(IdentityKind::Institution, "Venezuela"));
        g_bob.register(alice_comunidad.identity(IdentityKind::Community, "Vecinos del Valle"));
        assert_eq!(g_bob.attestation_count(), 0);

        let alice = AgoraNet::standalone(g_alice).expect("alice");
        let bob = AgoraNet::standalone(g_bob).expect("bob");

        let bob_pid = bob.peer_id();
        let _bob_addr = bob
            .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
            .await;
        let _bob_accept = bob.run_passive_accept();

        // Dial directo de alice → bob (sin DHT, sólo para el test).
        // Tomamos la dirección real con `/p2p/<peer_id>` para que el
        // upgrader entienda a quién está hablando.
        // En la práctica esto vendría del DHT (find_providers).
        // Para el test, esperamos que alice resuelva el peer_id.
        // Truco: dial por loopback explícito.
        let bob_listen = bob
            .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
            .await;
        let dial_addr: Multiaddr = format!("{}/p2p/{}", bob_listen, bob_pid).parse().unwrap();
        alice.dial(dial_addr);

        // Damos un poco de tiempo al swarm para que la conexión suba.
        // Reintentamos `gossip_with` hasta éxito o deadline.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let stats = loop {
            match alice.gossip_with(bob_pid).await {
                Ok(s) => break s,
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => panic!("gossip_with failed: {e}"),
            }
        };
        assert_eq!(stats.bundles_enviados, 2);

        // Damos margen para que bob procese el bundle y mergee.
        for _ in 0..20 {
            let g = bob.snapshot().await;
            if g.attestation_count() == 2 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let g = bob.snapshot().await;
        assert_eq!(
            g.attestation_count(),
            2,
            "bob no recibió las atestaciones de alice"
        );
    }

    #[tokio::test]
    async fn gossip_to_up_to_date_peer_pushes_nothing() {
        let yumaira = Keypair::from_seed([20; 32]);
        let venezuela = Keypair::from_seed([10; 32]);

        // Alice y Bob tienen exactamente las mismas atestaciones.
        let att = make_attestation(&venezuela, &yumaira, "nacionalidad", "venezolana");

        let mut g_alice = TrustGraph::new();
        g_alice.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
        g_alice.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
        g_alice.add_attestation(att.clone()).unwrap();

        let mut g_bob = TrustGraph::new();
        g_bob.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
        g_bob.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
        g_bob.add_attestation(att).unwrap();

        let alice = AgoraNet::standalone(g_alice).expect("alice");
        let bob = AgoraNet::standalone(g_bob).expect("bob");

        let bob_pid = bob.peer_id();
        let bob_addr = bob
            .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
            .await;
        let _accept = bob.run_passive_accept();

        let dial_addr: Multiaddr = format!("{}/p2p/{}", bob_addr, bob_pid).parse().unwrap();
        alice.dial(dial_addr);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let stats = loop {
            match alice.gossip_with(bob_pid).await {
                Ok(s) => break s,
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => panic!("gossip_with failed: {e}"),
            }
        };
        assert_eq!(stats.bundles_enviados, 0);
        assert!(stats.peer_al_dia);
        let _ = wait_for_dial; // keep import alive in case test changes
    }
}
