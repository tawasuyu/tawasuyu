//! media-core — productores de video y audio del dominio.
//!
//! Dos traits gemelos:
//!
//! - [`FrameSource`]: entrega bytes RGBA con un tamaño. Lo consume
//!   `llimphi-surface` para subirlo a una textura GPU.
//! - [`AudioSource`]: rellena un buffer de samples `f32` intercalados
//!   por canal a una sample rate dada. Lo consume un sink (cpal, JACK,
//!   wawa) que se encarga del realtime.
//!
//! Ambos vienen con una implementación procedural de referencia:
//! [`TestCard`] (gradiente animado + círculo rebotando) para video y
//! [`ToneSource`] (senoide configurable, default A4) para audio. Son
//! los "test patterns" del dominio: validan los pipelines completos
//! sin meter decoders externos.
//!
//! El crate es `std` y no tiene dependencias — la idea es que el
//! núcleo del dominio sea liviano y los backends pesados (ffmpeg,
//! gstreamer, v4l2, cpal…) vivan en crates `media-source-*` o
//! `media-audio-*` que impl los traits.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub mod control;
pub mod eq;
pub mod layout;
pub mod sync;

/// Productor de frames RGBA. `tick` avanza el tiempo `dt` y, si hay
/// un nuevo frame disponible, lo deja escrito en `buf` y devuelve
/// `Some((width, height))`. Si todavía no hay frame nuevo (p.ej. el
/// emisor corre a 30 fps y `dt` no alcanzó al próximo), devuelve
/// `None` y no toca `buf`.
///
/// `buf` se redimensiona si es necesario; el caller puede reusarlo
/// entre llamadas para evitar realocs.
pub trait FrameSource {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)>;
}

// ============================================================
// Seekable — transporte para fuentes con timeline conocido
// ============================================================

/// Capacidad opcional de las fuentes que tienen una posición y/o
/// duración conocida (WAV, GIF, una pista de Opus). Las fuentes
/// infinitas o procedurales (TestCard, ToneSource) la pueden NO
/// implementar — el caller debe `dyn Seekable` por separado.
///
/// La implementación es responsable de que `seek_to` no rompa el
/// estado del decoder: clampea al rango válido y, si la fuente es
/// loopable, hace módulo de la duración. `position` y `duration`
/// usan [`Duration`] para ser portátiles entre fuentes con sample
/// rates distintos.
pub trait Seekable {
    /// Tiempo actual de reproducción desde el inicio (ignorando loops
    /// pasados — siempre módulo `duration` si la fuente loopea).
    fn position(&self) -> Duration;

    /// Duración total de un loop completo. `None` para fuentes
    /// infinitas (tono, testcard, stream en vivo).
    fn duration(&self) -> Option<Duration>;

    /// Mueve la posición. Las fuentes deben clampear/módulo a su
    /// rango válido — el caller puede pasar valores fuera y esperar
    /// que se normalicen, no que panickeen.
    fn seek_to(&mut self, pos: Duration);
}

// Reenvíos para Box<dyn Seekable + ...> — mismo motivo que los
// blanket impls de FrameSource/AudioSource.
impl<T: Seekable + ?Sized> Seekable for Box<T> {
    fn position(&self) -> Duration {
        (**self).position()
    }
    fn duration(&self) -> Option<Duration> {
        (**self).duration()
    }
    fn seek_to(&mut self, pos: Duration) {
        (**self).seek_to(pos)
    }
}

// Reenvío para `Box<dyn FrameSource ...>`. Igual que el de
// `AudioSource`: permite componer wrappers (`PausableVideo<Box<dyn
// FrameSource + Send>>`) sin re-implementar el trait.
impl<T: FrameSource + ?Sized> FrameSource for Box<T> {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        (**self).tick(dt, buf)
    }
}

/// Generador procedural: gradiente animado + círculo que rebota.
/// Útil como "primer reproductor" del dominio para validar el
/// pipeline `media-core → llimphi-surface → frame` sin meter
/// dependencias de decoding.
pub struct TestCard {
    width: u32,
    height: u32,
    fps: f32,
    elapsed: f32,
    accum_since_frame: f32,
}

impl TestCard {
    pub fn new(width: u32, height: u32, fps: f32) -> Self {
        Self {
            width,
            height,
            fps: fps.max(1.0),
            elapsed: 0.0,
            accum_since_frame: f32::INFINITY,
        }
    }

    /// Frame interval objetivo (1 / fps).
    pub fn frame_interval(&self) -> Duration {
        Duration::from_secs_f32(1.0 / self.fps)
    }
}

impl FrameSource for TestCard {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        let dt = dt.as_secs_f32();
        self.elapsed += dt;
        self.accum_since_frame += dt;
        let target = 1.0 / self.fps;
        if self.accum_since_frame < target {
            return None;
        }
        self.accum_since_frame = 0.0;

        let w = self.width as usize;
        let h = self.height as usize;
        let needed = w * h * 4;
        if buf.len() != needed {
            buf.resize(needed, 0);
        }

        let t = self.elapsed;
        // Centro del círculo en lissajous lento dentro del frame.
        let cx = (0.5 + 0.35 * (t * 0.9).cos()) * w as f32;
        let cy = (0.5 + 0.30 * (t * 0.7).sin()) * h as f32;
        let r = (w.min(h) as f32) * 0.12;
        let r2 = r * r;

