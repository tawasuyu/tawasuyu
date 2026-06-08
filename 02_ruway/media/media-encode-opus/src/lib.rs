//! media-encode-opus — encoder **Opus nativo** (puro-Rust, `opus-wave`)
//! desde PCM f32 → paquetes Opus + cabecera `OpusHead`.
//!
//! La **contraparte** de [`media_source_opus`]: ese crate *decodea* Opus
//! nativo, este lo *produce*. Junto a `media-encode-av1` cierra la
//! producción del stack abierto de tawasuyu (PLAN.md §6.quinquies) — y es lo
//! que le faltaba a [`media_mux_webm`] para escribir un `.webm` AV1+Opus
//! 100% propio, sin un solo byte de ffmpeg.
//!
//! `opus-wave` es el mismo port de libopus (SILK+CELT) que el decoder usa;
//! sin C, sin FFI, compila a WASM y corre en wawa.
//!
//! El input es PCM f32 **intercalado** por canal en `[-1, 1]` (lo que mueve
//! el resto del dominio: `AudioSource::fill`); la salida son paquetes Opus
//! crudos del tamaño de frame elegido, más el `OpusHead` (RFC 7845 §5.1)
//! que el demuxer guarda como `CodecPrivate` del track.
//!
//! ```no_run
//! use media_encode_opus::{OpusEncoder, OpusEncoderConfig, FrameDuration};
//!
//! let cfg = OpusEncoderConfig { sample_rate: 48_000, channels: 2, ..Default::default() };
//! let mut enc = OpusEncoder::new(cfg)?;
//! let pcm: Vec<f32> = vec![/* L,R,L,R,... en [-1,1] */];
//! let packets = enc.encode_interleaved(&pcm)?;
//! let head = enc.opus_head();           // CodecPrivate para el muxer
//! let spp = enc.samples_per_packet();   // p.ej. 960 (20 ms @ 48 kHz)
//! # Ok::<(), media_encode_opus::OpusEncodeError>(())
//! ```

use opus_wave::{Application, Bitrate, Channels, OpusEncoder as WaveEncoder, SampleRate};

/// Duración del frame Opus. Define cuántas muestras por canal entran en cada
/// paquete; 20 ms es el default de streaming. Valores fuera de esta lista no
/// son representables en Opus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDuration {
    Ms2_5,
    Ms5,
    Ms10,
    Ms20,
    Ms40,
    Ms60,
}

impl FrameDuration {
    /// Muestras por canal a este sample rate.
    pub fn samples(self, sample_rate: u32) -> u32 {
        match self {
            Self::Ms2_5 => sample_rate / 400,
            Self::Ms5 => sample_rate / 200,
            Self::Ms10 => sample_rate / 100,
            Self::Ms20 => sample_rate / 50,
            Self::Ms40 => sample_rate / 25,
            Self::Ms60 => sample_rate * 60 / 1000,
        }
    }
}

/// Parámetros del encoder.
#[derive(Debug, Clone)]
pub struct OpusEncoderConfig {
    /// 8000 / 12000 / 16000 / 24000 / 48000 Hz.
    pub sample_rate: u32,
    /// 1 (mono) o 2 (estéreo).
    pub channels: u8,
    /// Bitrate objetivo en bits/s; `None` deja el default del encoder (VBR).
    pub bitrate_bps: Option<i32>,
    /// Tamaño de frame de cada paquete.
    pub frame: FrameDuration,
}

impl Default for OpusEncoderConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            bitrate_bps: None,
            frame: FrameDuration::Ms20,
        }
    }
}

#[derive(Debug)]
pub enum OpusEncodeError {
    /// `sample_rate` no es uno de los cinco que admite Opus.
    BadSampleRate(u32),
    /// `channels` no es 1 ni 2.
    BadChannels(u8),
    /// El backend `opus-wave` rechazó la config o un encode.
    Backend(String),
}

