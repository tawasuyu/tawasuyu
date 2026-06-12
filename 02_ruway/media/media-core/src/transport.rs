//! Controles de transporte compartidos: pausa, volumen y mezcla.
//!
//! - [`Pause`]: handle atómico de pausa/reanuda.
//! - [`PausableAudio`] / [`PausableVideo`]: wrappers de fuentes pausables.
//! - [`Volume`]: ganancia lineal atómica.
//! - [`VolumeAudio`]: wrapper de ganancia sobre un [`AudioSource`].
//! - [`MixerAudio`]: suma de N fuentes de audio.
//! - [`VideoSwitch`] / [`VideoSwitcher`]: selección de 1 de N FrameSources.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::audio::AudioSource;
use crate::frame::FrameSource;

// ============================================================
// Pause — transport mínimo compartido entre fuentes
// ============================================================

/// Handle compartido de pausa. Es `Clone` barato (sólo un `Arc`); una
/// instancia puede manejar simultáneamente la pausa de un
/// [`PausableAudio`] y un [`PausableVideo`] (o varios) para que el
/// usuario congele todo con un toggle. La UI sólo necesita conservar
/// una copia para mostrar el estado y emitir `toggle()`.
#[derive(Clone, Default)]
pub struct Pause {
    flag: Arc<AtomicBool>,
}

impl Pause {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_paused(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    pub fn pause(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }

    /// Invierte el estado y devuelve el nuevo valor (true = pausado).
    pub fn toggle(&self) -> bool {
        // fetch_xor con `true` invierte el bit y devuelve el ANTERIOR;
        // el nuevo es la negación.
        !self.flag.fetch_xor(true, Ordering::Relaxed)
    }
}

/// Wrapper de [`AudioSource`] que en pausa rellena con silencio (no
/// llama al inner). El cursor / fase del inner queda intacto — al
/// reanudar sigue donde estaba.
pub struct PausableAudio<S> {
    inner: S,
    pause: Pause,
}

impl<S> PausableAudio<S> {
    pub fn new(inner: S, pause: Pause) -> Self {
        Self { inner, pause }
    }
}

impl<S: AudioSource> AudioSource for PausableAudio<S> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        if self.pause.is_paused() {
            for s in buf.iter_mut() {
                *s = 0.0;
            }
            return;
        }
        self.inner.fill(buf, sample_rate, channels);
    }
}

/// Wrapper de [`FrameSource`] que en pausa devuelve `None` y no avanza
/// el tiempo interno del inner. El consumidor (que mantiene el último
/// frame en una textura) verá la imagen congelada.
pub struct PausableVideo<S> {
    inner: S,
    pause: Pause,
}

impl<S> PausableVideo<S> {
    pub fn new(inner: S, pause: Pause) -> Self {
        Self { inner, pause }
    }
}

impl<S: FrameSource> FrameSource for PausableVideo<S> {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        if self.pause.is_paused() {
            return None;
        }
        self.inner.tick(dt, buf)
    }
    fn pts(&self) -> Option<Duration> {
        self.inner.pts()
    }
}

// ============================================================
// Volume — ganancia lineal compartida
// ============================================================

/// Handle clonable de ganancia lineal aplicada a un [`VolumeAudio`].
/// Se almacena como `f32` bit-cast a `u32` para que el callback de
/// audio realtime no necesite tomar un lock. Rango efectivo
/// `[0.0, 4.0]` — sobre `1.0` amplifica (con riesgo de clipping).
#[derive(Clone)]
pub struct Volume {
    bits: Arc<AtomicU32>,
}

impl Default for Volume {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Volume {
    pub fn new(initial: f32) -> Self {
        let v = initial.clamp(0.0, 4.0).to_bits();
        Self {
            bits: Arc::new(AtomicU32::new(v)),
        }
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.bits.load(Ordering::Relaxed))
    }

    pub fn set(&self, v: f32) {
        let clamped = v.clamp(0.0, 4.0);
        self.bits.store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// Modifica el valor con una closure (read-modify-write con
    /// compare-exchange en loop — útil para "subí 5%").
    pub fn update(&self, f: impl Fn(f32) -> f32) {
        let mut cur = self.bits.load(Ordering::Relaxed);
        loop {
            let nv = f(f32::from_bits(cur)).clamp(0.0, 4.0).to_bits();
            match self.bits.compare_exchange(
                cur,
                nv,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => cur = actual,
            }
        }
    }
}

/// Wrapper de [`AudioSource`] que multiplica cada sample por el valor
/// actual de un [`Volume`] compartido. Sin estado interno: una
/// instancia del wrapper puede convivir con muchas copias del handle.
pub struct VolumeAudio<S> {
    inner: S,
    volume: Volume,
}

impl<S> VolumeAudio<S> {
    pub fn new(inner: S, volume: Volume) -> Self {
        Self { inner, volume }
    }
}

impl<S: AudioSource> AudioSource for VolumeAudio<S> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.inner.fill(buf, sample_rate, channels);
        let g = self.volume.get();
        if (g - 1.0).abs() < 1e-6 {
            return;
        }
        for s in buf.iter_mut() {
            *s *= g;
        }
    }
}