        for y in 0..h {
            for x in 0..w {
                // Gradiente diagonal animado.
                let u = x as f32 / w as f32;
                let v = y as f32 / h as f32;
                let g = ((u + v) * 0.5 + t * 0.15).fract();
                let rch = (g * 255.0) as u8;
                let gch = ((1.0 - g) * 200.0) as u8;
                let bch = ((u * (1.0 - v)) * 255.0) as u8;

                // Círculo brillante encima.
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let d2 = dx * dx + dy * dy;
                let (r_out, g_out, b_out) = if d2 < r2 {
                    let k = 1.0 - (d2 / r2).sqrt();
                    let mix = (k * 255.0) as u8;
                    (mix.saturating_add(rch / 4), mix, mix.saturating_add(80))
                } else {
                    (rch, gch, bch)
                };

                let i = (y * w + x) * 4;
                buf[i] = r_out;
                buf[i + 1] = g_out;
                buf[i + 2] = b_out;
                buf[i + 3] = 255;
            }
        }
        Some((self.width, self.height))
    }
}

// ============================================================
// Audio
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
// Probe — tap del stream de audio para visualización
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

// ============================================================
// Spectrum — análisis por bandas log-spaced (Goertzel)
// ============================================================

/// Banco de filtros Goertzel sobre un conjunto fijo de frecuencias
/// centro log-espaciadas. Pensado como motor del visor "barras"
/// (spectrogram instantáneo) sin traer dep de FFT.
///
/// Goertzel cuesta `2·N + 4` adds/mults por banda y por snapshot.
/// Para 32 bandas × 4096 samples ≈ 131k mults — barato a 30 fps.
///
/// El uso típico:
///
/// ```ignore
/// let mut spec = Spectrum::log_bands(32, 40.0, 16_000.0);
/// // ... más tarde, por frame:
/// spec.analyze(&snapshot, channels, sample_rate);
/// for (f, a) in spec.bands().iter().zip(spec.magnitudes()) { ... }
/// ```
pub struct Spectrum {
    centers: Vec<f32>,
    /// Magnitudes con suavizado temporal (attack/release simple).
    mags: Vec<f32>,
    /// Factor de release (0..1). Más cerca de 1 = decae más lento.
    release: f32,
}

impl Spectrum {
    /// Construye `n` bandas log-espaciadas entre `fmin` y `fmax`.
    /// Falla silenciosamente con `n == 0` (mags queda vacío y
    /// `analyze` no hace nada).
    pub fn log_bands(n: usize, fmin: f32, fmax: f32) -> Self {
        let fmin = fmin.max(1.0);
        let fmax = fmax.max(fmin * 2.0);
        let lo = fmin.ln();
        let hi = fmax.ln();
        let denom = (n.saturating_sub(1)).max(1) as f32;
        let centers: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / denom;
                (lo + (hi - lo) * t).exp()
            })
            .collect();
        Self {
            mags: vec![0.0; centers.len()],
            centers,
            release: 0.78,
        }
    }

    /// Factor de "release" del suavizado temporal: cuán rápido baja
    /// una banda cuando ya no hay señal. 0 = sin memoria; 0.95 = muy
    /// suave. Default 0.78 (≈ medio segundo a 30 fps).
    pub fn set_release(&mut self, release: f32) {
        self.release = release.clamp(0.0, 0.99);
    }

    pub fn bands(&self) -> &[f32] {
        &self.centers
    }

    pub fn magnitudes(&self) -> &[f32] {
        &self.mags
    }

    /// Corre Goertzel sobre `samples` (intercalados) plegando a mono y
    /// actualiza las magnitudes con attack inmediato + release
    /// exponencial. `sample_rate` y `channels` provienen del snapshot
    /// del probe.
    pub fn analyze(&mut self, samples: &[f32], channels: u16, sample_rate: u32) {
        if self.mags.is_empty() || samples.is_empty() || sample_rate == 0 {
            return;
        }
        let ch = channels.max(1) as usize;
        let frames = samples.len() / ch;
        if frames < 4 {
            return;
        }
        // Mono fold reusable. Lo construimos una vez por análisis para
        // que Goertzel itere sobre f32 contiguos.
        let mut mono: Vec<f32> = Vec::with_capacity(frames);
        let inv_ch = 1.0 / ch as f32;
        for f in 0..frames {
            let mut s = 0.0_f32;
            for c in 0..ch {
                s += samples[f * ch + c];
            }
            mono.push(s * inv_ch);
        }

        let n = frames as f32;
        let sr = sample_rate as f32;
        let nyquist = sr * 0.5;
        for (i, &freq) in self.centers.iter().enumerate() {
            if freq >= nyquist {
                // Sobre Nyquist no hay nada que medir; sólo decae.
                self.mags[i] *= self.release;
                continue;
            }
            // k continuo (no entero) sigue siendo válido para
            // visualización — distorsión leve cerca de bordes.
            let k = n * freq / sr;
            let w = std::f32::consts::TAU * k / n;
            let coeff = 2.0 * w.cos();
            let mut q1 = 0.0_f32;
            let mut q2 = 0.0_f32;
            for &s in &mono {
                let q0 = coeff * q1 - q2 + s;
                q2 = q1;
                q1 = q0;
            }
            // |X(k)|² = q1² + q2² - q1·q2·coeff
            let mag2 = (q1 * q1 + q2 * q2 - q1 * q2 * coeff).max(0.0);
            let mag = (mag2.sqrt() * 2.0 / n).min(1.0);
            // Attack inmediato, release suave.
            let prev = self.mags[i] * self.release;
            self.mags[i] = if mag > prev { mag } else { prev };
        }
    }
}

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
}

