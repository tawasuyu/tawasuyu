//! Invariantes del protocolo de sincronización recursivo.
//!
//! Tres familias de tests:
//! - **Convergencia funcional**: tras `run_sync`, ambos peers tienen
//!   el mismo `root_hash`, `MemStore` equivalente, y reconstruyen los
//!   árboles bit a bit.
//! - **Eficiencia estructural**: el short-circuit por hash de subárbol
//!   reduce probes y delivers cuando los repos comparten ramas.
//! - **Seguridad**: el receptor verifica `hash_stored(stored) == hash`
//!   y rechaza nodos manipulados.

use minga_core::{
    cas::hash_components, hash_node, hash_stored, parse, ContentHash, Keypair, MemStore, Mst,
    NodeStore, Signature, StoredNode,
};
use minga_p2p::{run_sync, Message, SyncSession};

fn kp(seed: u8) -> Keypair {
    Keypair::from_seed(&[seed; 32])
}

/// Helper que replica la construcción del payload firmado del `Hello`
/// dentro del protocolo Minga. Usado por los tests que inyectan
/// mensajes manualmente.
fn hello_payload(nonce: &[u8; 32], did: &minga_core::Did, root: &ContentHash) -> [u8; 96] {
    let mut p = [0u8; 96];
    p[..32].copy_from_slice(nonce);
    p[32..64].copy_from_slice(&did.0);
    p[64..96].copy_from_slice(&root.0);
    p
}

fn build_repo(sources: &[&str]) -> (Mst, MemStore, Vec<ContentHash>) {
    let mut mst = Mst::new();
    let mut store = MemStore::new();
    let mut roots = Vec::new();
    for src in sources {
        let n = parse::rust(src).unwrap();
        let h = store.put(&n);
        mst.insert(h);
        roots.push(h);
    }
    (mst, store, roots)
}

// ─── Convergencia funcional ────────────────────────────────────────

#[test]
fn sync_identical_is_noop() {
    let sources = &[
        "fn add(x: i32, y: i32) -> i32 { x + y }",
        "fn neg(x: i32) -> i32 { -x }",
    ];
    let (mst_a, store_a, _) = build_repo(sources);
    let (mst_b, store_b, _) = build_repo(sources);

    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let mut b = SyncSession::without_attestations(mst_b, store_b, kp(2));
    let stats = run_sync(&mut a, &mut b);

    // Mismas raíces de MST: el short-circuit en Hello evita cualquier
    // probe o transferencia. Solo cruzan los 2 Hellos y los 2 Dones.
    assert_eq!(stats.hellos, 2);
    assert_eq!(stats.probe_reqs, 0);
    assert_eq!(stats.probe_ress, 0);
    assert_eq!(stats.fetches, 0);
    assert_eq!(stats.delivers, 0);
    assert_eq!(stats.dones, 2);

    assert_eq!(a.mst().root_hash(), b.mst().root_hash());
}

#[test]
fn sync_one_empty_pulls_everything() {
    let sources = &["fn f(x: i32) -> i32 { x * 2 }"];
    let (mst_a, store_a, _) = build_repo(sources);
    let (mst_b, store_b, _) = build_repo(&[]);

    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let mut b = SyncSession::without_attestations(mst_b, store_b, kp(2));
    run_sync(&mut a, &mut b);

    assert_eq!(a.mst().root_hash(), b.mst().root_hash());
    assert_eq!(a.store().len(), b.store().len());

    for h in a.mst().iter() {
        assert!(b.store().contains(h));
        let a_tree = a.store().reconstruct(h).unwrap();
        let b_tree = b.store().reconstruct(h).unwrap();
        assert_eq!(a_tree, b_tree);
    }
}

#[test]
fn sync_disjoint_sets_merge() {
    let only_a = &[
        "fn alpha() -> i32 { 1 }",
        "fn beta(x: i32) -> i32 { x + 1 }",
    ];
    let only_b = &[
        "fn gamma(y: i32) -> bool { y > 0 }",
        "fn delta() -> &'static str { \"hello\" }",
    ];

    let (mst_a, store_a, _) = build_repo(only_a);
    let (mst_b, store_b, _) = build_repo(only_b);

    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let mut b = SyncSession::without_attestations(mst_b, store_b, kp(2));
    run_sync(&mut a, &mut b);

    assert_eq!(a.mst().root_hash(), b.mst().root_hash());
    assert_eq!(a.mst().len(), 4);
    assert_eq!(b.mst().len(), 4);
}

