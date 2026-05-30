//! Backend de **micrófono** vía cpal — la captura de audio análoga a la
//! cámara/pantalla del lado del video. Abre el input device default,
//! arranca un stream cuyo callback empuja muestras `f32` al
//! [`AudioLiveSink`]; el consumidor las saca por el [`AudioLiveSource`]
//! embebido (que es un [`AudioSource`]).
//!
//! Mismo molde que `media-audio-cpal` (el sink de salida) pero en el
//! sentido inverso: ahí el callback **tira** muestras de un
//! `AudioSource`, acá las **empuja** a un sink. Detrás de feature
//! opt-in `mic` (arrastra cpal → ALSA/CoreAudio); el núcleo `live_audio`
//! es puro y testeable sin dispositivo.
//!
//! Pide 48 kHz si el dispositivo lo ofrece — es el rate nativo de Opus,
//! así el `media-recorder-webm` encodea el audio sin degradar. Un
//! dispositivo que sólo da 44.1 kHz graba igual, pero el recorder lo
//! marca como no-Opus y la grabación queda video-solo.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, Stream, StreamConfig};

use media_core::AudioSource;

use crate::live_audio::{audio_channel, AudioLiveSink, AudioLiveSource};

/// Qué pedirle al input device. El driver puede negociar otra cosa —
/// la fuente reporta lo realmente abierto en [`MicSource::sample_rate`].
#[derive(Debug, Clone)]
pub struct MicOptions {
    /// Sample rate preferido. `None` → 48 kHz si está disponible, si no
    /// el default del dispositivo.
    pub sample_rate: Option<u32>,
}

impl Default for MicOptions {
    fn default() -> Self {
        Self { sample_rate: None }
    }
}

/// Lo que pudo salir mal al abrir el micrófono.
#[derive(Debug)]
pub enum MicError {
    /// No hay input device default (sin micrófono / sin permisos).
    NoInputDevice,
    /// El host no devolvió una config de entrada usable.
    DefaultConfig(String),
    /// El sample-format del dispositivo no lo sabemos convertir a f32.
    UnsupportedFormat(SampleFormat),
    /// `build_input_stream` falló.
    Build(String),
    /// `play()` falló.
    Play(String),
}

impl std::fmt::Display for MicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInputDevice => write!(f, "no hay input device default (¿micrófono?)"),
            Self::DefaultConfig(e) => write!(f, "default_input_config: {e}"),
            Self::UnsupportedFormat(fmt) => write!(f, "sample-format no soportado: {fmt:?}"),
            Self::Build(e) => write!(f, "build_input_stream: {e}"),
            Self::Play(e) => write!(f, "play: {e}"),
        }
    }
}

impl std::error::Error for MicError {}

/// Micrófono en vivo como [`AudioSource`]. Mantiene vivo el
/// `cpal::Stream` mientras esté en alcance; al dropearse se cierra el
/// stream y deja de capturar.
pub struct MicSource {
    source: AudioLiveSource,
    _stream: Stream,
    sample_rate: u32,
    channels: u16,
}

impl MicSource {
    /// Abre el input device default y arranca la captura. Bloquea hasta
    /// negociar el formato (o fallar) — el error de "no hay micrófono"
    /// llega sincrónico.
    pub fn open(opts: MicOptions) -> Result<Self, MicError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(MicError::NoInputDevice)?;

        // Elegir config: si pedimos (o por default queremos) 48 kHz y el
        // dispositivo lo soporta, usarlo; si no, el default del device.
        let want_sr = opts.sample_rate.unwrap_or(48_000);
        let supported = pick_config(&device, want_sr)?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let sample_rate = config.sample_rate.0;
        let channels = config.channels;

        let (sink, source) = audio_channel(sample_rate, channels);

        let err_fn = |e| eprintln!("media-capture-mic: error de stream: {e}");
        let stream = match sample_format {
            SampleFormat::F32 => build_input::<f32>(&device, &config, sink, err_fn),
            SampleFormat::I16 => build_input::<i16>(&device, &config, sink, err_fn),
            SampleFormat::U16 => build_input::<u16>(&device, &config, sink, err_fn),
            other => return Err(MicError::UnsupportedFormat(other)),
        }?;
        stream.play().map_err(|e| MicError::Play(e.to_string()))?;

        Ok(Self {
            source,
            _stream: stream,
            sample_rate,
            channels,
        })
    }

    /// Atajo: input device default, preferiendo 48 kHz.
    pub fn open_default() -> Result<Self, MicError> {
        Self::open(MicOptions::default())
    }

    /// Sample rate realmente negociado.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    /// Canales realmente negociados.
    pub fn channels(&self) -> u16 {
        self.channels
    }
    /// Muestras descartadas por overrun (consumidor lento).
    pub fn overruns(&self) -> u64 {
        self.source.overruns()
    }
}

impl AudioSource for MicSource {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.source.fill(buf, sample_rate, channels);
    }
}

/// Busca una config de entrada al `want_sr` pedido; si ninguna lo
/// cubre, cae al `default_input_config`.
fn pick_config(
    device: &cpal::Device,
    want_sr: u32,
) -> Result<cpal::SupportedStreamConfig, MicError> {
    if let Ok(ranges) = device.supported_input_configs() {
        for range in ranges {
            let sr = cpal::SampleRate(want_sr);
            if range.min_sample_rate() <= sr && sr <= range.max_sample_rate() {
                return Ok(range.with_sample_rate(sr));
            }
        }
    }
    device
        .default_input_config()
        .map_err(|e| MicError::DefaultConfig(e.to_string()))
}

/// Construye el input stream para el sample-format `T`, convirtiendo
/// cada muestra a `f32` y empujándola al sink. Buffer scratch reusado
/// entre callbacks (exclusivo del callback realtime).
fn build_input<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    sink: AudioLiveSink,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream, MicError>
where
    T: Sample + cpal::SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_input_stream(
            config,
            move |data: &[T], _info| {
                scratch.clear();
                scratch.extend(data.iter().map(|&s| f32::from_sample(s)));
                if !sink.is_orphan() {
                    sink.push(&scratch);
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| MicError::Build(e.to_string()))
}
