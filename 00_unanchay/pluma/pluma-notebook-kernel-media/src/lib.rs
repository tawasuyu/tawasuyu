//! `pluma-notebook-kernel-media` — kernel notebook que analiza
//! offline un archivo de audio (WAV/MP3) y devuelve PNG + observables.
//!
//! Cierra la integración del dominio media (`02_ruway/media/`)
//! con pluma: las primitivas de visualización (Spectrum, Waterfall,
//! Levels) que ya alimentan los visores live en `media-app` se
//! exponen acá como celdas reactivas del DAG del notebook. El kernel
//! es offline puro — sin cpal, sin Llimphi — para correr en CI,
//! sandboxes y futuro wawa userspace.
//!
//! ## Lenguaje reconocido
//!
//! `media`. El source son líneas `key = value`; comentarios con
//! `#` o `//` y líneas vacías se ignoran. Claves principales:
//!
//! ```text
//! source = /ruta/audio.wav      # obligatoria; .wav o .mp3
//! op     = waterfall            # info | levels | waveform | waterfall
//! ```
//!
//! Por operación:
//!
//! - `info` — devuelve duración / sample rate / canales como `Text`.
//!   No requiere keys extra.
//! - `levels` — calcula peak + RMS sobre el archivo entero; payload
//!   `Scalar(peak)` y stdout con ambos números.
//! - `waveform` — envelope min/max como PNG con la onda mono.
//!   Keys: `width` (def 640, cap 2048), `height` (def 160, cap 1024).
//! - `waterfall` — spectrogram con bandas log y filas en tiempo.
//!   Keys: `bands` (def 64, cap 256), `rows_per_sec` (def 16, cap 60),
//!   `fmin` (def 40), `fmax` (def 16_000).
//!
//! ## Salida
//!
//! - `stdout`: resumen humano (duración, sample rate, op, parámetros).
//! - `value`: para `levels`, peak en `dB`; para PNG ops, dimensiones.
//! - `payload`: `Text` (info), `Scalar` (levels) o `Image{png}`.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use media_core::{AudioSource, Levels, Waterfall};
use media_source_mp3::Mp3Source;
use media_source_wav::WavSource;
use pluma_notebook_core::cell::{CellOutput, OutputPayload};
use pluma_notebook_exec::{Kernel, KernelError, KernelOutput};

/// Kernel notebook media. Stateless — toda la config viene en el
/// `source` de la celda.
#[derive(Debug, Clone, Default)]
pub struct MediaKernel;

impl MediaKernel {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Kernel for MediaKernel {
    async fn execute(&self, source: &str, language: &str) -> Result<KernelOutput, KernelError> {
        if language != "media" {
            return Err(KernelError::Runtime(format!(
                "MediaKernel no maneja '{language}' (se esperaba 'media')"
            )));
        }
        let cfg = Config::parse(source).map_err(KernelError::Runtime)?;
        run(&cfg).map_err(KernelError::Runtime)
    }
}

// ─── Parsing del source ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Op {
    Info,
    Levels,
    Waveform,
    Waterfall,
}

#[derive(Debug, Clone, PartialEq)]
struct Config {
    source_path: PathBuf,
    op: Op,
    // Comunes
    width: u32,
    height: u32,
    // Waterfall
    bands: usize,
    rows_per_sec: u32,
    fmin: f32,
    fmax: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            source_path: PathBuf::new(),
            op: Op::Info,
            width: 640,
            height: 160,
            bands: 64,
            rows_per_sec: 16,
            fmin: 40.0,
            fmax: 16_000.0,
        }
    }
}

