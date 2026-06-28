//! Certifica la distribución sobre red real: dos peers libp2p (TCP+Noise+Yamux)
//! en proceso. B sirve el counter.wasm desde un DiskStore; A lo pide por hash,
//! `llimphi-wasm-dist` verifica la integridad del blob recibido y la app corre.
//!
//! Mismo harness que minga (`libp2p_integration.rs`): listen + dial + open_stream
//! con reintentos. Determinista (provider explícito, sin depender de timing DHT).

use std::sync::Arc;

use card_net::BrahmanNet;
use llimphi_wasm_dist::{
    bytecode_hash, resolve, verify_integrity, AppRef, BlobSource, DiskStore, Hash, TrustRing,
};
use llimphi_wasm_net::{fetch_blob, serve_blobs};

const COUNTER_WASM: &[u8] = include_bytes!("../../llimphi-wasm-runner/assets/counter.wasm");

/// Source de un solo blob ya en mano — para pasar los bytes de red por la
/// verificación de `dist::resolve` sin volver a la red.
struct OneShot(Vec<u8>);
impl BlobSource for OneShot {
    fn fetch(&self, _hash: &Hash) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dos_peers_distribuyen_y_corren_la_app() {
    // Dos nodos independientes (keypair efímera, peer_id nuevo).
    let node_a = BrahmanNet::new().expect("nodo A");
    let node_b = BrahmanNet::new().expect("nodo B");
    let peer_b = node_b.peer_id;

    // B inscribe el counter.wasm en su store y levanta el responder de blobs.
    let dir_b = std::env::temp_dir().join("llimphi-wasm-net-srv");
    let _ = std::fs::remove_dir_all(&dir_b);
    let store_b = Arc::new(DiskStore::open(&dir_b).expect("store B"));
    let hash = store_b.put(COUNTER_WASM).expect("put");
    assert_eq!(hash, bytecode_hash(COUNTER_WASM));
    let _srv = serve_blobs(&node_b, Arc::clone(&store_b));

    // B escucha en localhost; A dializa la dirección resuelta.
    let addr_b = node_b
        .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .await;
    node_a.dial(addr_b);

    // A pide el bytecode por hash sobre la red real.
    let bytes = fetch_blob(&node_a, peer_b, &hash)
        .await
        .expect("fetch del blob");

    // Integridad: lo recibido rehashea al hash pedido (un provider malicioso no
    // puede colar otro wasm — la red no es de fiar, el hash sí).
    assert!(verify_integrity(&bytes, &hash));
    assert_eq!(bytes, COUNTER_WASM);

    // La cadena de dist verifica los bytes de RED y entrega una VerifiedApp...
    let verified = resolve(&OneShot(bytes), &TrustRing::empty(), &AppRef::pure(hash))
        .expect("resolve verifica el blob de red");
    assert_eq!(verified.permisos, 0);

    // ...y la app distribuida por la red CORRE: incrementa de verdad.
    let mut guest = verified.load().expect("carga el guest");
    let n0 = guest.view().children[0].text.as_ref().unwrap().content.clone();
    assert_eq!(n0, "0");
    guest.dispatch(&[0]).unwrap(); // Msg::Increment
    let n1 = guest.view().children[0].text.as_ref().unwrap().content.clone();
    assert_eq!(n1, "1", "la app traída por la red P2P incrementa");
}