impl std::fmt::Display for OpusEncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadSampleRate(r) => {
                write!(f, "sample rate {r} no soportado (usá 8/12/16/24/48 kHz)")
            }
            Self::BadChannels(c) => write!(f, "{c} canales; Opus acá es mono o estéreo"),
            Self::Backend(e) => write!(f, "opus-wave: {e}"),
        }
    }
}

impl std::error::Error for OpusEncodeError {}

fn map_rate(r: u32) -> Result<SampleRate, OpusEncodeError> {
    Ok(match r {
        8000 => SampleRate::Hz8000,
        12000 => SampleRate::Hz12000,
        16000 => SampleRate::Hz16000,
        24000 => SampleRate::Hz24000,
        48000 => SampleRate::Hz48000,
        other => return Err(OpusEncodeError::BadSampleRate(other)),
    })
}

fn map_channels(c: u8) -> Result<Channels, OpusEncodeError> {
    Ok(match c {
        1 => Channels::Mono,
        2 => Channels::Stereo,
        other => return Err(OpusEncodeError::BadChannels(other)),
    })
}

/// `pre_skip` que escribimos en el `OpusHead`. `opus-wave` no expone su
/// lookahead exacto; 312 muestras (6.5 ms @ 48 kHz) es el valor de ejemplo
/// del RFC 7845 y aproxima el delay del encoder. El decoder lo usa sólo para
/// recortar el priming inicial — no afecta a la integridad del bitstream.
const PRE_SKIP: u16 = 312;

/// Encoder Opus con estado. `encode_interleaved` trocea el PCM en frames y
/// devuelve un paquete por frame; el último frame se rellena con silencio si
/// el PCM no es múltiplo del tamaño de frame.
pub struct OpusEncoder {
    enc: WaveEncoder,
    sample_rate: u32,
    channels: u8,
    frame_samples: u32,
}

impl OpusEncoder {
    pub fn new(cfg: OpusEncoderConfig) -> Result<Self, OpusEncodeError> {
        let rate = map_rate(cfg.sample_rate)?;
        let ch = map_channels(cfg.channels)?;
        let mut enc = WaveEncoder::new(rate, ch, Application::Audio)
            .map_err(|e| OpusEncodeError::Backend(format!("{e:?}")))?;
        if let Some(bps) = cfg.bitrate_bps {
            enc.set_bitrate(Bitrate::BitsPerSecond(bps));
        }
        Ok(Self {
            enc,
            sample_rate: cfg.sample_rate,
            channels: cfg.channels,
            frame_samples: cfg.frame.samples(cfg.sample_rate),
        })
    }

    /// Muestras por canal de cada paquete (lo que pide el `samples_per_packet`
    /// del `OpusTrack` del muxer).
    pub fn samples_per_packet(&self) -> u32 {
        self.frame_samples
    }

    /// La cabecera `OpusHead` (RFC 7845 §5.1, mapping family 0) lista para
    /// ir como `CodecPrivate` del track WebM/Matroska. El demuxer la lee con
    /// `OpusSource::from_opus_packets`.
    pub fn opus_head(&self) -> Vec<u8> {
        let mut h = Vec::with_capacity(19);
        h.extend_from_slice(b"OpusHead");
        h.push(1); // versión
        h.push(self.channels);
        h.extend_from_slice(&PRE_SKIP.to_le_bytes());
        h.extend_from_slice(&self.sample_rate.to_le_bytes()); // input rate (informativo)
        h.extend_from_slice(&0i16.to_le_bytes()); // output gain Q7.8 = 0 dB
        h.push(0); // channel mapping family
        h
    }

