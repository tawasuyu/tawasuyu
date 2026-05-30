//! Backend de captura de pantalla **Wayland** vía el protocolo
//! `wlr-screencopy` (`zwlr_screencopy_manager_v1`). Calca a
//! [`crate::ScreenSource`] (X11): un hilo dedicado copia el output a un
//! buffer shm, convierte a RGBA y empuja al [`LiveSink`]; el consumidor
//! lee por el [`LiveSource`] embebido sin bloquear.
//!
//! Puro-Rust: `wayland-client` + `wayland-protocols-wlr` hablan el
//! protocolo por socket; con la feature `dlopen` ni siquiera enlazan
//! `libwayland` en build (se carga en runtime). Mantiene el ethos del
//! crate igual que `x11rb` en X11.
//!
//! **Alcance**: `wlr-screencopy` lo implementan los compositores
//! **wlroots** (Sway, Hyprland, river, ...). GNOME y KDE NO lo exponen —
//! ahí la vía es xdg-desktop-portal + PipeWire (otro backend, que sí
//! arrastraría libpipewire en C). Wayland prohíbe por diseño que un
//! cliente lea la pantalla sin un protocolo sancionado; este es el
//! camino directo donde existe.

use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use media_core::FrameSource;
use memmap2::MmapMut;
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer::WlBuffer,
    wl_output::WlOutput,
    wl_registry::{self, WlRegistry},
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use crate::{live_channel, LiveSink, LiveSource, PixelFormat};

/// Qué output capturar y a qué ritmo.
#[derive(Debug, Clone)]
pub struct WaylandScreenOptions {
    /// Índice del `wl_output` (monitor): `0` = el primero anunciado.
    pub output: usize,
    /// Incluir el cursor en la captura.
    pub overlay_cursor: bool,
    /// Frames por segundo objetivo (un timer interno marca el ritmo;
    /// wlr-screencopy entrega un frame por petición, no un stream).
    pub fps: u32,
}

impl Default for WaylandScreenOptions {
    fn default() -> Self {
        Self {
            output: 0,
            overlay_cursor: true,
            fps: 30,
        }
    }
}

/// Lo que pudo salir mal al abrir la captura Wayland.
#[derive(Debug)]
pub enum WaylandError {
    Spawn(std::io::Error),
    /// No se pudo conectar al compositor (`$WAYLAND_DISPLAY`).
    Connect(String),
    /// El compositor no expone `zwlr_screencopy_manager_v1` (¿GNOME/KDE?).
    NoScreencopy,
    /// No hay `wl_shm` (improbable, pero lo verificamos).
    NoShm,
    /// El índice de output pedido no existe.
    NoOutput,
    /// El compositor entregó un formato shm que no sabemos convertir.
    UnsupportedFormat(u32),
    /// Falló la copia del frame (memfd/mmap o el propio `copy`).
    Capture(String),
    /// El hilo de captura murió antes de reportar estado.
    Closed,
}

impl std::fmt::Display for WaylandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "no se pudo lanzar el hilo de captura: {e}"),
            Self::Connect(e) => write!(f, "no se pudo conectar al compositor Wayland: {e}"),
            Self::NoScreencopy => write!(
                f,
                "el compositor no expone wlr-screencopy (GNOME/KDE necesitan portal+PipeWire)"
            ),
            Self::NoShm => write!(f, "el compositor no expone wl_shm"),
            Self::NoOutput => write!(f, "el output pedido no existe"),
            Self::UnsupportedFormat(c) => write!(f, "formato shm no soportado: {c:#x}"),
            Self::Capture(e) => write!(f, "falló la captura del frame: {e}"),
            Self::Closed => write!(f, "el hilo de captura murió al iniciar"),
        }
    }
}

impl std::error::Error for WaylandError {}

/// Geometría + formato realmente negociados.
#[derive(Debug, Clone, Copy)]
struct WaylandInfo {
    width: u32,
    height: u32,
    format: PixelFormat,
}

/// Captura de pantalla Wayland en vivo como [`FrameSource`]. Posee el
/// hilo de captura, que se detiene y se junta al dropearse.
pub struct WaylandScreenSource {
    source: LiveSource,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    info: WaylandInfo,
}

impl WaylandScreenSource {
    /// Conecta al compositor y arranca el hilo de captura. Bloquea
    /// hasta capturar (y validar) el primer frame, o fallar — el error
    /// llega sincrónico, igual que la cámara/X11.
    pub fn open(opts: WaylandScreenOptions) -> Result<Self, WaylandError> {
        let (sink, source) = live_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let (tx, rx) = mpsc::channel::<Result<WaylandInfo, WaylandError>>();

        let join = thread::Builder::new()
            .name("media-capture-wayland".into())
            .spawn(move || capture_thread(opts, sink, stop_thread, tx))
            .map_err(WaylandError::Spawn)?;

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
                Err(WaylandError::Closed)
            }
        }
    }

    /// Atajo: primer output, con cursor, 30 fps.
    pub fn open_default() -> Result<Self, WaylandError> {
        Self::open(WaylandScreenOptions::default())
    }

    pub fn width(&self) -> u32 {
        self.info.width
    }
    pub fn height(&self) -> u32 {
        self.info.height
    }
    pub fn format(&self) -> PixelFormat {
        self.info.format
    }
}

