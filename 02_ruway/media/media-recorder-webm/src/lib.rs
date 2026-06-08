//! media-recorder-webm — graba **video + audio** a un único `.webm`
//! AV1+Opus nativo.
//!
//! Unifica los dos recorders sueltos del dominio —
//! [`media_recorder_av1`](https://docs.rs) (frames → `.ivf`) y
//! `media-recorder-wav` (audio → `.wav`) — en el **contenedor nativo** de
//! tawasuyu. En vez de dos archivos separados, tee'a el video por
//! [`media_encode_av1`] y el audio por [`media_encode_opus`], acumula los
//! paquetes y al [`stop`](WebmRecorder::stop) los muxea juntos con
//! [`media_mux_webm`] — **sin un solo byte de ffmpeg**.
//!
//! Mismo patrón de composición que los otros recorders: un handle clonable
//! (`Arc<Mutex<…>>`) se enchufa al pipeline vía dos wrappers transparentes
//! —[`RecordedFrameSource`] sobre el [`FrameSource`] y
//! [`RecordedAudioSource`] sobre el [`AudioSource`]— que sólo capturan
//! cuando el recorder está armado; si no, son no-ops.
//!
//! Decisiones heredadas del códec:
//!
//! - **Video.** Dimensiones fijas descubiertas del primer frame y
//!   congeladas al `start()` (como en `media-recorder-av1`). rav1e
//!   bufferea: los paquetes viven en RAM y el `.webm` se escribe entero en
//!   `stop()`.
//! - **Audio.** El encoder Opus se crea **perezosamente** al primer bloque
//!   de audio que llega durante la grabación, capturando su sample-rate y
//!   canales. Opus sólo admite 8/12/16/24/48 kHz y mono/estéreo: un formato
//!   fuera de eso **degrada a video-solo** (se cuenta, no rompe el
//!   pipeline). Como el encoder pide frames exactos (p.ej. 960 muestras),
//!   el audio entrante se acumula en un buffer y se drena por frames
//!   completos; el resto parcial se rellena con silencio recién en `stop()`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use media_core::{AudioSource, FrameSource};
use media_encode_av1::{Av1Encoder, Av1EncoderConfig, EncodedPacket};
use media_encode_opus::{FrameDuration, OpusEncoder, OpusEncoderConfig};
use media_mux_webm::{mux_webm_file, OpusTrack, WebmMuxConfig};

#[derive(Debug)]
pub enum RecorderError {
    AlreadyArmed,
    NotArmed,
    /// Todavía no pasó ningún frame — no se conocen las dimensiones.
    NoFormatYet,
    Encoder(String),
    Io(std::io::Error),
}