#[test]
fn sync_partial_overlap_converges() {
    let common = &[
        "fn shared_one() -> i32 { 42 }",
        "fn shared_two(n: i32) -> i32 { n + 1 }",
    ];
    let extra_a = &["fn only_in_a() -> bool { true }"];
    let extra_b = &["fn only_in_b(s: &str) -> usize { s.len() }"];

    let mut sources_a: Vec<&str> = common.to_vec();
    sources_a.extend_from_slice(extra_a);
    let mut sources_b: Vec<&str> = common.to_vec();
    sources_b.extend_from_slice(extra_b);

    let (mst_a, store_a, _) = build_repo(&sources_a);
    let (mst_b, store_b, _) = build_repo(&sources_b);

    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let mut b = SyncSession::without_attestations(mst_b, store_b, kp(2));
    run_sync(&mut a, &mut b);

    assert_eq!(a.mst().root_hash(), b.mst().root_hash());
    assert_eq!(a.mst().len(), 4);
}

#[test]
fn sync_transitive_children_pulled() {
    let big_src = r#"
        fn complicated(x: i32, y: i32) -> i32 {
            let a = x + y;
            let b = a * 2;
            match b {
                n if n > 100 => n - 50,
                n if n < 0 => -n,
                _ => b,
            }
        }
    "#;
    let (mst_a, store_a, roots) = build_repo(&[big_src]);
    let store_a_size = store_a.len();
    let root_hash = roots[0];

    let (mst_b, store_b, _) = build_repo(&[]);

    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let mut b = SyncSession::without_attestations(mst_b, store_b, kp(2));
    run_sync(&mut a, &mut b);

    assert!(b.store().contains(&root_hash));
    assert_eq!(b.store().len(), store_a_size);

    let a_tree = a.store().reconstruct(&root_hash).unwrap();
    let b_tree = b.store().reconstruct(&root_hash).unwrap();
    assert_eq!(a_tree, b_tree);
}

#[test]
fn sync_idempotent_after_convergence() {
    let sources = &["fn p() -> i32 { 1 }", "fn q(x: i32) -> i32 { x + 1 }"];
    let (mst_a, store_a, _) = build_repo(sources);
    let (mst_b, store_b, _) = build_repo(&["fn r(y: i32) -> i32 { y - 1 }"]);

    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let mut b = SyncSession::without_attestations(mst_b, store_b, kp(2));
    run_sync(&mut a, &mut b);

    let (mst_a, store_a, _) = a.into_parts();
    let (mst_b, store_b, _) = b.into_parts();
    let mut a2 = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let mut b2 = SyncSession::without_attestations(mst_b, store_b, kp(2));
    let stats = run_sync(&mut a2, &mut b2);

    // Tras converger, la segunda corrida es 2 Hellos + 2 Dones, nada
    // estructural ni transferencias.
    assert_eq!(stats.probe_reqs, 0);
    assert_eq!(stats.probe_ress, 0);
    assert_eq!(stats.fetches, 0);
    assert_eq!(stats.delivers, 0);
    assert_eq!(stats.hellos, 2);
    assert_eq!(stats.dones, 2);
}

#[test]
fn sync_both_empty_terminates() {
    let mut a = SyncSession::without_attestations(Mst::new(), MemStore::new(), kp(1));
    let mut b = SyncSession::without_attestations(Mst::new(), MemStore::new(), kp(2));
    let stats = run_sync(&mut a, &mut b);
    assert_eq!(stats.hellos, 2);
    assert_eq!(stats.probe_reqs, 0);
    assert_eq!(stats.dones, 2);
    assert!(a.mst().is_empty());
    assert!(b.mst().is_empty());
}

#[test]
fn sync_three_way_via_pairwise_runs() {
    let sources_a = &["fn a1() -> i32 { 1 }", "fn shared() -> i32 { 0 }"];
    let sources_b = &["fn b1(x: i32) -> i32 { x }", "fn shared() -> i32 { 0 }"];
    let sources_c = &["fn c1() -> bool { true }"];

    let (mst_a, store_a, _) = build_repo(sources_a);
    let (mst_b, store_b, _) = build_repo(sources_b);
    let (mst_c, store_c, _) = build_repo(sources_c);

    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let mut b = SyncSession::without_attestations(mst_b, store_b, kp(2));
    run_sync(&mut a, &mut b);
    let (mst_a, store_a, _) = a.into_parts();
    let (mst_b, store_b, _) = b.into_parts();

    let mut b = SyncSession::without_attestations(mst_b, store_b, kp(2));
    let mut c = SyncSession::without_attestations(mst_c, store_c, kp(3));
    run_sync(&mut b, &mut c);
    let (mst_b, _, _) = b.into_parts();
    let (mst_c, store_c, _) = c.into_parts();

    let mut c = SyncSession::without_attestations(mst_c, store_c, kp(3));
    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    run_sync(&mut c, &mut a);
    let (mst_c, _, _) = c.into_parts();
    let (mst_a, _, _) = a.into_parts();

    assert_eq!(mst_a.root_hash(), mst_b.root_hash());
    assert_eq!(mst_b.root_hash(), mst_c.root_hash());
    assert_eq!(mst_a.len(), 4);
}

