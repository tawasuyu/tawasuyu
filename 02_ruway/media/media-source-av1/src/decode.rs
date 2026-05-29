//! Decode AV1 nativo vía `rav1d` (port puro-Rust de dav1d) → RGBA.
//!
//! rav1d expone el ABI C de dav1d: contexto opaco, `send_data` /
//! `get_picture` con `EAGAIN` como "dame más / dame tiempo", planos YUV
//! crudos con stride. Este módulo lo envuelve en un [`Av1VideoSource`]
//! que implementa [`media_core::FrameSource`] — el resto del dominio no
//! ve el `unsafe`. Forzamos `max_frame_delay = 1` y `n_threads = 1` para
//! un modelo de bajo retardo (una TU entra → un frame sale), que encaja
//! con el `tick(dt)` del bucle Elm.

use std::fs::File;
use std::io::BufReader;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::time::Duration;

use media_core::{FrameSource, Seekable};

use crate::ivf::IvfReader;

use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::{
    DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I422, DAV1D_PIXEL_LAYOUT_I444,
};
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{
    dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_flush,
    dav1d_get_picture, dav1d_open, dav1d_picture_unref, dav1d_send_data,
};

/// `DAV1D_ERR(EAGAIN)` — dav1d devuelve los errores negados.
fn eagain() -> i32 {
    -libc::EAGAIN
}

// ─── Contexto rav1d con RAII ─────────────────────────────────────────────────

struct Dav1dDecoder {
    ctx: Option<Dav1dContext>,
}

impl Dav1dDecoder {
    fn new() -> Result<Self, String> {
        // SAFETY: `dav1d_default_settings` escribe la estructura entera
        // (write, no read previo), así que `uninit` es válido; luego
        // `dav1d_open` la lee y construye el contexto.
        unsafe {
            let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
            dav1d_default_settings(NonNull::new_unchecked(settings.as_mut_ptr()));
            let mut settings = settings.assume_init();
            settings.n_threads = 1;
            settings.max_frame_delay = 1;

            let mut ctx: Option<Dav1dContext> = None;
            let r = dav1d_open(
                Some(NonNull::from(&mut ctx)),
                Some(NonNull::from(&mut settings)),
            );
            if r.0 != 0 || ctx.is_none() {
                return Err(format!("dav1d_open falló (código {})", r.0));
            }
            Ok(Self { ctx })
        }
    }
}

impl Drop for Dav1dDecoder {
    fn drop(&mut self) {
        // SAFETY: `ctx` viene de `dav1d_open` y no se cerró aún.
        unsafe {
            dav1d_close(Some(NonNull::from(&mut self.ctx)));
        }
    }
}

// ─── Av1VideoSource ──────────────────────────────────────────────────────────

/// Fuente de frames AV1 nativa: demuxea IVF y decodifica con rav1d. Es
/// la implementación del formato de video NATIVO de gioser (sin ffmpeg,
/// sin C, sin patentes).
pub struct Av1VideoSource {
    path: PathBuf,
    reader: IvfReader<BufReader<File>>,
    width: u32,
    height: u32,
    fps: f32,
    num_frames: u32,
    decoder: Dav1dDecoder,
    /// Datos enviados al decoder pero aún no consumidos (tras un EAGAIN).
    staged: Option<Dav1dData>,
    eof: bool,
    exhausted: bool,
    /// Índice absoluto del próximo frame a procesar.
    frame_index: u32,
    /// Frames < target se decodifican y descartan (soporte de seek).
    target_frame: u32,
    accum: Duration,
}