impl Config {
    fn parse(src: &str) -> Result<Self, String> {
        let mut cfg = Config::default();
        let mut saw_source = false;
        let mut saw_op = false;
        for (n, raw) in src.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            let (k, v) = line.split_once('=').ok_or_else(|| {
                format!("línea {}: se esperaba 'clave = valor'", n + 1)
            })?;
            let k = k.trim();
            let v = v.trim();
            match k {
                "source" => {
                    cfg.source_path = PathBuf::from(v);
                    saw_source = true;
                }
                "op" => {
                    cfg.op = match v {
                        "info" => Op::Info,
                        "levels" => Op::Levels,
                        "waveform" => Op::Waveform,
                        "waterfall" => Op::Waterfall,
                        other => {
                            return Err(format!(
                                "'op' desconocido '{other}' (info|levels|waveform|waterfall)"
                            ))
                        }
                    };
                    saw_op = true;
                }
                "width" => cfg.width = parse_u32(k, v)?,
                "height" => cfg.height = parse_u32(k, v)?,
                "bands" => cfg.bands = parse_usize(k, v)?,
                "rows_per_sec" => cfg.rows_per_sec = parse_u32(k, v)?,
                "fmin" => cfg.fmin = parse_f32(k, v)?,
                "fmax" => cfg.fmax = parse_f32(k, v)?,
                other => return Err(format!("clave desconocida '{other}'")),
            }
        }
        if !saw_source {
            return Err("falta 'source = /ruta/audio.{wav|mp3}'".into());
        }
        if !saw_op {
            return Err("falta 'op = info|levels|waveform|waterfall'".into());
        }
        // Caps defensivos: el kernel corre síncrono dentro de un
        // request del notebook, no es lugar para análisis masivos.
        if cfg.width == 0 || cfg.width > 2048 {
            return Err(format!("'width' fuera de rango (1..=2048): {}", cfg.width));
        }
        if cfg.height == 0 || cfg.height > 1024 {
            return Err(format!("'height' fuera de rango (1..=1024): {}", cfg.height));
        }
        if cfg.bands == 0 || cfg.bands > 256 {
            return Err(format!("'bands' fuera de rango (1..=256): {}", cfg.bands));
        }
        if cfg.rows_per_sec == 0 || cfg.rows_per_sec > 60 {
            return Err(format!(
                "'rows_per_sec' fuera de rango (1..=60): {}",
                cfg.rows_per_sec
            ));
        }
        if !(cfg.fmin > 0.0 && cfg.fmax > cfg.fmin) {
            return Err(format!(
                "fmin/fmax inválidos: fmin={} fmax={}",
                cfg.fmin, cfg.fmax
            ));
        }
        Ok(cfg)
    }
}

fn strip_comment(s: &str) -> &str {
    if let Some(i) = s.find('#') {
        return &s[..i];
    }
    if let Some(i) = s.find("//") {
        return &s[..i];
    }
    s
}

fn parse_u32(k: &str, v: &str) -> Result<u32, String> {
    v.parse::<u32>()
        .map_err(|_| format!("'{k}': se esperaba u32, vino '{v}'"))
}
fn parse_usize(k: &str, v: &str) -> Result<usize, String> {
    v.parse::<usize>()
        .map_err(|_| format!("'{k}': se esperaba usize, vino '{v}'"))
}
fn parse_f32(k: &str, v: &str) -> Result<f32, String> {
    v.parse::<f32>()
        .map_err(|_| format!("'{k}': se esperaba float, vino '{v}'"))
}

// ─── Apertura de la fuente audio ─────────────────────────────────────────────

enum DynSource {
    Wav(WavSource),
    Mp3(Mp3Source),
}

impl DynSource {
    fn open(path: &Path) -> Result<Self, String> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        match ext.as_str() {
            "wav" => WavSource::from_path(path)
                .map(DynSource::Wav)
                .map_err(|e| format!("WAV {path:?}: {e}")),
            "mp3" => Mp3Source::from_path(path)
                .map(DynSource::Mp3)
                .map_err(|e| format!("MP3 {path:?}: {e}")),
            other => Err(format!("extensión '{other}' no soportada (.wav o .mp3)")),
        }
    }

    fn sample_rate(&self) -> u32 {
        match self {
            DynSource::Wav(w) => w.source_sample_rate(),
            DynSource::Mp3(m) => m.source_sample_rate(),
        }
    }
    fn channels(&self) -> u16 {
        match self {
            DynSource::Wav(w) => w.source_channels(),
            DynSource::Mp3(m) => m.source_channels(),
        }
    }
    fn duration_seconds(&self) -> f32 {
        match self {
            DynSource::Wav(w) => w.duration_seconds(),
            DynSource::Mp3(m) => m.duration_seconds(),
        }
    }
    fn fill(&mut self, buf: &mut [f32], sr: u32, ch: u16) {
        match self {
            DynSource::Wav(w) => w.fill(buf, sr, ch),
            DynSource::Mp3(m) => m.fill(buf, sr, ch),
        }
    }
}

// ─── Ejecución por op ────────────────────────────────────────────────────────

