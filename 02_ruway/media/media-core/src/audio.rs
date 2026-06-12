//! Producción y captura de audio.
//!
//! - [`AudioSource`]: trait de producción de samples PCM f32.
//! - [`ToneSource`]: generador senoidal procedural.
//! - [`AudioProbe`]: ring buffer compartido para visualización.
//! - [`ProbedAudioSource`]: wrapper que duplica al probe sin overhead.

use std::sync::{Arc, Mutex};

// ============================================================
// AudioSource — productor de samples PCM
// ============================================================

/// Productor de samples de audio. El sink (cpal/JACK/wawa) llama
/// `fill` con un buffer ya dimensionado al frame requerido por el
/// driver, especificando `sample_rate` y `channels`. La fuente debe
/// llenar el buffer entero (no se permite "no hay nada" — para eso
/// rellenar con silencio) en formato intercalado por canal:
/// `[L0, R0, L1, R1, ...]` para stereo, `[M0, M1, ...]` para mono.
///
/// Implementadores deben ser baratos: la llamada típica ocurre en el
/// callback de audio realtime y no debe alocar ni bloquear.
pub trait AudioSource {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16);
}

// Reenvío para fuentes en `Box<dyn AudioSource ...>`. Permite componer
// (`ProbedAudioSource<Box<dyn AudioSource + Send>>`) sin volver a
// implementar el trait manualmente en cada wrapper.
impl<T: AudioSource + ?Sized> AudioSource for Box<T> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        (**self).fill(buf, sample_rate, channels);
    }
}

// ============================================================
// ToneSource — generador senoidal de referencia
// ============================================================

/// Generador de tono senoidal. Útil como `TestCard` del audio: valida
/// que el pipeline `AudioSource → sink → driver → speakers` ande sin
/// depender de un decoder o un archivo. Default: A4 (440 Hz) con
/// amplitud baja (-12 dB ~ 0.25) para no reventar oídos.
pub struct ToneSource {
    freq_hz: f32,
    amplitude: f32,
    phase: f32,
}

impl ToneSource {
    pub fn new(freq_hz: f32, amplitude: f32) -> Self {
        Self {
            freq_hz: freq_hz.max(1.0),
            amplitude: amplitude.clamp(0.0, 1.0),
            phase: 0.0,
        }
    }

    /// A4 a -12 dB. Lo suficientemente audible sin asustar.
    pub fn a4() -> Self {
        Self::new(440.0, 0.25)
    }

    pub fn set_frequency(&mut self, freq_hz: f32) {
        self.freq_hz = freq_hz.max(1.0);
    }
}

impl AudioSource for ToneSource {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        let channels = channels.max(1) as usize;
        let sr = sample_rate.max(1) as f32;
        let dphase = std::f32::consts::TAU * self.freq_hz / sr;
        let frames = buf.len() / channels;
        for frame in 0..frames {
            let v = self.phase.sin() * self.amplitude;
            for ch in 0..channels {
                buf[frame * channels + ch] = v;
            }
            self.phase += dphase;
            if self.phase >= std::f32::consts::TAU {
                self.phase -= std::f32::consts::TAU;
            }
        }
        // Si quedó cola por desalineación (channels > 1 y len no
        // múltiplo), rellenar con silencio.
        let tail = frames * channels;
        for s in &mut buf[tail..] {
            *s = 0.0;
        }
    }
}

// ============================================================
// AudioProbe — ring buffer compartido para visualización
// ============================================================

/// Ring buffer compartido con los últimos N samples (intercalados por
/// canal) que pasaron por una [`ProbedAudioSource`]. Diseñado para
/// que el callback de audio realtime escriba y un consumidor (la UI,
/// un visor, un grabador) lea snapshots ocasionales sin bloqueo
/// notable.
///
/// El probe es `Clone` barato (sólo clona el `Arc`); todas las copias
/// comparten el mismo buffer. La capacidad se fija al construir;
/// pensar "≈ X segundos · sample_rate · channels" al elegirla.
#[derive(Clone)]
pub struct AudioProbe {
    inner: Arc<Mutex<ProbeInner>>,
}

