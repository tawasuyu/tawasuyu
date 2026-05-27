//! Smoke tests del CLI: init → ingest → status, todo persistido.

use std::fs;

use minga_cli::{cmd_ingest, cmd_init, cmd_status, CliError};
use tempfile::TempDir;

#[test]
fn init_creates_keypair_and_repo() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    let did = cmd_init(&repo, "passphrase-secreta").unwrap();

    // El keypair existe en disco.
    assert!(repo.join("keypair").exists());
    // El repo sled existe (es un directorio).
    assert!(repo.join("repo").is_dir());
    // El DID retornado es no-trivial.
    assert_ne!(did, minga_core::Did([0u8; 32]));
}

#[test]
fn init_refuses_existing_non_empty_directory() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir(&repo).unwrap();
    fs::write(repo.join("garbage"), b"hello").unwrap();
    let r = cmd_init(&repo, "p");
    assert!(matches!(r, Err(CliError::AlreadyExists(_))));
}

#[test]
fn status_shows_empty_state_after_init() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    cmd_init(&repo, "p").unwrap();
    let s = cmd_status(&repo, "p").unwrap();
    assert_eq!(s.mst_len, 0);
    assert_eq!(s.nodes_len, 0);
    assert_eq!(s.attestations_len, 0);
}

#[test]
fn status_with_wrong_passphrase_errors() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    cmd_init(&repo, "correcta").unwrap();
    let r = cmd_status(&repo, "incorrecta");
    assert!(r.is_err());
}

#[test]
fn ingest_persists_function_with_self_attestation() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    let did = cmd_init(&repo, "p").unwrap();

    // Escribir un archivo Rust de ejemplo.
    let src = dir.path().join("ejemplo.rs");
    fs::write(&src, "fn add(x: i32, y: i32) -> i32 { x + y }").unwrap();

    let r = cmd_ingest(&repo, "p", &src).unwrap();
    assert_eq!(r.did, did, "la firma debe ser del repo, no de otro");

    let s = cmd_status(&repo, "p").unwrap();
    assert_eq!(s.mst_len, 1);
    assert!(s.nodes_len > 1, "el AST tiene más de un nodo");
    assert_eq!(s.attestations_len, 1, "una autoatestación");
}

#[test]
fn ingest_persists_across_runs() {
    // Simulamos "reiniciar el proceso": cmd_init en una llamada,
    // cmd_ingest en otra (que reabre el repo).
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    cmd_init(&repo, "p").unwrap();

    let src1 = dir.path().join("uno.rs");
    fs::write(&src1, "fn one() -> i32 { 1 }").unwrap();
    cmd_ingest(&repo, "p", &src1).unwrap();

    let src2 = dir.path().join("dos.rs");
    fs::write(&src2, "fn two() -> i32 { 2 }").unwrap();
    cmd_ingest(&repo, "p", &src2).unwrap();

    let s = cmd_status(&repo, "p").unwrap();
    assert_eq!(s.mst_len, 2);
    assert_eq!(s.attestations_len, 2);
}

#[test]
fn ingest_same_file_twice_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    cmd_init(&repo, "p").unwrap();

    let src = dir.path().join("f.rs");
    fs::write(&src, "fn f() -> i32 { 42 }").unwrap();

    let r1 = cmd_ingest(&repo, "p", &src).unwrap();
    let r2 = cmd_ingest(&repo, "p", &src).unwrap();
    assert_eq!(r1.alpha, r2.alpha);
    assert_eq!(r1.struct_hash, r2.struct_hash);

    let s = cmd_status(&repo, "p").unwrap();
    // El MST tiene 1 entrada (mismo hash). Atestaciones también: 1
    // por (autor, contenido) — idempotente.
    assert_eq!(s.mst_len, 1);
    assert_eq!(s.attestations_len, 1);
}

