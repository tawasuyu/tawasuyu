//! Núcleo agnóstico de **captura de audio en vivo** — el análogo de
//! [`crate::LiveSource`] para el lado del sonido.
//!
//! La diferencia con el video es la disciplina: un frame viejo se
//! descarta (queremos el ahora), pero el audio **no se descarta** —
//! necesita continuidad muestra a muestra, así que el slot es un **ring
//! buffer** que se drena en orden. El productor (callback del
//! micrófono, captura de sistema, red) empuja muestras `f32`
//! intercaladas; el consumidor las saca por [`AudioSource::fill`],
//! rellenando con silencio si el productor se atrasó (underrun).
//!
//! El ring está acotado (~4 s): si el consumidor se cuelga y el
//! productor sigue, se descarta lo más viejo y se cuenta el overrun, en
//! vez de crecer sin límite. Para una grabación al ritmo del reloj el
//! cap nunca se toca.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use media_core::AudioSource;

/// Cap del ring: ~4 s de stereo a 48 kHz. Suficiente para absorber
/// jitter del scheduler sin permitir crecimiento ilimitado.
const MAX_RING_SAMPLES: usize = 48_000 * 2 * 4;

/// Estado compartido entre [`AudioLiveSink`] (empuja) y
/// [`AudioLiveSource`] (drena). El formato (`sample_rate`/`channels`)
/// lo fija el dispositivo al crear el canal y no cambia.
struct AudioShared {
    ring: Mutex<VecDeque<f32>>,
    sample_rate: u32,
    channels: u16,
    /// Muestras descartadas por overrun (consumidor demasiado lento).
    overruns: AtomicU64,
}

/// Crea un par sink↔source de audio con el formato del dispositivo.
/// El productor se queda con el [`AudioLiveSink`] (clonable, `Send`) y
/// empuja desde su callback; el consumidor enchufa el
/// [`AudioLiveSource`] como cualquier [`AudioSource`].
pub fn audio_channel(sample_rate: u32, channels: u16) -> (AudioLiveSink, AudioLiveSource) {
    let shared = Arc::new(AudioShared {
        ring: Mutex::new(VecDeque::new()),
        sample_rate: sample_rate.max(1),
        channels: channels.max(1),
        overruns: AtomicU64::new(0),
    });
    (
        AudioLiveSink {
            shared: shared.clone(),
        },
        AudioLiveSource { shared },
    )
}

/// Extremo productor: empuja muestras `f32` intercaladas al ring.
/// Clonable y `Send` — vive en el callback realtime del dispositivo.
#[derive(Clone)]
pub struct AudioLiveSink {
    shared: Arc<AudioShared>,
}

impl AudioLiveSink {
    /// Encola muestras (intercaladas, en el formato del canal). Si el
    /// ring se pasa del cap, descarta las más viejas y las cuenta como
    /// overrun — el flujo sigue, no se bloquea el callback.
    pub fn push(&self, samples: &[f32]) {
        let mut ring = self.shared.ring.lock().unwrap();
        ring.extend(samples.iter().copied());
        if ring.len() > MAX_RING_SAMPLES {
            let exceso = ring.len() - MAX_RING_SAMPLES;
            ring.drain(..exceso);
            self.shared
                .overruns
                .fetch_add(exceso as u64, Ordering::Relaxed);
        }
    }

    /// `true` si ya no queda consumidor — el productor puede pararse.
    pub fn is_orphan(&self) -> bool {
        Arc::strong_count(&self.shared) <= 1
    }
}

/// Extremo consumidor: un [`AudioSource`] que drena el ring en orden,
/// rellenando con silencio en underrun.
pub struct AudioLiveSource {
    shared: Arc<AudioShared>,
}

impl AudioLiveSource {
    /// Sample rate negociado con el dispositivo. El consumidor debe
    /// pedir `fill` con este rate: el ring guarda muestras a este
    /// formato y `fill` no resamplea.
    pub fn sample_rate(&self) -> u32 {
        self.shared.sample_rate
    }
    /// Canales negociados.
    pub fn channels(&self) -> u16 {
        self.shared.channels
    }
    /// Muestras disponibles en el ring ahora mismo.
    pub fn available(&self) -> usize {
        self.shared.ring.lock().unwrap().len()
    }
    /// Total de muestras descartadas por overrun.
    pub fn overruns(&self) -> u64 {
        self.shared.overruns.load(Ordering::Relaxed)
    }
}

impl AudioSource for AudioLiveSource {
    fn fill(&mut self, buf: &mut [f32], _sample_rate: u32, _channels: u16) {
        // El rate/canales pedidos se ignoran a propósito: el contrato es
        // que el caller pide con el formato del dispositivo (ver
        // `sample_rate`/`channels`). El ring ya está en ese formato.
        let mut ring = self.shared.ring.lock().unwrap();
        let n = buf.len().min(ring.len());
        for slot in buf.iter_mut().take(n) {
            *slot = ring.pop_front().unwrap();
        }
        // Underrun: el resto a silencio.
        for slot in buf.iter_mut().skip(n) {
            *slot = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drena_en_orden() {
        let (sink, mut src) = audio_channel(48_000, 2);
        assert_eq!(src.sample_rate(), 48_000);
        assert_eq!(src.channels(), 2);
        sink.push(&[0.1, 0.2, 0.3, 0.4]);
        let mut buf = [0.0f32; 4];
        src.fill(&mut buf, 48_000, 2);
        assert_eq!(buf, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn underrun_rellena_silencio() {
        let (sink, mut src) = audio_channel(48_000, 1);
        sink.push(&[0.5, 0.6]);
        let mut buf = [9.0f32; 5]; // pedimos más de lo que hay
        src.fill(&mut buf, 48_000, 1);
        assert_eq!(buf, [0.5, 0.6, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn fill_parcial_consume_lo_pedido() {
        // Más en el ring que lo pedido: se drena sólo lo pedido, el
        // resto queda para el próximo fill (continuidad).
        let (sink, mut src) = audio_channel(48_000, 1);
        sink.push(&[1.0, 2.0, 3.0, 4.0]);
        let mut buf = [0.0f32; 2];
        src.fill(&mut buf, 48_000, 1);
        assert_eq!(buf, [1.0, 2.0]);
        assert_eq!(src.available(), 2);
        src.fill(&mut buf, 48_000, 1);
        assert_eq!(buf, [3.0, 4.0]);
        assert_eq!(src.available(), 0);
    }

    #[test]
    fn overrun_descarta_lo_viejo_y_cuenta() {
        let (sink, src) = audio_channel(48_000, 1);
        // Empujar más que el cap de un golpe.
        let exceso = 100;
        let total = MAX_RING_SAMPLES + exceso;
        let big = vec![0.0f32; total];
        sink.push(&big);
        assert_eq!(src.available(), MAX_RING_SAMPLES);
        assert_eq!(src.overruns(), exceso as u64);
    }

    #[test]
    fn orphan_cuando_no_hay_source() {
        let (sink, src) = audio_channel(48_000, 2);
        assert!(!sink.is_orphan());
        drop(src);
        assert!(sink.is_orphan());
    }
}