// ============================================================
// MixerAudio — suma de N AudioSources al mismo bus
// ============================================================

/// Mezclador puro: suma `n` fuentes en el mismo buffer de salida.
/// No aplica ganancia propia — para mezcla a niveles distintos
/// envolver cada fuente en un [`VolumeAudio`] antes de pasarla. Si
/// la suma satura, se clampea a [-1, 1] en el último paso (otherwise
/// el sink podría distorsionar feo).
///
/// Para no allocar en el callback realtime, mantiene un buffer
/// scratch interno del tamaño del último `fill` recibido.
pub struct MixerAudio {
    sources: Vec<Box<dyn AudioSource + Send>>,
    scratch: Vec<f32>,
}

impl MixerAudio {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            scratch: Vec::new(),
        }
    }

    pub fn with_sources(sources: Vec<Box<dyn AudioSource + Send>>) -> Self {
        Self {
            sources,
            scratch: Vec::new(),
        }
    }

    pub fn push(&mut self, source: Box<dyn AudioSource + Send>) {
        self.sources.push(source);
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

impl Default for MixerAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSource for MixerAudio {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        if self.sources.is_empty() {
            for s in buf.iter_mut() {
                *s = 0.0;
            }
            return;
        }
        if self.scratch.len() != buf.len() {
            self.scratch.resize(buf.len(), 0.0);
        }
        // Primera fuente: escribe directo al destino.
        self.sources[0].fill(buf, sample_rate, channels);
        for src in self.sources.iter_mut().skip(1) {
            src.fill(&mut self.scratch, sample_rate, channels);
            for (dst, &s) in buf.iter_mut().zip(self.scratch.iter()) {
                *dst += s;
            }
        }
        // Clamp para que el sink no reciba > 1.
        for s in buf.iter_mut() {
            if *s > 1.0 {
                *s = 1.0;
            } else if *s < -1.0 {
                *s = -1.0;
            }
        }
    }
}

// ============================================================
// VideoSwitcher — selección de 1 de N FrameSources
// ============================================================

/// Handle compartido del índice activo de un [`VideoSwitcher`]. Es
/// `Clone` barato; la UI lo guarda para flip rápido sin tocar el
/// switcher directo.
#[derive(Clone, Default)]
pub struct VideoSwitch {
    idx: Arc<AtomicUsize>,
}

impl VideoSwitch {
    pub fn new(initial: usize) -> Self {
        Self {
            idx: Arc::new(AtomicUsize::new(initial)),
        }
    }

    pub fn get(&self) -> usize {
        self.idx.load(Ordering::Relaxed)
    }

    pub fn set(&self, i: usize) {
        self.idx.store(i, Ordering::Relaxed);
    }

    /// Avanza 1 módulo `len` — para cyclar con un botón "next".
    pub fn next(&self, len: usize) {
        if len == 0 {
            return;
        }
        let cur = self.get();
        self.set((cur + 1) % len);
    }
}

/// Multiplexor de video: tiene `n` fuentes en memoria y `tick` la que
/// indica el [`VideoSwitch`] activo. Las fuentes no activas no
/// avanzan tiempo (no se llaman) — al volver a una el consumidor
/// recibe el primer frame "frío" del decoder.
pub struct VideoSwitcher {
    sources: Vec<Box<dyn FrameSource + Send>>,
    switch: VideoSwitch,
}

impl VideoSwitcher {
    pub fn new(sources: Vec<Box<dyn FrameSource + Send>>, switch: VideoSwitch) -> Self {
        Self { sources, switch }
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

impl FrameSource for VideoSwitcher {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        if self.sources.is_empty() {
            return None;
        }
        let n = self.sources.len();
        let i = self.switch.get() % n;
        self.sources[i].tick(dt, buf)
    }
    fn pts(&self) -> Option<Duration> {
        if self.sources.is_empty() {
            return None;
        }
        let n = self.sources.len();
        let i = self.switch.get() % n;
        self.sources[i].pts()
    }
}

#[cfg(test)]
mod tests_composition {
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
    fn mixer_sums_and_clamps() {
        let mut mix = MixerAudio::with_sources(vec![
            Box::new(Constant(0.4)),
            Box::new(Constant(0.4)),
        ]);
        let mut buf = vec![0.0_f32; 8];
        mix.fill(&mut buf, 48_000, 2);
        assert!(buf.iter().all(|&v| (v - 0.8).abs() < 1e-6));

        let mut mix = MixerAudio::with_sources(vec![
            Box::new(Constant(0.8)),
            Box::new(Constant(0.8)),
        ]);
        let mut buf = vec![0.0_f32; 8];
        mix.fill(&mut buf, 48_000, 2);
        // Suma 1.6 → clampado a 1.0.
        assert!(buf.iter().all(|&v| (v - 1.0).abs() < 1e-6));
    }

    #[test]
    fn mixer_empty_emits_silence() {
        let mut mix = MixerAudio::new();
        let mut buf = vec![1.0_f32; 4];
        mix.fill(&mut buf, 48_000, 1);
        assert!(buf.iter().all(|&v| v == 0.0));
    }
}