// ─── Eficiencia estructural ────────────────────────────────────────

#[test]
fn sync_subtree_short_circuit_skips_shared_branches() {
    // Construimos dos repos que comparten muchos nodos pero difieren en
    // uno. El short-circuit por hash de subárbol debería podar las
    // ramas compartidas: el número de probes y delivers debe estar
    // dominado por la divergencia, no por el tamaño total.
    let common: Vec<String> = (0..50)
        .map(|i| format!("fn shared_{}() -> i32 {{ {} }}", i, i))
        .collect();
    let common_refs: Vec<&str> = common.iter().map(|s| s.as_str()).collect();

    let extra_a = "fn only_a() -> bool { true }".to_string();
    let mut sources_a: Vec<&str> = common_refs.clone();
    sources_a.push(&extra_a);

    let extra_b = "fn only_b() -> bool { false }".to_string();
    let mut sources_b: Vec<&str> = common_refs.clone();
    sources_b.push(&extra_b);

    let (mst_a, store_a, _) = build_repo(&sources_a);
    let (mst_b, store_b, _) = build_repo(&sources_b);
    let store_a_size = store_a.len();

    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let mut b = SyncSession::without_attestations(mst_b, store_b, kp(2));
    let stats = run_sync(&mut a, &mut b);

    assert_eq!(a.mst().root_hash(), b.mst().root_hash());

    // Cota de eficiencia: cada peer debe pedir como máximo lo que
    // realmente le falta. En este escenario, cada peer ignora una sola
    // función nueva (~docena de StoredNodes). Si el short-circuit
    // estuviera roto, transferiríamos cerca del store entero (~varios
    // cientos). La cota es laxa pero detectaría esa regresión.
    assert!(
        stats.delivers < store_a_size / 2,
        "demasiados delivers ({}); esperaba << {}",
        stats.delivers,
        store_a_size,
    );
}

// ─── Seguridad: verificación criptográfica ─────────────────────────

#[test]
fn cas_hash_node_equals_hash_stored() {
    // El invariante fundacional para verificación: hashear el árbol
    // como `SemanticNode` y como `StoredNode` produce idéntico hash.
    // Sin esto, el receptor no podría confiar en lo que recibe.
    let node = parse::rust("fn add(x: i32, y: i32) -> i32 { x + y }").unwrap();
    let direct = hash_node(&node);

    let mut store = MemStore::new();
    let via_store = store.put(&node);
    assert_eq!(direct, via_store);

    let stored = store.get(&direct).unwrap();
    let recomputed = hash_stored(stored);
    assert_eq!(direct, recomputed);
}

#[test]
fn sync_rejects_tampered_deliver() {
    // Construimos un mensaje Deliver donde `hash` y `stored` no son
    // consistentes — simulando un peer malicioso o un bit flip en el
    // transporte. La sesión debe rechazarlo y no contaminar su estado.
    let (mst_a, store_a, _) = build_repo(&[]);
    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let initial_store_size = a.store().len();
    let initial_mst_size = a.mst().len();

    // Forjamos un StoredNode con identidad falsa: anunciamos un hash
    // arbitrario pero adjuntamos contenido distinto.
    let fake_stored = StoredNode {
        kind: "function_item".to_string(),
        field_name: None,
        leaf_text: None,
        children: Vec::new(),
    };
    // El hash real de fake_stored es x; anunciamos como otra cosa.
    let real_hash = hash_components("function_item", None, None, &[]);
    let bogus_hash = ContentHash([0xAB; 32]);
    assert_ne!(real_hash, bogus_hash);

    // Inyectamos como si viniera del peer (sesión recibe Hello primero
    // para que received_hello sea true; luego le metemos el Deliver
    // tóxico). El Hello se firma con la llave del peer simulado.
    let peer_kp = kp(99);
    let peer_root = minga_core::empty_subtree_hash();
    let peer_sig = peer_kp.sign(peer_root.as_bytes());
    a.handle(Message::Hello {
        peer_did: peer_kp.did(),
        root_subtree_hash: peer_root,
        signature: peer_sig,
    });
    let _ = a.handle(Message::Deliver {
        hash: bogus_hash,
        stored: fake_stored,
    });

    // El store y MST no deben cambiar; el contador de rechazos sí.
    assert_eq!(a.store().len(), initial_store_size);
    assert_eq!(a.mst().len(), initial_mst_size);
    assert_eq!(a.rejected_delivers(), 1);
    assert!(!a.store().contains(&bogus_hash));
}

