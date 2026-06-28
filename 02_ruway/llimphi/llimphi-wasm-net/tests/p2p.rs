//! Certifica la distribución sobre red real: dos peers libp2p (TCP+Noise+Yamux)
//! en proceso. B sirve el counter.wasm desde un DiskStore; A lo pide por hash,
//! `llimphi-wasm-dist` verifica la integridad del blob recibido y la app corre.
//!
//! Mismo harness que minga (`libp2p_integration.rs`): listen + dial + open_stream
//! con reintentos. Determinista (provider explícito, sin depender de timing DHT).

use std::sync::Arc;

use agora_core::Keypair;
use card_net::BrahmanNet;
use format::{ConcesionCapacidad, PERMISO_RED};
use llimphi_wasm_core::{
    bytecode_hash, resolve, resolve_manifest, verify_integrity, AppManifest, AppRef, BlobSource,
    DiskStore, Hash, MapSource, TrustRing,
};
use llimphi_wasm_net::{fetch_blob, serve_blobs};
use llimphi_wasm_runner::{EventPayload, WasmGuest};

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
    let mut guest = WasmGuest::load(&verified.wasm, verified.permisos).expect("carga el guest");
    let n0 = guest.view().children[0].text.as_ref().unwrap().content.clone();
    assert_eq!(n0, "0");
    guest.dispatch(0, EventPayload::Click).unwrap(); // Msg::Increment
    let n1 = guest.view().children[0].text.as_ref().unwrap().content.clone();
    assert_eq!(n1, "1", "la app traída por la red P2P incrementa");
}

/// Descubrimiento de concesiones end-to-end sobre la red: B sirve el bytecode
/// **y** su concesión firmada; A los trae AMBOS por hash, resuelve el manifiesto
/// y la app corre con permisos reales (no 0).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dos_peers_distribuyen_app_con_concesion() {
    let node_a = BrahmanNet::new().expect("nodo A");
    let node_b = BrahmanNet::new().expect("nodo B");
    let peer_b = node_b.peer_id;

    // B inscribe el wasm y una concesión firmada (PERMISO_RED) para ese bytecode.
    let dir_b = std::env::temp_dir().join("llimphi-wasm-net-grant-srv");
    let _ = std::fs::remove_dir_all(&dir_b);
    let store_b = Arc::new(DiskStore::open(&dir_b).expect("store B"));
    let bc = store_b.put(COUNTER_WASM).expect("put wasm");
    let kp = Keypair::from_seed([7; 32]);
    let mensaje = format::mensaje_capacidad(&bc, PERMISO_RED);
    let grant = ConcesionCapacidad {
        bytecode: bc,
        permisos: PERMISO_RED,
        autor: kp.public_key(),
        firma: kp.sign(&mensaje),
    };
    let gh = store_b.put_grant(&grant).expect("put grant");
    let _srv = serve_blobs(&node_b, Arc::clone(&store_b));

    let addr_b = node_b
        .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .await;
    node_a.dial(addr_b);

    // A trae los DOS blobs por hash sobre la red real.
    let wasm = fetch_blob(&node_a, peer_b, &bc).await.expect("fetch wasm");
    let grant_blob = fetch_blob(&node_a, peer_b, &gh).await.expect("fetch grant");

    // Los junta y resuelve el manifiesto: descubrimiento de concesión completo.
    let mut src = MapSource::new();
    src.insert(bc, wasm);
    src.insert(gh, grant_blob);
    let manifest = AppManifest {
        bytecode: bc,
        declarados: PERMISO_RED,
        concesion: Some(gh),
    };
    let trust = TrustRing::new(vec![kp.public_key()]);
    let verified = resolve_manifest(&src, &trust, &manifest).expect("resolve_manifest");
    assert_eq!(
        verified.permisos, PERMISO_RED,
        "la app traída por la red corre con permisos REALES, no 0"
    );

    // Y carga con ese permiso (que gatea host_net_request) y corre.
    let mut guest = WasmGuest::load(&verified.wasm, verified.permisos).expect("carga con permisos");
    guest.dispatch(0, EventPayload::Click).unwrap();
    assert_eq!(
        guest.view().children[0].text.as_ref().unwrap().content,
        "1"
    );
}