impl FrameSource for WaylandScreenSource {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        self.source.tick(dt, buf)
    }
}

impl Drop for WaylandScreenSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Especificación del buffer que pide el compositor para una copia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BufferSpec {
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

#[derive(PartialEq, Eq)]
enum FrameStatus {
    Pending,
    Ready,
    Failed,
}

/// Estado que mutan los callbacks de dispatch del frame.
struct CaptureState {
    spec: Option<BufferSpec>,
    status: FrameStatus,
}

// El frame es el único objeto con eventos que nos importan.
impl Dispatch<ZwlrScreencopyFrameV1, ()> for CaptureState {
    fn event(
        state: &mut Self,
        _frame: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::Event;
        match event {
            Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                if let WEnum::Value(fmt) = format {
                    state.spec = Some(BufferSpec {
                        format: fmt,
                        width,
                        height,
                        stride,
                    });
                }
            }
            Event::Ready { .. } => state.status = FrameStatus::Ready,
            Event::Failed => state.status = FrameStatus::Failed,
            // Flags (y-invert), Damage, LinuxDmabuf, BufferDone: ignorados
            // (binamos v1, así que dmabuf/buffer_done no llegan).
            _ => {}
        }
    }
}

// El registry con GlobalListContents lo exige `registry_queue_init`.
impl Dispatch<WlRegistry, GlobalListContents> for CaptureState {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// Objetos sin eventos relevantes: ignorar todo.
delegate_noop!(CaptureState: ignore WlShm);
delegate_noop!(CaptureState: ignore WlShmPool);
delegate_noop!(CaptureState: ignore WlBuffer);
delegate_noop!(CaptureState: ignore WlOutput);
delegate_noop!(CaptureState: ignore ZwlrScreencopyManagerV1);

/// Cuerpo del hilo: conecta, negocia, reporta estado por `tx` al primer
/// frame exitoso (o falla), y luego captura al ritmo de `fps`.
fn capture_thread(
    opts: WaylandScreenOptions,
    sink: LiveSink,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<Result<WaylandInfo, WaylandError>>,
) {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Err(WaylandError::Connect(e.to_string())));
            return;
        }
    };
    let (globals, mut queue) = match registry_queue_init::<CaptureState>(&conn) {
        Ok(x) => x,
        Err(e) => {
            let _ = tx.send(Err(WaylandError::Connect(e.to_string())));
            return;
        }
    };
    let qh = queue.handle();

    let shm: WlShm = match globals.bind(&qh, 1..=1, ()) {
        Ok(s) => s,
        Err(_) => {
            let _ = tx.send(Err(WaylandError::NoShm));
            return;
        }
    };
    let manager: ZwlrScreencopyManagerV1 = match globals.bind(&qh, 1..=1, ()) {
        Ok(m) => m,
        Err(_) => {
            let _ = tx.send(Err(WaylandError::NoScreencopy));
            return;
        }
    };
    let output = match bind_output(&globals, &qh, opts.output) {
        Some(o) => o,
        None => {
            let _ = tx.send(Err(WaylandError::NoOutput));
            return;
        }
    };

    let frame_interval = if opts.fps == 0 {
        Duration::from_millis(33)
    } else {
        Duration::from_micros(1_000_000 / opts.fps as u64)
    };

    let mut state = CaptureState {
        spec: None,
        status: FrameStatus::Pending,
    };
    // Buffer shm reusado entre frames mientras el spec no cambie.
    let mut current: Option<(WlShmPool, WlBuffer, MmapMut, BufferSpec)> = None;
    let mut reported = false;

    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if reported && sink.is_orphan() {
            break;
        }
        let start = Instant::now();

        state.spec = None;
        state.status = FrameStatus::Pending;
        let frame = manager.capture_output(opts.overlay_cursor as i32, &output, &qh, ());

        // Dispatch hasta conocer el spec (evento Buffer) o fallar.
        while state.spec.is_none() && state.status == FrameStatus::Pending {
            if queue.blocking_dispatch(&mut state).is_err() {
                bail(&tx, reported, WaylandError::Closed);
                frame.destroy();
                return;
            }
        }
        if state.status == FrameStatus::Failed {
            frame.destroy();
            if !reported {
                let _ = tx.send(Err(WaylandError::Capture("frame failed".into())));
                return;
            }
            break;
        }
        let spec = state.spec.unwrap();
        let pf = match wl_format_to_pf(spec.format) {
            Some(p) => p,
            None => {
                frame.destroy();
                if !reported {
                    let _ = tx.send(Err(WaylandError::UnsupportedFormat(spec.format as u32)));
                    return;
                }
                break;
            }
        };

        // (Re)crear el buffer shm si cambió el spec.
        if current.as_ref().map(|(_, _, _, s)| *s != spec).unwrap_or(true) {
            match make_shm(&shm, &qh, &spec) {
                Ok((pool, buffer, mmap)) => current = Some((pool, buffer, mmap, spec)),
                Err(e) => {
                    frame.destroy();
                    if !reported {
                        let _ = tx.send(Err(e));
                        return;
                    }
                    break;
                }
            }
        }
        let (_, buffer, mmap, _) = current.as_mut().unwrap();

        // Pedir la copia y esperar Ready/Failed.
        frame.copy(buffer);
        while state.status == FrameStatus::Pending {
            if queue.blocking_dispatch(&mut state).is_err() {
                break;
            }
        }
        frame.destroy();

        if state.status == FrameStatus::Ready {
            if !reported {
                let _ = tx.send(Ok(WaylandInfo {
                    width: spec.width,
                    height: spec.height,
                    format: pf,
                }));
                reported = true;
            }
            push_frame(&sink, pf, &spec, mmap);
        } else if !reported {
            let _ = tx.send(Err(WaylandError::Capture("frame failed".into())));
            return;
        }

        if let Some(rem) = frame_interval.checked_sub(start.elapsed()) {
            thread::sleep(rem);
        }
    }
}

