//! `agora-gossip` — protocolo de **convergencia de TrustGraphs** entre
//! pares. Transport-agnóstico: no abre sockets, no firma nada nuevo, no
//! piensa en redes. Sólo decide *qué mensaje* mandar dado lo que cada
//! par sabe, y *qué hacer* cuando uno entra.
//!
//! ## El intercambio
//!
//! Cuatro mensajes bastan. Tres componen una ronda anti-entropy en
//! sentido PUSH (el iniciador empuja sus novedades al peer):
//!
//! 1. **`Announce(Digest)`** — broadcast: «tengo estas atestaciones»
//!    (lista de [`AttestationHash`]). El receptor compara con su propio
//!    grafo y arma la lista de las que le faltan.
//! 2. **`Request(Vec<AttestationHash>)`** — unicast: «mandame estas».
//! 3. **`Bundle(Vec<Attestation>)`** — unicast: «aquí van». El receptor
//!    las pasa por [`TrustGraph::add_attestation`] una por una; las
//!    firmas se re-verifican ahí mismo, el grafo nunca incorpora una
//!    atestación rota.
//!
//! El cuarto invierte la iniciativa para hacer PULL desde el iniciador:
//!
//! 4. **`Pull`** — request del iniciador: «empezá vos el flow». El peer
//!    responde con su `Announce(Digest)` y la ronda sigue normal —
//!    `Pull` permite que un nodo sin conectividad de entrada pero con
//!    salida igual reciba novedades de sus pares.
//!
//! La identidad de cada atestación es su [`Attestation::stable_hash`] —
//! BLAKE3 sobre `claim.canonical_bytes() || attester_key || signature`,
//! determinista por la firma ed25519 también determinista. Dos nodos
//! con la misma atestación calculan el mismo hash.
//!
//! ## Por qué transport-agnóstico
//!
//! El gossip que necesita el escritorio cliente (TCP a un par conocido),
//! el que necesita Wawa en bare-metal (Akasha Over Ether, EtherType
//! 0x88B5) y el que se usaría sin red (`scp` de un JSON) comparten *la
//! misma lógica*: digest, missing, bundle. Encapsular eso en un crate
//! puro hace que el transporte sea trivial — `serde` se ocupa del
//! framing, el caller del wire.
//!
//! ## Lo que NO hace
//!
//! - No verifica firmas — `TrustGraph::add_attestation` ya las verifica.
//! - No persiste — `agora-store` ya persiste.
//! - No descubre peers — la lista de peers la trae el caller.
//! - No prioriza / banea — políticas más elaboradas viven encima.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use agora_core::Attestation;
use agora_graph::TrustGraph;
use serde::{Deserialize, Serialize};

/// Hash estable de una atestación. Igual al que devuelve
/// [`Attestation::stable_hash`]; los repetimos como tipo para que los
/// callers no tengan que importar `agora-core` cuando sólo manejan
/// digests.
pub type AttestationHash = [u8; 32];

// =============================================================================
//  Digest — el catálogo de lo que un par sabe
// =============================================================================

/// Catálogo ordenado y deduplicado de hashes de atestación que un par
/// declara tener. `BTreeSet` da orden estable (necesario para que el
/// digest sea reproducible entre nodos) y dedup automático.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Digest {
    /// Hashes de las atestaciones en posesión del emisor.
    pub haves: BTreeSet<AttestationHash>,
}

impl Digest {
    /// Construye un digest a partir del grafo local.
    pub fn from_graph(g: &TrustGraph) -> Self {
        let haves = g
            .attestations()
            .iter()
            .map(|a| a.stable_hash())
            .collect();
        Self { haves }
    }

    /// Cuántas atestaciones declara tener este digest.
    pub fn len(&self) -> usize {
        self.haves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.haves.is_empty()
    }

