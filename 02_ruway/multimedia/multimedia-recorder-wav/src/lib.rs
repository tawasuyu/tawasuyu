//! multimedia-recorder-wav — captura el stream de audio a un archivo WAV.
//!
//! [`WavRecorder`] es un handle clonable (`Arc<Mutex<...>>`) que se
//! enchufa al stream vía [`RecordedAudioSource`]: cada bloque que
//! pasa por `fill` se duplica al writer si el recorder está armado.
//! Cuando no lo está, el wrapper es un no-op transparente.
//!
//! La sample rate y los canales se descubren del primer bloque que
//! atraviesa el pipeline; el writer se crea perezosamente al
//! `start()` siguiente, usando el último `(sr, ch)` visto. Si nunca
//! pasó audio antes de `start()`, el archivo queda preparado al ver
//! el primer bloque.
//!
//! El callback de cpal **bloquea** brevemente en el lock al escribir
//! — el writer de hound es sync. Para una grabación corta (jingles,
//! samples) está bien; para grabaciones largas o multi-track habría
//! que mover la escritura a un thread separado con un canal.
//!
//! Formato fijo: PCM 16 bits intercalado. Es el WAV "universal" y
//! reduce el archivo a la mitad vs f32. La cuantización es honest
//! (clamp a [-1, 1] + round).

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hound::{SampleFormat, WavSpec, WavWriter};
use multimedia_core::AudioSource;

#[derive(Debug)]
pub enum RecorderError {
    AlreadyArmed,
    NotArmed,
    NoFormatYet,
    Io(std::io::Error),
    Hound(hound::Error),
}

impl std::fmt::Display for RecorderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyArmed => write!(f, "recorder ya armado"),
            Self::NotArmed => write!(f, "recorder no armado"),
            Self::NoFormatYet => write!(
                f,
                "todavía no pasó audio por el recorder — no sé sr/channels"
            ),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Hound(e) => write!(f, "hound: {e}"),
        }
    }
}

impl std::error::Error for RecorderError {}

impl From<std::io::Error> for RecorderError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<hound::Error> for RecorderError {
    fn from(e: hound::Error) -> Self {
        Self::Hound(e)
    }
}

/// Handle clonable que controla el estado de grabación.
#[derive(Clone, Default)]
pub struct WavRecorder {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    /// `Some` cuando hay grabación en curso.
    writer: Option<WavWriter<BufWriter<File>>>,
    /// Path del archivo activo — para reportar al stop.
    path: Option<PathBuf>,
    /// Último formato observado en el stream.
    last_sr: u32,
    last_ch: u16,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            writer: None,
            path: None,
            last_sr: 0,
            last_ch: 0,
        }
    }
}

impl WavRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Arma el recorder apuntando a `path`. Falla si ya estaba armado
    /// o si el formato del stream todavía no es conocido (es decir,
    /// nunca pasó un bloque por el wrapper). Sobrescribe el archivo
    /// si ya existía.
    pub fn start(&self, path: impl Into<PathBuf>) -> Result<PathBuf, RecorderError> {
        let path = path.into();
        let mut g = self.lock();
        if g.writer.is_some() {
            return Err(RecorderError::AlreadyArmed);
        }
        if g.last_sr == 0 || g.last_ch == 0 {
            return Err(RecorderError::NoFormatYet);
        }
        let spec = WavSpec {
            channels: g.last_ch,
            sample_rate: g.last_sr,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let file = File::create(&path)?;
        let writer = WavWriter::new(BufWriter::new(file), spec)?;
        g.writer = Some(writer);
        g.path = Some(path.clone());
        Ok(path)
    }

    /// Cierra el archivo activo y devuelve su path. Falla si no
    /// estaba armado.
    pub fn stop(&self) -> Result<PathBuf, RecorderError> {
        let mut g = self.lock();
        let writer = g.writer.take().ok_or(RecorderError::NotArmed)?;
        let path = g.path.take().unwrap_or_else(PathBuf::new);
        writer.finalize()?;
        Ok(path)
    }

    pub fn is_recording(&self) -> bool {
        self.lock().writer.is_some()
    }

    /// `Some(path)` si está grabando; `None` si no.
    pub fn current_path(&self) -> Option<PathBuf> {
        self.lock().path.clone()
    }

    /// Cantidad de canales/sr del último bloque visto. `(0, 0)` si
    /// todavía no pasó nada.
    pub fn last_format(&self) -> (u32, u16) {
        let g = self.lock();
        (g.last_sr, g.last_ch)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }
}