impl Av1VideoSource {
    /// Abre un `.ivf` con video AV1. Falla si no es IVF/AV1 o si el
    /// decoder no arranca.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let reader = IvfReader::open(&path).map_err(|e| format!("abrir IVF: {e}"))?;
        let h = *reader.header();
        if !h.is_av1() {
            return Err(format!(
                "el contenedor no declara AV1 (codec {:?})",
                String::from_utf8_lossy(&h.codec)
            ));
        }
        let decoder = Dav1dDecoder::new()?;
        Ok(Self {
            path,
            reader,
            width: h.width as u32,
            height: h.height as u32,
            fps: h.fps(),
            num_frames: h.num_frames,
            decoder,
            staged: None,
            eof: false,
            exhausted: false,
            frame_index: 0,
            target_frame: 0,
            accum: Duration::ZERO,
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn fps(&self) -> f32 {
        self.fps
    }

    /// Empaqueta una TU en un `Dav1dData` (copia a buffer interno de
    /// dav1d). `None` si la alocación falla.
    fn make_data(&self, bytes: &[u8]) -> Option<Dav1dData> {
        // SAFETY: `data_create` escribe el `Dav1dData` y devuelve un
        // puntero a `bytes.len()` bytes alocados; copiamos ahí.
        unsafe {
            let mut d = MaybeUninit::<Dav1dData>::uninit();
            let ptr = dav1d_data_create(Some(NonNull::new_unchecked(d.as_mut_ptr())), bytes.len());
            if ptr.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            Some(d.assume_init())
        }
    }

    /// Bombea el decoder hasta producir el próximo frame a EMITIR
    /// (saltando los anteriores a `target_frame`) y lo escribe en `out`.
    fn decode_emit(&mut self, out: &mut Vec<u8>) -> Option<(u32, u32)> {
        loop {
            if self.exhausted {
                return None;
            }
            // 1. Asegurar datos en vuelo si quedan TUs.
            if self.staged.is_none() && !self.eof {
                match self.reader.next_unit() {
                    Ok(Some(u)) => match self.make_data(&u.data) {
                        Some(d) => self.staged = Some(d),
                        None => {
                            self.exhausted = true;
                            return None;
                        }
                    },
                    Ok(None) => self.eof = true,
                    Err(_) => {
                        self.exhausted = true;
                        return None;
                    }
                }
            }
            // 2. Enviar (o reintentar) la TU staged.
            if let Some(d) = self.staged.as_mut() {
                // SAFETY: ctx vivo; `d` válido para read+write.
                let r = unsafe { dav1d_send_data(self.decoder.ctx, Some(NonNull::from(d))) };
                if r.0 == 0 {
                    self.staged = None; // consumida entera
                } else if r.0 == eagain() {
                    // Decoder lleno: no consumió, la reintentamos luego.
                } else {
                    self.exhausted = true;
                    return None;
                }
            }
            // 3. Pedir un frame.
            let mut pic = MaybeUninit::<Dav1dPicture>::uninit();
            // SAFETY: ctx vivo; `get_picture` SIEMPRE escribe `out`
            // (default en EAGAIN), por eso es seguro `assume_init` sólo
            // en la rama de éxito.
            let r = unsafe {
                dav1d_get_picture(
                    self.decoder.ctx,
                    Some(NonNull::new_unchecked(pic.as_mut_ptr())),
                )
            };
            if r.0 == 0 {
                let mut pic = unsafe { pic.assume_init() };
                let emit = self.frame_index >= self.target_frame;
                let dims = if emit {
                    // SAFETY: picture válida hasta el unref de abajo.
                    unsafe { convert_yuv_to_rgba(&pic, out) }
                } else {
                    None
                };
                // SAFETY: liberamos la referencia de la picture siempre.
                unsafe { dav1d_picture_unref(Some(NonNull::from(&mut pic))) };
                self.frame_index += 1;
                if emit {
                    return dims;
                }
                // frame descartado por seek: seguir bombeando.
                continue;
            } else if r.0 == eagain() {
                if self.eof && self.staged.is_none() {
                    self.exhausted = true;
                    return None; // drenado
                }
                // necesita más datos: volver a 1.
            } else {
                self.exhausted = true;
                return None;
            }
        }
    }

    /// Reabre desde el inicio y configura el descarte hasta `frame`.
    fn restart_to(&mut self, frame: u32) {
        if let Ok(r) = IvfReader::open(&self.path) {
            self.reader = r;
        }
        // SAFETY: ctx vivo; flush descarta estado interno.
        if let Some(ctx) = self.decoder.ctx {
            unsafe { dav1d_flush(ctx) };
        }
        self.unref_staged();
        self.eof = false;
        self.exhausted = false;
        self.frame_index = 0;
        self.target_frame = frame;
        self.accum = Duration::ZERO;
    }

    fn unref_staged(&mut self) {
        if let Some(mut d) = self.staged.take() {
            // SAFETY: `d` es un Dav1dData válido obtenido de data_create.
            unsafe { dav1d_data_unref(Some(NonNull::from(&mut d))) };
        }
    }
}

impl Drop for Av1VideoSource {
    fn drop(&mut self) {
        // Liberar la TU en vuelo ANTES de que el decoder se cierre.
        self.unref_staged();
    }
}

// SAFETY: el contexto de rav1d y la `Dav1dData` en vuelo contienen
// `NonNull` (de ahí que el auto-Send no aplique), pero rav1d es seguro de
// mover entre hilos mientras no se use concurrentemente — su estado
// interno está tras locks. En gioser la fuente vive siempre tras el
// `Mutex` del pipeline (un solo hilo la toca a la vez), así que moverla
// entre hilos es correcto.
unsafe impl Send for Av1VideoSource {}

impl FrameSource for Av1VideoSource {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        if self.exhausted {
            return None;
        }
        self.accum += dt;
        let interval = Duration::from_secs_f32(1.0 / self.fps.max(1.0));
        if self.accum < interval {
            return None;
        }
        self.accum -= interval;
        self.decode_emit(buf)
    }
}

impl Seekable for Av1VideoSource {
    fn position(&self) -> Duration {
        Duration::from_secs_f64(self.frame_index as f64 / self.fps.max(1.0) as f64)
    }

