//! `takiy-playback` — reproducción en vivo de un `AudioBuffer`.
//!
//! Abre el dispositivo de salida default del SO con [`cpal`] y va
//! consumiendo un buffer mono `f32` que entrega [`takiy-synth`]. El
//! stream se queda corriendo siempre; cuando no hay nada que tocar
//! emite silencio, así no hay que parar/reabrir el device entre
//! reproducciones (eso causa clicks horribles en ALSA/PulseAudio).
//!
//! ## Hot path lock-free
//!
//! El callback de audio **nunca toma un mutex**: las órdenes (`Play`/
//! `Stop`) del thread de UI llegan por un `std::sync::mpsc::sync_channel`
//! que el callback drena con `try_recv` al inicio de cada frame. La
//! posición de reproducción y el flag "playing" son `AtomicU64` /
//! `AtomicBool` que el callback escribe y la UI lee sin contención.
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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, StreamConfig};
use takiy_synth::AudioBuffer;

/// Capacidad del canal UI → audio. Las órdenes son raras (un play/stop por
/// acción del usuario, no por frame) — 16 alcanza con margen y mantiene
/// la memoria acotada incluso si el audio thread se quedara colgado.
const COMMAND_CHANNEL_CAP: usize = 16;

/// Opciones de reproducción: posición inicial y región de loop opcional.
///
/// El uso típico es `PlayOpts::default()` (play desde el inicio, sin loop);
/// `Player::play_from(b, s)` y `Player::play_loop(b, lo, hi)` envuelven los
/// dos atajos comunes.
///
/// **Unidades**: tanto `start_sample` como los bounds de `loop_range`
/// están en **frames** (muestras por canal), no en samples interleaved.
/// Para mono coinciden con el índice de muestra; para estéreo, frame `n`
/// corresponde al par interleaved `(2n, 2n+1)`. Mantener nombre
/// `start_sample` por compatibilidad histórica.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayOpts {
    /// Frame desde el cual empezar a leer el buffer. `0` = inicio.
    pub start_sample: u64,
    /// Región de loop `[start, end)` en frames. Si está, el cursor
    /// vuelve a `start` cada vez que alcanza `end`. Si `start >= end`
    /// o `end > buffer.frames()`, el callback ignora el loop y
    /// reproduce linealmente.
    pub loop_range: Option<(u64, u64)>,
}

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

/// Órdenes que la UI envía al callback de audio. Se reciben por
/// `try_recv` al inicio de cada frame; nunca bloquean.
enum Command {
    Play { buffer: Arc<AudioBuffer>, opts: PlayOpts },
    Stop,
}

/// Estado mutable que vive **dentro** del callback de audio. No se
/// comparte: el callback es el único dueño. Por eso no necesita Mutex.
struct AudioState {
    buffer: Option<Arc<AudioBuffer>>,
    cursor: u64,
    loop_range: Option<(u64, u64)>,
}

/// Reproductor de audio. Mientras el `Player` viva, el stream del
/// device queda abierto. Drop lo cierra.
pub struct Player {
    tx: SyncSender<Command>,
    /// Posición de reproducción en samples (el callback la actualiza
    /// cada frame). El thread de UI la consulta para pintar el cursor
    /// del piano roll con precisión de muestra.
    position: Arc<AtomicU64>,
    /// `true` mientras haya buffer sonando. El callback lo limpia al
    /// terminar el buffer (back-pressure pasiva: la UI puede notar que
    /// el playback acabó sin hablar con el audio thread).
    playing: Arc<AtomicBool>,
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

        let (tx, rx) = sync_channel::<Command>(COMMAND_CHANNEL_CAP);
        let position = Arc::new(AtomicU64::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let err_fn = |e| eprintln!("takiy-playback · stream error: {e}");

        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(
                &device, &config, rx, position.clone(), playing.clone(), err_fn,
            ),
            SampleFormat::I16 => build_stream::<i16>(
                &device, &config, rx, position.clone(), playing.clone(), err_fn,
            ),
            SampleFormat::U16 => build_stream::<u16>(
                &device, &config, rx, position.clone(), playing.clone(), err_fn,
            ),
            other => Err(OpenError::Backend(format!("formato {other:?} no soportado"))),
        }?;
        stream.play().map_err(|e| OpenError::Backend(e.to_string()))?;

