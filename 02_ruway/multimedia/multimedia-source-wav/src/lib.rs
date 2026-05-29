//! multimedia-source-wav — decoder PCM (WAV) como [`AudioSource`].
//!
//! Carga el archivo completo a `f32` al construir (PCM 8/16/24/32 y
//! float 32 los normaliza al rango [-1, 1]). El callback de audio
//! reproduce los samples en loop, intercalando los canales del WAV en
//! el formato que pida el sink. Si el sink pide más canales que el
//! WAV, duplica el último canal disponible; si pide menos, descarta
//! los extras.
//!
//! El resampleo cuando `wav.sample_rate != sink.sample_rate` es lineal
//! puro (zero-order hold con interpolación entre samples adyacentes).
//! Es honesto para MVP — para profesional habría que usar un
//! resampler de calidad (rubato, samplerate, etc.).
//!
//! Trade-off explícito: todo el archivo vive en RAM. Para WAVs cortos
//! (samples, stems de prueba, jingles) es lo más simple. Para audios
//! largos un futuro `multimedia-source-wav-stream` debería leer por
//! bloques.

use std::path::Path;
use std::time::Duration;

use hound::{SampleFormat, WavReader};
use multimedia_core::{AudioSource, Seekable};

#[derive(Debug)]
pub enum WavError {
    Open(hound::Error),
    UnsupportedFormat {
        bits: u16,
        sample_format: SampleFormat,
    },
    Empty,
}

impl std::fmt::Display for WavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open(e) => write!(f, "hound: {e}"),
            Self::UnsupportedFormat {
                bits,
                sample_format,
            } => {
                write!(f, "formato WAV no soportado: {sample_format:?} {bits} bits")
            }
            Self::Empty => write!(f, "WAV vacío"),
        }
    }
}

impl std::error::Error for WavError {}

impl From<hound::Error> for WavError {
    fn from(e: hound::Error) -> Self {
        Self::Open(e)
    }
}

/// Reproductor PCM en loop.
pub struct WavSource {
    /// Samples normalizados a [-1, 1], intercalados por canal del WAV.
    samples: Vec<f32>,
    src_channels: u16,
    src_sample_rate: u32,
    /// Posición en samples (sin agrupar por frame). Avanza
    /// monotónica; al pasarse del final hace `% samples.len()`.
    /// Es f64 para acumular la fracción del resampleo.
    cursor: f64,
}

impl WavSource {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, WavError> {
        let mut reader = WavReader::open(path)?;
        let spec = reader.spec();
        let channels = spec.channels.max(1);
        let sample_rate = spec.sample_rate.max(1);
        let samples = match (spec.sample_format, spec.bits_per_sample) {
            (SampleFormat::Float, 32) => reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(WavError::Open)?,
            (SampleFormat::Int, bits) => {
                // hound entrega el sample en i32 con el rango del bits
                // declarado: [-(2^(bits-1)), 2^(bits-1) - 1].
                let scale = 1.0_f32 / ((1u64 << (bits.saturating_sub(1))) as f32);
                reader
                    .samples::<i32>()
                    .map(|r| r.map(|v| v as f32 * scale))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(WavError::Open)?
            }
            (sample_format, bits) => {
                return Err(WavError::UnsupportedFormat {
                    bits,
                    sample_format,
                })
            }
        };
        if samples.is_empty() {
            return Err(WavError::Empty);
        }
        Ok(Self {
            samples,
            src_channels: channels,
            src_sample_rate: sample_rate,
            cursor: 0.0,
        })
    }

    pub fn source_channels(&self) -> u16 {
        self.src_channels
    }

    pub fn source_sample_rate(&self) -> u32 {
        self.src_sample_rate
    }

    /// Duración total del audio en segundos (sin contar loops).
    pub fn duration_seconds(&self) -> f32 {
        let frames = self.samples.len() as f32 / self.src_channels.max(1) as f32;
        frames / self.src_sample_rate as f32
    }

    /// Lee un sample en `frame_idx` (en frames del source) y `ch_idx`
    /// (en canales del source), con clampeo del canal al rango
    /// disponible. `frame_idx` es f64 — interpola linealmente entre
    /// los dos frames adyacentes.
    fn sample_at(&self, frame_idx: f64, ch_idx: u16) -> f32 {
        let src_ch = self.src_channels.max(1) as usize;
        let total_frames = (self.samples.len() / src_ch) as f64;
        let wrapped = frame_idx.rem_euclid(total_frames);
        let i0 = wrapped.floor();
        let frac = (wrapped - i0) as f32;
        let i0 = i0 as usize;
        let i1 = (i0 + 1) % (total_frames as usize);
        let ch = (ch_idx as usize).min(src_ch - 1);
        let s0 = self.samples[i0 * src_ch + ch];
        let s1 = self.samples[i1 * src_ch + ch];
        s0 + (s1 - s0) * frac
    }
}

impl Seekable for WavSource {
    fn position(&self) -> Duration {
        // cursor está en frames del source (no en samples), igual que
        // lo usa sample_at — ver fill().
        let secs = self.cursor.max(0.0) / self.src_sample_rate.max(1) as f64;
        Duration::from_secs_f64(secs.max(0.0))
    }

    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f32(self.duration_seconds()))
    }

    fn seek_to(&mut self, pos: Duration) {
        let src_ch = self.src_channels.max(1) as f64;
        let total_frames = (self.samples.len() as f64 / src_ch).max(1.0);
        let frames = pos.as_secs_f64() * self.src_sample_rate.max(1) as f64;
        self.cursor = frames.rem_euclid(total_frames);
    }
}

impl AudioSource for WavSource {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        let out_channels = channels.max(1) as usize;
        let sink_sr = sample_rate.max(1) as f64;
        let src_sr = self.src_sample_rate.max(1) as f64;
        let step = src_sr / sink_sr; // frames del source por frame del sink
        let frames = buf.len() / out_channels;
        let mut cursor = self.cursor;
        for frame in 0..frames {
            for ch in 0..out_channels {
                let v = self.sample_at(cursor, ch as u16);
                buf[frame * out_channels + ch] = v;
            }
            cursor += step;
        }
        // Mantiene cursor dentro del rango de frames del source para
        // que no crezca sin cota.
        let src_ch = self.src_channels.max(1) as usize;
        let total_frames = (self.samples.len() / src_ch) as f64;
        if total_frames > 0.0 {
            cursor = cursor.rem_euclid(total_frames);
        }
        self.cursor = cursor;

        // Tail: si len no es múltiplo de channels, silencio.
        let tail = frames * out_channels;
        for s in &mut buf[tail..] {
            *s = 0.0;
        }
    }
}
