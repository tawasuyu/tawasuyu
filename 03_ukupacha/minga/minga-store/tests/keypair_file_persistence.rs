//! Tests de persistencia del keypair cifrado en disco.

use minga_core::Keypair;
use minga_store::keypair_file;
use tempfile::TempDir;

#[test]
fn save_then_load_preserves_identity() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("keypair");

    let original = Keypair::from_seed(&[7; 32]);
    keypair_file::save(&original, &path, "secreto42").unwrap();

    let loaded = keypair_file::load(&path, "secreto42").unwrap();
    assert_eq!(loaded.did(), original.did());

    let msg = b"el peer sigue siendo el mismo";
    let sig = loaded.sign(msg);
    assert!(original.did().verify(msg, &sig));
}

#[test]
fn load_with_wrong_passphrase_errors() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("keypair");

    let kp = Keypair::from_seed(&[3; 32]);
    keypair_file::save(&kp, &path, "correcta").unwrap();

    let r = keypair_file::load(&path, "incorrecta");
    assert!(r.is_err());
}

#[test]
fn load_missing_file_errors() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("no-existe");
    let r = keypair_file::load(&path, "x");
    assert!(r.is_err());
}

#[test]
fn save_overwrites_existing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("keypair");

    let first = Keypair::from_seed(&[1; 32]);
    keypair_file::save(&first, &path, "pass").unwrap();

    let second = Keypair::from_seed(&[2; 32]);
    keypair_file::save(&second, &path, "pass").unwrap();

    let loaded = keypair_file::load(&path, "pass").unwrap();
    assert_eq!(loaded.did(), second.did());
    assert_ne!(loaded.did(), first.did());
}

#[test]
fn file_size_is_compact() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("keypair");
    keypair_file::save(&Keypair::from_seed(&[5; 32]), &path, "p").unwrap();
    let size = std::fs::metadata(&path).unwrap().len();
    // 8 magic + 1 version + 16 salt + 12 nonce + 32 secret + 16 tag = 85.
    assert_eq!(size, 85);
}
