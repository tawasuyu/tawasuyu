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
