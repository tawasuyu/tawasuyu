//! media-recorder-av1 — captura el stream de video a un archivo `.ivf`
//! AV1 nativo. La **contraparte de video** de `media-recorder-wav`: ese
//! tee'a el audio a WAV, éste tee'a los frames a AV1 (vía
//! [`media_encode_av1`]) — sin ffmpeg.
//!
//! [`Av1Recorder`] es un handle clonable (`Arc<Mutex<...>>`) que se
//! enchufa al pipeline vía [`RecordedFrameSource`]: cada frame que pasa
//! por `tick` se encodea al stream AV1 si el recorder está armado. Cuando
//! no lo está, el wrapper es un no-op transparente — exactamente el mismo
//! patrón de composición que `RecordedAudioSource`.
//!
//! Dos diferencias con el recorder de audio nacen del códec:
//!
//! - **Dimensiones fijas.** Un stream AV1 tiene tamaño constante; las
//!   dimensiones se descubren del primer frame que atraviesa el wrapper
//!   (como sr/channels en WAV) y quedan congeladas al `start()`. Frames de
//!   otro tamaño durante la grabación se descartan.
//! - **Latencia + cierre.** rav1e bufferea (lookahead): los paquetes se
//!   acumulan en memoria y el `.ivf` se escribe entero en `stop()`, cuando
//!   `finish()` vacía la tubería y ya se conoce el conteo de frames para la
//!   cabecera IVF. Para grabaciones muy largas convendría escribir
//!   incremental con num_frames=0; hoy se prioriza la simplicidad.
//!
//! El `tick` retiene el lock mientras encodea (rav1e no es instantáneo) —
//! igual tradeoff que el writer sync de hound en el recorder de audio.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use media_core::FrameSource;
use media_encode_av1::{Av1Encoder, Av1EncoderConfig, EncodedPacket, write_ivf_file};

#[derive(Debug)]
pub enum RecorderError {
    AlreadyArmed,
    NotArmed,
    /// Todavía no pasó ningún frame por el wrapper — no se conocen las
    /// dimensiones del video.
    NoFormatYet,
    /// rav1e rechazó la config (dimensiones inválidas, etc.).
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

/// Parámetros de encode del recorder. Las dimensiones NO van acá: se
/// descubren del stream. `fps` es la cadencia que se grabará en la
/// cabecera IVF (no se mide del `dt` real — el caller declara el target).
#[derive(Debug, Clone)]
pub struct Av1RecorderSettings {
    pub fps_num: u32,
    pub fps_den: u32,
    pub quantizer: usize,
    pub speed: u8,
}

impl Default for Av1RecorderSettings {
    fn default() -> Self {
        Self {
            fps_num: 30,
            fps_den: 1,
            quantizer: 100,
            speed: 8,
        }
    }
}

/// Handle clonable que controla el estado de grabación.
#[derive(Clone, Default)]
pub struct Av1Recorder {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// `Some` mientras hay grabación en curso.
    encoder: Option<Av1Encoder>,
    /// Config con la que se creó el encoder — para la cabecera IVF al stop.
    cfg: Option<Av1EncoderConfig>,
    /// Paquetes acumulados de la grabación en curso.
    packets: Vec<EncodedPacket>,
    /// Path del archivo activo.
    path: Option<PathBuf>,
    /// Últimas dimensiones observadas en el stream.
    last_w: u32,
    last_h: u32,
    /// Frames descartados por no coincidir con las dimensiones congeladas.
    dropped: u64,
    /// Parámetros de encode.
    settings: Av1RecorderSettings,
}

impl Av1Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construye con parámetros de encode propios (calidad, velocidad, fps).
    pub fn with_settings(settings: Av1RecorderSettings) -> Self {
        let r = Self::default();
        r.lock().settings = settings;
        r
    }

    /// Cambia los parámetros de encode. Sólo aplica a la PRÓXIMA grabación
    /// (no toca un encoder ya armado).
    pub fn set_settings(&self, settings: Av1RecorderSettings) {
        self.lock().settings = settings;
    }