fn run(cfg: &Config) -> Result<KernelOutput, String> {
    let mut src = DynSource::open(&cfg.source_path)?;
    let sr = src.sample_rate();
    let ch = src.channels();
    let dur = src.duration_seconds();

    let header = format!(
        "source: {}\nsample rate: {} Hz\nchannels: {}\nduration: {:.2}s\n",
        cfg.source_path.display(),
        sr,
        ch,
        dur,
    );

    match cfg.op {
        Op::Info => Ok(CellOutput {
            stdout: header.clone(),
            value: Some(format!("{:.2}s", dur)),
            payload: OutputPayload::Text(header),
        }),
        Op::Levels => run_levels(&mut src, sr, ch, dur, &header),
        Op::Waveform => run_waveform(cfg, &mut src, sr, ch, dur, &header),
        Op::Waterfall => run_waterfall(cfg, &mut src, sr, ch, dur, &header),
    }
}

fn run_levels(
    src: &mut DynSource,
    sr: u32,
    ch: u16,
    dur: f32,
    header: &str,
) -> Result<KernelOutput, String> {
    let mut lv = Levels::new();
    lv.set_release(0.0); // sin smoothing para reportar el peak/RMS reales
    let frames_per_chunk = (sr / 16).max(256);
    let samples_per_chunk = frames_per_chunk as usize * ch.max(1) as usize;
    let total_chunks = ((dur * 16.0) as usize).max(1);
    let mut buf = vec![0.0_f32; samples_per_chunk];
    let mut max_peak = 0.0_f32;
    let mut sum_sq = 0.0_f32;
    let mut count = 0u64;
    for _ in 0..total_chunks {
        src.fill(&mut buf, sr, ch);
        lv.analyze(&buf, ch);
        if lv.peak() > max_peak {
            max_peak = lv.peak();
        }
        sum_sq += lv.rms() * lv.rms();
        count += 1;
    }
    let avg_rms = (sum_sq / count.max(1) as f32).sqrt();
    let peak_db = lin_to_db(max_peak);
    let rms_db = lin_to_db(avg_rms);
    let stdout = format!(
        "{header}peak: {:.3} ({:.1} dBFS)\nrms (avg): {:.3} ({:.1} dBFS)\n",
        max_peak, peak_db, avg_rms, rms_db
    );
    Ok(CellOutput {
        stdout,
        value: Some(format!("peak {:.1} dBFS", peak_db)),
        payload: OutputPayload::Scalar(peak_db as f64),
    })
}

fn run_waveform(
    cfg: &Config,
    src: &mut DynSource,
    sr: u32,
    ch: u16,
    dur: f32,
    header: &str,
) -> Result<KernelOutput, String> {
    // Lee el archivo entero como mono fold y dibuja envelope min/max.
    let total_frames = (dur as f64 * sr as f64) as usize;
    let chunk_frames = (sr / 16).max(256) as usize;
    let chunk_samples = chunk_frames * ch.max(1) as usize;
    let mut buf = vec![0.0_f32; chunk_samples];
    let mut mono: Vec<f32> = Vec::with_capacity(total_frames);
    let mut emitted = 0usize;
    while emitted < total_frames {
        src.fill(&mut buf, sr, ch);
        let frames = buf.len() / ch.max(1) as usize;
        let inv_ch = 1.0 / ch.max(1) as f32;
        for f in 0..frames {
            let mut acc = 0.0_f32;
            for c in 0..ch as usize {
                acc += buf[f * ch as usize + c];
            }
            mono.push(acc * inv_ch);
            emitted += 1;
            if emitted >= total_frames {
                break;
            }
        }
    }

    let w = cfg.width;
    let h = cfg.height;
    let mut canvas = Canvas::filled(w, h, [18, 22, 30, 255]);
    let mid_y = (h / 2) as i32;
    let amp = (h as f32 * 0.45) as i32;
    let cols = w as usize;
    let bucket = (mono.len() / cols.max(1)).max(1);
    let bg_line = [80, 92, 110, 255];
    // Línea central
    canvas.hline(0, w as i32, mid_y, bg_line);
    let stroke = [120, 220, 170, 255];
    let fill = [120, 220, 170, 70];
    let mut prev_top = mid_y;
    let mut prev_bot = mid_y;
    for col in 0..cols {
        let f0 = col * bucket;
        let f1 = ((col + 1) * bucket).min(mono.len());
        if f0 >= f1 {
            break;
        }
        let mut vmin = f32::INFINITY;
        let mut vmax = f32::NEG_INFINITY;
        for i in f0..f1 {
            let v = mono[i].clamp(-1.0, 1.0);
            if v < vmin {
                vmin = v;
            }
            if v > vmax {
                vmax = v;
            }
        }
        let y_top = mid_y - (vmax * amp as f32) as i32;
        let y_bot = mid_y - (vmin * amp as f32) as i32;
        canvas.vline(col as i32, y_top, y_bot, fill);
        canvas.set_px(col as i32, y_top, stroke);
        canvas.set_px(col as i32, y_bot, stroke);
        // Conectar con la columna previa para suavidad.
        if col > 0 {
            canvas.vline(col as i32, prev_top.min(y_top), prev_top.max(y_top), stroke);
            canvas.vline(col as i32, prev_bot.min(y_bot), prev_bot.max(y_bot), stroke);
        }
        prev_top = y_top;
        prev_bot = y_bot;
    }

    let png = canvas.into_png()?;
    let stdout = format!(
        "{header}op: waveform · render {w}×{h} · {} muestras\n",
        mono.len()
    );
    Ok(CellOutput {
        stdout,
        value: Some(format!("waveform {w}x{h}")),
        payload: OutputPayload::Image {
            width: w,
            height: h,
            mime: "image/png".into(),
            bytes: png,
        },
    })
}