#[test]
fn sync_accepts_well_formed_deliver() {
    // Contraprueba del anterior: un Deliver con hash válido sí se
    // acepta. Verifica que el rechazo es selectivo, no global.
    let (mst_a, store_a, _) = build_repo(&[]);
    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));

    let stored = StoredNode {
        kind: "integer_literal".to_string(),
        field_name: None,
        leaf_text: Some(b"42".to_vec()),
        children: Vec::new(),
    };
    let real_hash = hash_stored(&stored);

    let peer_kp = kp(99);
    let peer_root = minga_core::empty_subtree_hash();
    let peer_sig = peer_kp.sign(peer_root.as_bytes());
    a.handle(Message::Hello {
        peer_did: peer_kp.did(),
        root_subtree_hash: peer_root,
        signature: peer_sig,
    });
    a.handle(Message::Deliver {
        hash: real_hash,
        stored,
    });

    // No estaba en awaiting_root (no llegó por probe), así que no
    // entra al MST — pero sí al store.
    assert!(a.store().contains(&real_hash));
    assert_eq!(a.rejected_delivers(), 0);
}

// ─── Identidad y autenticación ─────────────────────────────────────

#[test]
fn sync_captures_peer_did_after_valid_hello() {
    // Tras un sync exitoso, cada sesión conoce el DID del otro peer
    // — la primera afirmación criptográficamente verificable de la
    // identidad del interlocutor.
    let sources = &["fn f() -> i32 { 1 }"];
    let (mst_a, store_a, _) = build_repo(sources);
    let (mst_b, store_b, _) = build_repo(sources);

    let kp_a = kp(10);
    let kp_b = kp(20);
    let did_a = kp_a.did();
    let did_b = kp_b.did();

    let mut a = SyncSession::without_attestations(mst_a, store_a, kp_a);
    let mut b = SyncSession::without_attestations(mst_b, store_b, kp_b);

    assert_eq!(a.peer_did(), None);
    assert_eq!(b.peer_did(), None);

    run_sync(&mut a, &mut b);

    // Cada peer ahora tiene la identidad verificada del otro.
    assert_eq!(a.peer_did(), Some(did_b));
    assert_eq!(b.peer_did(), Some(did_a));
    assert_eq!(a.local_did(), did_a);
    assert_eq!(b.local_did(), did_b);
}

#[test]
fn sync_rejects_hello_with_tampered_signature() {
    // Un atacante que captura un Hello legítimo pero modifica un byte
    // de la firma debe ser rechazado. La sesión no marca
    // received_hello, no procesa el root, no emite ProbeReq — el
    // contador de rechazos se incrementa en su lugar.
    let (mst_a, store_a, _) = build_repo(&[]);
    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));

    let attacker = kp(2);
    let root = minga_core::empty_subtree_hash();
    let mut sig = attacker.sign(root.as_bytes());
    sig.0[5] ^= 0xFF;

    let out = a.handle(Message::Hello {
        peer_did: attacker.did(),
        root_subtree_hash: root,
        signature: sig,
    });

    assert!(out.is_empty(), "Hello con firma rota no debe producir respuesta");
    assert_eq!(a.rejected_hellos(), 1);
    assert_eq!(a.peer_did(), None);
}

#[test]
fn sync_rejects_hello_with_swapped_did() {
    // Otro vector: la firma es válida bajo el DID original, pero el
    // atacante reemplaza el campo `peer_did` por uno distinto. La
    // verificación falla porque la firma no fue producida por la
    // llave privada correspondiente al DID anunciado.
    let (mst_a, store_a, _) = build_repo(&[]);
    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));

    let real_signer = kp(50);
    let imposter = kp(51);
    let root = minga_core::empty_subtree_hash();
    let sig = real_signer.sign(root.as_bytes());

    a.handle(Message::Hello {
        peer_did: imposter.did(), // dice ser imposter pero la firma es de real_signer
        root_subtree_hash: root,
        signature: sig,
    });

    assert_eq!(a.rejected_hellos(), 1);
    assert_eq!(a.peer_did(), None);
}