impl std::fmt::Display for RecorderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyArmed => write!(f, "recorder ya armado"),
            Self::NotArmed => write!(f, "recorder no armado"),
            Self::NoFormatYet => write!(
                f,
                "todavía no pasó ningún frame por el recorder — no sé width/height"
            ),
            Self::Encoder(e) => write!(f, "encoder: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for RecorderError {}

impl From<std::io::Error> for RecorderError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Parámetros de encode del video (las dimensiones se descubren del stream).
#[derive(Debug, Clone)]
pub struct WebmRecorderSettings {
    pub fps_num: u32,
    pub fps_den: u32,
    pub quantizer: usize,
    pub speed: u8,
    /// Tamaño del frame Opus para el audio.
    pub audio_frame: FrameDuration,
    /// Bitrate Opus en bits/s; `None` deja el default del encoder.
    pub audio_bitrate_bps: Option<i32>,
}

impl Default for WebmRecorderSettings {
    fn default() -> Self {
        Self {
            fps_num: 30,
            fps_den: 1,
            quantizer: 100,
            speed: 8,
            audio_frame: FrameDuration::Ms20,
            audio_bitrate_bps: None,
        }
    }
}

/// Estado del audio dentro de la grabación. `None` hasta que llega el primer
/// bloque; `Unsupported` si el formato no es codificable por Opus.
enum AudioState {
    Idle,
    Encoding {
        enc: OpusEncoder,
        sample_rate: u32,
        channels: u8,
        /// Muestras intercaladas pendientes de completar un frame.
        accum: Vec<f32>,
        /// Paquetes Opus emitidos.
        packets: Vec<Vec<u8>>,
    },
    Unsupported {
        sample_rate: u32,
        channels: u16,
    },
}

#[derive(Default)]
struct Inner {
    encoder: Option<Av1Encoder>,
    cfg: Option<Av1EncoderConfig>,
    packets: Vec<EncodedPacket>,
    path: Option<PathBuf>,
    last_w: u32,
    last_h: u32,
    dropped: u64,
    settings: WebmRecorderSettings,
    audio: AudioStateSlot,
}

/// Pequeño wrapper para que `AudioState` (no-`Default`) viva en un `Default`
/// `Inner`. Arranca en `Idle`.
struct AudioStateSlot(AudioState);
impl Default for AudioStateSlot {
    fn default() -> Self {
        AudioStateSlot(AudioState::Idle)
    }
}

/// Handle clonable que controla la grabación.
#[derive(Clone, Default)]
pub struct WebmRecorder {
    inner: Arc<Mutex<Inner>>,
}

impl WebmRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_settings(settings: WebmRecorderSettings) -> Self {
        let r = Self::default();
        r.lock().settings = settings;
        r
    }

    /// Cambia los parámetros de encode. Sólo aplica a la PRÓXIMA grabación.
    pub fn set_settings(&self, settings: WebmRecorderSettings) {
        self.lock().settings = settings;
    }

    /// Arma el recorder apuntando a `path`. Falla si ya estaba armado o si
    /// todavía no pasó ningún frame (dimensiones desconocidas). El `.webm`
    /// se escribe recién en [`stop`](Self::stop).
    pub fn start(&self, path: impl Into<PathBuf>) -> Result<PathBuf, RecorderError> {
        let path = path.into();
        let mut g = self.lock();
        if g.encoder.is_some() {
            return Err(RecorderError::AlreadyArmed);
        }
        if g.last_w == 0 || g.last_h == 0 {
            return Err(RecorderError::NoFormatYet);
        }
        let cfg = Av1EncoderConfig {
            width: g.last_w,
            height: g.last_h,
            fps_num: g.settings.fps_num,
            fps_den: g.settings.fps_den,
            quantizer: g.settings.quantizer,
            speed: g.settings.speed,
            threads: 0,
        };
        let enc = Av1Encoder::new(cfg.clone()).map_err(|e| RecorderError::Encoder(e.to_string()))?;
        g.encoder = Some(enc);
        g.cfg = Some(cfg);
        g.packets.clear();
        g.dropped = 0;
        g.path = Some(path.clone());
        g.audio = AudioStateSlot(AudioState::Idle);
        Ok(path)
    }

    /// Cierra ambos streams, los muxea y escribe el `.webm`. Devuelve el path
    /// y un [`RecordingSummary`]. Falla si no estaba armado.
    pub fn stop(&self) -> Result<(PathBuf, RecordingSummary), RecorderError> {
        let mut g = self.lock();
        let mut enc = g.encoder.take().ok_or(RecorderError::NotArmed)?;
        let cfg = g.cfg.take().unwrap_or_default();
        let path = g.path.take().unwrap_or_default();

        // Vaciar la tubería del video.
        let tail = enc
            .finish()
            .map_err(|e| RecorderError::Encoder(e.to_string()))?;
        g.packets.extend(tail);
        let video: Vec<Vec<u8>> = std::mem::take(&mut g.packets)
            .into_iter()
            .map(|p| p.data)
            .collect();

        // Cerrar el audio: drenar el resto parcial (con relleno de silencio).
        let audio_state = std::mem::replace(&mut g.audio, AudioStateSlot(AudioState::Idle)).0;
        let (audio_track, audio_packets, audio_sr, audio_ch) = match audio_state {
            AudioState::Encoding {
                mut enc,
                sample_rate,
                channels,
                accum,
                mut packets,
            } => {
                if !accum.is_empty() {
                    if let Ok(tail) = enc.encode_interleaved(&accum) {
                        packets.extend(tail);
                    }
                }
                let np = packets.len();
                let track = OpusTrack {
                    head: enc.opus_head(),
                    sample_rate,
                    channels,
                    samples_per_packet: enc.samples_per_packet(),
                    packets,
                };
                (Some(track), np, sample_rate, channels as u16)
            }
            AudioState::Unsupported {
                sample_rate,
                channels,
            } => (None, 0, sample_rate, channels),
            AudioState::Idle => (None, 0, 0, 0),
        };

        let mux_cfg = WebmMuxConfig {
            width: cfg.width,
            height: cfg.height,
            fps_num: cfg.fps_num,
            fps_den: cfg.fps_den,
        };
        let nv = video.len();
        // Soltamos el lock antes del I/O.
        drop(g);

        mux_webm_file(&path, &mux_cfg, &video, audio_track.as_ref())?;
        Ok((
            path,
            RecordingSummary {
                video_frames: nv,
                audio_packets,
                audio_sample_rate: audio_sr,
                audio_channels: audio_ch,
            },
        ))
    }

    pub fn is_recording(&self) -> bool {
        self.lock().encoder.is_some()
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.lock().path.clone()
    }

    pub fn last_dimensions(&self) -> (u32, u32) {
        let g = self.lock();
        (g.last_w, g.last_h)
    }

    pub fn dropped_frames(&self) -> u64 {
        self.lock().dropped
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }
}

/// Resumen de lo que se grabó, devuelto por [`WebmRecorder::stop`].
#[derive(Debug, Clone, Copy)]
pub struct RecordingSummary {
    pub video_frames: usize,
    /// 0 si no se capturó audio (no llegó o el formato no era Opus-able).
    pub audio_packets: usize,
    pub audio_sample_rate: u32,
    pub audio_channels: u16,
}

/// Wrapper de [`FrameSource`] que encodea cada frame al recorder si está
/// armado. Idéntico al de `media-recorder-av1`.
pub struct RecordedFrameSource<S> {
    inner: S,
    recorder: WebmRecorder,
}

