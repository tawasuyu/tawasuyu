//! Capa DRM **best-effort** del splash. Espeja la filosofía de
//! `arje-loader::gop`: ante cualquier problema (sin nodo DRM, sin conector
//! conectado, sin dumb buffer) loguea y vuelve sin pintar — el arranque sigue
//! igual. Sólo corre el camino feliz cuando el hardware coopera.
//!
//! ## Principio: reusar el modo vigente, nunca re-modeset
//!
//! El parpadeo nace de cambiar el modo del CRTC. Acá leemos el modo que ya
//! dejó `efifb`/`simpledrm` (heredado del GOP del loader) con `get_crtc().mode()`
//! y presentamos sobre **ese mismo modo**. El `set_crtc` inicial reusa ese modo
//! (mismo timing → sin flash) y la animación avanza por `page_flip`, que sólo
//! intercambia el buffer de scanout en el vblank, sin tocar el modo. Si el
//! driver no soporta page-flip (algunos `simpledrm`), caemos a `set_crtc` con el
//! mismo modo — tampoco hay re-modeset porque el timing no cambia.
//!
//! Ver `SDD-ARRANQUE-SIN-PARPADEO.md` §«un solo modo, un solo framebuffer».

use std::os::unix::io::{AsFd, BorrowedFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use drm::buffer::{Buffer, DrmFourcc};
use drm::control::{connector, crtc, framebuffer, Device as ControlDevice, Mode, PageFlipFlags};
use drm::control::dumbbuffer::DumbBuffer;
use drm::Device as DrmDevice;

use crate::handoff::{self, Handoff};
use crate::render;

/// Bandera de parada compartida con el handler de señales (SIGTERM/SIGINT).
/// arje-zero manda SIGTERM cuando es hora de soltar la pantalla; en Fase 2 lo
/// hará al recibir el `READY` de mirada por el socket de handoff.
pub static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// Instala los handlers de señal que ordenan soltar la pantalla.
pub fn install_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
    }
}

/// Parámetros del splash.
pub struct Opts {
    /// Nodo DRM a abrir (p. ej. `/dev/dri/card0`).
    pub device: String,
    /// Tope de duración en ms tras el cual el splash suelta la pantalla solo
    /// (red de seguridad para Fase 1, sin socket de handoff todavía). `0` =
    /// sólo termina por señal.
    pub max_ms: u64,
    /// Frames por segundo objetivo de la animación.
    pub fps: u64,
}

/// Wrapper mínimo del nodo DRM (lo que pide el crate `drm`: `AsFd` + traits).
struct Card(std::fs::File);
impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl DrmDevice for Card {}
impl ControlDevice for Card {}

macro_rules! log {
    ($($a:tt)*) => { eprintln!("[arje-splash] {}", format_args!($($a)*)) };
}

/// Corre el splash. Best-effort: cualquier error se loguea y la función vuelve.
pub fn run(opts: &Opts) {
    if let Err(e) = try_run(opts) {
        log!("sin splash gráfico ({e}); el arranque continúa");
    }
}

/// Un buffer de la cadena de double-buffering: dumb buffer + su framebuffer.
struct Surface {
    db: DumbBuffer,
    fb: framebuffer::Handle,
}