#[test]
fn sync_rejects_hello_signed_over_different_root() {
    // El atacante firma un root diferente al que anuncia. La firma es
    // válida sobre `wrong_root`, pero el mensaje dice `claimed_root`.
    let (mst_a, store_a, _) = build_repo(&[]);
    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));

    let signer = kp(60);
    let claimed_root = ContentHash([0xAA; 32]);
    let wrong_root = ContentHash([0xBB; 32]);
    let sig_over_wrong = signer.sign(wrong_root.as_bytes());

    a.handle(Message::Hello {
        peer_did: signer.did(),
        root_subtree_hash: claimed_root,
        signature: sig_over_wrong,
    });

    assert_eq!(a.rejected_hellos(), 1);
    assert_eq!(a.peer_did(), None);
}

#[test]
fn sync_rejects_replay_of_hello_from_different_session() {
    // El test del bloque CRÍTICO: anti-replay anti-replay.
    //
    // Sesión 1: el peer "alice" responde a un Challenge de A1
    // firmando un Hello con el nonce de A1.
    //
    // Sesión 2: la misma A vuelve a abrir sesión (A2). A2 genera un
    // nonce nuevo. Un atacante intenta replicar el Hello capturado de
    // la sesión 1. Como el nonce es distinto, la firma no verifica.
    let alice = kp(50);
    let alice_root = ContentHash([0xAA; 32]);

    // Sesión 1.
    let mut a1 = SyncSession::without_attestations(Mst::new(), MemStore::new(), kp(1));
    let nonce_a1 = a1.self_nonce();

    // Alice firma su Hello sobre el nonce que A1 emitió.
    let payload_1 = hello_payload(&nonce_a1, &alice.did(), &alice_root);
    let sig_1 = alice.sign(&payload_1);
    let captured_hello = Message::Hello {
        peer_did: alice.did(),
        root_subtree_hash: alice_root,
        signature: sig_1,
    };

    // En sesión 1, el Hello se acepta limpiamente.
    a1.handle(captured_hello.clone());
    assert_eq!(a1.peer_did(), Some(alice.did()));
    assert_eq!(a1.rejected_hellos(), 0);

    // Sesión 2: A2 con nonce nuevo. El atacante replica `captured_hello`.
    let mut a2 = SyncSession::without_attestations(Mst::new(), MemStore::new(), kp(2));
    assert_ne!(a2.self_nonce(), nonce_a1, "los nonces son distintos por sesión");

    a2.handle(captured_hello);

    // Replay rechazado: la firma estaba sobre nonce_a1, A2 verifica
    // contra su propio nonce, mismatch criptográfico.
    assert_eq!(a2.rejected_hellos(), 1);
    assert_eq!(a2.peer_did(), None);
}

#[test]
fn sync_proceeds_after_valid_hello_following_rejection() {
    // Si llega un Hello inválido seguido de uno válido, la sesión se
    // recupera: acepta el válido y captura ese DID. No hay
    // "envenenamiento" persistente del estado.
    let (mst_a, store_a, _) = build_repo(&[]);
    let mut a = SyncSession::without_attestations(mst_a, store_a, kp(1));

    let bad_signer = kp(70);
    let mut bad_sig = bad_signer.sign(b"otro mensaje");
    bad_sig.0[0] ^= 0xFF;
    let root = minga_core::empty_subtree_hash();
    a.handle(Message::Hello {
        peer_did: bad_signer.did(),
        root_subtree_hash: root,
        signature: bad_sig,
    });
    assert_eq!(a.rejected_hellos(), 1);
    assert_eq!(a.peer_did(), None);

    let good_signer = kp(71);
    let nonce = a.self_nonce();
    let good_payload = hello_payload(&nonce, &good_signer.did(), &root);
    let good_sig = good_signer.sign(&good_payload);
    a.handle(Message::Hello {
        peer_did: good_signer.did(),
        root_subtree_hash: root,
        signature: good_sig,
    });
    assert_eq!(a.rejected_hellos(), 1);
    assert_eq!(a.peer_did(), Some(good_signer.did()));
}

// Aux: dejamos `Signature` importado para que el bloque arriba siga
// compilando en futuras refactorizaciones que lo necesiten.
#[allow(dead_code)]
fn _signature_marker(_: Signature) {}

// ─── Propagación de atestaciones ───────────────────────────────────

use minga_core::{Attestation, AttestationStore, Did};

