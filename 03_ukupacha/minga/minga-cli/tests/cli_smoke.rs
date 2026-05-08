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
    assert_eq!(r1.hash, r2.hash);

    let s = cmd_status(&repo, "p").unwrap();
    // El MST tiene 1 entrada (mismo hash). Atestaciones también: 1
    // por (autor, contenido) — idempotente.
    assert_eq!(s.mst_len, 1);
    assert_eq!(s.attestations_len, 1);
}
