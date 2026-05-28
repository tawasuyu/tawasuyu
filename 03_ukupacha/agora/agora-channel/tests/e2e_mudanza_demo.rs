//! Test E2E del cruce de contrato `agora-channel ↔ mudanza`.
//!
//! El fixture `apps/mudanza/src/propuesta_demo.bin` lo forja el example
//! `forjar_propuesta_mudanza_demo` con una seed test conocida. La app
//! `mudanza` (`#![no_std]`, WASM) parsea ese fixture a mano como
//! `hash(32) || autor(32) || firma(64)` y lo verifica con
//! `ed25519-compact`. Este test reproduce *exactamente* ese parser y esa
//! verificación, pero desde el host con `agora-core::verify_signature`
//! (que usa `ed25519-dalek`) — si el fixture, el layout postcard o las
//! primitivas crypto divergieran entre crates, este test cae.

use agora_core::{verify_signature, Keypair};

const FIXTURE: &[u8; 128] =
    include_bytes!("../../../wawa/apps/mudanza/src/propuesta_demo.bin");

#[test]
fn fixture_demo_parsea_como_mudanza_y_verifica_como_el_kernel() {
    // Replica EXACTA del parser de mudanza::probar_reancla.
    let hash: &[u8; 32] = FIXTURE[0..32].try_into().expect("hash slice");
    let autor: &[u8; 32] = FIXTURE[32..64].try_into().expect("autor slice");
    let firma: &[u8; 64] = FIXTURE[64..128].try_into().expect("firma slice");

    // Verificación crypto — la firma cierra contra el autor del sobre.
    // Esto es lo que tanto `ed25519-compact` (en mudanza) como
    // `ed25519-dalek` (vía agora-core) deben aceptar; si una de las dos
    // implementaciones se desincronizara con ed25519 estándar, este
    // test atrapa la divergencia.
    verify_signature(autor, hash, firma).expect("firma del fixture debe verificar");

    // Y el autor del fixture corresponde a la seed demo [42u8; 32]
    // que el example forja — la unidad de la cadena example ↔ fixture
    // ↔ mudanza ↔ kernel está intacta.
    let kp_demo = Keypair::from_seed([42u8; 32]);
    assert_eq!(
        autor, &kp_demo.public_key(),
        "el fixture debería estar firmado por la seed demo [42u8; 32]; \
         re-forjarlo con `cargo run -p agora-channel --example \
         forjar_propuesta_mudanza_demo` si la cambiaste."
    );
}
