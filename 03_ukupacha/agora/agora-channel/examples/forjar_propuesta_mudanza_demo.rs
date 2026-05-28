//! Forja el sobre demo que `apps/mudanza` lleva embebido vía
//! `include_bytes!("propuesta_demo.bin")`.
//!
//! El sobre es un `format::ManifiestoFirmado` postcard de 128 bytes:
//! `manifiesto_hash(32) || autor(32) || firma(64)`. La firma es real,
//! producida por una seed de DEMO (no del `AGORA_AUTH_RING` del kernel).
//! La app verifica criptográficamente que la firma cierra contra el
//! `autor`, llama al syscall, y el kernel rechaza con `CapacidadInsuficiente`
//! porque la pubkey demo no habita el anillo soberano.
//!
//! Corré con:
//!   cargo run -p agora-channel --example forjar_propuesta_mudanza_demo
//!
//! Re-escribe `03_ukupacha/wawa/apps/mudanza/src/propuesta_demo.bin`.

use std::path::PathBuf;

use agora_core::Keypair;
use agora_channel::firmar_manifiesto;

fn main() {
    // Seed determinista para la demo. NO es una clave de producción —
    // sólo sirve para mostrar que la app verifica firmas reales.
    let kp = Keypair::from_seed([42u8; 32]);

    // Hash de un "manifiesto" sintético: BLAKE3 de la cadena
    // "agora-mudanza-demo-manifiesto" — basta con que sea un hash
    // estable y distinguible. El kernel no va a aceptar el sobre de
    // todas formas (autor fuera del anillo).
    let manifiesto_hash = *blake3::hash(b"agora-mudanza-demo-manifiesto").as_bytes();

    let mf = firmar_manifiesto(&kp, &manifiesto_hash);

    // postcard de ManifiestoFirmado = 32 + 32 + 64 = 128 bytes raw.
    let bytes = mf.serializar().expect("serializar manifiesto_firmado");
    assert_eq!(
        bytes.len(),
        128,
        "ManifiestoFirmado postcard debe ser 128 B (got {})",
        bytes.len()
    );

    let salida: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("wawa/apps/mudanza/src/propuesta_demo.bin");
    std::fs::write(&salida, &bytes).expect("escribir propuesta_demo.bin");
    println!("forjado: {} bytes en {}", bytes.len(), salida.display());
    println!("  manifiesto_hash: {}", hex(&mf.manifiesto_hash));
    println!("  autor          : {}", hex(&mf.autor));
    println!("  (seed demo: [42u8; 32] — NO es clave de producción)");
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}
