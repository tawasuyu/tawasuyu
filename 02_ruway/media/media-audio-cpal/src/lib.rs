//! media-audio-cpal — sink de audio realtime sobre cpal.
//!
//! Abre el output device default del host (ALSA en Linux, CoreAudio en
//! macOS, WASAPI en Windows) y arranca un stream que bombea de un
//! [`AudioSource`] compartido. El stream queda corriendo mientras el
//! [`AudioSink`] viva; al dropearlo, cpal cierra el stream y libera el
//! device.
//!
//! Soporta formatos f32, i16 y u16 — los tres comunes que devuelve
//! `default_output_config`. La fuente entrega siempre `f32` (el
//! formato del trait `AudioSource`); el sink convierte por sample con
//! `cpal::Sample::from_sample`.

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, Stream, StreamConfig};
use media_core::AudioSource;
use parking_lot::Mutex;

#[derive(Debug)]
pub enum OpenError {
    NoOutputDevice,
    DefaultConfig(String),
    Build(String),
    Play(String),
    UnsupportedFormat(SampleFormat),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOutputDevice => write!(f, "no hay default output device"),
            Self::DefaultConfig(s) => write!(f, "default_output_config: {s}"),
            Self::Build(s) => write!(f, "build_output_stream: {s}"),
            Self::Play(s) => write!(f, "stream.play: {s}"),
            Self::UnsupportedFormat(fmt) => write!(f, "formato no soportado: {fmt:?}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// Sink que mantiene vivo un `cpal::Stream` mientras esté en alcance.
/// Drop = stream cerrado.
pub struct AudioSink {
    _stream: Stream,
    sample_rate: u32,
    channels: u16,
}

impl AudioSink {
    /// Abre el output device default y arranca el stream tirando de
    /// `source`. La fuente puede compartirse con otros consumidores
    /// vía el `Arc<Mutex>`; el sink la lockea en cada callback.
    pub fn open(source: Arc<Mutex<dyn AudioSource + Send>>) -> Result<Self, OpenError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(OpenError::NoOutputDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| OpenError::DefaultConfig(e.to_string()))?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let sample_rate = config.sample_rate.0;
        let channels = config.channels;

        let err_fn = |e| eprintln!("media-audio-cpal: stream error: {e}");

        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&device, &config, source, err_fn)?,
            SampleFormat::I16 => build_stream::<i16>(&device, &config, source, err_fn)?,
            SampleFormat::U16 => build_stream::<u16>(&device, &config, source, err_fn)?,
            other => return Err(OpenError::UnsupportedFormat(other)),
        };
        stream.play().map_err(|e| OpenError::Play(e.to_string()))?;
        Ok(Self {
            _stream: stream,
            sample_rate,
            channels,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    source: Arc<Mutex<dyn AudioSource + Send>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream, OpenError>
where
    T: Sample + cpal::SizedSample + cpal::FromSample<f32> + Send + 'static,
{
    let sample_rate = config.sample_rate.0;
    let channels = config.channels;
    // Buffer scratch de f32 reutilizado entre callbacks. Vive en el
    // closure → exclusivo del callback realtime → no necesita lock.
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _info| {
                if scratch.len() != out.len() {
                    scratch.resize(out.len(), 0.0);
                }
                {
                    let mut src = source.lock();
                    src.fill(&mut scratch, sample_rate, channels);
                }
                for (dst, &s) in out.iter_mut().zip(scratch.iter()) {
                    *dst = T::from_sample(s);
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| OpenError::Build(e.to_string()))
}
