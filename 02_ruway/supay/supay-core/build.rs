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

    // Reúne todos los .c relevantes. Excluimos:
    // - `doomgeneric_<plataforma>.c` (sdl/windows/x11/emscripten):
    //   traen su propio main + callbacks que chocan con los nuestros.
    // - `main.c` por la misma razón.
    // - Backends de audio opcionales `i_<lib>music.c` / `i_<lib>sound.c`
    //   que dependen de SDL, Allegro, ALSA, etc. Sin esas libs en el
    //   sistema no compilan, y nuestro host no tiene audio cableado.
    //   Chocolate Doom (que doomgeneric hereda) tiene un dispatcher
    //   `i_sound.c` que cae a no-op cuando ningún backend está activo,
    //   así que excluirlos no rompe el motor.
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let blocked_substrings = ["<SDL", "<allegro", "<emscripten"];
    let blocked_filenames: &[&str] = &[
        "i_sdlsound.c",
        "i_sdlmusic.c",
        "i_allegrosound.c",
        "i_allegromusic.c",
    ];
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
        if name.starts_with("doomgeneric_") && name != "doomgeneric.c" {
            continue;
        }
        if name == "main.c" {
            continue;
        }
        if blocked_filenames.contains(&name.as_str()) {
            skipped.push(name.clone());
            continue;
        }
        // Backstop: cualquier source con includes de libs externas
        // top-level (no protegidos por `#ifdef _WIN32` etc.).
        if let Ok(contents) = std::fs::read_to_string(&path) {
            let top_level_offender = blocked_substrings
                .iter()
                .any(|needle| contents.contains(needle));
            // Filtramos sólo los archivos cuyo nombre indica que son
            // backends de plataforma (no los .c del core que pueden
            // tener `#include <windows.h>` protegido por `#ifdef`).
            if top_level_offender
                && (name.starts_with("i_") && (name.contains("sound") || name.contains("music")))
            {
                skipped.push(name.clone());
                continue;
            }
        }
        sources.push(path);
    }
    for s in &skipped {
        println!("cargo:warning=skip {} (external lib backend)", s);
    }

    if sources.is_empty() {
        eprintln!(
            "cargo:warning=vendor/doomgeneric existe pero no contiene .c — \
             ¿el clone falló? Compilando como stub."
        );
        println!("cargo:rustc-cfg=doomgeneric_stub");
        return;
    }

    // Stubs no-op de la API de audio que `i_sound.c` proveería si lo
    // compiláramos. Como ese .c arrastra `<SDL_mixer.h>` lo filtramos
    // del build y resolvemos sus símbolos desde acá.
    let stubs = manifest.join("src/audio_stubs.c");
    println!("cargo:rerun-if-changed={}", stubs.display());

    // Fase 2: getters de estado interno (player, walls, sectors, mobjs)
    // que `supay-scene` consume desde Rust. Sólo tiene sentido si
    // doomgeneric está presente — incluye headers del motor.
    let scene = manifest.join("src/scene_export.c");
    println!("cargo:rerun-if-changed={}", scene.display());

    let mut build = cc::Build::new();
    build
        .files(&sources)
        .file(&stubs)
        .file(&scene)
        .include(&dg_dir)
        // doomgeneric tiene MUCHOS warnings legacy del id1 — los apagamos.
        .flag_if_supported("-w")
        .flag_if_supported("-Wno-everything")
        .flag_if_supported("-Wno-format")
        .flag_if_supported("-Wno-pointer-sign");

    // En Linux/glibc doomgeneric necesita _GNU_SOURCE para mkstemp, etc.
    if cfg!(target_os = "linux") {
        build.define("_GNU_SOURCE", None);
    }

    build.compile("doomgeneric");
}