fn build_repo_with_attests(
    sources: &[&str],
    signers: &[&Keypair],
) -> (Mst, MemStore, AttestationStore, Vec<ContentHash>) {
    let mut mst = Mst::new();
    let mut store = MemStore::new();
    let mut attests = AttestationStore::new();
    let mut roots = Vec::new();
    for src in sources {
        let n = parse::rust(src).unwrap();
        let h = store.put(&n);
        mst.insert(h);
        for kp in signers {
            attests.add(Attestation::create(kp, h)).unwrap();
        }
        roots.push(h);
    }
    (mst, store, attests, roots)
}

#[test]
fn sync_propagates_attestations_for_owned_content() {
    // Cada peer tiene su propio contenido y firma sus propias claves.
    // Tras sync, ambos peers conocen ambas atestaciones.
    let kp_a = kp(10);
    let kp_b = kp(20);

    let (mst_a, store_a, atts_a, roots_a) =
        build_repo_with_attests(&["fn from_a() -> i32 { 1 }"], &[&kp_a]);
    let (mst_b, store_b, atts_b, roots_b) =
        build_repo_with_attests(&["fn from_b() -> i32 { 2 }"], &[&kp_b]);

    let mut a = SyncSession::new(mst_a, store_a, atts_a, kp_a.clone());
    let mut b = SyncSession::new(mst_b, store_b, atts_b, kp_b.clone());
    run_sync(&mut a, &mut b);

    // A debe ahora conocer la atestación de B sobre roots_b[0], y
    // viceversa. Ambas verificables criptográficamente.
    let h_a = roots_a[0];
    let h_b = roots_b[0];

    let a_authors_for_a: Vec<Did> = a.attestations().authors_of(&h_a);
    let a_authors_for_b: Vec<Did> = a.attestations().authors_of(&h_b);
    assert_eq!(a_authors_for_a, vec![kp_a.did()]);
    assert_eq!(a_authors_for_b, vec![kp_b.did()]);

    let b_authors_for_a: Vec<Did> = b.attestations().authors_of(&h_a);
    let b_authors_for_b: Vec<Did> = b.attestations().authors_of(&h_b);
    assert_eq!(b_authors_for_a, vec![kp_a.did()]);
    assert_eq!(b_authors_for_b, vec![kp_b.did()]);
}

#[test]
fn sync_merges_multiple_authors_for_shared_content() {
    // Ambos peers tienen el MISMO contenido (mismo hash) pero
    // atestaciones de autores DISTINTOS. Tras sync, cada peer ve el
    // conjunto completo de autores que han respaldado ese contenido.
    let kp_a = kp(30);
    let kp_b = kp(40);
    let kp_c = kp(50);
    let kp_d = kp(60);

    let src = "fn shared() -> i32 { 99 }";

    // A tiene firmas de A y C sobre el contenido.
    let (mst_a, store_a, atts_a, _) = build_repo_with_attests(&[src], &[&kp_a, &kp_c]);
    // B tiene firmas de B y D sobre el MISMO contenido.
    let (mst_b, store_b, atts_b, roots_b) = build_repo_with_attests(&[src], &[&kp_b, &kp_d]);
    let h = roots_b[0];

    let mut a = SyncSession::new(mst_a, store_a, atts_a, kp_a.clone());
    let mut b = SyncSession::new(mst_b, store_b, atts_b, kp_b.clone());
    run_sync(&mut a, &mut b);

    // Ambos peers ven los cuatro autores.
    let mut a_authors = a.attestations().authors_of(&h);
    let mut b_authors = b.attestations().authors_of(&h);
    a_authors.sort_by_key(|d| d.0);
    b_authors.sort_by_key(|d| d.0);
    assert_eq!(a_authors, b_authors);
    assert_eq!(a_authors.len(), 4);
    assert!(a_authors.contains(&kp_a.did()));
    assert!(a_authors.contains(&kp_b.did()));
    assert!(a_authors.contains(&kp_c.did()));
    assert!(a_authors.contains(&kp_d.did()));
}