    fn duration(&self) -> Option<Duration> {
        if self.num_frames == 0 {
            None
        } else {
            Some(Duration::from_secs_f64(
                self.num_frames as f64 / self.fps.max(1.0) as f64,
            ))
        }
    }

    fn seek_to(&mut self, pos: Duration) {
        let frame = (pos.as_secs_f64() * self.fps.max(1.0) as f64).floor() as u32;
        let frame = if self.num_frames > 0 {
            frame.min(self.num_frames.saturating_sub(1))
        } else {
            frame
        };
        self.restart_to(frame);
    }
}

// ─── Conversión YUV → RGBA ───────────────────────────────────────────────────

/// Lee un sample de 8 bits desde un plano (soporta 10/12 bits con shift).
///
/// # Safety
/// `base` debe apuntar a un plano de al menos `(y+1)*stride` bytes, con
/// el sample `x` dentro de la fila.
unsafe fn sample(base: *const u8, stride: usize, x: usize, y: usize, shift: u32) -> u8 {
    if shift == 0 {
        unsafe { *base.add(y * stride + x) }
    } else {
        let p = unsafe { base.add(y * stride + x * 2) } as *const u16;
        (unsafe { p.read_unaligned() } >> shift) as u8
    }
}

#[inline]
fn clamp8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// BT.601 (suficiente para un visor; el color exacto no es el objetivo
/// de esta fase).
#[inline]
fn yuv_to_rgb(y: i32, u: i32, v: i32) -> (u8, u8, u8) {
    let d = (u - 128) as f32;
    let e = (v - 128) as f32;
    let c = y as f32;
    let r = (c + 1.402 * e).round() as i32;
    let g = (c - 0.344136 * d - 0.714136 * e).round() as i32;
    let b = (c + 1.772 * d).round() as i32;
    (clamp8(r), clamp8(g), clamp8(b))
}

/// Convierte una `Dav1dPicture` (YUV planar) a RGBA8 en `out`. Devuelve
/// las dimensiones, o `None` si la picture está vacía.
///
/// # Safety
/// `pic` debe ser una picture válida recién obtenida de `get_picture`
/// (planos y strides coherentes), aún no liberada.
unsafe fn convert_yuv_to_rgba(pic: &Dav1dPicture, out: &mut Vec<u8>) -> Option<(u32, u32)> {
    let w = pic.p.w as usize;
    let h = pic.p.h as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let shift = (pic.p.bpc as u32).saturating_sub(8);
    let mono = pic.p.layout == DAV1D_PIXEL_LAYOUT_I400;
    // Subsampling de croma por layout.
    let (ss_x, ss_y) = if pic.p.layout == DAV1D_PIXEL_LAYOUT_I444 {
        (0usize, 0usize)
    } else if pic.p.layout == DAV1D_PIXEL_LAYOUT_I422 {
        (1, 0)
    } else {
        (1, 1) // I420 (y fallback)
    };

    let y_ptr = pic.data[0]?.as_ptr() as *const u8;
    let y_stride = pic.stride[0] as usize;
    let (u_ptr, v_ptr, c_stride) = if mono {
        (std::ptr::null(), std::ptr::null(), 0usize)
    } else {
        (
            pic.data[1]?.as_ptr() as *const u8,
            pic.data[2]?.as_ptr() as *const u8,
            pic.stride[1] as usize,
        )
    };

    out.resize(w * h * 4, 0);
    for yy in 0..h {
        for xx in 0..w {
            let y = unsafe { sample(y_ptr, y_stride, xx, yy, shift) } as i32;
            let (u, v) = if mono {
                (128, 128)
            } else {
                let cx = xx >> ss_x;
                let cy = yy >> ss_y;
                (
                    unsafe { sample(u_ptr, c_stride, cx, cy, shift) } as i32,
                    unsafe { sample(v_ptr, c_stride, cx, cy, shift) } as i32,
                )
            };
            let (r, g, b) = yuv_to_rgb(y, u, v);
            let i = (yy * w + xx) * 4;
            out[i] = r;
            out[i + 1] = g;
            out[i + 2] = b;
            out[i + 3] = 255;
        }
    }
    Some((w as u32, h as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuv_white_and_black() {
        // Y=235,U=V=128 (blanco BT.601 limited) ≈ casi blanco.
        let (r, g, b) = yuv_to_rgb(235, 128, 128);
        assert!(r > 230 && g > 230 && b > 230);
        // Y=16 ≈ negro.
        let (r, g, b) = yuv_to_rgb(16, 128, 128);
        assert!(r < 30 && g < 30 && b < 30);
    }

    #[test]
    fn decodes_real_fixture() {
        // El path al fixture: copiamos a un temp porque Av1VideoSource
        // abre por ruta (reabre en seek).
        let bytes = include_bytes!("../tests/fixtures/testsrc_64x48.ivf");
        let dir = std::env::temp_dir();
        let path = dir.join("media_av1_test_fixture.ivf");
        std::fs::write(&path, bytes).unwrap();

        let mut src = Av1VideoSource::open(&path).unwrap();
        assert_eq!(src.dimensions(), (64, 48));

        // Pedir un frame: con dt grande pasamos el gate de fps.
        let mut buf = Vec::new();
        let dims = src.tick(Duration::from_secs(1), &mut buf);
        assert_eq!(dims, Some((64, 48)), "el primer frame debería decodificar");
        assert_eq!(buf.len(), 64 * 48 * 4);
        // testsrc es una imagen con barras de color → no es todo negro
        // ni todo el mismo valor.
        let alpha_ok = buf.chunks_exact(4).all(|p| p[3] == 255);
        assert!(alpha_ok, "alpha debe ser 255");
        let distinct = buf.chunks_exact(4).map(|p| (p[0], p[1], p[2])).collect::<std::collections::HashSet<_>>();
        assert!(distinct.len() > 4, "esperaba variedad de color, hubo {}", distinct.len());

        let _ = std::fs::remove_file(&path);
    }
}
