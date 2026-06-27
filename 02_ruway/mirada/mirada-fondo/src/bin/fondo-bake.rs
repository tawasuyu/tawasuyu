//! `fondo-bake` — pre-renderiza un fondo Lottie/rive a la cache de frames.
//!
//! Lo invoca wawa-panel (o se corre a mano) cuando el usuario elige un Lottie/
//! rive como fondo de una de las tres superficies: abre la GPU una vez, renderiza
//! el loop a PNG en `~/.cache/mirada/fondo/<clave>/` y sale. Después el splash y
//! el compositor —sin vello— sólo bliteant esos frames.
//!
//! Uso:
//!
//! ```text
//! fondo-bake lottie <ruta.json> [--w 1280] [--h 720] [--fps 30] [--loop 6]
//! fondo-bake rive   <ruta.ron>  [--w 1280] [--h 720] [--fps 30] [--loop 6]
//! ```

use std::process::ExitCode;

use mirada_fondo::bake::{bake, BakeOpts};
use mirada_fondo::FondoSpec;

fn main() -> ExitCode {
    bitacora::abrir("mirada");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!(
            "uso: fondo-bake <lottie|rive> <ruta> [--w N] [--h N] [--fps N] [--loop SEGS]"
        );
        return ExitCode::FAILURE;
    }
    let kind = args[0].as_str();
    let path = args[1].clone();
    let spec = match kind {
        "lottie" => FondoSpec::Lottie { path },
        "rive" => FondoSpec::Rive { path },
        other => {
            eprintln!("fondo-bake: tipo desconocido «{other}» (esperaba lottie|rive)");
            return ExitCode::FAILURE;
        }
    };

    let mut opts = BakeOpts::default();
    let mut it = args[2..].iter();
    while let Some(flag) = it.next() {
        let val = it.next();
        match (flag.as_str(), val) {
            ("--w", Some(v)) => opts.width = v.parse().unwrap_or(opts.width),
            ("--h", Some(v)) => opts.height = v.parse().unwrap_or(opts.height),
            ("--fps", Some(v)) => opts.fps = v.parse().unwrap_or(opts.fps),
            ("--loop", Some(v)) => opts.loop_secs = v.parse().ok(),
            _ => {
                eprintln!("fondo-bake: flag inválido «{flag}»");
                return ExitCode::FAILURE;
            }
        }
    }

    match bake(&spec, &opts) {
        Ok((dir, meta)) => {
            println!(
                "{} frames {}x{} @ {} fps → {}",
                meta.frame_count,
                meta.width,
                meta.height,
                meta.fps,
                dir.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("fondo-bake: {e}");
            ExitCode::FAILURE
        }
    }
}
