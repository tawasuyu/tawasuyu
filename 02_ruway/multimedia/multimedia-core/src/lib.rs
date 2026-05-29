//! multimedia-core — emisores de frames RGBA para `llimphi-surface`.
//!
//! Define el trait [`FrameSource`]: un productor que entrega una vista
//! sobre un buffer RGBA del frame actual + su tamaño. Implementaciones
//! posibles a futuro: decoder de video (ffmpeg/gst), cámara, screen
//! capture, generador procedural. Para el MVP sólo trae [`TestCard`]:
//! un patrón sintético que pinta un gradiente animado y un círculo
//! que rebota — verifica el pipeline GPU sin depender de decoders
//! externos.
//!
//! El crate es `std` y no tiene dependencias — la idea es que el
//! núcleo del dominio sea liviano y los backends pesados (ffmpeg,
//! gstreamer, v4l2…) vivan en crates `multimedia-source-*` que
//! impl `FrameSource`.

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
