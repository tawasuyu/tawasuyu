//! Análisis offline de un audio (WAV/MP3) → PNG waterfall.
//!
//! Demuestra que las primitivas del dominio (sources + Waterfall +
//! image) componen sin Llimphi ni cpal — un pipeline puro de archivo
//! a archivo, útil para batch o para correr desde un notebook kernel.
//!
//! Uso:
//!
//! ```text
//! cargo run -p multimedia-app --example analyze --release -- audio.wav
//! cargo run -p multimedia-app --example analyze --release -- track.mp3 out.png
//! ```
//!
//! Sin segundo argumento escribe `<input>-waterfall.png` al lado del
//! archivo de entrada.

use std::path::{Path, PathBuf};

use multimedia_core::{AudioSource, Waterfall};
use multimedia_source_mp3::Mp3Source;
use multimedia_source_wav::WavSource;

const BANDS: usize = 64;
const ROWS_PER_SECOND: u32 = 16;
const FMIN: f32 = 40.0;
const FMAX: f32 = 16_000.0;
const PNG_HEIGHT_MAX: u32 = 2048;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let input = match args.first() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!(
                "uso: cargo run -p multimedia-app --example analyze --release -- <audio> [out.png]"
            );
            std::process::exit(2);
        }
    };
    let output = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| default_output(&input));

    let (mut source, sr, ch, duration): (Box<dyn AudioSource>, u32, u16, f32) = match input
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("wav") => match WavSource::from_path(&input) {
            Ok(w) => {
                let sr = w.source_sample_rate();
                let ch = w.source_channels();
                let dur = w.duration_seconds();
                (Box::new(w), sr, ch, dur)
            }
            Err(e) => {
                eprintln!("analyze: error abriendo WAV {input:?}: {e}");
                std::process::exit(1);
            }
        },
        Some("mp3") => match Mp3Source::from_path(&input) {
            Ok(m) => {
                let sr = m.source_sample_rate();
                let ch = m.source_channels();
                let dur = m.duration_seconds();
                (Box::new(m), sr, ch, dur)
            }
            Err(e) => {
                eprintln!("analyze: error abriendo MP3 {input:?}: {e}");
                std::process::exit(1);
            }
        },
        other => {
            eprintln!("analyze: extensión {other:?} no soportada (.wav o .mp3)");
            std::process::exit(2);
        }
    };
    eprintln!(
        "analyze: {} · {} ch @ {} Hz · {:.1}s",
        input.display(),
        ch,
        sr,
        duration,
    );

    // Cálculo del grid: cada fila = un chunk de
    // `sr / ROWS_PER_SECOND` frames, en samples intercalados.
    let frames_per_row = (sr / ROWS_PER_SECOND).max(1);
    let samples_per_row = (frames_per_row as usize) * ch.max(1) as usize;
    let target_rows = ((duration * ROWS_PER_SECOND as f32) as u32).max(1);
    let rows = target_rows.min(PNG_HEIGHT_MAX) as usize;

    let mut waterfall = Waterfall::new(BANDS, rows, FMIN, FMAX);
    let mut buf = vec![0.0_f32; samples_per_row];
    for _ in 0..rows {
        source.fill(&mut buf, sr, ch);
        waterfall.analyze(&buf, ch, sr);
    }

    let mut grid = Vec::new();
    let (rows_out, bands_out) = waterfall.snapshot(&mut grid);

    // Render: bandas en X (fmin a la izquierda), tiempo en Y
    // (más nuevo arriba). PNG con un píxel por celda — el viewer
    // puede escalarla con su zoom natural.
    let img_w = bands_out as u32;
    let img_h = rows_out as u32;
    let mut img: image::RgbaImage = image::ImageBuffer::new(img_w, img_h);
    for r in 0..rows_out {
        for b in 0..bands_out {
            let m = grid[r * bands_out + b];
            let (cr, cg, cb) = heat_rgb(m);
            img.put_pixel(b as u32, r as u32, image::Rgba([cr, cg, cb, 255]));
        }
    }
    if let Err(e) = img.save(&output) {
        eprintln!("analyze: error escribiendo {output:?}: {e}");
        std::process::exit(1);
    }
    eprintln!(
        "analyze: waterfall {}×{} ({} bandas log {:.0}-{:.0} Hz, {} fps) → {}",
        img_w,
        img_h,
        BANDS,
        FMIN,
        FMAX,
        ROWS_PER_SECOND,
        output.display(),
    );
}

fn default_output(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("out");
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{stem}-waterfall.png"))
}

/// Gradiente "heat" análogo al del waterfall_panel pero como (r, g, b).
fn heat_rgb(v: f32) -> (u8, u8, u8) {
    let v = v.clamp(0.0, 1.0);
    if v < 0.25 {
        let t = v / 0.25;
        ((60.0 + 110.0 * t) as u8, (20.0 + 30.0 * t) as u8, (20.0 + 10.0 * t) as u8)
    } else if v < 0.6 {
        let t = (v - 0.25) / 0.35;
        ((170.0 + 70.0 * t) as u8, (50.0 + 110.0 * t) as u8, (30.0 + 40.0 * t) as u8)
    } else {
        let t = (v - 0.6) / 0.4;
        ((240.0 + 15.0 * t) as u8, (160.0 + 80.0 * t) as u8, (70.0 + 160.0 * t) as u8)
    }
}