// ============================================================
// Waterfall — historial 2D del espectro
// ============================================================

/// Historial rotativo de magnitudes del [`Spectrum`]. Cada `analyze`
/// corre el Goertzel sobre el snapshot recibido y guarda la fila
/// resultante en un ring buffer de `rows` filas × `bands` columnas
/// — el visor (spectrogram waterfall) pinta el ring en orden
/// newest-first para que la onda nueva entre por arriba y empuje a
/// la vieja hacia abajo.
///
/// Las filas anteriores a la primera escritura quedan en 0.0
/// (silencio). El consumidor puede leer con [`Waterfall::snapshot`]
/// en orden cronológico inverso (fila 0 = más nueva).
pub struct Waterfall {
    spectrum: Spectrum,
    /// Buffer plano `rows × bands` (fila i, banda j en `[i*bands + j]`).
    grid: Vec<f32>,
    bands: usize,
    rows: usize,
    /// Índice de la fila a sobrescribir en el próximo analyze.
    head: usize,
    /// Cuántas filas se escribieron históricamente (clampada a rows).
    written: usize,
}

impl Waterfall {
    /// Crea un waterfall sobre `bands` bandas log-espaciadas y `rows`
    /// filas de historial. `bands == 0` o `rows == 0` se clampean a 1.
    pub fn new(bands: usize, rows: usize, fmin: f32, fmax: f32) -> Self {
        let bands = bands.max(1);
        let rows = rows.max(1);
        Self {
            spectrum: Spectrum::log_bands(bands, fmin, fmax),
            grid: vec![0.0; bands * rows],
            bands,
            rows,
            head: 0,
            written: 0,
        }
    }

    pub fn bands(&self) -> usize {
        self.bands
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Frecuencias centro de cada banda — espejo de [`Spectrum::bands`].
    pub fn band_freqs(&self) -> &[f32] {
        self.spectrum.bands()
    }

    /// Corre el spectrum sobre `samples` y agrega la fila resultante
    /// al ring. La fila vieja en `head` queda sobrescrita.
    pub fn analyze(&mut self, samples: &[f32], channels: u16, sample_rate: u32) {
        self.spectrum.analyze(samples, channels, sample_rate);
        let mags = self.spectrum.magnitudes();
        let bands = self.bands;
        let start = self.head * bands;
        // Copia la fila — `mags.len()` ya es == bands por construcción.
        self.grid[start..start + bands].copy_from_slice(mags);
        self.head = (self.head + 1) % self.rows;
        self.written = (self.written + 1).min(self.rows);
    }

    /// Copia el grid a `out` en orden newest-first: la fila 0 de
    /// `out` es la última analizada, fila `rows-1` la más vieja.
    /// `out` se redimensiona a `rows * bands`. Devuelve `(rows, bands)`.
    pub fn snapshot(&self, out: &mut Vec<f32>) -> (usize, usize) {
        let total = self.rows * self.bands;
        if out.len() != total {
            out.resize(total, 0.0);
        }
        if self.written == 0 {
            for v in out.iter_mut() {
                *v = 0.0;
            }
            return (self.rows, self.bands);
        }
        // newest = (head + rows - 1) % rows.
        let newest = (self.head + self.rows - 1) % self.rows;
        for i in 0..self.rows {
            // out[i] = grid[(newest - i) mod rows]
            let src_row = (newest + self.rows - i) % self.rows;
            let src_off = src_row * self.bands;
            let dst_off = i * self.bands;
            out[dst_off..dst_off + self.bands]
                .copy_from_slice(&self.grid[src_off..src_off + self.bands]);
        }
        (self.rows, self.bands)
    }
}

#[cfg(test)]
mod tests_waterfall {
    use super::*;

