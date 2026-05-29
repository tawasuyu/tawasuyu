//! multimedia-core — productores de video y audio del dominio.
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
//! gstreamer, v4l2, cpal…) vivan en crates `multimedia-source-*` o
//! `multimedia-audio-*` que impl los traits.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
/// pipeline `multimedia-core → llimphi-surface → frame` sin meter
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
