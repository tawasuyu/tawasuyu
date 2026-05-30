//! Genera una animación de gradiente y la encodea a AV1 nativo (`.ivf`)
//! sin ffmpeg — demo del camino encode. Reproducible luego con
//! `cargo run -p media-app -- /tmp/gradient.ivf` o con cualquier consumidor
//! de `media-source-av1`.
//!
//! ```bash
//! cargo run -p media-encode-av1 --example gradient --release
//! ```

use media_encode_av1::{Av1EncoderConfig, encode_rgba_to_ivf_file};

fn main() {
    let (w, h) = (320usize, 180usize);
    let n_frames = 48;

    // Un gradiente diagonal que se desplaza con el tiempo.
    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(n_frames);
    for t in 0..n_frames {
        let mut f = vec![0u8; w * h * 4];
        let phase = t * 6;
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                f[i] = ((x + phase) * 255 / w) as u8;
                f[i + 1] = ((y + phase) * 255 / h) as u8;
                f[i + 2] = (((x + y) / 2 + phase) % 256) as u8;
                f[i + 3] = 255;
            }
        }
        frames.push(f);
    }

    let cfg = Av1EncoderConfig {
        width: w as u32,
        height: h as u32,
        fps_num: 24,
        fps_den: 1,
        quantizer: 80,
        speed: 8,
        threads: 0,
    };

    let path = std::env::temp_dir().join("gradient.ivf");
    let refs: Vec<&[u8]> = frames.iter().map(|f| f.as_slice()).collect();
    let n = encode_rgba_to_ivf_file(&path, cfg, refs).expect("encode AV1 → IVF");
    println!("escrito {} ({n} frames AV1 nativos, sin ffmpeg)", path.display());
}
