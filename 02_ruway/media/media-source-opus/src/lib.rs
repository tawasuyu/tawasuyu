//! media-source-opus — decode **Opus nativo** (puro-Rust) del dominio media.
//!
//! Opus es el formato de audio NATIVO de gioser (PLAN.md §6.quinquies),
//! par del video AV1 (`media-source-av1`): sin C, sin FFI, sin patentes.
//! Este crate abre un archivo **Ogg Opus** (`.opus`/`.ogg`), demuxea con
//! el crate `ogg`, decodifica los paquetes con `opus-wave` (port de
//! libopus) y expone el resultado como [`media_core::AudioSource`].
//!
//! Misma forma que [`media_source_mp3`](https://docs.rs)/`WavSource`:
//! decodifica el archivo entero a `f32` intercalado al construir (Opus
//! siempre sale a 48 kHz) y el callback reproduce con resampleo lineal
//! cuando el sample rate del sink difiere. Trade-off explícito: RAM =
//! duración · 48000 · channels · 4 bytes.
//!
//! Soporta mono y estéreo (mapping family 0, el caso común). El
//! multicanal (family 1, 5.1/ambisonics) necesitaría `OpusMSDecoder` —
//! pendiente. Aplica el `output_gain` de la cabecera y descarta el
//! `pre_skip` (delay del encoder).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use media_core::{AudioSource, Seekable};
use ogg::PacketReader;
use opus_wave::types::{Channels, SampleRate};
use opus_wave::OpusDecoder;

/// Opus siempre se decodifica a 48 kHz (la sample rate interna del
/// códec; la cabecera `input_sample_rate` es solo informativa).
const OPUS_RATE: u32 = 48_000;
/// Máximo de samples por canal en un frame Opus (120 ms a 48 kHz).
const MAX_FRAME: usize = 5_760;

#[derive(Debug)]
pub enum OpusError {
    Io(std::io::Error),
    Ogg(String),
    Decode(String),
    NoOpusHead,
    /// Mapping family != 0 (multicanal) todavía no soportado.
    Multicanal(u8),
    Empty,
}

impl std::fmt::Display for OpusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Ogg(e) => write!(f, "ogg: {e}"),
            Self::Decode(e) => write!(f, "decode opus: {e}"),
            Self::NoOpusHead => write!(f, "el primer paquete no es OpusHead (¿no es Ogg Opus?)"),
            Self::Multicanal(n) => write!(f, "mapping family {n} (multicanal) no soportado todavía"),
            Self::Empty => write!(f, "Opus sin samples"),
        }
    }
}

impl std::error::Error for OpusError {}

impl From<std::io::Error> for OpusError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Reproductor Opus en loop. Misma semántica de `cursor`/resampleo/loop
/// que `Mp3Source` y `WavSource`.
pub struct OpusSource {
    samples: Vec<f32>,
    src_channels: u16,
    cursor: f64,
    speed: f32,
    looped: bool,
    finished: bool,
}

impl OpusSource {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, OpusError> {
        let file = File::open(path.as_ref())?;
        let mut reader = PacketReader::new(BufReader::new(file));

        let mut decoder: Option<OpusDecoder> = None;
        let mut channels: u16 = 1;
        let mut pre_skip: u32 = 0;
        let mut gain_factor: f32 = 1.0;
        let mut interleaved: Vec<f32> = Vec::new();
        let mut pkt_idx = 0usize;