    fn synthetic_block(freq: f32, frames: usize, sr: u32) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames);
        let dphi = std::f32::consts::TAU * freq / sr as f32;
        let mut phi = 0.0_f32;
        for _ in 0..frames {
            v.push(phi.sin() * 0.5);
            phi += dphi;
        }
        v
    }

    #[test]
    fn snapshot_is_newest_first() {
        let mut w = Waterfall::new(8, 4, 100.0, 4_000.0);
        // Primero un análisis con señal fuerte (482 Hz ≈ banda 3),
        // después uno con silencio. El release del Spectrum hace que
        // la fila más nueva tenga ENERGÍA MENOR que la fila anterior
        // (que vio la señal fresca).
        let hot = synthetic_block(482.0, 4096, 48_000);
        let silence = vec![0.0_f32; 4096];
        w.analyze(&hot, 1, 48_000);
        w.analyze(&silence, 1, 48_000);

        let mut snap = Vec::new();
        let (rows, bands) = w.snapshot(&mut snap);
        assert_eq!(rows, 4);
        assert_eq!(bands, 8);

        let row0_sum: f32 = snap[0..8].iter().sum();
        let row1_sum: f32 = snap[8..16].iter().sum();
        assert!(row1_sum > 0.0, "row1 debería capturar la señal");
        assert!(
            row1_sum > row0_sum,
            "row1 (señal fresca, {row1_sum}) debería superar a row0 (post-silencio, {row0_sum})"
        );
    }

    #[test]
    fn empty_snapshot_is_zero() {
        let w = Waterfall::new(4, 4, 100.0, 1_000.0);
        let mut snap = Vec::new();
        let (rows, bands) = w.snapshot(&mut snap);
        assert_eq!((rows, bands), (4, 4));
        assert!(snap.iter().all(|&v| v == 0.0));
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

// ============================================================
// Levels — medidor peak + RMS sobre snapshots de audio
// ============================================================

/// Niveles instantáneos del stream: pico absoluto y RMS, ambos
/// normalizados a [0, 1] sobre el mono fold del snapshot. Mantiene
/// suavizado attack-inmediato + release-exponencial entre llamadas
/// (igual filosofía que [`Spectrum`]) para que las barras del visor
/// no titilen.
#[derive(Clone, Copy)]
pub struct Levels {
    peak: f32,
    rms: f32,
    release: f32,
}

impl Default for Levels {
    fn default() -> Self {
        Self::new()
    }
}

impl Levels {
    pub fn new() -> Self {
        Self {
            peak: 0.0,
            rms: 0.0,
            release: 0.82,
        }
    }

    pub fn set_release(&mut self, release: f32) {
        self.release = release.clamp(0.0, 0.99);
    }

    pub fn peak(&self) -> f32 {
        self.peak
    }

    pub fn rms(&self) -> f32 {
        self.rms
    }

    /// Procesa un snapshot intercalado y actualiza los niveles. El
    /// mono fold es promedio simple de canales; el RMS es sqrt(media
    /// de cuadrados) sobre los frames mono. Con `samples` vacío sólo
    /// aplica el release.
    pub fn analyze(&mut self, samples: &[f32], channels: u16) {
        let ch = channels.max(1) as usize;
        let frames = samples.len() / ch;
        if frames == 0 {
            self.peak *= self.release;
            self.rms *= self.release;
            return;
        }
        let inv_ch = 1.0 / ch as f32;
        let mut peak_inst = 0.0_f32;
        let mut sq_acc = 0.0_f32;
        for f in 0..frames {
            let mut s = 0.0_f32;
            for c in 0..ch {
                s += samples[f * ch + c];
            }
            let mono = s * inv_ch;
            let abs = mono.abs();
            if abs > peak_inst {
                peak_inst = abs;
            }
            sq_acc += mono * mono;
        }
        let rms_inst = (sq_acc / frames as f32).sqrt();

        // Attack inmediato, release exponencial.
        let prev_peak = self.peak * self.release;
        self.peak = if peak_inst > prev_peak {
            peak_inst.min(1.0)
        } else {
            prev_peak
        };
        let prev_rms = self.rms * self.release;
        self.rms = if rms_inst > prev_rms {
            rms_inst.min(1.0)
        } else {
            prev_rms
        };
    }
}

#[cfg(test)]
mod tests_audio_primitives {
    use super::*;

    fn sine(freq: f32, frames: usize, sr: u32, amp: f32) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames);
        let dphi = std::f32::consts::TAU * freq / sr as f32;
        let mut phi = 0.0_f32;
        for _ in 0..frames {
            v.push(phi.sin() * amp);
            phi += dphi;
        }
        v
    }

    // ---------- Spectrum ----------

    #[test]
    fn spectrum_peaks_at_dominant_band() {
        // Senoide alineada exactamente al centro de banda 2.
        // Goertzel resuena → esa banda gana sin ambigüedad.
        let mut spec = Spectrum::log_bands(4, 100.0, 4_000.0);
        spec.set_release(0.0); // sin smoothing — análisis puro.
        let target = spec.bands()[2];
        let sig = sine(target, 4096, 48_000, 0.5);
        spec.analyze(&sig, 1, 48_000);
        let mags = spec.magnitudes();
        let argmax = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert_eq!(argmax, 2, "esperaba banda 2, mags={mags:?}");
        assert!(mags[2] > 0.2, "magnitud banda dominante = {}", mags[2]);
    }

    #[test]
    fn spectrum_silence_decays() {
        let mut spec = Spectrum::log_bands(8, 40.0, 16_000.0);
        // Cargo energía y después silencio: el release debe bajar.
        let sig = sine(440.0, 4096, 48_000, 0.5);
        spec.analyze(&sig, 1, 48_000);
        let after_hot = spec.magnitudes().iter().sum::<f32>();
        spec.analyze(&[0.0; 4096], 1, 48_000);
        let after_silence = spec.magnitudes().iter().sum::<f32>();
        assert!(
            after_silence < after_hot,
            "silencio ({after_silence}) debería ser menor que hot ({after_hot})"
        );
    }

    // ---------- Levels ----------

    #[test]
    fn levels_peak_matches_signal_amplitude() {
        let mut lv = Levels::new();
        lv.set_release(0.0);
        // Senoide de amplitud 0.4 — pico debería estar cerca de 0.4.
        let sig = sine(440.0, 4096, 48_000, 0.4);
        lv.analyze(&sig, 1);
        assert!(
            (lv.peak() - 0.4).abs() < 0.02,
            "peak = {}, esperaba ≈ 0.4",
            lv.peak()
        );
        // RMS senoide = amp / sqrt(2) ≈ 0.283 para amp=0.4.
        let expected_rms = 0.4_f32 / std::f32::consts::SQRT_2;
        assert!(
            (lv.rms() - expected_rms).abs() < 0.02,
            "rms = {}, esperaba ≈ {expected_rms}",
            lv.rms()
        );
    }

    #[test]
    fn levels_silence_zeros_with_no_release() {
        let mut lv = Levels::new();
        lv.set_release(0.0);
        lv.analyze(&[0.0; 1024], 1);
        assert_eq!(lv.peak(), 0.0);
        assert_eq!(lv.rms(), 0.0);
    }

    #[test]
    fn levels_mono_fold_averages_channels() {
        let mut lv = Levels::new();
        lv.set_release(0.0);
        // Stereo donde L=+0.5 y R=-0.5: mono fold = 0, peak debería
        // estar cerca de 0 (cancela), no de 0.5.
        let mut sig = Vec::new();
        for _ in 0..1024 {
            sig.push(0.5);
            sig.push(-0.5);
        }
        lv.analyze(&sig, 2);
        assert!(lv.peak() < 1e-4, "peak con cancelación = {}", lv.peak());
    }

    // ---------- AudioProbe ----------

    #[test]
    fn probe_push_then_snapshot_is_chronological() {
        // Capacidad mínima del probe es 64 (ver AudioProbe::new) —
        // los tests trabajan a ese tamaño y validan los slots
        // ocupados al final del snapshot.
        let probe = AudioProbe::new(64);
        let data: Vec<f32> = (1..=6).map(|i| i as f32).collect();
        probe.push(&data, 48_000, 1);
        let mut out = Vec::new();
        let (sr, ch) = probe.snapshot(&mut out);
        assert_eq!(sr, 48_000);
        assert_eq!(ch, 1);
        assert_eq!(out.len(), 64);
        // Los primeros 58 slots quedaron en silencio (no se llenó
        // todavía la vuelta); los últimos 6 son el bloque empujado
        // en orden cronológico.
        assert!(out[..58].iter().all(|&v| v == 0.0));
        assert_eq!(&out[58..64], &data[..]);
    }

    #[test]
    fn probe_wrap_overwrites_oldest() {
        let probe = AudioProbe::new(64);
        // Empuja 70 valores en un ring de cap=64: los 6 primeros se
        // sobrescriben, el snapshot trae [7..70] en orden cronológico.
        let data: Vec<f32> = (1..=70).map(|i| i as f32).collect();
        probe.push(&data, 44_100, 2);
        let mut out = Vec::new();
        let (sr, ch) = probe.snapshot(&mut out);
        assert_eq!(sr, 44_100);
        assert_eq!(ch, 2);
        let expected: Vec<f32> = (7..=70).map(|i| i as f32).collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn probed_audio_source_is_transparent_and_caches() {
        struct Const(f32);
        impl AudioSource for Const {
            fn fill(&mut self, buf: &mut [f32], _: u32, _: u16) {
                for s in buf.iter_mut() {
                    *s = self.0;
                }
            }
        }
        let probe = AudioProbe::new(16);
        let mut probed = ProbedAudioSource::new(Const(0.3), probe.clone());
        let mut buf = vec![0.0_f32; 8];
        probed.fill(&mut buf, 48_000, 1);
        // El sink ve el mismo flujo que el inner.
        assert!(buf.iter().all(|&v| (v - 0.3).abs() < 1e-6));
        // El probe vio el bloque entero.
        let mut snap = Vec::new();
        probe.snapshot(&mut snap);
        let tail: Vec<f32> = snap.iter().rev().take(8).cloned().collect();
        assert!(tail.iter().all(|&v| (v - 0.3).abs() < 1e-6));
    }
}