fn run_waterfall(
    cfg: &Config,
    src: &mut DynSource,
    sr: u32,
    ch: u16,
    dur: f32,
    header: &str,
) -> Result<KernelOutput, String> {
    let frames_per_row = (sr / cfg.rows_per_sec).max(1);
    let samples_per_row = frames_per_row as usize * ch.max(1) as usize;
    let target_rows = ((dur * cfg.rows_per_sec as f32) as u32).max(1);
    // Cap por height del PNG (no tiene sentido más filas que píxeles).
    let rows = target_rows.min(cfg.height) as usize;
    let bands = cfg.bands;
    let mut wf = Waterfall::new(bands, rows, cfg.fmin, cfg.fmax);
    let mut buf = vec![0.0_f32; samples_per_row];
    for _ in 0..rows {
        src.fill(&mut buf, sr, ch);
        wf.analyze(&buf, ch, sr);
    }
    let mut grid = Vec::new();
    let (rows_out, bands_out) = wf.snapshot(&mut grid);

    // Renderiza el grid escalado al tamaño pedido. Cada celda es
    // un rect (width_per_band × height_per_row).
    let w = cfg.width;
    let h = cfg.height;
    let mut canvas = Canvas::filled(w, h, [14, 16, 22, 255]);
    let cell_w_f = w as f32 / bands_out.max(1) as f32;
    let cell_h_f = h as f32 / rows_out.max(1) as f32;
    for r in 0..rows_out {
        let y0 = (r as f32 * cell_h_f) as i32;
        let y1 = ((r as f32 + 1.0) * cell_h_f).ceil() as i32;
        for b in 0..bands_out {
            let m = grid[r * bands_out + b];
            if m < 0.02 {
                continue;
            }
            let x0 = (b as f32 * cell_w_f) as i32;
            let x1 = ((b as f32 + 1.0) * cell_w_f).ceil() as i32;
            let color = heat_rgba(m);
            canvas.fill_rect(x0, y0, x1, y1, color);
        }
    }
    let png = canvas.into_png()?;
    let stdout = format!(
        "{header}op: waterfall · {rows} filas × {bands} bandas log {fmin:.0}-{fmax:.0} Hz @ {rows_per_sec} fps · render {w}×{h}\n",
        rows = rows_out,
        bands = bands_out,
        fmin = cfg.fmin,
        fmax = cfg.fmax,
        rows_per_sec = cfg.rows_per_sec,
        w = w,
        h = h,
    );
    Ok(CellOutput {
        stdout,
        value: Some(format!("waterfall {w}x{h}")),
        payload: OutputPayload::Image {
            width: w,
            height: h,
            mime: "image/png".into(),
            bytes: png,
        },
    })
}

// ─── Canvas RGBA + encoder PNG (copia de patrón tinkuy) ─────────────────────

struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Canvas {
    fn filled(w: u32, h: u32, color: [u8; 4]) -> Self {
        let mut px = vec![0u8; (w as usize) * (h as usize) * 4];
        for chunk in px.chunks_exact_mut(4) {
            chunk.copy_from_slice(&color);
        }
        Self { w, h, px }
    }

    #[inline]
    fn set_px(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let i = (y as usize * self.w as usize + x as usize) * 4;
        // Composite alfa simple: si color.a < 255, blend con fondo.
        if color[3] == 255 {
            self.px[i..i + 4].copy_from_slice(&color);
            return;
        }
        let a = color[3] as u32;
        let inv = 255 - a;
        for c in 0..3 {
            let bg = self.px[i + c] as u32;
            let fg = color[c] as u32;
            self.px[i + c] = ((fg * a + bg * inv) / 255) as u8;
        }
        self.px[i + 3] = 255;
    }

    fn hline(&mut self, x0: i32, x1: i32, y: i32, color: [u8; 4]) {
        let (xa, xb) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        for x in xa..xb {
            self.set_px(x, y, color);
        }
    }

    fn vline(&mut self, x: i32, y0: i32, y1: i32, color: [u8; 4]) {
        let (ya, yb) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        for y in ya..=yb {
            self.set_px(x, y, color);
        }
    }

    fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: [u8; 4]) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.set_px(x, y, color);
            }
        }
    }

    fn into_png(self) -> Result<Vec<u8>, String> {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, self.w, self.h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().map_err(|e| format!("png header: {e}"))?;
            writer
                .write_image_data(&self.px)
                .map_err(|e| format!("png data: {e}"))?;
        }
        Ok(buf)
    }
}

fn heat_rgba(v: f32) -> [u8; 4] {
    let v = v.clamp(0.0, 1.0);
    if v < 0.25 {
        let t = v / 0.25;
        [
            (60.0 + 110.0 * t) as u8,
            (20.0 + 30.0 * t) as u8,
            (20.0 + 10.0 * t) as u8,
            255,
        ]
    } else if v < 0.6 {
        let t = (v - 0.25) / 0.35;
        [
            (170.0 + 70.0 * t) as u8,
            (50.0 + 110.0 * t) as u8,
            (30.0 + 40.0 * t) as u8,
            255,
        ]
    } else {
        let t = (v - 0.6) / 0.4;
        [
            (240.0 + 15.0 * t) as u8,
            (160.0 + 80.0 * t) as u8,
            (70.0 + 160.0 * t) as u8,
            255,
        ]
    }
}

fn lin_to_db(v: f32) -> f32 {
    if v <= 1e-6 {
        -120.0
    } else {
        20.0 * v.log10()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let cfg = Config::parse(
            "source = /tmp/a.wav\nop = info\n",
        )
        .unwrap();
        assert_eq!(cfg.source_path, PathBuf::from("/tmp/a.wav"));
        assert_eq!(cfg.op, Op::Info);
    }

    #[test]
    fn parse_waterfall_with_opts() {
        let src = r#"
            # waterfall de prueba
            source = /tmp/x.mp3
            op     = waterfall
            bands  = 32
            rows_per_sec = 8
            width  = 800
            height = 200
            fmin   = 80
            fmax   = 8000
        "#;
        let cfg = Config::parse(src).unwrap();
        assert_eq!(cfg.op, Op::Waterfall);
        assert_eq!(cfg.bands, 32);
        assert_eq!(cfg.rows_per_sec, 8);
        assert_eq!(cfg.width, 800);
        assert_eq!(cfg.height, 200);
        assert_eq!(cfg.fmin, 80.0);
        assert_eq!(cfg.fmax, 8000.0);
    }

    #[test]
    fn missing_source_fails() {
        let err = Config::parse("op = info").unwrap_err();
        assert!(err.contains("source"), "err={err}");
    }

    #[test]
    fn missing_op_fails() {
        let err = Config::parse("source = /tmp/a.wav").unwrap_err();
        assert!(err.contains("op"), "err={err}");
    }

    #[test]
    fn unknown_op_fails() {
        let err = Config::parse("source = /tmp/a.wav\nop = mezclar").unwrap_err();
        assert!(err.contains("op"), "err={err}");
    }

    #[test]
    fn caps_reject_oversized() {
        let err = Config::parse(
            "source = /tmp/a.wav\nop = waterfall\nwidth = 5000",
        )
        .unwrap_err();
        assert!(err.contains("width"), "err={err}");
    }

    #[test]
    fn fmin_fmax_validated() {
        let err = Config::parse(
            "source = /tmp/a.wav\nop = waterfall\nfmin = 1000\nfmax = 500",
        )
        .unwrap_err();
        assert!(err.contains("fmin") || err.contains("fmax"), "err={err}");
    }

    #[test]
    fn rejects_unknown_keys() {
        let err =
            Config::parse("source = /tmp/a.wav\nop = info\nblargh = 1").unwrap_err();
        assert!(err.contains("blargh"), "err={err}");
    }
}