        Ok(Self { tx, position, playing, sample_rate, channels, _stream: stream })
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

    /// Encola un buffer para reproducción desde el inicio, sin loop. Es
    /// el atajo más común; equivale a `play_with(buffer, PlayOpts::default())`.
    pub fn play(&self, buffer: AudioBuffer) {
        self.play_with(buffer, PlayOpts::default());
    }

    /// Encola un buffer para reproducción a partir de `start_sample`. Si
    /// ya hay un buffer sonando, lo reemplaza sin reabrir el stream — el
    /// callback recibe la orden en el próximo período (~20ms) y arranca
    /// limpio desde el sample pedido.
    pub fn play_from(&self, buffer: AudioBuffer, start_sample: u64) {
        self.play_with(buffer, PlayOpts { start_sample, loop_range: None });
    }

    /// Encola un buffer para reproducción con loop entre `[loop_start, loop_end)`.
    /// El cursor arranca en `loop_start`; al alcanzar `loop_end` vuelve a
    /// `loop_start`. Útil para previsualizar un compás en bucle.
    pub fn play_loop(&self, buffer: AudioBuffer, loop_start: u64, loop_end: u64) {
        self.play_with(buffer, PlayOpts {
            start_sample: loop_start,
            loop_range: Some((loop_start, loop_end)),
        });
    }

    /// Encola un buffer con opciones completas. Reemplaza cualquier
    /// buffer en curso (no se mezclan — "tocar esto ahora"). El buffer
    /// se envuelve en `Arc` para que el callback lo pueda compartir.
    ///
    /// La orden se envía por canal SPSC; si el canal está saturado (no
    /// debería pasar nunca: capacidad 16, las órdenes son raras), se
    /// descarta con un log. La UI siempre asume éxito.
    pub fn play_with(&self, buffer: AudioBuffer, opts: PlayOpts) {
        // Marcamos playing inmediatamente para que la UI repinte sin
        // esperar al primer frame del callback (sería ~20ms de delay).
        self.playing.store(true, Ordering::Release);
        self.position.store(opts.start_sample, Ordering::Release);
        let cmd = Command::Play { buffer: Arc::new(buffer), opts };
        if let Err(e) = self.tx.try_send(cmd) {
            match e {
                TrySendError::Full(_) => {
                    eprintln!("takiy-playback · canal de órdenes saturado, play descartado");
                    self.playing.store(false, Ordering::Release);
                }
                TrySendError::Disconnected(_) => {
                    eprintln!("takiy-playback · audio thread desconectado");
                    self.playing.store(false, Ordering::Release);
                }
            }
        }
    }

    /// Detiene la reproducción y deja sonando silencio.
    pub fn stop(&self) {
        self.playing.store(false, Ordering::Release);
        let _ = self.tx.try_send(Command::Stop);
    }

    /// `true` si todavía queda buffer pendiente por reproducir. Lock-free.
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    /// Posición de reproducción en **frames** (muestras por canal) desde
    /// el inicio del buffer actual. Lock-free. Devuelve `0` cuando no hay
    /// buffer sonando. El nombre se mantiene por compatibilidad histórica
    /// (en mono frames == samples interleaved).
    pub fn position_samples(&self) -> u64 {
        self.position.load(Ordering::Acquire)
    }

    /// Alias semánticamente claro: posición en cuadros (frames). Equivale
    /// a [`position_samples`].
    pub fn position_frames(&self) -> u64 {
        self.position_samples()
    }