// ============================================================
// Subtitles — SRT parser + query por timestamp
// ============================================================

/// Una entrada de subtítulo con su rango temporal y el texto a
/// mostrar mientras dure. `text` puede contener saltos de línea
/// (las líneas múltiples del SRT se preservan con `\n`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleCue {
    pub start: Duration,
    pub end: Duration,
    pub text: String,
}

/// Pista de subtítulos ordenada por tiempo. Querys binarias para
/// resolver "qué cue está activo en t". El consumidor (UI) le pasa
/// la posición actual del audio y recibe el texto a pintar.
#[derive(Debug, Clone, Default)]
pub struct SubtitleTrack {
    cues: Vec<SubtitleCue>,
}

impl SubtitleTrack {
    pub fn new(mut cues: Vec<SubtitleCue>) -> Self {
        cues.sort_by_key(|c| c.start);
        Self { cues }
    }

    pub fn cues(&self) -> &[SubtitleCue] {
        &self.cues
    }

    pub fn len(&self) -> usize {
        self.cues.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// Devuelve el cue activo en `t`, si existe. Si dos cues se
    /// solapan, gana el de `start` más cercano por debajo de `t`
    /// (el último que arrancó).
    pub fn at(&self, t: Duration) -> Option<&SubtitleCue> {
        // Binary search por start; el cue candidato es el último con
        // start <= t. Si su end > t, es el activo.
        if self.cues.is_empty() {
            return None;
        }
        let idx = match self.cues.binary_search_by_key(&t, |c| c.start) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        let c = &self.cues[idx];
        if t < c.end {
            Some(c)
        } else {
            None
        }
    }

    /// Parsea un cuerpo SRT. Tolerante: salta entradas malformadas
    /// con un mensaje en el log de errores devuelto. Si el archivo
    /// entero no tiene cues válidos, devuelve `Err`.
    ///
    /// Formato SRT esperado por entrada:
    ///
    /// ```text
    /// 1
    /// 00:00:01,000 --> 00:00:03,500
    /// Línea uno
    /// Línea dos
    ///
    /// 2
    /// ...
    /// ```
    ///
    /// El número de índice se ignora. El separador `,` o `.` para
    /// los milisegundos se acepta indistinto (compat WebVTT mínimo).
    pub fn parse_srt(text: &str) -> Result<Self, String> {
        let mut cues: Vec<SubtitleCue> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        // Normalizamos line endings y partimos por bloques separados
        // por línea vacía.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        for (i, block) in text.split("\n\n").enumerate() {
            let block = block.trim_matches('\n');
            if block.is_empty() {
                continue;
            }
            let mut lines = block.lines();
            // Primera línea puede ser índice (numérico) o ya la
            // línea de timing si el SRT lo omitió.
            let first = match lines.next() {
                Some(l) => l.trim(),
                None => continue,
            };
            let timing_line = if first.contains("-->") {
                first
            } else {
                match lines.next() {
                    Some(l) => l.trim(),
                    None => {
                        warnings.push(format!("bloque {i}: falta línea de timing"));
                        continue;
                    }
                }
            };
            let (start, end) = match parse_timing_line(timing_line) {
                Ok(t) => t,
                Err(e) => {
                    warnings.push(format!("bloque {i}: timing '{timing_line}' — {e}"));
                    continue;
                }
            };
            let rest: Vec<&str> = lines.collect();
            let text = rest.join("\n").trim().to_string();
            if text.is_empty() {
                continue;
            }
            cues.push(SubtitleCue { start, end, text });
        }
        if cues.is_empty() {
            return Err(format!(
                "ningún cue válido en el SRT (avisos: {})",
                warnings.join(" · ")
            ));
        }
        Ok(Self::new(cues))
    }

