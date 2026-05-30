//! Round-trip nativo end-to-end: encode AV1 con `media-encode-av1` →
//! escribe IVF → decode con `media-source-av1` (rav1d) → verifica que el
//! color sobrevive. Prueba que el ciclo encode↔decode del formato nativo
//! de gioser cierra **sin ffmpeg en ningún extremo**.

use std::time::Duration;

use media_core::FrameSource;
use media_encode_av1::{Av1EncoderConfig, encode_rgba_to_ivf_file};
use media_source_av1::Av1VideoSource;

/// Genera un frame RGBA de color sólido.
fn solid(w: usize, h: usize, r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut f = vec![0u8; w * h * 4];
    for px in f.chunks_exact_mut(4) {
        px[0] = r;
        px[1] = g;
        px[2] = b;
        px[3] = 255;
    }
    f
}

#[test]
fn encode_then_decode_preserves_color() {
    let (w, h) = (96usize, 64usize);
    let cfg = Av1EncoderConfig {
        width: w as u32,
        height: h as u32,
        fps_num: 30,
        fps_den: 1,
        // Calidad alta (quantizer bajo) para que el color sólido no se
        // desvíe por compresión; speed alto para que el test sea rápido.
        quantizer: 20,
        speed: 10,
        threads: 0,
    };

    // Un naranja inconfundible repetido en varios frames.
    let (or, og, ob) = (220u8, 120u8, 30u8);
    let frame = solid(w, h, or, og, ob);
    let frames: Vec<&[u8]> = (0..6).map(|_| frame.as_slice()).collect();

    let path = std::env::temp_dir().join("media_encode_av1_roundtrip.ivf");
    let n = encode_rgba_to_ivf_file(&path, cfg, frames).expect("encode + write IVF");
    assert_eq!(n, 6, "6 frames in → 6 packets out");

    // Decode con el decoder nativo puro-Rust.
    let mut src = Av1VideoSource::open(&path).expect("abrir IVF con media-source-av1");
    assert_eq!(src.dimensions(), (w as u32, h as u32));

    let mut out = Vec::new();
    let dims = src.tick(Duration::from_secs(1), &mut out);
    assert_eq!(dims, Some((w as u32, h as u32)), "primer frame decodifica");
    assert_eq!(out.len(), w * h * 4);

    // El centro del frame debe seguir siendo naranja (tolerancia por
    // YUV420 + cuantización). Muestreamos el pixel central.
    let i = ((h / 2) * w + (w / 2)) * 4;
    let (dr, dg, db) = (out[i] as i32, out[i + 1] as i32, out[i + 2] as i32);
    let tol = 16;
    assert!(
        (dr - or as i32).abs() <= tol
            && (dg - og as i32).abs() <= tol
            && (db - ob as i32).abs() <= tol,
        "color round-trip dentro de ±{tol}: esperaba ({or},{og},{ob}), fue ({dr},{dg},{db})"
    );

    let _ = std::fs::remove_file(&path);
}