        loop {
            let packet = match reader.read_packet() {
                Ok(Some(p)) => p,
                Ok(None) => break,
                Err(e) => return Err(OpusError::Ogg(format!("{e:?}"))),
            };
            let data = &packet.data;
            match pkt_idx {
                0 => {
                    // OpusHead
                    let head = parse_opus_head(data)?;
                    channels = head.channels as u16;
                    pre_skip = head.pre_skip as u32;
                    gain_factor = head.gain_factor();
                    let ch = if channels >= 2 {
                        Channels::Stereo
                    } else {
                        Channels::Mono
                    };
                    decoder = Some(
                        OpusDecoder::new(SampleRate::Hz48000, ch)
                            .map_err(|e| OpusError::Decode(format!("{e:?}")))?,
                    );
                }
                1 => {
                    // OpusTags (metadata Vorbis comment) — lo ignoramos.
                }
                _ => {
                    let dec = decoder.as_mut().ok_or(OpusError::NoOpusHead)?;
                    let ch = channels.max(1) as usize;
                    let mut pcm = vec![0f32; MAX_FRAME * ch];
                    match dec.decode_float(Some(data), &mut pcm, MAX_FRAME as i32, false) {
                        Ok(n) if n > 0 => {
                            let got = n as usize * ch;
                            interleaved.extend_from_slice(&pcm[..got]);
                        }
                        Ok(_) => {}
                        Err(e) => return Err(OpusError::Decode(format!("{e:?}"))),
                    }
                }
            }
            pkt_idx += 1;
        }

        if decoder.is_none() {
            return Err(OpusError::NoOpusHead);
        }

        // Descartar el pre-skip (delay del encoder) del frente.
        let ch = channels.max(1) as usize;
        let skip = (pre_skip as usize).saturating_mul(ch).min(interleaved.len());
        if skip > 0 {
            interleaved.drain(0..skip);
        }
        // Aplicar output gain si no es unidad.
        if (gain_factor - 1.0).abs() > 1e-6 {
            for s in interleaved.iter_mut() {
                *s = (*s * gain_factor).clamp(-1.0, 1.0);
            }
        }

        if interleaved.is_empty() {
            return Err(OpusError::Empty);
        }
        Ok(Self {
            samples: interleaved,
            src_channels: channels.max(1),
            cursor: 0.0,
            speed: 1.0,
            looped: true,
            finished: false,
        })
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(0.1, 4.0);
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn set_loop(&mut self, looped: bool) {
        self.looped = looped;
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn source_channels(&self) -> u16 {
        self.src_channels
    }

    /// Opus siempre decodifica a 48 kHz.
    pub fn source_sample_rate(&self) -> u32 {
        OPUS_RATE
    }

    pub fn duration_seconds(&self) -> f32 {
        let frames = self.samples.len() as f32 / self.src_channels.max(1) as f32;
        frames / OPUS_RATE as f32
    }

    fn sample_at(&self, frame_idx: f64, ch_idx: u16) -> f32 {
        let src_ch = self.src_channels.max(1) as usize;
        let total_frames = (self.samples.len() / src_ch) as f64;
        if total_frames == 0.0 {
            return 0.0;
        }
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

impl Seekable for OpusSource {
    fn position(&self) -> Duration {
        let secs = self.cursor.max(0.0) / OPUS_RATE as f64;
        Duration::from_secs_f64(secs.max(0.0))
    }

    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f32(self.duration_seconds()))
    }

    fn seek_to(&mut self, pos: Duration) {
        let src_ch = self.src_channels.max(1) as f64;
        let total_frames = (self.samples.len() as f64 / src_ch).max(1.0);
        let frames = pos.as_secs_f64() * OPUS_RATE as f64;
        self.cursor = frames.rem_euclid(total_frames);
        self.finished = false;
    }
}

impl AudioSource for OpusSource {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        let out_channels = channels.max(1) as usize;
        let sink_sr = sample_rate.max(1) as f64;
        let step = (OPUS_RATE as f64 / sink_sr) * self.speed as f64;
        let frames = buf.len() / out_channels;
        let src_ch = self.src_channels.max(1) as usize;
        let total_frames = (self.samples.len() / src_ch) as f64;

        if !self.looped && self.finished {
            for s in buf.iter_mut() {
                *s = 0.0;
            }
            return;
        }