    /// Hashes que están en `self` y no en `other`. Es lo que el dueño de
    /// `other` debería pedir a quien emitió `self`.
    pub fn diff_against<'a>(&'a self, other: &'a Digest) -> Vec<&'a AttestationHash> {
        self.haves.difference(&other.haves).collect()
    }
}

// =============================================================================
//  Message — la unidad del protocolo
// =============================================================================

/// Un mensaje del protocolo de gossip. Cabe en cualquier transporte que
/// mueva bytes: serde lo serializa, el wire lo decide el caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    /// «Tengo estas atestaciones». Broadcast natural — quien recibe
    /// puede contestar con [`Message::Request`] si le faltan.
    Announce(Digest),
    /// «Mandame estas». Unicast al que anunció el digest.
    Request(Vec<AttestationHash>),
    /// «Aquí van las que pediste». Unicast en respuesta a `Request`.
    Bundle(Vec<Attestation>),
    /// «Empezá vos» — invierte la iniciativa: el receptor responde con
    /// su [`Message::Announce`] y la ronda sigue normal. Sirve para
    /// PULL: un nodo abre stream pero pide que el peer le anuncie
    /// primero, en vez de empujar lo propio.
    Pull,
}

// =============================================================================
//  Lógica pura del nodo — decide qué mandar y qué hacer al recibir
// =============================================================================

/// Estadísticas que recoge un nodo durante un intercambio. No persisten;
/// son útiles para tracing y para tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GossipStats {
    /// Atestaciones absorbidas con éxito desde un `Bundle`.
    pub bundles_recibidos_ok: usize,
    /// Atestaciones que llegaron en un `Bundle` y fueron rechazadas por
    /// el grafo (firma rota, atestador desalineado). Es importante
    /// contarlas — un par malicioso o roto se delata así.
    pub bundles_recibidos_rechazados: usize,
    /// `Request`s atendidos: hashes que sí teníamos.
    pub requests_atendidos: usize,
    /// `Request`s sin match: hashes pedidos que no teníamos.
    pub requests_sin_match: usize,
}

/// Resultado de procesar un [`Message::Announce`]: lista de hashes que
/// nos faltan. Si está vacía, ya estamos al día con ese par.
pub fn al_recibir_announce(local: &TrustGraph, announce: &Digest) -> Vec<AttestationHash> {
    let mio = Digest::from_graph(local);
    announce
        .diff_against(&mio)
        .into_iter()
        .copied()
        .collect()
}

/// Resultado de procesar un [`Message::Request`]: las atestaciones que
/// tenemos y nos pidieron. Las que no tengamos se omiten — el remitente
/// puede pedirlas a otro par en otra ronda. También cuenta las que no
/// matchearon para que el caller pueda tracear discrepancias.
pub fn al_recibir_request(
    local: &TrustGraph,
    pedidos: &[AttestationHash],
    stats: &mut GossipStats,
) -> Vec<Attestation> {
    // Indexamos por hash una sola vez para evitar O(n*m) en grafos grandes.
    let local_attestations = local.attestations();
    let mut bundle = Vec::with_capacity(pedidos.len());
    for hash in pedidos {
        match local_attestations.iter().find(|a| a.stable_hash() == *hash) {
            Some(a) => {
                bundle.push(a.clone());
                stats.requests_atendidos += 1;
            }
            None => stats.requests_sin_match += 1,
        }
    }
    bundle
}

/// Absorbe un [`Message::Bundle`] al grafo local. Cada atestación pasa
/// por [`TrustGraph::add_attestation`] — la firma se re-verifica ahí, así
/// que un par malicioso no puede inyectar evidencia falsa por gossip.
/// Devuelve cuántas se aceptaron (las rechazadas viven en `stats`).
pub fn al_recibir_bundle(
    local: &mut TrustGraph,
    bundle: Vec<Attestation>,
    stats: &mut GossipStats,
) -> usize {
    let mut aceptadas = 0;
    for att in bundle {
        match local.add_attestation(att) {
            Ok(()) => {
                aceptadas += 1;
                stats.bundles_recibidos_ok += 1;
            }
            Err(_) => {
                stats.bundles_recibidos_rechazados += 1;
            }
        }
    }
    aceptadas
}

