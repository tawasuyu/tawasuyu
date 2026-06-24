//! `churay-bundle` — forja el **bundle precompilado** (lado A) que consume el
//! instalador: copia los binarios de release de cada unidad del catálogo,
//! calcula su hash BLAKE3 + tamaño, y emite un manifiesto (firmado con ed25519
//! si se da una semilla).
//!
//! Uso:
//!   churay-bundle <out_dir> [--release-dir <dir>]
//!   CHURAY_SIGN_SEED=<hex-64>  churay-bundle <out_dir>   # firma el manifiesto
//!
//! No compila nada: asume que los binarios ya están en `target/release` (los
//! produce `cargo build --release --bin <prog>` o el script del bundle). Las
//! unidades sin binario presente se omiten con un aviso.

use std::path::{Path, PathBuf};

use churay_core::manifest::Manifest;
use churay_core::{suite_catalog, ArtifactHash};

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = match args.next() {
        Some(d) => PathBuf::from(d),
        None => {
            eprintln!("uso: churay-bundle <out_dir> [--release-dir <dir>]");
            std::process::exit(2);
        }
    };
    let mut release_dir: Option<PathBuf> = None;
    while let Some(flag) = args.next() {
        if flag == "--release-dir" {
            release_dir = args.next().map(PathBuf::from);
        }
    }
    let release_dir = release_dir.unwrap_or_else(default_release_dir);

    let bin_out = out_dir.join("bin");
    let blobs_out = out_dir.join("blobs");
    for d in [&bin_out, &blobs_out] {
        if let Err(e) = std::fs::create_dir_all(d) {
            eprintln!("no se pudo crear {}: {e}", d.display());
            std::process::exit(1);
        }
    }

    let mut units = suite_catalog();
    let mut incluidas = 0usize;
    let mut omitidas = Vec::new();

    for u in units.iter_mut() {
        let src = release_dir.join(&u.program);
        if !src.exists() {
            omitidas.push(u.program.clone());
            continue;
        }
        let dst = bin_out.join(&u.program);
        if let Err(e) = std::fs::copy(&src, &dst) {
            eprintln!("× {}: {e}", u.program);
            omitidas.push(u.program.clone());
            continue;
        }
        let bytes = std::fs::read(&dst).expect("releer binario copiado");
        let hash = ArtifactHash::of_bytes(&bytes);
        // Espejo direccionado por contenido: `blobs/<hex>`, lo que sirve el repo
        // remoto. Un bundle servido por HTTP es, así, un CHURAY_REPO válido.
        let hex = hash.as_str().strip_prefix("b3:").unwrap_or(hash.as_str());
        let _ = std::fs::copy(&dst, blobs_out.join(hex));
        u.bin_hash = Some(hash);
        u.size_bytes = Some(bytes.len() as u64);
        incluidas += 1;
        println!("✓ {}  ({} bytes)", u.program, bytes.len());
    }

    // Sólo las unidades que efectivamente entraron al bundle van al manifiesto.
    let presentes: Vec<_> = units.into_iter().filter(|u| u.bin_hash.is_some()).collect();
    let manifest = Manifest::new(churay_core::SUITE_VERSION, presentes);

    // Manifiesto sin firmar, siempre.
    let plain = serde_json::to_string_pretty(&manifest).expect("manifest serializa");
    write_or_die(&out_dir.join("manifest.json"), &plain);

    // Manifiesto firmado, si hay semilla.
    if let Some(seed) = sign_seed() {
        let kp = agora_core::Keypair::from_seed(seed);
        let signed = manifest.sign(&kp);
        write_or_die(&out_dir.join("manifest.signed.json"), &signed.to_json());
        write_or_die(
            &out_dir.join("pubkey.hex"),
            &hex(&kp.public_key()),
        );
        println!("firmado por {}", hex(&kp.public_key()));
    }

    println!(
        "\nbundle en {} — {} unidad(es) incluidas, {} omitidas",
        out_dir.display(),
        incluidas,
        omitidas.len()
    );
    if !omitidas.is_empty() {
        println!("omitidas (sin binario en {}): {}", release_dir.display(), omitidas.join(", "));
    }
}

fn default_release_dir() -> PathBuf {
    // Sube desde el cwd buscando un Cargo.toml de workspace; usa su target/release.
    let mut cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        let manifest = cur.join("Cargo.toml");
        if manifest.exists() {
            if let Ok(txt) = std::fs::read_to_string(&manifest) {
                if txt.contains("[workspace]") {
                    return cur.join("target").join("release");
                }
            }
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => return PathBuf::from("target/release"),
        }
    }
}

fn sign_seed() -> Option<[u8; 32]> {
    let hexs = std::env::var("CHURAY_SIGN_SEED").ok()?;
    let v = (0..hexs.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hexs.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    v.try_into().ok()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn write_or_die(path: &Path, contents: &str) {
    if let Err(e) = std::fs::write(path, contents) {
        eprintln!("no se pudo escribir {}: {e}", path.display());
        std::process::exit(1);
    }
}