    /// Parsea un cuerpo WebVTT — el formato de subtítulos nativo de la
    /// web (par del stack WebM + AV1 + Opus). Tolerante igual que
    /// [`Self::parse_srt`]: salta bloques malformados y devuelve `Err`
    /// sólo si no quedó ningún cue.
    ///
    /// Diferencias con SRT que cubre el parser:
    /// - Cabecera `WEBVTT` (con texto opcional en la misma línea) que
    ///   se descarta, más el BOM `\u{FEFF}` si está presente.
    /// - Bloques `NOTE`, `STYLE` y `REGION` que se ignoran enteros.
    /// - Identificador de cue opcional (línea previa al timing sin
    ///   `-->`) que se descarta.
    /// - Timestamps `MM:SS.mmm` (sin hora) además de `HH:MM:SS.mmm`.
    /// - Ajustes de posición tras el timestamp final
    ///   (`line:0 position:50%`…) que se ignoran.
    /// - Etiquetas en línea (`<b>`, `<i>`, `<c.foo>`, timestamps
    ///   `<00:00:01.000>`) que se eliminan, y entidades HTML comunes
    ///   (`&amp;` `&lt;` `&gt;` `&nbsp;` `&lrm;` `&rlm;`) que se
    ///   decodifican — queda texto plano listo para pintar.
    pub fn parse_webvtt(text: &str) -> Result<Self, String> {
        let mut cues: Vec<SubtitleCue> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        // Normalizamos line endings y quitamos el BOM si está.
        let text = text
            .trim_start_matches('\u{FEFF}')
            .replace("\r\n", "\n")
            .replace('\r', "\n");

        for (i, block) in text.split("\n\n").enumerate() {
            let block = block.trim_matches('\n');
            if block.is_empty() {
                continue;
            }
            // La cabecera WEBVTT vive en el primer bloque; el cue (si lo
            // hay pegado a ella) viene tras un \n, así que sólo
            // descartamos esa línea, no el bloque entero.
            let block = if i == 0 && block.starts_with("WEBVTT") {
                match block.split_once('\n') {
                    Some((_, rest)) => rest.trim_matches('\n'),
                    None => continue, // bloque era sólo la cabecera
                }
            } else {
                block
            };
            // Bloques de metadatos que no son cues.
            let head = block.lines().next().unwrap_or("").trim_start();
            if head == "NOTE"
                || head.starts_with("NOTE ")
                || head == "STYLE"
                || head == "REGION"
            {
                continue;
            }

            let mut lines = block.lines();
            let first = match lines.next() {
                Some(l) => l.trim(),
                None => continue,
            };
            // Identificador de cue opcional: si la primera línea no
            // tiene `-->`, es el id y la siguiente es el timing.
            let timing_line = if first.contains("-->") {
                first
            } else {
                match lines.next() {
                    Some(l) => l.trim(),
                    None => {
                        warnings.push(format!("bloque {i}: falta línea de timing"));
                        continue;
                    }
                }
            };
            let (start, end) = match parse_vtt_timing_line(timing_line) {
                Ok(t) => t,
                Err(e) => {
                    warnings.push(format!("bloque {i}: timing '{timing_line}' — {e}"));
                    continue;
                }
            };
            let rest: Vec<&str> = lines.collect();
            let raw = rest.join("\n");
            let text = strip_vtt_markup(&raw).trim().to_string();
            if text.is_empty() {
                continue;
            }
            cues.push(SubtitleCue { start, end, text });
        }
        if cues.is_empty() {
            return Err(format!(
                "ningún cue válido en el WebVTT (avisos: {})",
                warnings.join(" · ")
            ));
        }
        Ok(Self::new(cues))
    }

