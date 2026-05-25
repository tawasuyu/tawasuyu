//! Invariantes de la identidad criptográfica: roundtrip de firma,
//! determinismo desde semilla, detección de manipulaciones.

use minga_core::{Did, Keypair, KeypairCryptoError, Signature};

fn kp(seed: u8) -> Keypair {
    Keypair::from_seed(&[seed; 32])
}

#[test]
fn keypair_from_seed_is_deterministic() {
    let a = kp(7);
    let b = kp(7);
    assert_eq!(a.did(), b.did());
    let msg = b"hola minga";
    assert_eq!(a.sign(msg), b.sign(msg));
}

#[test]
fn distinct_seeds_produce_distinct_dids() {
    let a = kp(1);
    let b = kp(2);
    assert_ne!(a.did(), b.did());
}

#[test]
fn generate_produces_unique_dids() {
    // Dos `generate()` consecutivos deben dar DIDs distintos con
    // probabilidad abrumadora (chance de colisión ≈ 2^-256).
    let a = Keypair::generate();
    let b = Keypair::generate();
    assert_ne!(a.did(), b.did());
}

#[test]
fn sign_verify_roundtrip() {
    let k = kp(42);
    let msg = b"mensaje arbitrario de longitud variable, con UTF-8: cafe \xc3\xa9";
    let sig = k.sign(msg);
    assert!(k.did().verify(msg, &sig));
}

#[test]
fn verify_fails_with_wrong_did() {
    let signer = kp(10);
    let msg = b"contenido";
    let sig = signer.sign(msg);
    let imposter = kp(11).did();
    assert!(!imposter.verify(msg, &sig));
}

#[test]
fn verify_fails_with_tampered_message() {
    let k = kp(99);
    let sig = k.sign(b"mensaje original");
    assert!(!k.did().verify(b"mensaje modificado", &sig));
}

#[test]
fn verify_fails_with_tampered_signature() {
    let k = kp(99);
    let mut sig = k.sign(b"x");
    sig.0[0] ^= 0xFF;
    assert!(!k.did().verify(b"x", &sig));
}

#[test]
fn verify_handles_invalid_did_bytes() {
    // Did con bytes que no forman un punto válido en la curva debería
    // fallar verificación silenciosamente (sin pánico).
    let bogus_did = Did([0xFF; 32]);
    let sig = Signature([0u8; 64]);
    assert!(!bogus_did.verify(b"anything", &sig));
}

#[test]
fn did_display_uses_did_key_prefix() {
    let did = kp(0).did();
    let s = format!("{}", did);
    assert!(s.starts_with("did:key:"));
    assert_eq!(s.len(), "did:key:".len() + 64); // 32 bytes en hex = 64 chars
}

#[test]
fn encrypt_decrypt_roundtrip_preserves_identity() {
    let original = kp(7);
    let blob = original.encrypt("contraseña-correcta").unwrap();
    let restored = Keypair::decrypt(&blob, "contraseña-correcta").unwrap();

    // El DID se preserva: misma identidad pública.
    assert_eq!(original.did(), restored.did());

    // Y la capacidad de firmar — un mensaje firmado por uno verifica
    // contra el DID del otro (porque son la misma llave).
    let msg = b"prueba post-cifrado";
    let sig_original = original.sign(msg);
    let sig_restored = restored.sign(msg);
    assert_eq!(sig_original, sig_restored);
    assert!(restored.did().verify(msg, &sig_original));
}

#[test]
fn decrypt_with_wrong_passphrase_fails() {
    let kp = kp(11);
    let blob = kp.encrypt("correcta").unwrap();
    let r = Keypair::decrypt(&blob, "incorrecta");
    assert!(matches!(r, Err(KeypairCryptoError::DecryptFailed)));
}

#[test]
fn decrypt_rejects_tampered_ciphertext() {
    // AES-GCM es authenticated: cualquier modificación del cipher
    // (incluyendo el tag) hace fallar la verificación.
    let kp = kp(13);
    let mut blob = kp.encrypt("pass").unwrap();
    let last = blob.len() - 1;
    blob[last] ^= 0xFF;
    let r = Keypair::decrypt(&blob, "pass");
    assert!(matches!(r, Err(KeypairCryptoError::DecryptFailed)));
}

#[test]
fn decrypt_rejects_invalid_format() {
    assert!(matches!(
        Keypair::decrypt(b"too short", "x"),
        Err(KeypairCryptoError::InvalidFormat)
    ));
    let mut bogus = vec![0xFFu8; 100];
    bogus[0..8].copy_from_slice(b"NOTMINGA");
    assert!(matches!(
        Keypair::decrypt(&bogus, "x"),
        Err(KeypairCryptoError::InvalidFormat)
    ));
}

#[test]
fn distinct_passphrases_produce_distinct_blobs() {
    // Cifrar la misma key con dos passphrases distintas produce blobs
    // distintos (también porque salt y nonce son aleatorios — no es
    // determinismo, es solo que no colisionan).
    let kp = kp(17);
    let a = kp.encrypt("alpha").unwrap();
    let b = kp.encrypt("beta").unwrap();
    assert_ne!(a, b);
}

#[test]
fn re_encrypting_same_keypair_produces_distinct_blobs() {
    // Salt y nonce aleatorios: el mismo keypair y la misma passphrase
    // producen cipher distintos en cada llamada. Sin patrón observable.
    let kp = kp(19);
    let blob1 = kp.encrypt("p").unwrap();
    let blob2 = kp.encrypt("p").unwrap();
    assert_ne!(blob1, blob2);
    // Pero ambos descifran a la misma identidad.
    assert_eq!(
        Keypair::decrypt(&blob1, "p").unwrap().did(),
        Keypair::decrypt(&blob2, "p").unwrap().did()
    );
}

#[test]
fn keypair_debug_does_not_leak_private_key() {
    // El derive de Debug expondría los bytes secretos. Lo
    // sobreescribimos para que solo muestre el DID.
    let k = kp(1);
    let s = format!("{:?}", k);
    assert!(s.contains("did:key:"));
    // No debería aparecer ningún byte de la semilla [1u8; 32] en hex
    // contiguo (fragmento "010101..." sería sospechoso si emergiera).
    assert!(!s.contains("0101010101010101"));
}
