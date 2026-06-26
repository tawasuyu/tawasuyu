//! Diag headless del path de decodificación del wallpaper en video.
//!
//! Reproduce lo que hace el worker (`drm_backend/video_wallpaper.rs`): abre el
//! archivo con `foreign_av` (probe → session → source) y decodifica unos frames
//! a RGBA, imprimiendo **stats numéricas** (dimensiones, bytes no-cero, variedad
//! de color, y que el `seek_to(0)` del loop reengancha). Certifica el
//! decode+readback en texto, sin GPU ni compositor (regla 8).
//!
//! Uso: `cargo run -p mirada-compositor --example video_wallpaper_decode_diag -- <archivo.mp4>`
//! (sin argumento usa el ejemplo de `assets/`).

use std::time::Duration;

use media_core::{FrameSource, Seekable};

fn color_buckets(rgba: &[u8]) -> usize {
    // Cuantizá cada pixel a 4 bits por canal (16³ buckets) y contá distintos.
    let mut seen = std::collections::HashSet::new();
    for px in rgba.chunks_exact(4) {
        let key = ((px[0] >> 4) as u32) << 8 | ((px[1] >> 4) as u32) << 4 | (px[2] >> 4) as u32;
        seen.insert(key);
    }
    seen.len()
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/wallpaper-ejemplo-animado.mp4").to_string()
    });
    println!("archivo: {path}");

    let info = match foreign_av::probe(&path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("probe falló: {e}");
            std::process::exit(1);
        }
    };
    let session = foreign_av::MediaSession::open(info).expect("MediaSession::open");
    let mut source =
        foreign_av::FfmpegVideoSource::from_session(session).expect("FfmpegVideoSource");
    println!("fps nativo: {:.2}", source.fps());

    let mut buf = Vec::new();
    let mut first: Option<(Vec<u8>, u32, u32)> = None;
    let mut decoded = 0u32;
    for n in 0..6 {
        match source.step_frame(&mut buf) {
            Some((w, h)) => {
                let nonzero = buf.iter().filter(|&&b| b != 0).count();
                let buckets = color_buckets(&buf);
                println!(
                    "frame {n}: {w}x{h}  bytes={}  no-cero={} ({:.0}%)  color-buckets={buckets}",
                    buf.len(),
                    nonzero,
                    100.0 * nonzero as f32 / buf.len().max(1) as f32,
                );
                decoded += 1;
                if first.is_none() {
                    first = Some((buf.clone(), w, h));
                }
            }
            None => {
                println!("frame {n}: EOF (no debería tan pronto)");
                break;
            }
        }
    }

    // El loop del worker: al EOF rebobina con seek_to(0) y sigue. Probá que tras
    // un seek explícito el próximo step_frame entrega un frame válido otra vez.
    source.seek_to(Duration::ZERO);
    match source.step_frame(&mut buf) {
        Some((w, h)) => println!("tras seek_to(0): frame {w}x{h} OK (loop reengancha)"),
        None => println!("tras seek_to(0): None (¡loop NO reengancha!)"),
    }

    // ¿El frame cambia entre tiempos? (gradiente que deriva → algún pixel difiere)
    if let Some((f0, w, h)) = first {
        source.seek_to(Duration::from_millis(2000));
        let _ = source.step_frame(&mut buf);
        let diff = if buf.len() == f0.len() {
            buf.iter().zip(&f0).filter(|(a, b)| a != b).count()
        } else {
            buf.len()
        };
        println!(
            "diff frame0 vs ~2s: {diff} bytes ({:.1}%) → {}",
            100.0 * diff as f32 / f0.len().max(1) as f32,
            if diff > 0 { "ANIMA" } else { "estático" }
        );
        let _ = (w, h);
    }

    println!("\nVEREDICTO: {decoded}/6 frames decodificados a RGBA.");
    if decoded == 0 {
        std::process::exit(2);
    }
}
