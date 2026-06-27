//! `mirada-plugin-sign` — genera claves y firma plugins para el anillo de
//! confianza del host.
//!
//! ```text
//! mirada-plugin-sign keygen [--out key.seed]
//!     Genera un par Ed25519. Guarda la semilla (hex) en --out y muestra la
//!     pubkey `ed25519:…` para pegar en trust.ron.
//!
//! mirada-plugin-sign sign --seed key.seed --wasm plugin.wasm --caps keys,spawn
//!     Firma `blake3(wasm) ‖ caps` y muestra las líneas `signer:`/`signature:`
//!     para pegar en el .ron del plugin.
//! ```

use std::process::exit;

use mirada_plugin_host::caps::parse_cap;
use mirada_plugin_host::trust::grant_message;

fn main() {
    bitacora::abrir("mirada");
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("keygen") => keygen(&args[2..]),
        Some("sign") => sign(&args[2..]),
        _ => {
            eprintln!("uso: mirada-plugin-sign <keygen|sign> …");
            eprintln!("  keygen [--out key.seed]");
            eprintln!("  sign --seed key.seed --wasm plugin.wasm --caps keys,spawn");
            exit(2);
        }
    }
}

/// Busca `--flag valor` en los argumentos.
fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn die(msg: impl AsRef<str>) -> ! {
    eprintln!("error: {}", msg.as_ref());
    exit(1);
}

/// 32 bytes de aleatoriedad del kernel (Linux). El compositor es Linux-only.
fn random_seed() -> [u8; 32] {
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom").unwrap_or_else(|e| die(format!("/dev/urandom: {e}")));
    let mut seed = [0u8; 32];
    f.read_exact(&mut seed).unwrap_or_else(|e| die(format!("leyendo entropía: {e}")));
    seed
}

fn keygen(args: &[String]) {
    let out = flag(args, "--out").unwrap_or("key.seed");
    let seed = random_seed();
    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
    let pubkey = signing.verifying_key().to_bytes();

    std::fs::write(out, hex::encode(seed)).unwrap_or_else(|e| die(format!("escribiendo {out}: {e}")));
    // La semilla es secreta: permisos 0600.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o600));
    }

    println!("semilla privada → {out}  (¡guardala, no la compartas!)");
    println!();
    println!("pubkey para ~/.config/mirada/plugins/trust.ron:");
    println!("    \"ed25519:{}\",", hex::encode(pubkey));
}

fn sign(args: &[String]) {
    let seed_path = flag(args, "--seed").unwrap_or_else(|| die("falta --seed"));
    let wasm_path = flag(args, "--wasm").unwrap_or_else(|| die("falta --wasm"));
    let caps_str = flag(args, "--caps").unwrap_or("");

    let seed_hex = std::fs::read_to_string(seed_path)
        .unwrap_or_else(|e| die(format!("leyendo {seed_path}: {e}")));
    let seed: [u8; 32] = hex::decode(seed_hex.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or_else(|| die("la semilla no es hex de 32 bytes"));
    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);

    let wasm = std::fs::read(wasm_path).unwrap_or_else(|e| die(format!("leyendo {wasm_path}: {e}")));

    let mut caps: u32 = 0;
    for c in caps_str.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match parse_cap(c) {
            Some(bit) => caps |= bit,
            None => die(format!("capacidad desconocida: {c:?}")),
        }
    }

    let msg = grant_message(&wasm, caps);
    use ed25519_dalek::Signer;
    let sig = signing.sign(&msg).to_bytes();
    let pubkey = signing.verifying_key().to_bytes();

    println!("// pegá esto en el .ron del plugin (caps deben coincidir):");
    println!("    signer: \"ed25519:{}\",", hex::encode(pubkey));
    println!("    signature: \"{}\",", hex::encode(sig));
}