    /// Codifica PCM f32 intercalado (`L,R,L,R,…` para estéreo) en una
    /// secuencia de paquetes Opus. El PCM se trocea en frames del tamaño
    /// configurado; un resto parcial se rellena con silencio.
    pub fn encode_interleaved(&mut self, pcm: &[f32]) -> Result<Vec<Vec<u8>>, OpusEncodeError> {
        let ch = self.channels as usize;
        let frame = self.frame_samples as usize;
        let per_packet = frame * ch;
        if per_packet == 0 {
            return Ok(Vec::new());
        }

        let mut packets = Vec::new();
        let mut scratch = vec![0f32; per_packet];
        // Buffer de salida generoso: un paquete Opus de un solo frame jamás
        // supera ~1275 bytes por canal.
        let mut out = vec![0u8; 4000];
        let cap = out.len() as i32;

        let mut i = 0;
        while i < pcm.len() {
            let end = (i + per_packet).min(pcm.len());
            let chunk = &pcm[i..end];
            let input: &[f32] = if chunk.len() == per_packet {
                chunk
            } else {
                // Último frame parcial: copiar y rellenar con silencio.
                scratch[..chunk.len()].copy_from_slice(chunk);
                for s in &mut scratch[chunk.len()..] {
                    *s = 0.0;
                }
                &scratch
            };
            let n = self
                .enc
                .encode_float(input, frame as i32, &mut out, cap)
                .map_err(|e| OpusEncodeError::Backend(format!("{e:?}")))?;
            packets.push(out[..n as usize].to_vec());
            i += per_packet;
        }
        Ok(packets)
    }
}

/// Conveniencia de un tiro: encodea PCM y devuelve lo que el muxer necesita
/// para armar el track Opus — `(head, packets, samples_per_packet)`.
pub fn encode_to_opus_track(
    cfg: OpusEncoderConfig,
    pcm: &[f32],
) -> Result<(Vec<u8>, Vec<Vec<u8>>, u32), OpusEncodeError> {
    let mut enc = OpusEncoder::new(cfg)?;
    let head = enc.opus_head();
    let spp = enc.samples_per_packet();
    let packets = enc.encode_interleaved(pcm)?;
    Ok((head, packets, spp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_sizes_at_48k() {
        assert_eq!(FrameDuration::Ms2_5.samples(48_000), 120);
        assert_eq!(FrameDuration::Ms20.samples(48_000), 960);
        assert_eq!(FrameDuration::Ms60.samples(48_000), 2880);
    }

    #[test]
    fn rejects_bad_config() {
        assert!(matches!(
            OpusEncoder::new(OpusEncoderConfig {
                sample_rate: 44_100,
                ..Default::default()
            }),
            Err(OpusEncodeError::BadSampleRate(44_100))
        ));
        assert!(matches!(
            OpusEncoder::new(OpusEncoderConfig {
                channels: 3,
                ..Default::default()
            }),
            Err(OpusEncodeError::BadChannels(3))
        ));
    }

    #[test]
    fn opus_head_is_rfc_shaped() {
        let enc = OpusEncoder::new(OpusEncoderConfig {
            sample_rate: 48_000,
            channels: 2,
            ..Default::default()
        })
        .unwrap();
        let h = enc.opus_head();
        assert_eq!(&h[0..8], b"OpusHead");
        assert_eq!(h[8], 1); // versión
        assert_eq!(h[9], 2); // canales
        assert_eq!(u32::from_le_bytes([h[12], h[13], h[14], h[15]]), 48_000);
        assert_eq!(h[18], 0); // mapping family
        assert_eq!(h.len(), 19);
    }

    #[test]
    fn encodes_tone_into_packets() {
        let cfg = OpusEncoderConfig {
            sample_rate: 48_000,
            channels: 1,
            ..Default::default()
        };
        let mut enc = OpusEncoder::new(cfg).unwrap();
        // 100 ms de tono A4 a 48 kHz mono → 4800 muestras = 5 frames de 20 ms.
        let mut pcm = vec![0f32; 4800];
        for (i, s) in pcm.iter_mut().enumerate() {
            *s = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin() * 0.5;
        }
        let packets = enc.encode_interleaved(&pcm).unwrap();
        assert_eq!(packets.len(), 5, "5 frames de 20 ms");
        assert!(packets.iter().all(|p| !p.is_empty()), "ningún paquete vacío");
    }
}