fn try_run(opts: &Opts) -> Result<(), String> {
    let card = open(&opts.device)?;

    // Conector conectado + su CRTC y modo vigentes (los que dejó efifb).
    let res = card
        .resource_handles()
        .map_err(|e| format!("resource_handles: {e}"))?;
    let con = res
        .connectors()
        .iter()
        .filter_map(|h| card.get_connector(*h, true).ok())
        .find(|c| c.state() == connector::State::Connected)
        .ok_or("ningún conector conectado")?;

    let crtc_handle = current_crtc(&card, &con, &res).ok_or("sin CRTC para el conector")?;
    let mode = present_mode(&card, crtc_handle, &con).ok_or("sin modo presentable")?;
    let (w, h) = mode.size();
    let (w, h) = (w as usize, h as usize);
    log!(
        "conector {:?} crtc {:?} modo {}x{} — reusando modo vigente",
        con.handle(),
        crtc_handle,
        w,
        h
    );

    // Dos superficies para alternar (double buffer).
    let mut a = make_surface(&card, w as u32, h as u32)?;
    let mut b = make_surface(&card, w as u32, h as u32)?;
    let con_handle = con.handle();

    // Socket de handoff (Fase 2). Best-effort: si no bindea, sólo tope de tiempo.
    let mut handoff = Handoff::bind(&handoff::sock_path());
    if handoff.active() {
        log!("handoff escuchando en {}", handoff::sock_path().display());
    } else {
        log!("sin socket de handoff — sólo tope de tiempo (Fase 1)");
    }

    let start = Instant::now();
    // Primer present: set_crtc reusando el modo → mismo timing, sin flash.
    paint_into(&card, &mut a, w, h, 0, 0.0)?;
    card.set_crtc(crtc_handle, Some(a.fb), (0, 0), &[con_handle], Some(mode))
        .map_err(|e| format!("set_crtc inicial: {e}"))?;

    let frame_dt = Duration::from_millis((1000 / opts.fps.max(1)).max(1));
    let deadline = (opts.max_ms > 0).then(|| start + Duration::from_millis(opts.max_ms));
    // ¿El driver soporta page-flip? Lo descubrimos en el primer intento y
    // recordamos para no reintentar un ioctl que ya sabemos que no está.
    let mut can_flip = true;
    let mut do_handoff = false;

    loop {
        if STOP.load(Ordering::SeqCst) {
            log!("señal de parada — soltando la pantalla");
            break;
        }
        if let Some(d) = deadline {
            if Instant::now() >= d {
                log!("tope de {} ms alcanzado — soltando la pantalla", opts.max_ms);
                break;
            }
        }
        if handoff.poll_ready() {
            log!("READY de mirada — fade-out + handoff");
            do_handoff = true;
            break;
        }

        let t = start.elapsed().as_millis() as u64;
        present_one(&card, &mut b, crtc_handle, con_handle, mode, w, h, t, 0.0,
                    &mut can_flip, frame_dt)?;
        std::mem::swap(&mut a, &mut b);
    }

    // Fade-out del handoff: fundimos el contenido al fondo de marca `BG` (no a
    // negro) en ~FADE_MS. Al terminar la pantalla queda en el mismo `bg_app`
    // que mirada va a mostrar, así el traspaso de master no se nota.
    if do_handoff {
        const FADE_MS: u64 = 400;
        let fade_start = Instant::now();
        loop {
            let e = fade_start.elapsed().as_millis() as u64;
            if e >= FADE_MS {
                break;
            }
            let f = e as f32 / FADE_MS as f32;
            let t = start.elapsed().as_millis() as u64;
            present_one(&card, &mut b, crtc_handle, con_handle, mode, w, h, t, f,
                        &mut can_flip, frame_dt)?;
            std::mem::swap(&mut a, &mut b);
        }
        // Frame final: BG sólido garantizado.
        let t = start.elapsed().as_millis() as u64;
        present_one(&card, &mut b, crtc_handle, con_handle, mode, w, h, t, 1.0,
                    &mut can_flip, frame_dt)?;
    }

    // Salida: NO destruimos el framebuffer en scanout ni reponemos el modo —
    // dejamos el último frame en pantalla con el mismo timing. Al cerrar el fd
    // soltamos el DRM master; mirada tomará master y presentará su frame ya
    // compuesto sobre el mismo modo (el crossfade percibido de Fase 2).
    drop(a);
    drop(b);
    drop(card); // ← suelta el DRM master

    // Recién con el master ya soltado le avisamos a mirada que puede tomarlo.
    if do_handoff {
        handoff.send_released();
        log!("RELEASED enviado — mirada toma la pantalla");
    }
    Ok(())
}