impl<S> RecordedFrameSource<S> {
    pub fn new(inner: S, recorder: WebmRecorder) -> Self {
        Self { inner, recorder }
    }
}

impl<S: FrameSource> FrameSource for RecordedFrameSource<S> {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        let dims = self.inner.tick(dt, buf)?;
        let (w, h) = dims;
        let mut g = self.recorder.lock();
        g.last_w = w;
        g.last_h = h;
        if let Some(cfg) = g.cfg.clone() {
            if (cfg.width, cfg.height) == dims && buf.len() == (w * h * 4) as usize {
                if let Some(enc) = g.encoder.as_mut() {
                    match enc.encode_rgba(buf) {
                        Ok(pkts) => g.packets.extend(pkts),
                        Err(_) => g.dropped += 1,
                    }
                }
            } else {
                g.dropped += 1;
            }
        }
        Some(dims)
    }
}

/// Wrapper de [`AudioSource`] que tee'a el audio al recorder, encodeándolo
/// a Opus por frames completos. No-op cuando el recorder no está armado.
pub struct RecordedAudioSource<S> {
    inner: S,
    recorder: WebmRecorder,
}

impl<S> RecordedAudioSource<S> {
    pub fn new(inner: S, recorder: WebmRecorder) -> Self {
        Self { inner, recorder }
    }
}

impl<S: AudioSource> AudioSource for RecordedAudioSource<S> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.inner.fill(buf, sample_rate, channels);
        let mut g = self.recorder.lock();
        // Sólo capturamos durante una grabación de video activa.
        if g.encoder.is_none() {
            return;
        }
        let frame = g.settings.audio_frame;
        let bitrate = g.settings.audio_bitrate_bps;

        // Inicialización perezosa del encoder Opus al primer bloque.
        if matches!(g.audio.0, AudioState::Idle) {
            let ch8 = channels.min(2).max(1) as u8;
            let cfg = OpusEncoderConfig {
                sample_rate,
                channels: ch8,
                bitrate_bps: bitrate,
                frame,
            };
            match OpusEncoder::new(cfg) {
                Ok(enc) => {
                    g.audio = AudioStateSlot(AudioState::Encoding {
                        enc,
                        sample_rate,
                        channels: ch8,
                        accum: Vec::new(),
                        packets: Vec::new(),
                    });
                }
                Err(_) => {
                    g.audio = AudioStateSlot(AudioState::Unsupported {
                        sample_rate,
                        channels,
                    });
                }
            }
        }

        if let AudioState::Encoding {
            enc,
            channels: enc_ch,
            accum,
            packets,
            ..
        } = &mut g.audio.0
        {
            let ch = *enc_ch as usize;
            let per_packet = enc.samples_per_packet() as usize * ch;
            if per_packet == 0 {
                return;
            }
            // Acumular el bloque entrante (recortado/expandido a los canales
            // del encoder no hace falta: confiamos en que el pipeline mantiene
            // el formato; si el caller cambia de canales mid-stream, el audio
            // se desalinea — igual contrato que el resto del dominio).
            accum.extend_from_slice(buf);
            // Drenar todos los frames completos disponibles.
            let complete = (accum.len() / per_packet) * per_packet;
            if complete > 0 {
                let chunk: Vec<f32> = accum.drain(..complete).collect();
                if let Ok(pkts) = enc.encode_interleaved(&chunk) {
                    packets.extend(pkts);
                }
            }
        }
    }
}

/// Conveniencia: nombra archivos `media-rec-<epoch>.webm` (orden
/// lexicográfico = cronológico, sin dep de chrono).
pub fn default_recording_path(dir: impl AsRef<Path>) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    dir.as_ref().join(format!("media-rec-{secs}.webm"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SolidSource {
        w: u32,
        h: u32,
    }
    impl FrameSource for SolidSource {
        fn tick(&mut self, _dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
            buf.resize((self.w * self.h * 4) as usize, 0);
            for px in buf.chunks_exact_mut(4) {
                px.copy_from_slice(&[120, 120, 120, 255]);
            }
            Some((self.w, self.h))
        }
    }

    #[test]
    fn start_without_frame_fails() {
        let rec = WebmRecorder::new();
        assert!(matches!(
            rec.start("/tmp/nope.webm").unwrap_err(),
            RecorderError::NoFormatYet
        ));
    }

    #[test]
    fn stop_when_not_armed_fails() {
        let rec = WebmRecorder::new();
        assert!(matches!(rec.stop().unwrap_err(), RecorderError::NotArmed));
    }

    #[test]
    fn transparent_when_not_armed() {
        let rec = WebmRecorder::new();
        let mut src = RecordedFrameSource::new(SolidSource { w: 32, h: 32 }, rec.clone());
        let mut buf = Vec::new();
        assert_eq!(src.tick(Duration::from_millis(33), &mut buf), Some((32, 32)));
        assert_eq!(rec.last_dimensions(), (32, 32));
        assert!(!rec.is_recording());
    }
}
