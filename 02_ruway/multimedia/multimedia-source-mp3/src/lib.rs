//! multimedia-source-mp3 — decoder MP3 (vía symphonia) como [`AudioSource`].
//!
//! Misma forma que [`multimedia_source_wav::WavSource`]: decodea el
//! archivo entero a `f32` intercalado al construir, y el callback
//! reproduce en loop con resampleo lineal cuando el sample rate del
//! sink difiere del del MP3. Trade-off explícito: RAM = duración ·
//! sample_rate · channels · 4 bytes. Para audios largos habría que
//! hacer streaming por bloques.
//!
//! Implementa [`multimedia_core::Seekable`] sobre el cursor de
//! frames del source, igual que WavSource — así el wrapper del app
//! puede mover la posición sin saber el formato.

use std::fs::File;
use std::path::Path;
use std::time::Duration;

use multimedia_core::{AudioSource, Seekable};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug)]
pub enum Mp3Error {
    Io(std::io::Error),
    Symphonia(SymphError),
    NoAudioTrack,
    Empty,
}

impl std::fmt::Display for Mp3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Symphonia(e) => write!(f, "symphonia: {e}"),
            Self::NoAudioTrack => write!(f, "el archivo no tiene un track de audio"),
            Self::Empty => write!(f, "MP3 sin samples"),
        }
    }
}

impl std::error::Error for Mp3Error {}

impl From<std::io::Error> for Mp3Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<SymphError> for Mp3Error {
    fn from(e: SymphError) -> Self {
        Self::Symphonia(e)
    }
}

/// Reproductor MP3 en loop. Misma semántica de `cursor` y resampleo
/// lineal que `WavSource`.
pub struct Mp3Source {
    samples: Vec<f32>,
    src_channels: u16,
    src_sample_rate: u32,
    cursor: f64,
    /// Multiplicador de velocidad — mismo modo varispeed que WAV (no
    /// hay time-stretching: cambia pitch). Clampeado en `set_speed`.
    speed: f32,
}

impl Mp3Source {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Mp3Error> {
        let file = File::open(path.as_ref())?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.as_ref().extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        } else {
            hint.with_extension("mp3");
        }
        let probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;
        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| {
                t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL
            })
            .ok_or(Mp3Error::NoAudioTrack)?;
        let track_id = track.id;
        let codec_params = track.codec_params.clone();
        let mut decoder = symphonia::default::get_codecs()
            .make(&codec_params, &DecoderOptions::default())?;

        let mut interleaved: Vec<f32> = Vec::new();
        let mut sample_rate: u32 = codec_params.sample_rate.unwrap_or(44_100);
        let mut channels: u16 = codec_params
            .channels
            .map(|c| c.count() as u16)
            .unwrap_or(2);

        loop {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(SymphError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(SymphError::ResetRequired) => {
                    // Cambio de stream: reabrimos el decoder.
                    decoder = symphonia::default::get_codecs()
                        .make(&codec_params, &DecoderOptions::default())?;
                    continue;
                }
                Err(e) => return Err(Mp3Error::Symphonia(e)),
            };
            if packet.track_id() != track_id {
                continue;
            }
            let decoded = match decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymphError::DecodeError(_)) => continue,
                Err(e) => return Err(Mp3Error::Symphonia(e)),
            };
            let spec = *decoded.spec();
            sample_rate = spec.rate;
            channels = spec.channels.count() as u16;
            append_interleaved(&decoded, &mut interleaved);
        }

        if interleaved.is_empty() {
            return Err(Mp3Error::Empty);
        }
        Ok(Self {
            samples: interleaved,
            src_channels: channels.max(1),
            src_sample_rate: sample_rate.max(1),
            cursor: 0.0,
            speed: 1.0,
        })
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(0.1, 4.0);
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn source_channels(&self) -> u16 {
        self.src_channels
    }

    pub fn source_sample_rate(&self) -> u32 {
        self.src_sample_rate
    }

    pub fn duration_seconds(&self) -> f32 {
        let frames = self.samples.len() as f32 / self.src_channels.max(1) as f32;
        frames / self.src_sample_rate as f32
    }

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

impl Seekable for Mp3Source {
    fn position(&self) -> Duration {
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

impl AudioSource for Mp3Source {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        let out_channels = channels.max(1) as usize;
        let sink_sr = sample_rate.max(1) as f64;
        let src_sr = self.src_sample_rate.max(1) as f64;
        let step = (src_sr / sink_sr) * self.speed as f64;
        let frames = buf.len() / out_channels;
        let mut cursor = self.cursor;
        for frame in 0..frames {
            for ch in 0..out_channels {
                let v = self.sample_at(cursor, ch as u16);
                buf[frame * out_channels + ch] = v;
            }
            cursor += step;
        }
        let src_ch = self.src_channels.max(1) as usize;
        let total_frames = (self.samples.len() / src_ch) as f64;
        if total_frames > 0.0 {
            cursor = cursor.rem_euclid(total_frames);
        }
        self.cursor = cursor;
        let tail = frames * out_channels;
        for s in &mut buf[tail..] {
            *s = 0.0;
        }
    }
}

/// Convierte un `AudioBufferRef` (planar, tipo variable) a `f32`
/// intercalado y lo apendea al destino. Soporta todos los formatos
/// que symphonia entrega — para los mp3 normalmente es F32 o S16/S32
/// según build, así que cubrimos los comunes.
fn append_interleaved(decoded: &AudioBufferRef<'_>, out: &mut Vec<f32>) {
    fn push_planar<S, F>(buf: &symphonia::core::audio::AudioBuffer<S>, out: &mut Vec<f32>, conv: F)
    where
        S: symphonia::core::sample::Sample + Copy,
        F: Fn(S) -> f32,
    {
        let spec = *buf.spec();
        let ch = spec.channels.count();
        let frames = buf.frames();
        for f in 0..frames {
            for c in 0..ch {
                let v = buf.chan(c)[f];
                out.push(conv(v));
            }
        }
    }
    match decoded {
        AudioBufferRef::F32(b) => push_planar(b, out, |s| s),
        AudioBufferRef::S16(b) => push_planar(b, out, |s| s as f32 / i16::MAX as f32),
        AudioBufferRef::S32(b) => push_planar(b, out, |s| s as f32 / i32::MAX as f32),
        AudioBufferRef::U8(b) => push_planar(b, out, |s| (s as f32 - 128.0) / 128.0),
        AudioBufferRef::U16(b) => push_planar(b, out, |s| (s as f32 - 32768.0) / 32768.0),
        AudioBufferRef::U32(b) => {
            push_planar(b, out, |s| (s as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32)
        }
        AudioBufferRef::S8(b) => push_planar(b, out, |s| s as f32 / i8::MAX as f32),
        AudioBufferRef::S24(b) => push_planar(b, out, |s| s.inner() as f32 / (1 << 23) as f32),
        AudioBufferRef::U24(b) => push_planar(b, out, |s| {
            (s.inner() as f32 - (1 << 23) as f32) / (1 << 23) as f32
        }),
        AudioBufferRef::F64(b) => push_planar(b, out, |s| s as f32),
    }
}