struct ProbeInner {
    ring: Vec<f32>,
    /// Próximo índice a escribir (módulo `ring.len()`).
    head: usize,
    /// Total de samples escritos (sin módulo) — sirve para saber si
    /// el ring ya dio una vuelta completa.
    written: u64,
    sample_rate: u32,
    channels: u16,
}

impl AudioProbe {
    /// Crea un probe con capacidad `capacity_samples` (cuenta samples
    /// intercalados, no frames). Valores típicos: ≈ 4096..16384 para
    /// visores responsivos a 44.1/48 kHz.
    pub fn new(capacity_samples: usize) -> Self {
        let cap = capacity_samples.max(64);
        Self {
            inner: Arc::new(Mutex::new(ProbeInner {
                ring: vec![0.0; cap],
                head: 0,
                written: 0,
                sample_rate: 0,
                channels: 0,
            })),
        }
    }

    /// Empuja un bloque de samples al ring. Sobrescribe lo más viejo.
    /// `sample_rate` y `channels` se cachean para que el consumidor
    /// pueda interpretarlos sin acoplar al sink.
    pub fn push(&self, samples: &[f32], sample_rate: u32, channels: u16) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.sample_rate = sample_rate;
        g.channels = channels;
        let cap = g.ring.len();
        for &s in samples {
            let head = g.head;
            g.ring[head] = s;
            g.head = (head + 1) % cap;
            g.written = g.written.saturating_add(1);
        }
    }

    /// Copia los samples del ring en orden cronológico (más viejo →
    /// más nuevo) en `out`. `out` se redimensiona a la capacidad del
    /// ring. Devuelve `(sample_rate, channels)` registrados por la
    /// última escritura — `(0, 0)` si todavía no hubo ninguna.
    pub fn snapshot(&self, out: &mut Vec<f32>) -> (u32, u16) {
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let cap = g.ring.len();
        if out.len() != cap {
            out.resize(cap, 0.0);
        }
        // Si todavía no se llenó una vuelta, los slots no escritos
        // siguen en 0.0 — eso es correcto (silencio inicial).
        let (head_to_end, start_to_head) = g.ring.split_at(g.head);
        // El bloque más viejo es desde `head` hasta el final (lo que
        // estaba por sobrescribirse), seguido del bloque desde 0
        // hasta `head` (lo recién escrito).
        let (out_a, out_b) = out.split_at_mut(start_to_head.len());
        out_a.copy_from_slice(start_to_head);
        out_b.copy_from_slice(head_to_end);
        (g.sample_rate, g.channels)
    }

    /// Cantidad de samples nunca escritos al ring desde la creación
    /// (clampada a la capacidad). 0 significa "ring lleno y dando
    /// vueltas". Útil para decidir si el snapshot vale la pena.
    pub fn unfilled(&self) -> usize {
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let cap = g.ring.len() as u64;
        cap.saturating_sub(g.written).min(cap) as usize
    }
}

// ============================================================
// ProbedAudioSource — wrapper de probe transparente
// ============================================================

/// Wrapper de [`AudioSource`] que duplica al [`AudioProbe`] cada
/// bloque que pasa por `fill`. Compone transparentemente: el sink no
/// ve diferencia con la fuente original.
pub struct ProbedAudioSource<S> {
    inner: S,
    probe: AudioProbe,
}

impl<S> ProbedAudioSource<S> {
    pub fn new(inner: S, probe: AudioProbe) -> Self {
        Self { inner, probe }
    }

    pub fn probe(&self) -> AudioProbe {
        self.probe.clone()
    }
}

impl<S: AudioSource> AudioSource for ProbedAudioSource<S> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.inner.fill(buf, sample_rate, channels);
        self.probe.push(buf, sample_rate, channels);
    }
}

