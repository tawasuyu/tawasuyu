//! Backend **nativo**: cliente `zwlr_screencopy_v1` propio (raw `wayland-client`),
//! el mismo protocolo que mirada implementa del lado servidor. Es el camino
//! soberano — no depende de grim. Buffer por `wl_shm` respaldado por un archivo
//! temporal mapeado (`tempfile` + `memmap2`).
//!
//! Estado: compila y sigue el flujo estándar (capture_output → buffer →
//! buffer_done → copy → ready → leer mmap). **Pendiente de verificar en vivo**
//! contra mirada en una máquina con compositor; por eso es opt-in (feature
//! `wayland`) y `--backend auto` cae a grim si algo falla.

use std::fs::File;
use std::os::fd::AsFd;

use hapiy_core::{Capturer, OutputInfo, Shot};
use memmap2::MmapMut;
use wayland_client::protocol::{
    wl_buffer::WlBuffer,
    wl_output::{self, WlOutput},
    wl_registry,
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

#[derive(Default)]
struct OutputAcc {
    name: Option<String>,
    width: i32,
    height: i32,
    /// Posición de la salida en el espacio global del compositor (px lógicos),
    /// del evento `geometry`. Sirve para componer todos los monitores.
    x: i32,
    y: i32,
}

/// Parámetros del buffer shm que anuncia el compositor.
#[derive(Clone, Copy)]
struct BufFmt {
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

#[derive(Default)]
struct App {
    shm: Option<WlShm>,
    manager: Option<ZwlrScreencopyManagerV1>,
    outputs: Vec<(WlOutput, OutputAcc)>,
    // estado de una captura en curso:
    buf_fmt: Option<BufFmt>,
    mapping: Option<(MmapMut, WlBuffer)>,
    y_invert: bool,
    done: Option<Result<(), String>>,
}

/// Conexión + globals listos para capturar.
pub struct WaylandCapturer {
    conn: Connection,
}

impl WaylandCapturer {
    pub fn connect() -> Result<WaylandCapturer, String> {
        let conn = Connection::connect_to_env()
            .map_err(|e| format!("no se pudo conectar al compositor Wayland: {e}"))?;
        Ok(WaylandCapturer { conn })
    }

    /// Arranca un App con registry + roundtrip para tener shm/manager/outputs.
    fn bootstrap(&self) -> Result<(wayland_client::EventQueue<App>, App), String> {
        let mut queue = self.conn.new_event_queue::<App>();
        let qh = queue.handle();
        let display = self.conn.display();
        display.get_registry(&qh, ());
        let mut app = App::default();
        queue
            .roundtrip(&mut app)
            .map_err(|e| format!("roundtrip de registry falló: {e}"))?;
        // Segundo roundtrip: deja llegar los eventos de cada wl_output (mode/name).
        queue
            .roundtrip(&mut app)
            .map_err(|e| format!("roundtrip de outputs falló: {e}"))?;
        if app.shm.is_none() {
            return Err("el compositor no expone wl_shm".into());
        }
        if app.manager.is_none() {
            return Err("el compositor no expone zwlr_screencopy_manager_v1".into());
        }
        Ok((queue, app))
    }
}

impl Capturer for WaylandCapturer {
    fn outputs(&self) -> Result<Vec<OutputInfo>, String> {
        let (_q, app) = self.bootstrap()?;
        Ok(app
            .outputs
            .iter()
            .enumerate()
            .map(|(i, (_, acc))| OutputInfo {
                name: acc.name.clone().unwrap_or_else(|| format!("output-{i}")),
                width: acc.width.max(0) as u32,
                height: acc.height.max(0) as u32,
            })
            .collect())
    }

    fn capture(&self, output: Option<&str>) -> Result<Shot, String> {
        let (mut queue, mut app) = self.bootstrap()?;
        if app.outputs.is_empty() {
            return Err("el compositor no anunció ninguna salida".into());
        }

        match output {
            // Una salida puntual, por nombre.
            Some(want) => {
                let idx = app
                    .outputs
                    .iter()
                    .position(|(_, a)| a.name.as_deref() == Some(want))
                    .ok_or_else(|| format!("no hay una salida llamada «{want}»"))?;
                let wl_out = app.outputs[idx].0.clone();
                capture_one(&mut queue, &mut app, &wl_out)
            }
            // Sin salida pedida: TODO el escritorio — capturar cada monitor y
            // componerlos por su posición global. Es el comportamiento esperado
            // de un "screenshot" multi-monitor (no un monitor arbitrario).
            None => {
                let outs: Vec<(WlOutput, i32, i32)> = app
                    .outputs
                    .iter()
                    .map(|(o, a)| (o.clone(), a.x, a.y))
                    .collect();
                if outs.len() == 1 {
                    return capture_one(&mut queue, &mut app, &outs[0].0);
                }
                let mut tiles: Vec<(i32, i32, Shot)> = Vec::with_capacity(outs.len());
                for (wl_out, x, y) in &outs {
                    let shot = capture_one(&mut queue, &mut app, wl_out)?;
                    tiles.push((*x, *y, shot));
                }
                Ok(compose(tiles))
            }
        }
    }
}

/// Captura **una** salida: resetea el estado del frame, pide `capture_output`,
/// bombea hasta `ready`/`failed`, y arma el [`Shot`] desde el buffer mapeado.
fn capture_one(
    queue: &mut wayland_client::EventQueue<App>,
    app: &mut App,
    wl_out: &WlOutput,
) -> Result<Shot, String> {
    let qh = queue.handle();
    app.buf_fmt = None;
    app.mapping = None;
    app.y_invert = false;
    app.done = None;

    let manager = app.manager.clone().ok_or("sin zwlr_screencopy_manager")?;
    manager.capture_output(0, wl_out, &qh, ());

    while app.done.is_none() {
        queue
            .blocking_dispatch(app)
            .map_err(|e| format!("dispatch durante la captura falló: {e}"))?;
    }
    app.done.take().unwrap()?;

    let fmt = app.buf_fmt.ok_or("el compositor no anunció el formato del buffer")?;
    let (mmap, _buffer) = app.mapping.as_ref().ok_or("no se mapeó el buffer de captura")?;
    let rgba = to_rgba(&mmap[..], fmt, app.y_invert)?;
    Shot::new(fmt.width, fmt.height, rgba)
}

/// Compone varios monitores en un solo [`Shot`] ubicándolos por su posición
/// global `(x, y)`. Asume escala 1 (px lógicos ≈ px físicos); con escalado
/// fraccionario por monitor puede quedar desalineado — caso raro y mejorable.
fn compose(tiles: Vec<(i32, i32, Shot)>) -> Shot {
    let min_x = tiles.iter().map(|(x, _, _)| *x).min().unwrap_or(0);
    let min_y = tiles.iter().map(|(_, y, _)| *y).min().unwrap_or(0);
    let max_x = tiles.iter().map(|(x, _, s)| x + s.width as i32).max().unwrap_or(0);
    let max_y = tiles.iter().map(|(_, y, s)| y + s.height as i32).max().unwrap_or(0);
    let w = (max_x - min_x).max(1) as usize;
    let h = (max_y - min_y).max(1) as usize;

    let mut canvas = vec![0u8; w * h * 4];
    for px in canvas.chunks_exact_mut(4) {
        px[3] = 255; // opaco; los huecos entre monitores quedan negros
    }
    for (x, y, s) in &tiles {
        let ox = (x - min_x).max(0) as usize;
        let oy = (y - min_y).max(0) as usize;
        let sw = s.width as usize;
        for row in 0..s.height as usize {
            let dy = oy + row;
            if dy >= h {
                break;
            }
            let copy_w = sw.min(w.saturating_sub(ox));
            let so = row * sw * 4;
            let do_ = (dy * w + ox) * 4;
            canvas[do_..do_ + copy_w * 4].copy_from_slice(&s.rgba[so..so + copy_w * 4]);
        }
    }
    Shot::new(w as u32, h as u32, canvas).unwrap_or(Shot {
        width: w as u32,
        height: h as u32,
        rgba: vec![0; w * h * 4],
    })
}

/// Convierte el buffer shm crudo (según su formato) a RGBA8 contiguo.
fn to_rgba(src: &[u8], fmt: BufFmt, y_invert: bool) -> Result<Vec<u8>, String> {
    let (w, h, stride) = (fmt.width as usize, fmt.height as usize, fmt.stride as usize);
    if src.len() < stride * h {
        return Err("el buffer de captura es más chico de lo anunciado".into());
    }
    // (swap_rb, has_alpha) por formato. wlroots suele dar Xrgb/Argb (BGRA en
    // memoria, little-endian) o Xbgr/Abgr (RGBA en memoria).
    let (swap_rb, _alpha) = match fmt.format {
        wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => (true, true),
        wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => (false, true),
        other => return Err(format!("formato de buffer no soportado: {other:?}")),
    };
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let sy = if y_invert { h - 1 - y } else { y };
        let srow = &src[sy * stride..sy * stride + w * 4];
        let drow = &mut out[y * w * 4..(y + 1) * w * 4];
        for x in 0..w {
            let s = &srow[x * 4..x * 4 + 4];
            let d = &mut drow[x * 4..x * 4 + 4];
            if swap_rb {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
            } else {
                d[0] = s[0];
                d[1] = s[1];
                d[2] = s[2];
            }
            d[3] = 255; // opaco: el wallpaper/escritorio no se mezcla
        }
    }
    Ok(out)
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_shm" => {
                    state.shm = Some(registry.bind::<WlShm, _, _>(name, version.min(1), qh, ()));
                }
                "zwlr_screencopy_manager_v1" => {
                    state.manager = Some(registry.bind::<ZwlrScreencopyManagerV1, _, _>(
                        name,
                        version.min(3),
                        qh,
                        (),
                    ));
                }
                "wl_output" => {
                    let out = registry.bind::<WlOutput, _, _>(name, version.min(4), qh, ());
                    state.outputs.push((out, OutputAcc::default()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlOutput, ()> for App {
    fn event(
        state: &mut Self,
        proxy: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, acc)) = state.outputs.iter_mut().find(|(o, _)| o == proxy) else {
            return;
        };
        match event {
            wl_output::Event::Geometry { x, y, .. } => {
                acc.x = x;
                acc.y = y;
            }
            wl_output::Event::Mode { flags, width, height, .. } => {
                if let WEnum::Value(f) = flags {
                    if f.contains(wl_output::Mode::Current) {
                        acc.width = width;
                        acc.height = height;
                    }
                }
            }
            wl_output::Event::Name { name } => acc.name = Some(name),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ZwlrScreencopyManagerV1,
        _: <ZwlrScreencopyManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for App {
    fn event(
        state: &mut Self,
        frame: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::Event;
        match event {
            Event::Buffer { format, width, height, stride } => {
                if let WEnum::Value(format) = format {
                    state.buf_fmt = Some(BufFmt { format, width, height, stride });
                }
            }
            Event::Flags { flags } => {
                if let WEnum::Value(f) = flags {
                    state.y_invert = f.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
                }
            }
            Event::BufferDone => {
                // Crear el buffer shm y pedir la copia.
                let Some(fmt) = state.buf_fmt else {
                    state.done = Some(Err("buffer_done sin formato previo".into()));
                    return;
                };
                let Some(shm) = state.shm.clone() else {
                    state.done = Some(Err("sin wl_shm".into()));
                    return;
                };
                match alloc_buffer(&shm, fmt, qh) {
                    Ok((mmap, buffer)) => {
                        frame.copy(&buffer);
                        state.mapping = Some((mmap, buffer));
                    }
                    Err(e) => state.done = Some(Err(e)),
                }
            }
            Event::Ready { .. } => state.done = Some(Ok(())),
            Event::Failed => state.done = Some(Err("el compositor rechazó la captura".into())),
            _ => {}
        }
    }
}

/// Reserva un `wl_shm` pool de `stride*height` respaldado por un archivo temporal
/// mapeado, y crea el `wl_buffer` para que el compositor escriba la captura.
fn alloc_buffer(
    shm: &WlShm,
    fmt: BufFmt,
    qh: &QueueHandle<App>,
) -> Result<(MmapMut, WlBuffer), String> {
    let size = fmt.stride as usize * fmt.height as usize;
    let file: File = tempfile::tempfile().map_err(|e| format!("tempfile: {e}"))?;
    file.set_len(size as u64).map_err(|e| format!("ftruncate: {e}"))?;
    let mmap = unsafe { MmapMut::map_mut(&file).map_err(|e| format!("mmap: {e}"))? };
    let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        fmt.width as i32,
        fmt.height as i32,
        fmt.stride as i32,
        fmt.format,
        qh,
        (),
    );
    Ok((mmap, buffer))
}

impl Dispatch<WlShm, ()> for App {
    fn event(_: &mut Self, _: &WlShm, _: wl_shm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<WlShmPool, ()> for App {
    fn event(_: &mut Self, _: &WlShmPool, _: <WlShmPool as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<WlBuffer, ()> for App {
    fn event(_: &mut Self, _: &WlBuffer, _: <WlBuffer as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
