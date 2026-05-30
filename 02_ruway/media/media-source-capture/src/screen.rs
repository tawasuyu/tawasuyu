//! Backend de **captura de pantalla** X11. Calca a [`crate::CameraSource`]:
//! corre un hilo dedicado que lee el framebuffer del servidor, lo
//! convierte a RGBA y lo empuja al [`LiveSink`]; el consumidor lee por
//! el [`LiveSource`] embebido sin bloquear su bucle de render.
//!
//! Reusa el mismo núcleo `LiveSource`/`LiveSink` que la cámara — esa
//! era la promesa del crate: "cámara hoy, captura de pantalla mañana
//! sin crate nuevo". La diferencia es la **fuente** (X11 `GetImage` del
//! root window en vez de v4l2) y el **pacing** (la pantalla no marca
//! ritmo, así que un timer interno limita a `fps`).
//!
//! `x11rb` es **puro-Rust** (habla el protocolo X11 por socket, sin
//! enlazar `libX11`/`libxcb` C) — mismo criterio que `v4l` en la
//! cámara: backend de hardware/sistema detrás de feature opt-in, núcleo
//! puro y testeable afuera. X11 sólo por ahora; Wayland (portal +
//! PipeWire) es otro backend futuro sin tocar este núcleo.
//!
//! `GetImage` copia el framebuffer por el socket cada frame — suficiente
//! para un MVP; la extensión MIT-SHM (memoria compartida, cero copia por
//! el socket) es la optimización natural cuando duela, igual que el hook
//! cero-copia `llimphi-surface` del lado de salida.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use media_core::FrameSource;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, ImageOrder};

use crate::{live_channel, LiveSink, LiveSource, PixelFormat};

/// Qué capturar y a qué ritmo. Por default: pantalla completa del
/// display por `$DISPLAY`, 30 fps.
#[derive(Debug, Clone)]
pub struct ScreenOptions {
    /// Display X11 a abrir. `None` → usa `$DISPLAY`.
    pub display: Option<String>,
    /// Región a capturar `(x, y, width, height)` relativa al root.
    /// `None` → pantalla completa (geometría del root).
    pub region: Option<(i16, i16, u16, u16)>,
    /// Frames por segundo objetivo. El hilo duerme entre capturas para
    /// no quemar CPU re-grabando un framebuffer que no cambió.
    pub fps: u32,
}

impl Default for ScreenOptions {
    fn default() -> Self {
        Self {
            display: None,
            region: None,
            fps: 30,
        }
    }
}

/// Lo que pudo salir mal al abrir la captura de pantalla.
#[derive(Debug)]
pub enum ScreenError {
    /// No se pudo lanzar el hilo de captura.
    Spawn(std::io::Error),
    /// No se pudo conectar al servidor X (`$DISPLAY` ausente/inválido).
    Connect(String),
    /// La región pedida cae fuera del root window.
    RegionFueraDeRango,
    /// El servidor devolvió una profundidad de color que no sabemos
    /// convertir (esperamos 24/32 bpp empaquetado a 4 bytes).
    UnsupportedDepth(u8),
    /// El hilo de captura murió antes de reportar estado.
    Closed,
}

impl std::fmt::Display for ScreenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "no se pudo lanzar el hilo de captura: {e}"),
            Self::Connect(e) => write!(f, "no se pudo conectar al servidor X: {e}"),
            Self::RegionFueraDeRango => write!(f, "la región pedida cae fuera de la pantalla"),
            Self::UnsupportedDepth(d) => write!(f, "profundidad de color no soportada: {d} bpp"),
            Self::Closed => write!(f, "el hilo de captura murió al iniciar"),
        }
    }
}

impl std::error::Error for ScreenError {}

/// Geometría + formato realmente negociados con el servidor.
#[derive(Debug, Clone, Copy)]
struct ScreenInfo {
    width: u32,
    height: u32,
    format: PixelFormat,
}

/// Captura de pantalla en vivo como [`FrameSource`]. Posee el hilo de
/// captura, que se detiene y se junta al dropearse.
pub struct ScreenSource {
    source: LiveSource,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    info: ScreenInfo,
}