/// Pinta `surf` para el instante `t` con `fade` y lo presenta (page-flip si el
/// driver lo soporta; si no, `set_crtc` con el mismo modo — nunca re-modeset).
/// Marca el ritmo del frame (`frame_dt`).
#[allow(clippy::too_many_arguments)]
fn present_one(
    card: &Card,
    surf: &mut Surface,
    crtc_handle: crtc::Handle,
    con_handle: connector::Handle,
    mode: Mode,
    w: usize,
    h: usize,
    t: u64,
    fade: f32,
    can_flip: &mut bool,
    frame_dt: Duration,
) -> Result<(), String> {
    let frame_start = Instant::now();
    paint_into(card, surf, w, h, t, fade)?;
    let presented = if *can_flip {
        match card.page_flip(crtc_handle, surf.fb, PageFlipFlags::EVENT, None) {
            Ok(()) => {
                wait_flip(card, frame_dt);
                true
            }
            Err(e) => {
                log!("page_flip no disponible ({e}); caigo a set_crtc (mismo modo)");
                *can_flip = false;
                false
            }
        }
    } else {
        false
    };
    if !presented {
        // Fallback: set_crtc con el MISMO modo (no re-modeset, sólo cambia el
        // puntero de scanout). Pacing por sleep.
        card.set_crtc(crtc_handle, Some(surf.fb), (0, 0), &[con_handle], Some(mode))
            .map_err(|e| format!("set_crtc frame: {e}"))?;
        if let Some(rem) = frame_dt.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(rem);
        }
    }
    Ok(())
}

fn open(path: &str) -> Result<Card, String> {
    if !Path::new(path).exists() {
        return Err(format!("{path} no existe"));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("open {path}: {e}"))?;
    Ok(Card(file))
}

/// CRTC vigente del conector: por su encoder actual; si no, el primero de la
/// lista de recursos (degradación).
fn current_crtc(
    card: &Card,
    con: &connector::Info,
    res: &drm::control::ResourceHandles,
) -> Option<crtc::Handle> {
    con.current_encoder()
        .and_then(|enc| card.get_encoder(enc).ok())
        .and_then(|enc| enc.crtc())
        .or_else(|| res.crtcs().first().copied())
}

/// Modo a presentar: el que YA tiene puesto el CRTC (el de efifb/simpledrm,
/// heredado del GOP) — reusarlo es lo que evita el parpadeo. Si el CRTC no
/// tiene modo (raro), caemos al preferido del conector y, en última instancia,
/// al primero.
fn present_mode(card: &Card, crtc_handle: crtc::Handle, con: &connector::Info) -> Option<Mode> {
    if let Ok(info) = card.get_crtc(crtc_handle) {
        if let Some(m) = info.mode() {
            return Some(m);
        }
    }
    use drm::control::ModeTypeFlags;
    con.modes()
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or_else(|| con.modes().first())
        .copied()
}

fn make_surface(card: &Card, w: u32, h: u32) -> Result<Surface, String> {
    let db = card
        .create_dumb_buffer((w, h), DrmFourcc::Xrgb8888, 32)
        .map_err(|e| format!("create_dumb_buffer {w}x{h}: {e}"))?;
    let fb = card
        .add_framebuffer(&db, 24, 32)
        .map_err(|e| format!("add_framebuffer: {e}"))?;
    Ok(Surface { db, fb })
}

/// Mapea el dumb buffer de la superficie y pinta el frame `t` (con `fade`) en él.
fn paint_into(card: &Card, s: &mut Surface, w: usize, h: usize, t: u64, fade: f32) -> Result<(), String> {
    let pitch = s.db.pitch() as usize;
    let mut map = card
        .map_dumb_buffer(&mut s.db)
        .map_err(|e| format!("map_dumb_buffer: {e}"))?;
    render::paint_frame(map.as_mut(), w, h, pitch, t, fade);
    Ok(())
}

/// Espera el evento de page-flip (o un timeout) para sincronizar con el vblank.
fn wait_flip(card: &Card, timeout: Duration) {
    use std::os::unix::io::AsRawFd;
    let raw = card.0.as_raw_fd();
    let mut pfd = libc::pollfd {
        fd: raw,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // poll con el deadline del frame; si llega el evento, drenamos la cola.
    let r = unsafe { libc::poll(&mut pfd, 1, ms.max(1)) };
    if r > 0 {
        if let Ok(events) = card.receive_events() {
            for _ in events {} // drenar; sólo nos importa que el flip terminó
        }
    }
}