/// Wrapper de [`AudioSource`] que duplica cada bloque al [`WavRecorder`]
/// si está armado. Igual orden de composición que [`ProbedAudioSource`]:
/// el wrapper externo decide si captura, el inner no se entera.
///
/// [`ProbedAudioSource`]: multimedia_core::ProbedAudioSource
pub struct RecordedAudioSource<S> {
    inner: S,
    recorder: WavRecorder,
}

impl<S> RecordedAudioSource<S> {
    pub fn new(inner: S, recorder: WavRecorder) -> Self {
        Self { inner, recorder }
    }
}

impl<S: AudioSource> AudioSource for RecordedAudioSource<S> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.inner.fill(buf, sample_rate, channels);
        let mut g = self.recorder.lock();
        g.last_sr = sample_rate;
        g.last_ch = channels;
        if let Some(writer) = g.writer.as_mut() {
            // hound::WavWriter es buffered; el cost por sample es
            // bajo. Errores los descartamos silenciosamente para no
            // matar el callback realtime — un eprintln tampoco corre
            // bien acá.
            for &s in buf.iter() {
                let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
                let _ = writer.write_sample(v);
            }
        }
    }
}

/// Atajo: stop con manejo defensivo (no panickea si ya fue cerrado).
/// Útil al dropear el handle del lado UI.
pub fn try_stop(rec: &WavRecorder) -> Option<PathBuf> {
    if !rec.is_recording() {
        return None;
    }
    rec.stop().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Constant(f32);
    impl AudioSource for Constant {
        fn fill(&mut self, buf: &mut [f32], _: u32, _: u16) {
            for s in buf.iter_mut() {
                *s = self.0;
            }
        }
    }

    #[test]
    fn round_trip_short_recording() {
        let dir = std::env::temp_dir();
        let path = dir.join("multimedia_recorder_wav_test.wav");
        let _ = std::fs::remove_file(&path);

        let rec = WavRecorder::new();
        let mut src = RecordedAudioSource::new(Constant(0.5), rec.clone());

        // Bombeamos un bloque para que se descubra el formato.
        let mut buf = vec![0.0_f32; 480 * 2];
        src.fill(&mut buf, 48_000, 2);
        assert_eq!(rec.last_format(), (48_000, 2));

        let started = rec.start(&path).unwrap();
        assert_eq!(started, path);
        for _ in 0..10 {
            src.fill(&mut buf, 48_000, 2);
        }
        let closed = rec.stop().unwrap();
        assert_eq!(closed, path);
        assert!(!rec.is_recording());

        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.sample_rate, 48_000);
        assert_eq!(spec.bits_per_sample, 16);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn start_without_format_fails() {
        let rec = WavRecorder::new();
        let path = std::env::temp_dir().join("multimedia_recorder_wav_no_fmt.wav");
        let err = rec.start(&path).unwrap_err();
        assert!(matches!(err, RecorderError::NoFormatYet));
    }

    #[test]
    fn stop_when_not_armed_fails() {
        let rec = WavRecorder::new();
        let err = rec.stop().unwrap_err();
        assert!(matches!(err, RecorderError::NotArmed));
    }
}

/// Conveniencia para nombrar archivos `multimedia-rec-YYYYMMDD-HHMMSS.wav`.
/// El timestamp es UTC en segundos desde EPOCH formateado a mano para
/// no traer dep de chrono — el orden lexicográfico igual queda
/// cronológico.
pub fn default_recording_path(dir: impl AsRef<Path>) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = format!("multimedia-rec-{secs}.wav");
    dir.as_ref().join(name)
}