impl ScreenSource {
    /// Conecta al servidor X y arranca el hilo de captura. Bloquea
    /// hasta que el hilo negoció geometría y formato (o falló) — así el
    /// error de "no hay display" / "región inválida" llega sincrónico
    /// al caller, igual que [`crate::CameraSource::open`].
    pub fn open(opts: ScreenOptions) -> Result<Self, ScreenError> {
        let (sink, source) = live_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let (tx, rx) = mpsc::channel::<Result<ScreenInfo, ScreenError>>();

        let join = thread::Builder::new()
            .name("media-capture-screen".into())
            .spawn(move || screen_loop(opts, sink, stop_thread, tx))
            .map_err(ScreenError::Spawn)?;

        match rx.recv() {
            Ok(Ok(info)) => Ok(Self {
                source,
                stop,
                join: Some(join),
                info,
            }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => {
                let _ = join.join();
                Err(ScreenError::Closed)
            }
        }
    }

    /// Atajo: pantalla completa del display por `$DISPLAY`, 30 fps.
    pub fn open_default() -> Result<Self, ScreenError> {
        Self::open(ScreenOptions::default())
    }

    /// Ancho realmente capturado.
    pub fn width(&self) -> u32 {
        self.info.width
    }
    /// Alto realmente capturado.
    pub fn height(&self) -> u32 {
        self.info.height
    }
    /// Pixel-format que entrega el servidor (`Bgrx32`/`Xrgb32` según
    /// byte-order).
    pub fn format(&self) -> PixelFormat {
        self.info.format
    }
}

impl FrameSource for ScreenSource {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        self.source.tick(dt, buf)
    }
}

impl Drop for ScreenSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Cuerpo del hilo: conecta, resuelve geometría/formato, reporta estado
/// por `tx` una sola vez, y luego captura frames al ritmo de `fps`
/// hasta que se pida parar o el consumidor se vaya.
fn screen_loop(
    opts: ScreenOptions,
    sink: LiveSink,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<Result<ScreenInfo, ScreenError>>,
) {
    let display = opts.display.as_deref();
    let (conn, screen_num) = match x11rb::connect(display) {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Err(ScreenError::Connect(e.to_string())));
            return;
        }
    };

    let setup = conn.setup();
    let byte_order = setup.image_byte_order;
    let screen = &setup.roots[screen_num];
    let root = screen.root;
    let root_w = screen.width_in_pixels;
    let root_h = screen.height_in_pixels;

    // Región: la pedida o el root completo.
    let (x, y, w, h) = match opts.region {
        Some((x, y, w, h)) => (x, y, w, h),
        None => (0, 0, root_w, root_h),
    };
    // Validar que la región cabe dentro del root.
    if w == 0
        || h == 0
        || x < 0
        || y < 0
        || (x as i32 + w as i32) > root_w as i32
        || (y as i32 + h as i32) > root_h as i32
    {
        let _ = tx.send(Err(ScreenError::RegionFueraDeRango));
        return;
    }

    // X11 `GetImage` con ZPixmap entrega 4 bytes por píxel a 24/32 bpp;
    // el orden de canales lo decide el byte-order del servidor.
    let format = match byte_order {
        ImageOrder::LSB_FIRST => PixelFormat::Bgrx32,
        ImageOrder::MSB_FIRST => PixelFormat::Xrgb32,
        _ => PixelFormat::Bgrx32, // X11 sólo define estos dos; default sano.
    };

    // Sondeo inicial: un GetImage para validar profundidad antes de
    // declarar éxito (igual que la cámara negocia formato al abrir).
    match grab(&conn, root, x, y, w, h) {
        Ok((depth, _data)) if depth == 24 || depth == 32 => {}
        Ok((depth, _)) => {
            let _ = tx.send(Err(ScreenError::UnsupportedDepth(depth)));
            return;
        }
        Err(e) => {
            let _ = tx.send(Err(ScreenError::Connect(e)));
            return;
        }
    }

    if tx
        .send(Ok(ScreenInfo {
            width: w as u32,
            height: h as u32,
            format,
        }))
        .is_err()
    {
        return; // el caller ya se fue.
    }
    drop(tx);

    let frame_interval = if opts.fps == 0 {
        Duration::from_millis(33)
    } else {
        Duration::from_micros(1_000_000 / opts.fps as u64)
    };

    while !stop.load(Ordering::Acquire) {
        if sink.is_orphan() {
            break; // nadie consume: parar antes que quemar CPU.
        }
        let start = Instant::now();
        match grab(&conn, root, x, y, w, h) {
            Ok((_depth, data)) => {
                sink.push_raw(format, w as u32, h as u32, &data);
            }
            Err(_) => break, // servidor caído / error fatal.
        }
        // Pacing: dormir lo que falte para el siguiente frame.
        if let Some(rem) = frame_interval.checked_sub(start.elapsed()) {
            thread::sleep(rem);
        }
    }
}

/// Un `GetImage` del root: devuelve `(depth, data ZPixmap)`. El
/// `plane_mask` `!0` pide todos los planos de bits.
fn grab<C: Connection>(
    conn: &C,
    root: x11rb::protocol::xproto::Window,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
) -> Result<(u8, Vec<u8>), String> {
    let cookie = conn
        .get_image(ImageFormat::Z_PIXMAP, root, x, y, w, h, !0u32)
        .map_err(|e| e.to_string())?;
    let reply = cookie.reply().map_err(|e| e.to_string())?;
    Ok((reply.depth, reply.data))
}
