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
use std::path::{Path, PathBuf};

/// Clona doomgeneric en `vendor/` automáticamente (shallow). Best-effort:
/// si `git` no está, no hay red, o ya existe el directorio destino, no
/// rompe el build — simplemente no deja el clone y el caller cae a stub.
///
/// Se puede desactivar con `SUPAY_NO_AUTOCLONE=1` (p.ej. en CI offline o
/// para forzar el modo stub a propósito).
fn try_clone_doomgeneric(vendor: &Path) {
    if env::var_os("SUPAY_NO_AUTOCLONE").is_some() {
        eprintln!("cargo:warning=SUPAY_NO_AUTOCLONE — salto el clone automático de doomgeneric");
        return;
    }
    let dest = vendor.join("doomgeneric");
    if dest.exists() {
        return; // un clone a medias o ajeno; no lo tocamos
    }
    if let Err(e) = std::fs::create_dir_all(vendor) {
        eprintln!("cargo:warning=no pude crear {}: {e}", vendor.display());
        return;
    }
    eprintln!("cargo:warning=doomgeneric ausente — clonándolo automáticamente (una vez)…");
    let status = std::process::Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "https://github.com/ozkl/doomgeneric.git",
        ])
        .arg(&dest)
        .status();
    match status {
        Ok(s) if s.success() => {
            eprintln!("cargo:warning=doomgeneric clonado OK — compilando el motor real.");
        }
        Ok(s) => {
            eprintln!("cargo:warning=git clone de doomgeneric falló (status {s}).");
            // Limpiamos un clone parcial para reintentar limpio la próxima.
            let _ = std::fs::remove_dir_all(&dest);
        }
        Err(e) => {
            eprintln!("cargo:warning=no pude ejecutar git ({e}). ¿git instalado / hay red?");
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Registrar la cfg custom para que rustc no warning-ee.
    println!("cargo::rustc-check-cfg=cfg(doomgeneric_stub)");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor = manifest.join("vendor");
    let dg_dir = manifest.join("vendor/doomgeneric/doomgeneric");
    println!("cargo:rerun-if-changed={}", dg_dir.display());

    // El motor C de doomgeneric es un clone gitignored (no trackeado). Si
    // falta, antes caíamos a stub y la app pedía clonar a mano en la
    // terminal. Ahora lo clonamos automáticamente acá — durante el `cargo
    // build` que el usuario ya corre — para que el motor real quede siempre
    // linkeado sin pasos manuales. Si no hay red/git (offline), avisamos y
    // caemos a stub como antes; reintenta en el próximo build.
    if !dg_dir.exists() {
        try_clone_doomgeneric(&vendor);
    }

    if !dg_dir.exists() {
        eprintln!(
            "cargo:warning=doomgeneric NO encontrado en {} y el clone automático no funcionó.",
            dg_dir.display()
        );
        eprintln!("cargo:warning=Para activar el motor real, corré (con red):");
        eprintln!(
            "cargo:warning=  cd {} && git clone --depth 1 https://github.com/ozkl/doomgeneric.git",
            vendor.display()
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

    // Fase 4.5: parche idempotente de oclusión acústica. El vendored
    // (`vendor/doomgeneric/`) es un clone gitignored y NO trackeado, así
    // que una edición manual a `s_sound.c` se perdería en un clone fresco.
    // En cambio inyectamos acá la sonda — la autoridad vive en código
    // trackeado (este build.rs) y se reaplica sobre cualquier clone.
    patch_sound_origin_probe(&dg_dir);

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

/// Fase 4.5 — inyecta una sonda observacional en `s_sound.c` del clone
/// vendored para capturar la posición de la fuente de cada sfx (necesaria
/// para la oclusión acústica). Es el único punto donde el `mobj` origen
/// está disponible: `I_StartSound` sólo recibe `vol`/`sep`.
///
/// Idempotente: si el marcador `SUPAY 4.5` ya está presente (clone ya
/// parchado), no hace nada. La sonda llama a `supay_snd_set_origin`
/// (definida en `audio_stubs.c`) justo antes de `I_StartSound`. No toca
/// lógica/timing/RNG del motor — sólo lee `origin->{x,y}`.
///
/// Si el archivo o el ancla no existen (upstream cambió la línea), avisa
/// con un warning y sigue: el feature degrada a "sin oclusión" (todos los
/// sfx quedan secos), no rompe el build.
fn patch_sound_origin_probe(dg_dir: &std::path::Path) {
    let path = dg_dir.join("s_sound.c");
    let Ok(src) = std::fs::read_to_string(&path) else {
        println!("cargo:warning=Fase 4.5: no pude leer s_sound.c — sin oclusión");
        return;
    };
    if src.contains("SUPAY 4.5") {
        return; // ya parchado
    }
    // Ancla: la llamada a I_StartSound en S_StartSound (no en S_UpdateSounds,
    // que usa I_UpdateSoundParams). Es única en el archivo upstream.
    let anchor = "channels[cnum].handle = I_StartSound(sfx, cnum, volume, sep);";
    if !src.contains(anchor) {
        println!(
            "cargo:warning=Fase 4.5: ancla I_StartSound no encontrada en \
             s_sound.c (¿upstream cambió?) — sin oclusión"
        );
        return;
    }
    let probe = "\
/* SUPAY 4.5: sonda observacional de oclusión acústica (inyectada por
 * build.rs). Registra la posición de la fuente justo antes de I_StartSound,
 * el único punto donde `origin` está disponible. origin == jugador ⇒ has=0
 * (sin oclusión: el arma suena seca). No toca lógica/timing/RNG. */
    {
        extern void supay_snd_set_origin(int has, int x, int y);
        int supay_has = origin != NULL && origin != players[consoleplayer].mo;
        supay_snd_set_origin(supay_has,
                             supay_has ? origin->x : 0,
                             supay_has ? origin->y : 0);
    }
    ";
    let patched = src.replacen(anchor, &format!("{probe}{anchor}"), 1);
    if let Err(e) = std::fs::write(&path, patched) {
        println!("cargo:warning=Fase 4.5: no pude escribir s_sound.c parchado: {e}");
    }
}