    /// Autodetecta SRT vs WebVTT por la cabecera `WEBVTT` (tras un BOM
    /// opcional) y delega al parser correspondiente. Lo que usa el
    /// consumidor cuando no sabe el formato de antemano.
    pub fn parse_subtitles(text: &str) -> Result<Self, String> {
        let head = text.trim_start_matches('\u{FEFF}').trim_start();
        if head.starts_with("WEBVTT") {
            Self::parse_webvtt(text)
        } else {
            Self::parse_srt(text)
        }
    }
}

/// Timing WebVTT: como el de SRT pero el lado derecho puede arrastrar
/// ajustes de posición tras el timestamp (`... --> 00:00:03.000 line:0
/// position:50%`). Tomamos sólo el primer token de cada lado.
fn parse_vtt_timing_line(s: &str) -> Result<(Duration, Duration), String> {
    let parts: Vec<&str> = s.split("-->").map(str::trim).collect();
    if parts.len() != 2 {
        return Err("esperaba 'MM:SS.mmm --> MM:SS.mmm'".into());
    }
    // El primer token whitespace-separado es el timestamp; el resto
    // (settings del cue) se ignora.
    let start_tok = parts[0].split_whitespace().next().unwrap_or(parts[0]);
    let end_tok = parts[1].split_whitespace().next().unwrap_or(parts[1]);
    let start = parse_timestamp(start_tok)?;
    let end = parse_timestamp(end_tok)?;
    Ok((start, end))
}

/// Elimina las etiquetas en línea de WebVTT (`<b>`, `<i>`, `<c.foo>`,
/// timestamps `<00:00:01.000>`, etc.) y decodifica las entidades HTML
/// comunes — deja texto plano para pintar. No es un parser HTML: sólo
/// borra todo lo que está entre `<` y `>`.
fn strip_vtt_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0u32;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&lrm;", "")
        .replace("&rlm;", "")
}

fn parse_timing_line(s: &str) -> Result<(Duration, Duration), String> {
    let parts: Vec<&str> = s.split("-->").map(str::trim).collect();
    if parts.len() != 2 {
        return Err("esperaba 'HH:MM:SS,mmm --> HH:MM:SS,mmm'".into());
    }
    let start = parse_timestamp(parts[0])?;
    let end = parse_timestamp(parts[1])?;
    Ok((start, end))
}

fn parse_timestamp(s: &str) -> Result<Duration, String> {
    // Acepta HH:MM:SS,mmm o HH:MM:SS.mmm (SRT) y MM:SS.mmm (WebVTT
    // omite la hora cuando es 0). Trim para tolerar espacios.
    let s = s.trim();
    let (hms, ms_part) = match s.rsplit_once(',').or_else(|| s.rsplit_once('.')) {
        Some(p) => p,
        None => (s, "0"),
    };
    let hms_parts: Vec<&str> = hms.split(':').collect();
    // 3 partes = HH:MM:SS ; 2 partes = MM:SS (la hora es implícita 0).
    let (h, m, sec) = match hms_parts.as_slice() {
        [hh, mm, ss] => (
            hh.parse::<u64>().map_err(|_| format!("hora inválida en '{s}'"))?,
            mm.parse::<u64>().map_err(|_| format!("minuto inválido en '{s}'"))?,
            ss.parse::<u64>().map_err(|_| format!("segundo inválido en '{s}'"))?,
        ),
        [mm, ss] => (
            0,
            mm.parse::<u64>().map_err(|_| format!("minuto inválido en '{s}'"))?,
            ss.parse::<u64>().map_err(|_| format!("segundo inválido en '{s}'"))?,
        ),
        _ => return Err(format!("timestamp inválido '{s}'")),
    };
    let ms: u64 = ms_part
        .parse()
        .map_err(|_| format!("ms inválidos en '{s}'"))?;
    let total_ms = ((h * 3600) + (m * 60) + sec) * 1000 + ms;
    Ok(Duration::from_millis(total_ms))
}

#[cfg(test)]
mod tests_subtitles {
    use super::*;

