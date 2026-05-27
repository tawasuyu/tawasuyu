//! `takiy-playback` — reproducción en vivo de un `AudioBuffer`.
//!
//! Abre el dispositivo de salida default del SO con [`cpal`] y va
//! consumiendo un buffer mono `f32` que entrega [`takiy-synth`]. El
//! stream se queda corriendo siempre; cuando no hay nada que tocar
//! emite silencio, así no hay que parar/reabrir el device entre
//! reproducciones (eso causa clicks horribles en ALSA/PulseAudio).
//!
//! Modelo de uso:
//!
//! ```no_run
//! use takiy_playback::Player;
//! use takiy_synth::{OscRenderer, Renderer};
//! use takiy_core::Score;
//!
//! let player = Player::open().unwrap();
//! let score = Score::new(120.0);
//! let buf = OscRenderer { sample_rate: player.sample_rate(), ..Default::default() }
//!     .render(&score);
//! player.play(buf);
//! ```
//!
//! El sample rate del [`AudioBuffer`] debe coincidir con
//! [`Player::sample_rate`]; si no, se reproduce a la velocidad del
//! device (queda más agudo o más grave). Resamplear es problema del
//! renderer — la idea es que cada renderer apunte directo al SR del
//! device.

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, StreamConfig};
use takiy_synth::AudioBuffer;

/// Errores al abrir el dispositivo de audio.
#[derive(Debug)]
pub enum OpenError {
    /// No hay dispositivo de salida default disponible.
    NoOutputDevice,
    /// No se pudo enumerar configuraciones soportadas.
    NoSupportedConfig,
    /// Falla genérica del backend (cpal devuelve `String` en muchos sitios).
    Backend(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOutputDevice => write!(f, "no hay dispositivo de audio de salida"),
            Self::NoSupportedConfig => write!(f, "el dispositivo no expone configuraciones de salida"),
            Self::Backend(e) => write!(f, "backend de audio: {e}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// Estado compartido con el callback de audio. El callback lee de aquí
/// muestra-a-muestra; el thread de UI escribe nuevos buffers al disparar
/// `play` o `stop`.
#[derive(Default)]
struct Shared {
    /// Buffer activo. `None` = silencio.
    buffer: Option<Arc<AudioBuffer>>,
    /// Posición de lectura dentro del buffer activo, en muestras.
    cursor: usize,
}

/// Reproductor de audio. Mientras el `Player` viva, el stream del
/// device queda abierto. Drop lo cierra.
pub struct Player {
    shared: Arc<Mutex<Shared>>,
    sample_rate: u32,
    channels: u16,
    // El stream debe vivir tanto como el player: si lo dropeás, cpal
    // para el callback. No es Send en algunas plataformas; el `Player`
    // tampoco lo necesita ser, vive en el thread de UI.
    _stream: cpal::Stream,
}

impl Player {
    /// Abre el dispositivo de salida default y arranca el stream.
    pub fn open() -> Result<Self, OpenError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(OpenError::NoOutputDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| OpenError::Backend(e.to_string()))?;

        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.config();
        let sample_rate = config.sample_rate.0;
        let channels = config.channels;

        let shared: Arc<Mutex<Shared>> = Arc::new(Mutex::new(Shared::default()));
        let err_fn = |e| eprintln!("takiy-playback · stream error: {e}");

        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&device, &config, shared.clone(), err_fn),
            SampleFormat::I16 => build_stream::<i16>(&device, &config, shared.clone(), err_fn),
            SampleFormat::U16 => build_stream::<u16>(&device, &config, shared.clone(), err_fn),
            other => Err(OpenError::Backend(format!("formato {other:?} no soportado"))),
        }?;
        stream.play().map_err(|e| OpenError::Backend(e.to_string()))?;

        Ok(Self { shared, sample_rate, channels, _stream: stream })
    }

    /// Sample rate del device — usalo al pedirle al renderer.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Canales del device (1 = mono, 2 = estéreo). Útil sólo para diagnóstico;
    /// el callback duplica mono → todos los canales automáticamente.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Encola un buffer para reproducción. Reemplaza cualquier buffer en curso
    /// (no se mezclan — la idea es "tocar esto ahora"). El buffer se envuelve
    /// en `Arc` para que el callback lo pueda compartir con quien lo creó.
    pub fn play(&self, buffer: AudioBuffer) {
        let mut s = self.shared.lock().expect("audio shared lock");
        s.buffer = Some(Arc::new(buffer));
        s.cursor = 0;
    }

    /// Detiene la reproducción y deja sonando silencio.
    pub fn stop(&self) {
        let mut s = self.shared.lock().expect("audio shared lock");
        s.buffer = None;
        s.cursor = 0;
    }

    /// `true` si todavía queda buffer pendiente por reproducir.
    pub fn is_playing(&self) -> bool {
        let s = self.shared.lock().expect("audio shared lock");
        s.buffer.as_ref().is_some_and(|b| s.cursor < b.samples.len())
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<Mutex<Shared>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, OpenError>
where
    T: Sample + cpal::SizedSample + cpal::FromSample<f32> + Send + 'static,
{
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _info| fill::<T>(out, channels, &shared),
            err_fn,
            None,
        )
        .map_err(|e| OpenError::Backend(e.to_string()))
}

fn fill<T>(out: &mut [T], channels: usize, shared: &Mutex<Shared>)
where
    T: Sample + cpal::FromSample<f32>,
{
    let silence = T::from_sample(0.0f32);
    let mut s = match shared.lock() {
        Ok(g) => g,
        Err(_) => {
            for sample in out.iter_mut() {
                *sample = silence;
            }
            return;
        }
    };

    let buffer = match s.buffer.clone() {
        Some(b) => b,
        None => {
            for sample in out.iter_mut() {
                *sample = silence;
            }
            return;
        }
    };

    let mut cursor = s.cursor;
    let total = buffer.samples.len();

    for frame in out.chunks_mut(channels) {
        let value = if cursor < total {
            let v = buffer.samples[cursor];
            cursor += 1;
            T::from_sample(v)
        } else {
            silence
        };
        for sample in frame.iter_mut() {
            *sample = value;
        }
    }

    s.cursor = cursor;
    if cursor >= total {
        // Terminó el buffer: liberamos la referencia para que el caller
        // pueda recibir back-pressure ("is_playing → false") y, si tiene
        // el Arc, eventualmente liberar la memoria del audio.
        s.buffer = None;
        s.cursor = 0;
    }
}