/// Punto de entrada conveniente: dado un mensaje entrante y el grafo
/// local, devuelve la respuesta apropiada (o `None` si el mensaje no
/// requiere réplica). Atajo para callers simples; los avanzados pueden
/// usar las funciones específicas y elegir cómo encolar las respuestas.
pub fn responder(
    local: &mut TrustGraph,
    entrante: &Message,
    stats: &mut GossipStats,
) -> Option<Message> {
    match entrante {
        Message::Announce(d) => {
            let faltantes = al_recibir_announce(local, d);
            if faltantes.is_empty() {
                None
            } else {
                Some(Message::Request(faltantes))
            }
        }
        Message::Request(hashes) => {
            let bundle = al_recibir_request(local, hashes, stats);
            if bundle.is_empty() {
                None
            } else {
                Some(Message::Bundle(bundle))
            }
        }
        Message::Bundle(b) => {
            al_recibir_bundle(local, b.clone(), stats);
            // El bundle es el cierre del ciclo — sin réplica.
            None
        }
        Message::Pull => {
            // El iniciador pide que arranquemos nosotros. Le devolvemos
            // nuestro digest; el flow sigue normal desde su lado.
            Some(Message::Announce(Digest::from_graph(local)))
        }
    }
}

// =============================================================================
//  Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::{Attestation, Claim, IdentityKind, Keypair};

    fn make_attestation(by: &Keypair, subject: &Keypair, pred: &str, val: &str) -> Attestation {
        Attestation::create(
            by,
            Claim::new(subject.identity_id(), pred, val, 1_700_000_000),
        )
    }

    fn poblar_grafo(g: &mut TrustGraph) -> (Keypair, Keypair, Keypair, Keypair) {
        let yumaira = Keypair::from_seed([20; 32]);
        let venezuela = Keypair::from_seed([10; 32]);
        let comunidad = Keypair::from_seed([30; 32]);
        let vecina = Keypair::from_seed([40; 32]);
        g.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
        g.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
        g.register(comunidad.identity(IdentityKind::Community, "Vecinos del Valle"));
        g.register(vecina.identity(IdentityKind::Person, "Carmen"));
        (yumaira, venezuela, comunidad, vecina)
    }

    #[test]
    fn digest_de_grafo_vacio_es_vacio() {
        let g = TrustGraph::new();
        let d = Digest::from_graph(&g);
        assert!(d.is_empty());
        assert_eq!(d.len(), 0);
    }

    #[test]
    fn digest_recoge_un_hash_por_atestacion() {
        let mut g = TrustGraph::new();
        let (yumaira, venezuela, comunidad, _) = poblar_grafo(&mut g);
        g.add_attestation(make_attestation(&venezuela, &yumaira, "nacionalidad", "venezolana"))
            .unwrap();
        g.add_attestation(make_attestation(&comunidad, &yumaira, "miembro-de", "El Valle"))
            .unwrap();
        let d = Digest::from_graph(&g);
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn digest_es_estable_aunque_cambie_el_orden_de_insercion() {
        let mut a = TrustGraph::new();
        let mut b = TrustGraph::new();
        let (yumaira, venezuela, comunidad, _) = poblar_grafo(&mut a);
        let _ = poblar_grafo(&mut b);
        let att1 = make_attestation(&venezuela, &yumaira, "nacionalidad", "venezolana");
        let att2 = make_attestation(&comunidad, &yumaira, "miembro-de", "El Valle");
        a.add_attestation(att1.clone()).unwrap();
        a.add_attestation(att2.clone()).unwrap();
        b.add_attestation(att2).unwrap(); // orden invertido
        b.add_attestation(att1).unwrap();
        assert_eq!(Digest::from_graph(&a), Digest::from_graph(&b));
    }

    #[test]
    fn diff_devuelve_los_que_le_faltan_al_otro() {
        let mut a = TrustGraph::new();
        let mut b = TrustGraph::new();
        let (yumaira, venezuela, comunidad, _) = poblar_grafo(&mut a);
        let _ = poblar_grafo(&mut b);
        let solo_a = make_attestation(&venezuela, &yumaira, "nacionalidad", "venezolana");
        let comun = make_attestation(&comunidad, &yumaira, "miembro-de", "El Valle");
        a.add_attestation(solo_a.clone()).unwrap();
        a.add_attestation(comun.clone()).unwrap();
        b.add_attestation(comun).unwrap();
        let d_a = Digest::from_graph(&a);
        let d_b = Digest::from_graph(&b);
        let faltantes = d_a.diff_against(&d_b);
        assert_eq!(faltantes.len(), 1);
        assert_eq!(*faltantes[0], solo_a.stable_hash());
    }

    #[test]
    fn dos_nodos_convergen_en_una_ronda_completa() {
        // Nodo A tiene la atestación de nacionalidad; nodo B la de comunidad.
        // Tras una ronda anti-entropy completa (announce → request → bundle,
        // en ambos sentidos) deberían tener exactamente las dos.
        let mut a = TrustGraph::new();
        let mut b = TrustGraph::new();
        let (yumaira, venezuela, comunidad, _) = poblar_grafo(&mut a);
        let _ = poblar_grafo(&mut b);
        let only_a = make_attestation(&venezuela, &yumaira, "nacionalidad", "venezolana");
        let only_b = make_attestation(&comunidad, &yumaira, "miembro-de", "El Valle");
        a.add_attestation(only_a.clone()).unwrap();
        b.add_attestation(only_b.clone()).unwrap();

        let mut sa = GossipStats::default();
        let mut sb = GossipStats::default();

        // A → anuncia su digest. B detecta lo que le falta y pide.
        let anun_a = Message::Announce(Digest::from_graph(&a));
        let req_b = responder(&mut b, &anun_a, &mut sb).expect("B debe pedir");
        // A atiende el request: serve only_a.
        let bun_a = responder(&mut a, &req_b, &mut sa).expect("A debe servir");
        // B absorbe.
        let _ = responder(&mut b, &bun_a, &mut sb);

        // Y la simétrica: B → anuncia, A detecta y pide.
        let anun_b = Message::Announce(Digest::from_graph(&b));
        let req_a = responder(&mut a, &anun_b, &mut sa).expect("A debe pedir");
        let bun_b = responder(&mut b, &req_a, &mut sb).expect("B debe servir");
        let _ = responder(&mut a, &bun_b, &mut sa);

        // Convergencia: ambos digests deben coincidir y tener las 2 attestations.
        let d_a = Digest::from_graph(&a);
        let d_b = Digest::from_graph(&b);
        assert_eq!(d_a, d_b);
        assert_eq!(d_a.len(), 2);
        assert!(d_a.haves.contains(&only_a.stable_hash()));
        assert!(d_a.haves.contains(&only_b.stable_hash()));
        // Stats: cada nodo absorbió 1 atestación y atendió 1 request.
        assert_eq!(sa.bundles_recibidos_ok, 1);
        assert_eq!(sb.bundles_recibidos_ok, 1);
        assert_eq!(sa.requests_atendidos, 1);
        assert_eq!(sb.requests_atendidos, 1);
    }

    #[test]
    fn announce_sin_diferencias_no_genera_request() {
        let mut a = TrustGraph::new();
        let mut b = TrustGraph::new();
        let (yumaira, venezuela, _, _) = poblar_grafo(&mut a);
        let _ = poblar_grafo(&mut b);
        let comun = make_attestation(&venezuela, &yumaira, "nacionalidad", "venezolana");
        a.add_attestation(comun.clone()).unwrap();
        b.add_attestation(comun).unwrap();
        let mut stats = GossipStats::default();
        let resp = responder(&mut b, &Message::Announce(Digest::from_graph(&a)), &mut stats);
        assert!(resp.is_none(), "no debería pedir nada — ya estamos al día");
    }

    #[test]
    fn bundle_corrupto_es_rechazado_y_contado() {
        // Un par malicioso manda un bundle con una firma rota — el grafo
        // local lo rechaza y stats lo cuenta. Convergencia no se rompe.
        let mut local = TrustGraph::new();
        let (yumaira, venezuela, _, _) = poblar_grafo(&mut local);
        let mut falsa = make_attestation(&venezuela, &yumaira, "nacionalidad", "venezolana");
        falsa.claim.value = "marciana".into(); // rompe la firma
        let mut stats = GossipStats::default();
        let aceptadas = al_recibir_bundle(&mut local, vec![falsa], &mut stats);
        assert_eq!(aceptadas, 0);
        assert_eq!(stats.bundles_recibidos_rechazados, 1);
        assert_eq!(local.attestation_count(), 0);
    }

    #[test]
    fn request_pide_hashes_que_no_tenemos_y_se_omiten_silenciosamente() {
        let mut g = TrustGraph::new();
        let _ = poblar_grafo(&mut g);
        let mut stats = GossipStats::default();
        let bundle = al_recibir_request(&g, &[[0xAA; 32], [0xBB; 32]], &mut stats);
        assert!(bundle.is_empty());
        assert_eq!(stats.requests_sin_match, 2);
        assert_eq!(stats.requests_atendidos, 0);
    }

    #[test]
    fn pull_dispara_announce_del_receptor() {
        // Iniciador manda Pull a un peer; el peer debe responder con
        // su propio Announce(digest). El iniciador ya tiene material
        // para diff'ar contra él.
        let mut local = TrustGraph::new();
        let (yumaira, venezuela, _, _) = poblar_grafo(&mut local);
        local
            .add_attestation(make_attestation(&venezuela, &yumaira, "nacionalidad", "venezolana"))
            .unwrap();
        let mut stats = GossipStats::default();
        let resp = responder(&mut local, &Message::Pull, &mut stats);
        match resp {
            Some(Message::Announce(d)) => {
                assert_eq!(d.len(), 1);
            }
            other => panic!("esperaba Announce(...) tras Pull, fui {:?}", other),
        }
    }

    #[test]
    fn pull_de_grafo_vacio_devuelve_announce_vacio() {
        // Edge: un peer vacío contestando Pull manda un digest vacío
        // (no `None`) — el iniciador así puede saber que no hay nada
        // y cerrar la stream limpio.
        let mut local = TrustGraph::new();
        let mut stats = GossipStats::default();
        let resp = responder(&mut local, &Message::Pull, &mut stats);
        match resp {
            Some(Message::Announce(d)) => assert!(d.is_empty()),
            other => panic!("esperaba Announce(...) vacío, fui {:?}", other),
        }
    }

    #[test]
    fn convergencia_es_idempotente_en_una_segunda_ronda() {
        // Una vez convergidos, otra ronda no produce mensajes (digests iguales).
        let mut a = TrustGraph::new();
        let mut b = TrustGraph::new();
        let (yumaira, venezuela, _, _) = poblar_grafo(&mut a);
        let _ = poblar_grafo(&mut b);
        let att = make_attestation(&venezuela, &yumaira, "nacionalidad", "venezolana");
        a.add_attestation(att.clone()).unwrap();
        b.add_attestation(att).unwrap();

        let mut sa = GossipStats::default();
        let anun_a = Message::Announce(Digest::from_graph(&a));
        let r1 = responder(&mut b, &anun_a, &mut sa);
        assert!(r1.is_none());
        let anun_b = Message::Announce(Digest::from_graph(&b));
        let r2 = responder(&mut a, &anun_b, &mut sa);
        assert!(r2.is_none());
    }
}