#[test]
fn sync_attestations_are_verified_at_receiver() {
    // Inyectamos manualmente un AttestPush con una firma corrupta
    // entre las legítimas. La sesión solo acepta las legítimas e
    // incrementa rejected_attests.
    let mut a = SyncSession::without_attestations(Mst::new(), MemStore::new(), kp(1));

    // Hello válido del peer simulado, para que received_hello sea true.
    let peer_kp = kp(80);
    let peer_root = minga_core::empty_subtree_hash();
    let nonce = a.self_nonce();
    let peer_payload = hello_payload(&nonce, &peer_kp.did(), &peer_root);
    let peer_sig = peer_kp.sign(&peer_payload);
    a.handle(Message::Hello {
        peer_did: peer_kp.did(),
        root_subtree_hash: peer_root,
        signature: peer_sig,
    });

    // Tres atestaciones: dos legítimas y una con firma rota.
    let alice = kp(81);
    let bob = kp(82);
    let h1 = ContentHash([1u8; 32]);
    let h2 = ContentHash([2u8; 32]);
    let h3 = ContentHash([3u8; 32]);

    let valid1 = Attestation::create(&alice, h1);
    let valid2 = Attestation::create(&bob, h2);
    let mut tampered = Attestation::create(&alice, h3);
    tampered.signature.0[10] ^= 0xFF;

    a.handle(Message::AttestPush {
        attestations: vec![valid1.clone(), tampered, valid2.clone()],
    });

    // Las dos válidas se mergean; la corrupta se rechaza.
    assert_eq!(a.attestations().len(), 2);
    assert_eq!(a.rejected_attests(), 1);
    assert_eq!(a.attestations().authors_of(&h1), vec![alice.did()]);
    assert_eq!(a.attestations().authors_of(&h2), vec![bob.did()]);
    assert!(a.attestations().get(&h3).is_empty());
}

#[test]
fn sync_attest_push_before_hello_is_rejected() {
    // Una atestación que llega antes del Hello autenticado se descarta
    // — no podemos confiar en lo que dice el remitente hasta saber
    // quién es.
    let mut a = SyncSession::without_attestations(Mst::new(), MemStore::new(), kp(1));

    let alice = kp(90);
    let h = ContentHash([7u8; 32]);
    let att = Attestation::create(&alice, h);

    let out = a.handle(Message::AttestPush {
        attestations: vec![att],
    });
    assert!(out.is_empty());
    assert_eq!(a.rejected_attests(), 1);
    assert_eq!(a.attestations().len(), 0);
}

#[test]
fn sync_attestations_are_idempotent_across_runs() {
    // Re-correr el sync no duplica atestaciones (gracias a la
    // idempotencia de AttestationStore::add por (autor, contenido)).
    let kp_a = kp(100);
    let kp_b = kp(101);

    let (mst_a, store_a, atts_a, _) =
        build_repo_with_attests(&["fn run_one() -> i32 { 1 }"], &[&kp_a]);
    let (mst_b, store_b, atts_b, _) =
        build_repo_with_attests(&["fn run_two() -> i32 { 2 }"], &[&kp_b]);

    let mut a = SyncSession::new(mst_a, store_a, atts_a, kp_a.clone());
    let mut b = SyncSession::new(mst_b, store_b, atts_b, kp_b.clone());
    run_sync(&mut a, &mut b);
    let after_first_a = a.attestations().len();
    let after_first_b = b.attestations().len();
    assert_eq!(after_first_a, 2);
    assert_eq!(after_first_b, 2);

    let (mst_a, store_a, atts_a) = a.into_parts();
    let (mst_b, store_b, atts_b) = b.into_parts();
    let mut a2 = SyncSession::new(mst_a, store_a, atts_a, kp_a);
    let mut b2 = SyncSession::new(mst_b, store_b, atts_b, kp_b);
    run_sync(&mut a2, &mut b2);

    assert_eq!(a2.attestations().len(), after_first_a);
    assert_eq!(b2.attestations().len(), after_first_b);
}

#[test]
fn sync_attestations_about_remote_content() {
    // Caso interesante: A tiene una atestación sobre contenido que
    // **NO** posee (lo recibió por gossip de un tercero). Tras sync
    // con B, B aprende esa atestación aunque A nunca tuvo el contenido
    // en su store.
    let kp_a = kp(110);
    let kp_third_party = kp(111);

    // A no tiene contenido propio pero sí una atestación de
    // `kp_third_party` sobre un hash arbitrario.
    let phantom_hash = ContentHash([0xCD; 32]);
    let mut atts_a = AttestationStore::new();
    atts_a
        .add(Attestation::create(&kp_third_party, phantom_hash))
        .unwrap();

    let kp_b = kp(112);
    let mut a = SyncSession::new(Mst::new(), MemStore::new(), atts_a, kp_a);
    let mut b = SyncSession::without_attestations(Mst::new(), MemStore::new(), kp_b);
    run_sync(&mut a, &mut b);

    // B ahora conoce la atestación, aunque ni A ni B tienen el
    // contenido en su store.
    assert_eq!(b.attestations().len(), 1);
    assert_eq!(b.attestations().authors_of(&phantom_hash), vec![kp_third_party.did()]);
    assert!(!b.store().contains(&phantom_hash));
}