        let mut cursor = self.cursor;
        for frame in 0..frames {
            if !self.looped && cursor >= total_frames {
                for ch in 0..out_channels {
                    buf[frame * out_channels + ch] = 0.0;
                }
                self.finished = true;
                continue;
            }
            for ch in 0..out_channels {
                buf[frame * out_channels + ch] = self.sample_at(cursor, ch as u16);
            }
            cursor += step;
        }
        if self.looped && total_frames > 0.0 {
            cursor = cursor.rem_euclid(total_frames);
        } else if !self.looped {
            cursor = cursor.min(total_frames);
        }
        self.cursor = cursor;
        let tail = frames * out_channels;
        for s in &mut buf[tail..] {
            *s = 0.0;
        }
    }
}

// ─── OpusHead ────────────────────────────────────────────────────────────────

struct OpusHead {
    channels: u8,
    pre_skip: u16,
    /// Output gain en Q7.8 dB (entero con signo).
    output_gain_q78: i16,
    mapping_family: u8,
}

impl OpusHead {
    /// Factor lineal del output gain (10^(dB/20)).
    fn gain_factor(&self) -> f32 {
        if self.output_gain_q78 == 0 {
            return 1.0;
        }
        let db = self.output_gain_q78 as f32 / 256.0;
        10f32.powf(db / 20.0)
    }
}

/// Parsea la cabecera `OpusHead` (RFC 7845 §5.1). Solo soporta mapping
/// family 0 (mono/estéreo).
fn parse_opus_head(data: &[u8]) -> Result<OpusHead, OpusError> {
    if data.len() < 19 || &data[0..8] != b"OpusHead" {
        return Err(OpusError::NoOpusHead);
    }
    let channels = data[9];
    let pre_skip = u16::from_le_bytes([data[10], data[11]]);
    let output_gain_q78 = i16::from_le_bytes([data[16], data[17]]);
    let mapping_family = data[18];
    if mapping_family != 0 {
        return Err(OpusError::Multicanal(mapping_family));
    }
    Ok(OpusHead {
        channels,
        pre_skip,
        output_gain_q78,
        mapping_family,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> std::path::PathBuf {
        // Copiamos el fixture embebido a un temp porque from_path abre por
        // ruta. Nombre único por test → sin carreras al correr en paralelo.
        let bytes = include_bytes!("../tests/fixtures/tone_440_mono.opus");
        let path = std::env::temp_dir().join(format!("media_opus_test_{tag}.opus"));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn parse_head_del_fixture() {
        let bytes = include_bytes!("../tests/fixtures/tone_440_mono.opus");
        // El OpusHead vive dentro de la primera página Ogg; lo busca el demux.
        let mut r = PacketReader::new(std::io::Cursor::new(&bytes[..]));
        let first = r.read_packet().unwrap().unwrap();
        let head = parse_opus_head(&first.data).unwrap();
        assert_eq!(head.channels, 1);
        assert_eq!(head.mapping_family, 0);
    }

    #[test]
    fn decodes_real_opus_fixture() {
        let path = fixture("decode");
        let src = OpusSource::from_path(&path).unwrap();
        assert_eq!(src.source_channels(), 1);
        assert_eq!(src.source_sample_rate(), 48_000);
        // ~1 s de tono → cerca de 48000 frames (con tolerancia por pre-skip).
        let d = src.duration_seconds();
        assert!(d > 0.9 && d < 1.2, "duración inesperada: {d}s");
        // El tono tiene energía.
        let energetic = src.samples.iter().filter(|s| s.abs() > 0.01).count();
        assert!(energetic > 1000, "esperaba señal del tono, hubo {energetic}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fill_resamplea_a_44100_sin_panic() {
        let path = fixture("fill");
        let mut src = OpusSource::from_path(&path).unwrap();
        let mut buf = vec![0f32; 2 * 1024]; // estéreo, 1024 frames
        src.fill(&mut buf, 44_100, 2);
        // Mono → estéreo: ambos canales iguales, con señal.
        let energetic = buf.iter().filter(|s| s.abs() > 0.01).count();
        assert!(energetic > 100, "fill resampleado debería traer señal");
        let _ = std::fs::remove_file(&path);
    }
}