    #[test]
    fn parse_simple_srt() {
        let src = "1\n\
            00:00:01,000 --> 00:00:03,500\n\
            Hola mundo\n\
            \n\
            2\n\
            00:00:04,000 --> 00:00:06,000\n\
            Segunda línea\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        assert_eq!(track.len(), 2);
        assert_eq!(track.cues()[0].text, "Hola mundo");
        assert_eq!(track.cues()[0].start, Duration::from_millis(1000));
        assert_eq!(track.cues()[0].end, Duration::from_millis(3500));
    }

    #[test]
    fn query_active_cue() {
        let src = "1\n\
            00:00:01,000 --> 00:00:03,000\n\
            uno\n\
            \n\
            2\n\
            00:00:05,000 --> 00:00:07,000\n\
            dos\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        assert!(track.at(Duration::from_millis(500)).is_none());
        assert_eq!(track.at(Duration::from_millis(2000)).unwrap().text, "uno");
        // Entre cues: gap, sin activo.
        assert!(track.at(Duration::from_millis(4000)).is_none());
        assert_eq!(track.at(Duration::from_millis(6500)).unwrap().text, "dos");
    }

    #[test]
    fn multiline_text_preserved() {
        let src = "1\n\
            00:00:01,000 --> 00:00:02,000\n\
            primera\n\
            segunda\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        assert_eq!(track.cues()[0].text, "primera\nsegunda");
    }

    #[test]
    fn dot_separator_accepted() {
        let src = "1\n00:00:01.500 --> 00:00:03.250\nhola\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        assert_eq!(track.cues()[0].start, Duration::from_millis(1500));
        assert_eq!(track.cues()[0].end, Duration::from_millis(3250));
    }

    #[test]
    fn empty_srt_fails() {
        let err = SubtitleTrack::parse_srt("").unwrap_err();
        assert!(err.contains("cue"));
    }

    #[test]
    fn malformed_block_skipped() {
        let src = "1\n\
            no-es-timing\n\
            texto\n\
            \n\
            2\n\
            00:00:01,000 --> 00:00:02,000\n\
            válido\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        // Sólo el segundo bloque entra.
        assert_eq!(track.len(), 1);
        assert_eq!(track.cues()[0].text, "válido");
    }

    // --- WebVTT ---

    #[test]
    fn parse_simple_webvtt() {
        let src = "WEBVTT\n\
            \n\
            00:00:01.000 --> 00:00:03.500\n\
            Hola mundo\n\
            \n\
            00:00:04.000 --> 00:00:06.000\n\
            Segunda línea\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.len(), 2);
        assert_eq!(track.cues()[0].text, "Hola mundo");
        assert_eq!(track.cues()[0].start, Duration::from_millis(1000));
        assert_eq!(track.cues()[0].end, Duration::from_millis(3500));
    }

    #[test]
    fn webvtt_mm_ss_timestamp() {
        // WebVTT permite omitir la hora cuando es 0.
        let src = "WEBVTT\n\n01:02.500 --> 01:05.000\nbreve\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.cues()[0].start, Duration::from_millis(62_500));
        assert_eq!(track.cues()[0].end, Duration::from_millis(65_000));
    }

    #[test]
    fn webvtt_cue_id_and_settings_ignored() {
        let src = "WEBVTT\n\
            \n\
            intro\n\
            00:00:01.000 --> 00:00:03.000 line:0 position:50% align:start\n\
            con ajustes\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.len(), 1);
        assert_eq!(track.cues()[0].text, "con ajustes");
        assert_eq!(track.cues()[0].end, Duration::from_millis(3000));
    }

    #[test]
    fn webvtt_note_style_region_skipped() {
        let src = "WEBVTT\n\
            \n\
            NOTE este bloque es un comentario\n\
            que ocupa varias líneas\n\
            \n\
            STYLE\n\
            ::cue { color: yellow }\n\
            \n\
            00:00:01.000 --> 00:00:02.000\n\
            sólo este cuenta\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.len(), 1);
        assert_eq!(track.cues()[0].text, "sólo este cuenta");
    }

    #[test]
    fn webvtt_strips_inline_tags_and_entities() {
        let src = "WEBVTT\n\
            \n\
            00:00:01.000 --> 00:00:02.000\n\
            <c.loud>Hola</c> <b>mundo</b> <00:00:01.500>cruel & feo\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.cues()[0].text, "Hola mundo cruel & feo");
    }

    #[test]
    fn webvtt_header_with_trailing_text() {
        // La cabecera puede llevar texto y el primer cue venir pegado.
        let src = "WEBVTT - Mi película\n\
            \n\
            00:00:01.000 --> 00:00:02.000\n\
            primero\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.len(), 1);
        assert_eq!(track.cues()[0].text, "primero");
    }

    #[test]
    fn parse_subtitles_autodetects() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nvtt\n";
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nsrt\n";
        assert_eq!(SubtitleTrack::parse_subtitles(vtt).unwrap().cues()[0].text, "vtt");
        assert_eq!(SubtitleTrack::parse_subtitles(srt).unwrap().cues()[0].text, "srt");
    }

    #[test]
    fn empty_webvtt_fails() {
        let err = SubtitleTrack::parse_webvtt("WEBVTT\n").unwrap_err();
        assert!(err.contains("cue"));
    }
}