    /// Posición de reproducción en segundos. Cero si no está tocando.
    pub fn position_seconds(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.position_samples() as f32 / self.sample_rate as f32
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    rx: Receiver<Command>,
    position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, OpenError>
where
    T: Sample + cpal::SizedSample + cpal::FromSample<f32> + Send + 'static,
{
    let channels = config.channels as usize;
    let mut state = AudioState { buffer: None, cursor: 0, loop_range: None };
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _info| {
                fill::<T>(out, channels, &mut state, &rx, &position, &playing);
            },
            err_fn,
            None,
        )
        .map_err(|e| OpenError::Backend(e.to_string()))
}

fn fill<T>(
    out: &mut [T],
    channels: usize,
    state: &mut AudioState,
    rx: &Receiver<Command>,
    position: &AtomicU64,
    playing: &AtomicBool,
) where
    T: Sample + cpal::FromSample<f32>,
{
    // Drenar órdenes pendientes — no bloquea.
    while let Ok(cmd) = rx.try_recv() {
        match cmd {
            Command::Play { buffer: b, opts } => {
                let total = b.samples.len() as u64;
                let valid_loop = opts.loop_range.and_then(|(lo, hi)| {
                    if lo < hi && hi <= total { Some((lo, hi)) } else { None }
                });
                let start = opts.start_sample.min(total);
                state.buffer = Some(b);
                state.cursor = start;
                state.loop_range = valid_loop;
                position.store(start, Ordering::Release);
                playing.store(true, Ordering::Release);
            }
            Command::Stop => {
                state.buffer = None;
                state.cursor = 0;
                state.loop_range = None;
                position.store(0, Ordering::Release);
                playing.store(false, Ordering::Release);
            }
        }
    }

    let silence = T::from_sample(0.0f32);
    let Some(buffer) = state.buffer.clone() else {
        for sample in out.iter_mut() {
            *sample = silence;
        }
        return;
    };

    let in_channels = buffer.channels.max(1) as usize;
    let frames_total = (buffer.samples.len() / in_channels) as u64;
    let loop_range = state.loop_range;
    // `cursor` cuenta cuadros (frames), no samples. Mantenemos el
    // contador en frames para que loop_range y position_samples()
    // sigan teniendo el mismo significado independientemente de
    // los canales del buffer fuente.
    let mut cursor = state.cursor;

    for frame in out.chunks_mut(channels) {
        if let Some((lo, hi)) = loop_range {
            if cursor >= hi {
                cursor = lo;
            }
        }
        if cursor < frames_total {
            let base = cursor as usize * in_channels;
            // Estéreo→estéreo directo. Si input es mono y output stereo,
            // replicamos. Si input es stereo y output mono, promediamos.
            // Cualquier otra combinación: tomamos el primer canal del
            // input y replicamos a todos los del output.
            match (in_channels, channels) {
                (1, _) => {
                    let v = T::from_sample(buffer.samples[base]);
                    for sample in frame.iter_mut() {
                        *sample = v;
                    }
                }
                (2, 1) => {
                    let mix = (buffer.samples[base] + buffer.samples[base + 1]) * 0.5;
                    frame[0] = T::from_sample(mix);
                }
                (2, 2) => {
                    frame[0] = T::from_sample(buffer.samples[base]);
                    frame[1] = T::from_sample(buffer.samples[base + 1]);
                }
                (2, _) => {
                    // Output con > 2 canales (poco común): L→canales pares,
                    // R→impares; los extras quedan con copia del último.
                    let l = T::from_sample(buffer.samples[base]);
                    let r = T::from_sample(buffer.samples[base + 1]);
                    for (i, sample) in frame.iter_mut().enumerate() {
                        *sample = if i % 2 == 0 { l } else { r };
                    }
                }
                _ => {
                    let v = T::from_sample(buffer.samples[base]);
                    for sample in frame.iter_mut() {
                        *sample = v;
                    }
                }
            }
            cursor += 1;
        } else {
            for sample in frame.iter_mut() {
                *sample = silence;
            }
        }
    }

    state.cursor = cursor;
    position.store(cursor, Ordering::Release);
    if loop_range.is_none() && cursor >= frames_total {
        // Terminó el buffer y no hay loop: liberamos la referencia para
        // back-pressure (`is_playing → false`) y eventual liberación de
        // memoria del Arc en el lado de UI.
        state.buffer = None;
        state.cursor = 0;
        position.store(0, Ordering::Release);
        playing.store(false, Ordering::Release);
    }
}