#[test]
fn sync_attest_push_count_in_stats() {
    // Cuando ambos peers tienen atestaciones, el harness registra dos
    // AttestPushes (uno por dirección).
    let kp_a = kp(120);
    let kp_b = kp(121);
    let (mst_a, store_a, atts_a, _) =
        build_repo_with_attests(&["fn ax() -> i32 { 0 }"], &[&kp_a]);
    let (mst_b, store_b, atts_b, _) =
        build_repo_with_attests(&["fn bx() -> i32 { 0 }"], &[&kp_b]);

    let mut a = SyncSession::new(mst_a, store_a, atts_a, kp_a);
    let mut b = SyncSession::new(mst_b, store_b, atts_b, kp_b);
    let stats = run_sync(&mut a, &mut b);

    assert_eq!(stats.attest_pushes, 2);
}

// ─── Propagación de retracciones ────────────────────────────────────

use minga_core::{Retraction, RetractionStore};

#[test]
fn sync_propagates_retractions_for_owned_content() {
    // A retira su propia atestación; tras sync, B conoce la retracción.
    let kp_a = kp(30);
    let kp_b = kp(40);

    let (mst_a, store_a, atts_a, roots_a) =
        build_repo_with_attests(&["fn x() -> i32 { 9 }"], &[&kp_a]);
    let mut rets_a = RetractionStore::new();
    rets_a.add(Retraction::create(&kp_a, roots_a[0])).unwrap();

    let (mst_b, store_b, atts_b, _roots_b) =
        build_repo_with_attests(&["fn y() -> i32 { 8 }"], &[&kp_b]);
    let rets_b = RetractionStore::new();

    let mut a = SyncSession::with_retractions(mst_a, store_a, atts_a, rets_a, kp_a.clone());
    let mut b = SyncSession::with_retractions(mst_b, store_b, atts_b, rets_b, kp_b.clone());
    let stats = run_sync(&mut a, &mut b);

    assert!(stats.retract_pushes >= 1, "debe haber al menos un RetractPush");
    // B ahora conoce la retracción que firmó A.
    let b_retract_authors = b.retractions().authors_of(&roots_a[0]);
    assert_eq!(b_retract_authors, vec![kp_a.did()]);
    // Y A, que no recibió retracciones de B, tiene la suya sola.
    assert_eq!(a.retractions().len(), 1);
    assert_eq!(a.rejected_retracts(), 0);
    assert_eq!(b.rejected_retracts(), 0);
}

#[test]
fn forged_retraction_signature_is_rejected() {
    // Una retracción con firma rota debe contarse como rechazada y NO
    // entrar al store del peer receptor.
    let kp_a = kp(50);
    let kp_b = kp(60);

    let (mst_a, store_a, atts_a, _) =
        build_repo_with_attests(&["fn a() -> i32 { 1 }"], &[&kp_a]);
    let mut rets_a = RetractionStore::new();
    // Forjamos una retracción con firma inválida pasándola por dentro
    // del wire de un push manual.
    let bogus = Retraction {
        content: ContentHash([7u8; 32]),
        author: kp_a.did(),
        signature: minga_core::Signature([0u8; 64]),
    };
    // No la agregamos al RetractionStore (`add` rechazaría); la mandamos
    // por wire manualmente.
    let _ = &mut rets_a; // silencio

    let (mst_b, store_b, atts_b, _) =
        build_repo_with_attests(&["fn b() -> i32 { 2 }"], &[&kp_b]);

    let mut a = SyncSession::new(mst_a, store_a, atts_a, kp_a.clone());
    let mut b = SyncSession::new(mst_b, store_b, atts_b, kp_b.clone());

    // Avanzamos hasta que ambos hayan intercambiado Hello.
    let m1 = a.start();
    let mut from_a: Vec<Message> = m1;
    let mut from_b: Vec<Message> = b.start();
    while !from_a.is_empty() || !from_b.is_empty() {
        let take_a = from_a;
        from_a = Vec::new();
        for m in take_a {
            from_b.extend(b.handle(m));
        }
        let take_b = from_b;
        from_b = Vec::new();
        for m in take_b {
            from_a.extend(a.handle(m));
        }
        if b.received_hello() && a.received_hello() {
            break;
        }
    }
    // Inyectamos el push forjado.
    b.handle(Message::RetractPush {
        retractions: vec![bogus],
    });
    assert_eq!(b.rejected_retracts(), 1);
    assert_eq!(b.retractions().len(), 0);
}