/// Reporta un error por `tx` sólo si todavía no se había reportado éxito.
fn bail(
    tx: &mpsc::Sender<Result<WaylandInfo, WaylandError>>,
    reported: bool,
    e: WaylandError,
) {
    if !reported {
        let _ = tx.send(Err(e));
    }
}

/// Mapea el formato shm de Wayland a nuestro [`PixelFormat`]. Los XRGB
/// little-endian caen como `[B,G,R,X]` (Bgrx32); los XBGR como
/// `[R,G,B,X]` (Rgbx32).
fn wl_format_to_pf(f: wl_shm::Format) -> Option<PixelFormat> {
    match f {
        wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 => Some(PixelFormat::Bgrx32),
        wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888 => Some(PixelFormat::Rgbx32),
        _ => None,
    }
}

/// Crea un buffer shm (memfd + mmap) del tamaño del spec y lo registra
/// en el compositor. Devuelve `(pool, buffer, mmap)`; el `mmap` es
/// nuestra vista de la memoria que el compositor rellenará en `copy`.
fn make_shm(
    shm: &WlShm,
    qh: &QueueHandle<CaptureState>,
    spec: &BufferSpec,
) -> Result<(WlShmPool, WlBuffer, MmapMut), WaylandError> {
    let size = spec.stride as usize * spec.height as usize;
    let fd = rustix::fs::memfd_create("media-capture-wl", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| WaylandError::Capture(format!("memfd: {e}")))?;
    let file = std::fs::File::from(fd);
    file.set_len(size as u64)
        .map_err(|e| WaylandError::Capture(format!("ftruncate: {e}")))?;
    // SAFETY: el archivo es un memfd recién creado de `size` bytes; nadie
    // más lo mapea con un tamaño distinto.
    let mmap = unsafe {
        MmapMut::map_mut(&file).map_err(|e| WaylandError::Capture(format!("mmap: {e}")))?
    };
    let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        spec.width as i32,
        spec.height as i32,
        spec.stride as i32,
        spec.format,
        qh,
        (),
    );
    Ok((pool, buffer, mmap))
}

/// Empuja el frame copiado al sink, repackeando filas si el stride trae
/// padding (la conversión asume filas contiguas `width*4`).
fn push_frame(sink: &LiveSink, pf: PixelFormat, spec: &BufferSpec, mmap: &MmapMut) {
    let row = spec.width as usize * 4;
    let stride = spec.stride as usize;
    if stride == row {
        sink.push_raw(pf, spec.width, spec.height, &mmap[..row * spec.height as usize]);
    } else {
        let mut tight = Vec::with_capacity(row * spec.height as usize);
        for y in 0..spec.height as usize {
            let off = y * stride;
            tight.extend_from_slice(&mmap[off..off + row]);
        }
        sink.push_raw(pf, spec.width, spec.height, &tight);
    }
}

/// Vincula el `wl_output` en el índice pedido (orden de anuncio).
fn bind_output(
    globals: &GlobalList,
    qh: &QueueHandle<CaptureState>,
    index: usize,
) -> Option<WlOutput> {
    let registry = globals.registry();
    globals.contents().with_list(|list| {
        let mut seen = 0;
        for g in list {
            if g.interface == "wl_output" {
                if seen == index {
                    let version = g.version.min(4);
                    return Some(registry.bind::<WlOutput, _, _>(g.name, version, qh, ()));
                }
                seen += 1;
            }
        }
        None
    })
}
