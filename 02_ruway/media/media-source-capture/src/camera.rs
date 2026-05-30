//! Backend de cámara v4l2 (Linux). Abre `/dev/videoN`, negocia un
//! formato y corre un hilo dedicado que convierte cada frame a RGBA y
//! lo empuja al [`LiveSink`]. El consumidor lee por el [`LiveSource`]
//! embebido sin bloquear su bucle.
//!
//! `v4l` es puro-Rust (ioctl vía libc); no enlaza ninguna lib C
//! externa. Compila donde haya cabeceras `videodev2`; correr necesita
//! un dispositivo real (igual que `media-audio-cpal` necesita un sink
//! de sonido). Por eso esta capa es fina y la lógica testeable
//! (conversión + slot latest-frame) vive fuera, en `convert` y `lib`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use media_core::FrameSource;
use v4l::buffer::Type;
use v4l::io::mmap::Stream as MmapStream;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;
use v4l::{Device, Format, FourCC};

use crate::{live_channel, LiveSink, LiveSource, PixelFormat};

/// Qué cámara abrir y con qué formato pedirle. El driver puede
/// devolver dimensiones/formato distintos a los pedidos — la cámara
/// reporta lo realmente negociado en [`CameraSource::width`] etc.
#[derive(Debug, Clone)]
pub struct CameraOptions {
    /// Índice del dispositivo: `0` → `/dev/video0`.
    pub index: usize,
    pub width: u32,
    pub height: u32,
    /// FourCC pedido. Default `YUYV` (universal en webcams). `MJPG`
    /// rinde mejor a resoluciones altas.
    pub fourcc: [u8; 4],
}

impl Default for CameraOptions {
    fn default() -> Self {
        Self {
            index: 0,
            width: 640,
            height: 480,
            fourcc: *b"YUYV",
        }
    }
}

/// Lo que pudo salir mal al abrir la cámara.
#[derive(Debug)]
pub enum CaptureError {
    /// No se pudo lanzar el hilo de captura.
    Spawn(std::io::Error),
    /// No se pudo abrir `/dev/videoN`.
    Open(std::io::Error),
    /// El driver rechazó el formato pedido.
    SetFormat(std::io::Error),
    /// No se pudo iniciar el stream mmap.
    Stream(std::io::Error),
    /// El driver entregó un FourCC que no sabemos convertir a RGBA.
    UnsupportedFormat([u8; 4]),
    /// El hilo de captura murió antes de reportar estado.
    Closed,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "no se pudo lanzar el hilo de captura: {e}"),
            Self::Open(e) => write!(f, "no se pudo abrir la cámara: {e}"),
            Self::SetFormat(e) => write!(f, "el driver rechazó el formato: {e}"),
            Self::Stream(e) => write!(f, "no se pudo iniciar el stream: {e}"),
            Self::UnsupportedFormat(cc) => {
                write!(f, "formato no soportado: {}", String::from_utf8_lossy(cc))
            }
            Self::Closed => write!(f, "el hilo de captura murió al iniciar"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Formato realmente negociado con el dispositivo.
#[derive(Debug, Clone, Copy)]
struct CameraInfo {
    width: u32,
    height: u32,
    format: PixelFormat,
}

/// Cámara en vivo como [`FrameSource`]. Posee el hilo de captura, que
/// se detiene y se junta al dropearse.
pub struct CameraSource {
    source: LiveSource,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    info: CameraInfo,
}

impl CameraSource {
    /// Abre la cámara y arranca el hilo de captura. Bloquea hasta que
    /// el hilo negoció el formato (o falló) — así el error de "no hay
    /// cámara" / "formato inválido" llega sincrónico al caller, no en
    /// silencio a media reproducción.
    pub fn open(opts: CameraOptions) -> Result<Self, CaptureError> {
        let (sink, source) = live_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let (tx, rx) = mpsc::channel::<Result<CameraInfo, CaptureError>>();

        let join = thread::Builder::new()
            .name("media-capture-camera".into())
            .spawn(move || camera_loop(opts, sink, stop_thread, tx))
            .map_err(CaptureError::Spawn)?;

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
                Err(CaptureError::Closed)
            }
        }
    }

    /// Atajo: cámara default (`/dev/video0`, 640×480, YUYV).
    pub fn open_default() -> Result<Self, CaptureError> {
        Self::open(CameraOptions::default())
    }

    /// Ancho realmente negociado (puede diferir del pedido).
    pub fn width(&self) -> u32 {
        self.info.width
    }
    /// Alto realmente negociado.
    pub fn height(&self) -> u32 {
        self.info.height
    }
    /// Pixel-format realmente negociado.
    pub fn format(&self) -> PixelFormat {
        self.info.format
    }
}

impl FrameSource for CameraSource {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        self.source.tick(dt, buf)
    }
}

impl Drop for CameraSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Cuerpo del hilo: negocia, reporta estado por `tx` una sola vez, y
/// luego bombea frames al `sink` hasta que se pida parar o el stream
/// muera.
fn camera_loop(
    opts: CameraOptions,
    sink: LiveSink,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<Result<CameraInfo, CaptureError>>,
) {
    let dev = match Device::new(opts.index) {
        Ok(d) => d,
        Err(e) => {
            let _ = tx.send(Err(CaptureError::Open(e)));
            return;
        }
    };

    let want = Format::new(opts.width, opts.height, FourCC::new(&opts.fourcc));
    let got = match Capture::set_format(&dev, &want) {
        Ok(f) => f,
        Err(e) => {
            let _ = tx.send(Err(CaptureError::SetFormat(e)));
            return;
        }
    };

    let Some(format) = PixelFormat::from_fourcc(got.fourcc.repr) else {
        let _ = tx.send(Err(CaptureError::UnsupportedFormat(got.fourcc.repr)));
        return;
    };
    let (width, height) = (got.width, got.height);

    let mut stream = match MmapStream::with_buffers(&dev, Type::VideoCapture, 4) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(Err(CaptureError::Stream(e)));
            return;
        }
    };

    // A partir de acá la apertura fue exitosa: el caller puede seguir.
    if tx
        .send(Ok(CameraInfo {
            width,
            height,
            format,
        }))
        .is_err()
    {
        return; // el caller ya se fue.
    }
    drop(tx);

    while !stop.load(Ordering::Acquire) {
        if sink.is_orphan() {
            break; // nadie consume: parar antes que quemar CPU.
        }
        match stream.next() {
            Ok((buf, _meta)) => {
                sink.push_raw(format, width, height, buf);
            }
            Err(_) => break, // dispositivo desconectado / error fatal.
        }
    }
}