    /// Arma el recorder apuntando a `path`. Falla si ya estaba armado o si
    /// todavía no pasó ningún frame (dimensiones desconocidas). El archivo
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
        Ok(path)
    }

    /// Cierra el stream, vacía la tubería del encoder y escribe el `.ivf`.
    /// Devuelve el path y la cantidad de frames escritos. Falla si no
    /// estaba armado.
    pub fn stop(&self) -> Result<(PathBuf, usize), RecorderError> {
        let mut g = self.lock();
        let mut enc = g.encoder.take().ok_or(RecorderError::NotArmed)?;
        let cfg = g.cfg.take().unwrap_or_default();
        let path = g.path.take().unwrap_or_default();
        let tail = enc
            .finish()
            .map_err(|e| RecorderError::Encoder(e.to_string()))?;
        g.packets.extend(tail);
        let packets = std::mem::take(&mut g.packets);
        let n = packets.len();
        // Soltamos el lock antes del I/O por si el writer bloquea.
        drop(g);
        write_ivf_file(&path, &cfg, &packets)?;
        Ok((path, n))
    }

    pub fn is_recording(&self) -> bool {
        self.lock().encoder.is_some()
    }

    /// `Some(path)` si está grabando; `None` si no.
    pub fn current_path(&self) -> Option<PathBuf> {
        self.lock().path.clone()
    }

    /// Últimas dimensiones vistas en el stream. `(0, 0)` si nada pasó aún.
    pub fn last_dimensions(&self) -> (u32, u32) {
        let g = self.lock();
        (g.last_w, g.last_h)
    }

    /// Frames descartados en la grabación actual por dimensiones distintas
    /// a las congeladas al `start()`.
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

/// Wrapper de [`FrameSource`] que encodea cada frame al [`Av1Recorder`] si
/// está armado. Mismo orden de composición que `RecordedAudioSource`: el
/// wrapper externo decide si captura, el inner no se entera.
pub struct RecordedFrameSource<S> {
    inner: S,
    recorder: Av1Recorder,
}

impl<S> RecordedFrameSource<S> {
    pub fn new(inner: S, recorder: Av1Recorder) -> Self {
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
            // Sólo encodeamos si el frame coincide con las dimensiones
            // congeladas; de lo contrario lo contamos como descartado.
            if (cfg.width, cfg.height) == dims && buf.len() == (w * h * 4) as usize {
                if let Some(enc) = g.encoder.as_mut() {
                    // Errores los tragamos para no romper el pipeline de
                    // playback — igual criterio que el recorder de audio.
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

/// Conveniencia para nombrar archivos `media-vid-<epoch>.ivf`. El orden
/// lexicográfico queda cronológico; sin dep de chrono.
pub fn default_recording_path(dir: impl AsRef<Path>) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    dir.as_ref().join(format!("media-vid-{secs}.ivf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FrameSource de prueba: emite siempre un frame de color sólido del
    /// tamaño dado.
    struct SolidSource {
        w: u32,
        h: u32,
        rgb: (u8, u8, u8),
    }

    impl FrameSource for SolidSource {
        fn tick(&mut self, _dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
            buf.resize((self.w * self.h * 4) as usize, 0);
            for px in buf.chunks_exact_mut(4) {
                px[0] = self.rgb.0;
                px[1] = self.rgb.1;
                px[2] = self.rgb.2;
                px[3] = 255;
            }
            Some((self.w, self.h))
        }
    }

    #[test]
    fn start_without_frame_fails() {
        let rec = Av1Recorder::new();
        let err = rec.start("/tmp/nope.ivf").unwrap_err();
        assert!(matches!(err, RecorderError::NoFormatYet));
    }

    #[test]
    fn stop_when_not_armed_fails() {
        let rec = Av1Recorder::new();
        let err = rec.stop().unwrap_err();
        assert!(matches!(err, RecorderError::NotArmed));
    }

    #[test]
    fn transparent_when_not_armed() {
        // Sin armar, el wrapper pasa frames sin tocar nada.
        let rec = Av1Recorder::new();
        let mut src = RecordedFrameSource::new(
            SolidSource { w: 32, h: 32, rgb: (10, 20, 30) },
            rec.clone(),
        );
        let mut buf = Vec::new();
        let dims = src.tick(Duration::from_millis(33), &mut buf);
        assert_eq!(dims, Some((32, 32)));
        assert_eq!(rec.last_dimensions(), (32, 32));
        assert!(!rec.is_recording());
    }
}
