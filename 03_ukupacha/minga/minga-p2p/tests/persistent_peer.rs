//! Tests del `MingaPeer` con backing persistente.
//!
//! Verifica que:
//! - Abrir un path nuevo crea un repo vacío.
//! - Datos ingresados a un peer abierto se persisten a disco.
//! - Tras cerrar y reabrir el mismo path, el estado completo se
//!   recupera (MST con mismo `root_hash`, store con todos los nodos
//!   reconstruibles, atestaciones intactas y verificables).
//! - El sync sobre red poblando un peer persistente sobrevive
//!   reinicio.

use std::time::Duration;

use minga_core::{parse, Attestation, AttestationStore, Keypair, MemStore, Mst, NodeStore};
use minga_p2p::{MingaPeer, SyncSession};
use tempfile::TempDir;

fn kp(seed: u8) -> Keypair {
    Keypair::from_seed(&[seed; 32])
}

#[tokio::test]
async fn open_creates_empty_repo_at_new_path() {
    let dir = TempDir::new().unwrap();
    let peer = MingaPeer::open(kp(1), dir.path()).unwrap();
    let (mst, store, atts) = peer.snapshot().await;
    assert!(mst.is_empty());
    assert!(store.is_empty());
    assert!(atts.is_empty());
}

#[tokio::test]
async fn ingest_persists_across_restart() {
    let dir = TempDir::new().unwrap();
    let kp_a = kp(1);

    let n = parse::rust("fn add(x: i32, y: i32) -> i32 { x + y }").unwrap();
    let h_expected = minga_core::hash_node(&n);

    // Sesión 1: abrir, ingerir, flush, drop.
    {
        let peer = MingaPeer::open(kp_a.clone(), dir.path()).unwrap();
        let h = peer.ingest(&n).await;
        assert_eq!(h, h_expected);
        peer.flush().await.unwrap();
    }

    // Sesión 2: reabrir, verificar que todo está intacto.
    {
        let peer = MingaPeer::open(kp_a, dir.path()).unwrap();
        let (mst, store, _) = peer.snapshot().await;
        assert_eq!(mst.len(), 1);
        assert!(mst.contains(&h_expected));
        assert!(store.contains(&h_expected));

        // Reconstrucción exacta del árbol original.
        let reconstructed = store.reconstruct(&h_expected).unwrap();
        assert_eq!(reconstructed, n);
    }
}

#[tokio::test]
async fn ingest_attestation_persists_across_restart() {
    let dir = TempDir::new().unwrap();
    let kp_owner = kp(1);
    let kp_signer = kp(2);

    let n = parse::rust("fn signed_function() -> i32 { 42 }").unwrap();
    let h = minga_core::hash_node(&n);

    {
        let peer = MingaPeer::open(kp_owner.clone(), dir.path()).unwrap();
        peer.ingest(&n).await;
        let att = Attestation::create(&kp_signer, h);
        peer.ingest_attestation(att).await.unwrap();
        peer.flush().await.unwrap();
    }

    {
        let peer = MingaPeer::open(kp_owner, dir.path()).unwrap();
        let (_, _, atts) = peer.snapshot().await;
        let authors = atts.authors_of(&h);
        assert_eq!(authors, vec![kp_signer.did()]);

        // La firma sigue verificando tras viajar disco→memoria.
        let stored_atts = atts.get(&h);
        assert_eq!(stored_atts.len(), 1);
        assert!(stored_atts[0].verify());
    }
}

#[tokio::test]
async fn ingest_multiple_authors_for_same_content_persist() {
    let dir = TempDir::new().unwrap();
    let kp_owner = kp(1);
    let alice = kp(10);
    let bob = kp(20);
    let carol = kp(30);

    let n = parse::rust("fn shared() -> i32 { 0 }").unwrap();
    let h = minga_core::hash_node(&n);

    {
        let peer = MingaPeer::open(kp_owner.clone(), dir.path()).unwrap();
        peer.ingest(&n).await;
        peer.ingest_attestation(Attestation::create(&alice, h))
            .await
            .unwrap();
        peer.ingest_attestation(Attestation::create(&bob, h))
            .await
            .unwrap();
        peer.ingest_attestation(Attestation::create(&carol, h))
            .await
            .unwrap();
        peer.flush().await.unwrap();
    }

    {
        let peer = MingaPeer::open(kp_owner, dir.path()).unwrap();
        let (_, _, atts) = peer.snapshot().await;
        let mut authors = atts.authors_of(&h);
        authors.sort_by_key(|d| d.0);
        assert_eq!(authors.len(), 3);
        let mut expected = vec![alice.did(), bob.did(), carol.did()];
        expected.sort_by_key(|d| d.0);
        assert_eq!(authors, expected);
    }
}

