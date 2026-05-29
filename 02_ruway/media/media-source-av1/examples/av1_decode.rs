//! Decodifica un `.ivf` AV1 de punta a punta y reporta estadísticas.
//! Smoke test del camino nativo sin Llimphi ni GPU.
//!
//! ```bash
//! cargo run -p media-source-av1 --example av1_decode --release -- clip.ivf
//! ```

use std::time::Duration;

use media_core::{FrameSource, Seekable};
use media_source_av1::{split_obus, Av1VideoSource, IvfReader};

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("uso: av1_decode <archivo.ivf>");
            std::process::exit(2);
        }
    };

    // Capa 1+2: demux + inspección de bitstream (sin decoder).
    match IvfReader::open(&path) {
        Ok(mut r) => {
            let h = *r.header();
            println!(
                "IVF: codec={} {}×{} @ {:.3} fps · {} frames declarados",
                String::from_utf8_lossy(&h.codec),
                h.width,
                h.height,
                h.fps(),
                h.num_frames
            );
            if let Ok(Some(tu)) = r.next_unit() {
                let obus = split_obus(&tu.data);
                println!(
                    "primera TU: {} bytes, {} OBUs {:?}",
                    tu.data.len(),
                    obus.len(),
                    obus.iter().map(|o| o.kind).collect::<Vec<_>>()
                );
            }
        }
        Err(e) => {
            eprintln!("no pude abrir IVF: {e}");
            std::process::exit(1);
        }
    }

    // Capa 3: decode real frame por frame.
    let mut src = match Av1VideoSource::open(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("no pude abrir el decoder AV1: {e}");
            std::process::exit(1);
        }
    };
    let (w, h) = src.dimensions();
    let mut buf = Vec::new();
    let mut frames = 0u32;
    // dt grande para pasar siempre el gate de framerate.
    while let Some((fw, fh)) = src.tick(Duration::from_secs(1), &mut buf) {
        assert_eq!((fw, fh), (w, h));
        frames += 1;
    }
    println!(
        "decodificados {frames} frames de {w}×{h} (pos final {:.2}s, dur {:?})",
        src.position().as_secs_f32(),
        src.duration()
    );
}
