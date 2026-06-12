//! Producción de frames RGBA y capacidad de transporte seekable.
//!
//! Traits centrales del pipeline de video:
//! - [`FrameSource`]: entrega bytes RGBA con un tamaño.
//! - [`Seekable`]: capacidad opcional para fuentes con timeline conocido.
//! - [`TestCard`]: generador procedural de referencia.

use std::time::Duration;

// ============================================================
// FrameSource — productor de frames RGBA
// ============================================================

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

    /// PTS (presentation timestamp) del último frame que `tick` dejó en
    /// `buf`, si la fuente lo conoce: el momento, desde el inicio del
    /// stream, en que ese frame debe mostrarse. Lo consume la política de
    /// [`crate::sync`] para atar el video al reloj de audio (M1 de
    /// `PARIDAD.md`). Default `None` — fuentes sin noción de tiempo
    /// (imágenes fijas) o que aún no lo implementan. Sólo es válido
    /// leerlo justo después de un `tick` que devolvió `Some`.
    fn pts(&self) -> Option<Duration> {
        None
    }
}

// Reenvío para `Box<dyn FrameSource ...>`. Igual que el de
// `AudioSource`: permite componer wrappers (`PausableVideo<Box<dyn
// FrameSource + Send>>`) sin re-implementar el trait.
impl<T: FrameSource + ?Sized> FrameSource for Box<T> {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        (**self).tick(dt, buf)
    }
    fn pts(&self) -> Option<Duration> {
        (**self).pts()
    }
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

// ============================================================
// TestCard — generador procedural de referencia
// ============================================================

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
    /// Frames emitidos hasta ahora — define el PTS del próximo (índice/fps).
    emitted: u64,
    /// PTS del último frame emitido, expuesto por [`FrameSource::pts`].
    last_pts: Option<Duration>,
}

impl TestCard {
    pub fn new(width: u32, height: u32, fps: f32) -> Self {
        Self {
            width,
            height,
            fps: fps.max(1.0),
            elapsed: 0.0,
            accum_since_frame: f32::INFINITY,
            emitted: 0,
            last_pts: None,
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
        // PTS = índice/fps: tiempo de presentación estable contra el reloj
        // de audio (ver crate::sync). Se fija antes de pintar; se lee con
        // pts() tras este tick.
        self.last_pts = Some(Duration::from_secs_f32(self.emitted as f32 / self.fps));
        self.emitted += 1;

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

    fn pts(&self) -> Option<Duration> {
        self.last_pts
    }
}

#[cfg(test)]
mod tests_frame_pts {
    use super::*;
    use crate::transport::{Pause, PausableVideo};

    struct Dummy;
    impl FrameSource for Dummy {
        fn tick(&mut self, _dt: Duration, _buf: &mut Vec<u8>) -> Option<(u32, u32)> {
            None
        }
    }

    #[test]
    fn pts_default_es_none() {
        // Una fuente que no implementa pts() devuelve None por el default
        // del trait — no rompe a las fuentes sin noción de tiempo.
        assert_eq!(Dummy.pts(), None);
    }

    #[test]
    fn testcard_pts_avanza_por_frame() {
        // 10 fps → 100 ms por frame; PTS = índice/fps.
        let mut tc = TestCard::new(16, 16, 10.0);
        let mut buf = Vec::new();
        assert_eq!(tc.pts(), None, "sin frames todavía no hay PTS");
        // Primer tick emite (accum arranca en infinito) → frame 0, PTS 0.
        assert!(tc.tick(Duration::from_millis(100), &mut buf).is_some());
        assert_eq!(tc.pts(), Some(Duration::ZERO));
        // Segundo frame → PTS = 1/10 s = 100 ms.
        assert!(tc.tick(Duration::from_millis(100), &mut buf).is_some());
        let p = tc.pts().expect("hay PTS tras el segundo frame");
        assert!((p.as_secs_f32() - 0.1).abs() < 1e-4, "pts = {p:?}");
    }

    #[test]
    fn pausable_reenvia_pts_del_inner() {
        let mut pv = PausableVideo::new(TestCard::new(16, 16, 10.0), Pause::new());
        let mut buf = Vec::new();
        pv.tick(Duration::from_millis(100), &mut buf);
        assert_eq!(pv.pts(), Some(Duration::ZERO));
    }

    #[test]
    fn box_dyn_reenvia_pts() {
        let mut b: Box<dyn FrameSource + Send> = Box::new(TestCard::new(16, 16, 10.0));
        let mut buf = Vec::new();
        b.tick(Duration::from_millis(100), &mut buf);
        assert_eq!(b.pts(), Some(Duration::ZERO));
    }
}