#[tokio::test]
async fn root_hash_stable_across_restart() {
    // El `root_hash` del MST es función pura del set de claves. Tras
    // reabrir desde disco, debe ser idéntico.
    let dir = TempDir::new().unwrap();
    let kp_a = kp(1);

    let target_root_hash;
    {
        let peer = MingaPeer::open(kp_a.clone(), dir.path()).unwrap();
        for src in &[
            "fn one() -> i32 { 1 }",
            "fn two() -> i32 { 2 }",
            "fn three(x: i32) -> i32 { x * x }",
        ] {
            peer.ingest(&parse::rust(src).unwrap()).await;
        }
        target_root_hash = peer.snapshot().await.0.root_hash();
        peer.flush().await.unwrap();
    }

    {
        let peer = MingaPeer::open(kp_a, dir.path()).unwrap();
        let (mst, _, _) = peer.snapshot().await;
        assert_eq!(mst.root_hash(), target_root_hash);
        assert_eq!(mst.len(), 3);
    }
}

#[tokio::test]
async fn sync_into_persistent_peer_survives_restart() {
    // Caso end-to-end: peer A pasivo y persistente. B sincroniza con
    // A. A persiste lo que recibió. Cerramos A. Reabrimos. El estado
    // sincronizado sigue ahí.
    let dir = TempDir::new().unwrap();
    let kp_a = kp(1);

    let n = parse::rust("fn from_b(z: i32) -> i32 { z + 7 }").unwrap();
    let h_b = minga_core::hash_node(&n);

    // ── Sesión 1: A persistente acepta sync de B ─────────────────
    {
        let a = MingaPeer::open(kp_a.clone(), dir.path()).unwrap();
        let addr_a = a.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
        let accept = a.run_passive_accept();

        // B en memoria, le sincroniza su contenido.
        let mut store_b = MemStore::new();
        let mut mst_b = Mst::new();
        let h = store_b.put(&n);
        mst_b.insert(h);
        let b = MingaPeer::new(kp(2), mst_b, store_b, AttestationStore::new()).unwrap();
        b.dial(addr_a);

        // Reintentar sync hasta éxito.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if b.sync_with(a.peer_id()).await.is_ok() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                panic!("sync no completó en 5s");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Esperar a que A's accept handler haya mergeado.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let (mst_a, _, _) = a.snapshot().await;
            if mst_a.contains(&h_b) {
                break;
            }
            if std::time::Instant::now() >= deadline {
                panic!("merge en A no se vio en 2s");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        a.flush().await.unwrap();

        // Cleanup explícito: abort la accept task y espera a que
        // termine para liberar el lock de sled.
        accept.abort();
        let _ = accept.await;
    }

    // Pequeño margen para que tasks spawneadas terminen y los Arc
    // se liberen.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // ── Sesión 2: reabrir A, verificar contenido sincronizado ────
    {
        let a = MingaPeer::open(kp_a, dir.path()).unwrap();
        let (mst_a, store_a, _) = a.snapshot().await;
        assert!(
            mst_a.contains(&h_b),
            "el contenido de B no sobrevivió al reinicio"
        );
        assert!(store_a.contains(&h_b));

        // Reconstruimos: lo que B firmó sigue ahí íntegro.
        let reconstructed = store_a.reconstruct(&h_b).unwrap();
        assert_eq!(reconstructed, n);
    }
}

// Helper: silencia un warning si SyncSession se importa pero no se usa.
#[allow(dead_code)]
fn _session_marker(_: SyncSession) {}