#[test]
fn rename_local_var_keeps_same_alpha_hash() {
    // Item #1 manifestándose: dos archivos Rust α-equivalentes (sólo
    // difieren en el nombre de la variable ligada) producen el mismo
    // α-hash → mismo MST → mismo "archivo" desde el punto de vista
    // del VCS semántico.
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    cmd_init(&repo, "p").unwrap();

    let a = dir.path().join("a.rs");
    fs::write(&a, "fn f() -> i32 { let x = 1; x }").unwrap();
    let b = dir.path().join("b.rs");
    fs::write(&b, "fn f() -> i32 { let y = 1; y }").unwrap();

    let r1 = cmd_ingest(&repo, "p", &a).unwrap();
    let r2 = cmd_ingest(&repo, "p", &b).unwrap();
    assert_eq!(
        r1.alpha, r2.alpha,
        "α-equivalencia: cambiar nombre de variable ligada no cambia el α-hash"
    );
    assert_ne!(
        r1.struct_hash, r2.struct_hash,
        "estructuralmente sí difieren (los leaf_text de los `identifier` son distintos)"
    );

    let s = cmd_status(&repo, "p").unwrap();
    assert_eq!(s.mst_len, 1, "una sola raíz canónica en el MST");
    assert_eq!(s.roots_len, 1);
}

#[test]
fn diff_detects_changes_between_versions() {
    use minga_cli::{cmd_diff, DiffLine};

    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    cmd_init(&repo, "p").unwrap();

    let v1 = dir.path().join("v1.rs");
    fs::write(&v1, "fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();
    let v2 = dir.path().join("v2.rs");
    // Genuinamente distinto (no sólo rename): cambia el cuerpo.
    fs::write(&v2, "fn add(a: i32, b: i32) -> i32 { a - b }").unwrap();

    let r1 = cmd_ingest(&repo, "p", &v1).unwrap();
    let r2 = cmd_ingest(&repo, "p", &v2).unwrap();
    assert_ne!(r1.alpha, r2.alpha, "cambio sustantivo cambia el α-hash");

    let d = cmd_diff(&repo, "p", &r1.alpha.to_string(), &r2.alpha.to_string()).unwrap();
    assert!(d.additions > 0 || d.deletions > 0, "debe haber cambios visibles");
    assert!(d.left_is_root && d.right_is_root, "ambos son raíces");
    assert!(d.lines.iter().any(|l| matches!(l, DiffLine::Add(_) | DiffLine::Remove(_))));
}

#[test]
fn retire_removes_root_and_persists_signed_retraction() {
    use minga_cli::{cmd_retire, cmd_status};

    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    cmd_init(&repo, "p").unwrap();

    let src = dir.path().join("f.rs");
    fs::write(&src, "fn f() -> i32 { 1 }").unwrap();
    let ing = cmd_ingest(&repo, "p", &src).unwrap();
    assert_eq!(cmd_status(&repo, "p").unwrap().roots_len, 1);

    let r = cmd_retire(&repo, "p", &ing.alpha.to_string()).unwrap();
    assert!(r.was_root);
    assert_eq!(r.alpha, ing.alpha);

    let s = cmd_status(&repo, "p").unwrap();
    assert_eq!(s.roots_len, 0, "raíz retirada del tree roots");
    assert_eq!(s.mst_len, 0, "raíz retirada del MST");
    // La atestación original NO se borra: sigue siendo prueba de
    // que el autor firmó este hash en algún momento.
    assert_eq!(s.attestations_len, 1);
}

#[test]
fn retire_unknown_hash_still_signs_negative_attestation() {
    use minga_cli::cmd_retire;
    // Útil para sync: un peer puede firmar "yo no respaldo X" sobre
    // un hash que llegó por la red sin que tenga que existir en su
    // tree local de roots.
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    cmd_init(&repo, "p").unwrap();

    let fake = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    let r = cmd_retire(&repo, "p", fake).unwrap();
    assert!(!r.was_root);
}

#[test]
fn verify_root_matches_dialect_used_to_ingest() {
    use minga_cli::cmd_verify_root;

    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    cmd_init(&repo, "p").unwrap();

    let src = dir.path().join("f.py");
    fs::write(&src, "def f():\n    return 1\n").unwrap();
    let ing = cmd_ingest(&repo, "p", &src).unwrap();

    let v = cmd_verify_root(&repo, "p", &ing.alpha.to_string()).unwrap();
    assert!(v.is_consistent(), "el α verificado debe matchear");
    assert_eq!(v.verified_dialect, Some(minga_core::parse::Dialect::Python));
    assert_eq!(v.stored_dialect, Some(minga_core::parse::Dialect::Python));
    assert!(v.matches_stored());
}
