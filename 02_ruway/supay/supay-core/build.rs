//! Build script de `supay-core`.
//!
//! Busca doomgeneric en `vendor/doomgeneric/doomgeneric/*.c`. Si está,
//! lo compila con `cc` y lo linkea. Si no, emite `cfg(doomgeneric_stub)`
//! para que `lib.rs` use stubs y el workspace siga compilando — los
//! consumidores deben proveer el código C corriendo:
//!
//! ```sh
//! cd 02_ruway/supay/supay-core/vendor
//! git clone https://github.com/ozkl/doomgeneric.git
//! ```

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Registrar la cfg custom para que rustc no warning-ee.
    println!("cargo::rustc-check-cfg=cfg(doomgeneric_stub)");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dg_dir = manifest.join("vendor/doomgeneric/doomgeneric");
    println!("cargo:rerun-if-changed={}", dg_dir.display());

    if !dg_dir.exists() {
        eprintln!(
            "cargo:warning=doomgeneric NO encontrado en {}",
            dg_dir.display()
        );
        eprintln!(
            "cargo:warning=Para activar el motor real, corré:"
        );
        eprintln!(
            "cargo:warning=  cd {} && git clone https://github.com/ozkl/doomgeneric.git",
            manifest.join("vendor").display()
        );
        eprintln!("cargo:warning=Compilando supay-core como stub (Doom no corre).");
        println!("cargo:rustc-cfg=doomgeneric_stub");
        return;
    }

    // Reúne todos los .c relevantes. Excluimos los `doomgeneric_*.c`
    // específicos de host (SDL, Windows, etc.) porque proveemos
    // nuestros propios callbacks en lib.rs. Mantenemos `doomgeneric.c`
    // (el core común) e `i_*.c` neutrales.
    let mut sources: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&dg_dir).unwrap_or_else(|e| {
        panic!("read_dir {}: {}", dg_dir.display(), e)
    }) {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("c") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        // Mantener `doomgeneric.c` (loop común, define DG_ScreenBuffer)
        // pero excluir los `doomgeneric_<plataforma>.c` (sdl/windows/x11).
        if name.starts_with("doomgeneric_") && name != "doomgeneric.c" {
            continue;
        }
        // Excluir mains de plataforma si quedan colgados.
        if name == "main.c" {
            continue;
        }
        sources.push(path);
    }

    if sources.is_empty() {
        eprintln!(
            "cargo:warning=vendor/doomgeneric existe pero no contiene .c — \
             ¿el clone falló? Compilando como stub."
        );
        println!("cargo:rustc-cfg=doomgeneric_stub");
        return;
    }

    let mut build = cc::Build::new();
    build
        .files(&sources)
        .include(&dg_dir)
        // doomgeneric tiene MUCHOS warnings legacy del id1 — los apagamos.
        .flag_if_supported("-w")
        .flag_if_supported("-Wno-everything")
        .flag_if_supported("-Wno-format")
        .flag_if_supported("-Wno-pointer-sign")
        // FEATURE_SOUND off: nuestro host no tiene audio cableado.
        .define("FEATURE_SOUND", None);

    // En Linux/glibc doomgeneric necesita _GNU_SOURCE para mkstemp, etc.
    if cfg!(target_os = "linux") {
        build.define("_GNU_SOURCE", None);
    }

    build.compile("doomgeneric");
}
