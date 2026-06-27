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
    /// Config del splash (fuente: builtin/imagen/frames, colores). La escribe
    /// wawa-panel; la lee el binario del initramfs. Ver `config::SplashCfg`.
    pub cfg: crate::config::SplashCfg,
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

/// Greeter **simulado** (`arje-splash --greeter-sim`): hace de cliente del
/// handoff (avisa READY al splash y espera RELEASED), luego toma el DRM
/// reusando el mismo modo y hace **aparecer** la tarjeta de login desde BG.
/// No es el greeter real de mirada (eso necesita GPU/EGL) — es para VER el
/// crossfade end-to-end en QEMU sin GPU. Best-effort.
pub fn run_greeter(opts: &Opts) {
    // Demora opcional antes de pedir la pantalla: deja que el splash se vea un
    // rato antes del crossfade (sólo para la demo). `ARJE_GREETER_DELAY_MS`.
    let delay = std::env::var("ARJE_GREETER_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    if delay > 0 {
        log!("greeter-sim: espero {delay} ms antes del handoff");
        std::thread::sleep(Duration::from_millis(delay));
    }
    // 1 · Handoff como cliente: pedir la pantalla y esperar a que el splash
    // suelte el master. Si no hay splash, seguimos igual (tomamos el DRM ya).
    match handoff::poke(&handoff::sock_path()) {
        Ok(()) => log!("greeter-sim: RELEASED recibido, tomo la pantalla"),
        Err(e) => log!("greeter-sim: sin splash en el socket ({e}); tomo la pantalla directo"),
    }
    if let Err(e) = try_run_greeter(opts) {
        log!("greeter-sim sin gráfico ({e})");
    }
}

fn try_run_greeter(opts: &Opts) -> Result<(), String> {
    let Drm { card, crtc: crtc_handle, con: con_handle, mode, w, h } = setup(&opts.device)?;
    let mut a = make_surface(&card, w as u32, h as u32)?;
    let mut b = make_surface(&card, w as u32, h as u32)?;

    let bg = opts.cfg.bg;
    // Primer frame = BG puro (appear=0): idéntico al frame final del fade-out
    // del splash, así el traspaso de master no introduce ningún salto.
    paint_into(&card, &mut a, w, h, 0, 0.0, Scene::Greeter, bg, None)?;
    card.set_crtc(crtc_handle, Some(a.fb), (0, 0), &[con_handle], Some(mode))
        .map_err(|e| format!("set_crtc inicial: {e}"))?;

    let frame_dt = Duration::from_millis((1000 / opts.fps.max(1)).max(1));
    let mut can_flip = true;

    // Aparición de la tarjeta (~600 ms) y luego sostener hasta el tope.
    const APPEAR_MS: u64 = 600;
    let start = Instant::now();
    loop {
        if STOP.load(Ordering::SeqCst) {
            break;
        }
        let e = start.elapsed().as_millis() as u64;
        if opts.max_ms > 0 && e >= opts.max_ms {
            break;
        }
        let appear = ((e as f32) / APPEAR_MS as f32).min(1.0);
        present_one(&card, &mut b, crtc_handle, con_handle, mode, w, h, e, appear,
                    Scene::Greeter, bg, None, &mut can_flip, frame_dt)?;
        std::mem::swap(&mut a, &mut b);
    }
    Ok(())
}

/// Un buffer de la cadena de double-buffering: dumb buffer + su framebuffer.
struct Surface {
    db: DumbBuffer,
    fb: framebuffer::Handle,
}

/// Qué escena pinta el bucle de present (con la imagen ya resuelta para el
/// cuadro actual, así `paint_into` no necesita el catálogo de frames ni fps).
#[derive(Clone, Copy)]
enum Scene<'a> {
    /// El splash nativo (logo respirando + barra). `blend` = fade-out a BG.
    Splash,
    /// La chakana animada de la marca (default unificado). `blend` = fade-out.
    Chakana,
    /// Una imagen (PNG estático o el cuadro N de una animación). `blend` = fade.
    Image(&'a crate::image::Image),
    /// El greeter simulado (tarjeta de login). `blend` = aparición desde BG.
    Greeter,
}

/// La fuente del splash ya cargada en memoria (decodificada una sola vez).
enum Loaded {
    Chakana,
    Builtin,
    Image(crate::image::Image),
    Frames(Vec<crate::image::Image>),
}

impl Loaded {
    /// Carga la fuente declarada en la config; best-effort (cae a Builtin).
    fn from_cfg(src: &crate::config::Source) -> Self {
        use crate::config::Source;
        match src {
            Source::Chakana => Loaded::Chakana,
            Source::Builtin => Loaded::Builtin,
            Source::Image(p) => match crate::image::load_png(p) {
                Some(img) => {
                    log!("splash: imagen {} ({}x{})", p.display(), img.w, img.h);
                    Loaded::Image(img)
                }
                None => {
                    log!("splash: no pude leer la imagen {} — uso el splash nativo", p.display());
                    Loaded::Builtin
                }
            },
            Source::Frames(dir) => {
                let frames = load_frames(dir);
                if frames.is_empty() {
                    log!("splash: sin frames en {} — uso el splash nativo", dir.display());
                    Loaded::Builtin
                } else {
                    log!("splash: {} frames desde {}", frames.len(), dir.display());
                    Loaded::Frames(frames)
                }
            }
            Source::Lottie(path) => Self::from_baked(
                mirada_fondo::FondoSpec::Lottie { path: path.clone() },
                "Lottie",
                path,
            ),
            Source::Rive(path) => Self::from_baked(
                mirada_fondo::FondoSpec::Rive { path: path.clone() },
                "rive",
                path,
            ),
        }
    }

    /// Carga los frames *bakeados* de un Lottie/rive desde su cache. Como el
    /// splash no tiene GPU al boot, no puede rasterizar vello: si no hay cache
    /// (nadie corrió `fondo-bake`) cae a la **chakana**, no a un cuadro vacío.
    fn from_baked(spec: mirada_fondo::FondoSpec, label: &str, path: &str) -> Self {
        let dir = mirada_fondo::cache::cache_dir(&spec);
        let frames = load_frames(&dir);
        if frames.is_empty() {
            log!("splash: {label} «{path}» sin cache bakeada — uso la chakana");
            Loaded::Chakana
        } else {
            log!("splash: {} frames {label} desde {}", frames.len(), dir.display());
            Loaded::Frames(frames)
        }
    }

    /// Resuelve la escena del cuadro `t` (elige el frame de la animación).
    fn scene_at(&self, t: u64, fps: u64) -> Scene<'_> {
        match self {
            Loaded::Chakana => Scene::Chakana,
            Loaded::Builtin => Scene::Splash,
            Loaded::Image(img) => Scene::Image(img),
            Loaded::Frames(v) => {
                let dt = (1000 / fps.max(1)).max(1);
                let idx = ((t / dt) as usize) % v.len();
                Scene::Image(&v[idx])
            }
        }
    }
}

/// Lee los `*.png` de un directorio en orden alfabético.
fn load_frames(dir: &Path) -> Vec<crate::image::Image> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut paths: Vec<_> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("png")).unwrap_or(false))
        .collect();
    paths.sort();
    paths.iter().filter_map(|p| crate::image::load_png(p)).collect()
}

/// Lo esencial del nodo DRM ya resuelto: tarjeta abierta, CRTC/conector/modo
/// vigentes y la resolución. Lo comparten el splash y el greeter simulado.
struct Drm {
    card: Card,
    crtc: crtc::Handle,
    con: connector::Handle,
    mode: Mode,
    w: usize,
    h: usize,
}

/// Abre el nodo DRM y resuelve conector conectado + CRTC + modo vigentes (los
/// que dejó efifb/simpledrm, heredados del GOP). Reusar ese modo es lo que
/// evita el parpadeo.
fn setup(device: &str) -> Result<Drm, String> {
    let card = open(device)?;
    let res = card
        .resource_handles()
        .map_err(|e| format!("resource_handles: {e}"))?;
    let con = res
        .connectors()
        .iter()
        .filter_map(|h| card.get_connector(*h, true).ok())
        .find(|c| c.state() == connector::State::Connected)
        .ok_or("ningún conector conectado")?;
    let crtc = current_crtc(&card, &con, &res).ok_or("sin CRTC para el conector")?;
    let mode = present_mode(&card, crtc, &con).ok_or("sin modo presentable")?;
    let (w, h) = mode.size();
    log!(
        "conector {:?} crtc {:?} modo {}x{} — reusando modo vigente",
        con.handle(),
        crtc,
        w,
        h
    );
    Ok(Drm { card, crtc, con: con.handle(), mode, w: w as usize, h: h as usize })
}

fn try_run(opts: &Opts) -> Result<(), String> {
    let Drm { card, crtc: crtc_handle, con: con_handle, mode, w, h } = setup(&opts.device)?;

    // Dos superficies para alternar (double buffer).
    let mut a = make_surface(&card, w as u32, h as u32)?;
    let mut b = make_surface(&card, w as u32, h as u32)?;

    // Fuente del splash (builtin / imagen / frames) decodificada una vez.
    let loaded = Loaded::from_cfg(&opts.cfg.source);
    let bg = opts.cfg.bg;

    // Panel de logs automático (sólo si la config lo permite). Aparece si el
    // arranque tarda más de `log_after_ms` o si el kernel reporta un error.
    let logs_auto = matches!(opts.cfg.logs, crate::config::LogMode::Auto);
    let log_after_ms = std::env::var("ARJE_SPLASH_LOG_AFTER_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(opts.cfg.log_after_ms);
    let mut kmsg = if logs_auto {
        let k = crate::logs::Kmsg::open();
        if !k.active() {
            log!("logs: /dev/kmsg no disponible — sin panel de logs");
        }
        Some(k)
    } else {
        None
    };
    let mut log_reveal = 0.0f32;

    // Socket de handoff (Fase 2). Best-effort: si no bindea, sólo tope de tiempo.
    let mut handoff = Handoff::bind(&handoff::sock_path());
    if handoff.active() {
        log!("handoff escuchando en {}", handoff::sock_path().display());
    } else {
        log!("sin socket de handoff — sólo tope de tiempo (Fase 1)");
    }

    let start = Instant::now();
    // Primer present: set_crtc reusando el modo → mismo timing, sin flash.
    paint_into(&card, &mut a, w, h, 0, 0.0, loaded.scene_at(0, opts.fps), bg, None)?;
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
        // Panel de logs: ¿lo mostramos? (boot lento o error). Una vez activo,
        // se queda; revelamos progresivamente (~250 ms).
        if let Some(k) = kmsg.as_mut() {
            k.poll();
            if k.error_seen() || t >= log_after_ms {
                if log_reveal == 0.0 {
                    log!("panel de logs: {}", if k.error_seen() { "error en kmsg" } else { "boot lento" });
                }
                log_reveal = (log_reveal + frame_dt.as_millis() as f32 / 250.0).min(1.0);
            }
        }
        let lines = if log_reveal > 0.0 {
            kmsg.as_ref().map(|k| k.recent(64)).unwrap_or_default()
        } else {
            Vec::new()
        };
        let overlay = (log_reveal > 0.0).then(|| (lines.as_slice(), log_reveal));
        present_one(&card, &mut b, crtc_handle, con_handle, mode, w, h, t, 0.0,
                    loaded.scene_at(t, opts.fps), bg, overlay,
                    &mut can_flip, frame_dt)?;
        std::mem::swap(&mut a, &mut b);
    }

    // Fade-out del handoff: fundimos el contenido al fondo de marca `bg` (no a
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
                        loaded.scene_at(t, opts.fps), bg, None, &mut can_flip, frame_dt)?;
            std::mem::swap(&mut a, &mut b);
        }
        // Frame final: bg sólido garantizado.
        let t = start.elapsed().as_millis() as u64;
        present_one(&card, &mut b, crtc_handle, con_handle, mode, w, h, t, 1.0,
                    loaded.scene_at(t, opts.fps), bg, None, &mut can_flip, frame_dt)?;
    }

    // Salida. En el handoff (Fase 2) NO cerramos el fd todavía: soltamos sólo el
    // DRM master (`release_master_lock`) y mantenemos el framebuffer del splash
    // VIVO en scanout. Así, mientras mirada toma master y flipea su primer frame,
    // el panel sigue mostrando el slate (mismo `BG`) — sin un cuadro de hueco con
    // un FB ya destruido, que se veía como parpadeo. Recién tras una ventana
    // corta soltamos todo. Cerrar el fd de una (como antes) mataba el FB antes de
    // que mirada presentara → el CRTC quedaba un cuadro sin imagen válida.
    if do_handoff {
        let _ = card.release_master_lock(); // suelta master, deja el fd/FB vivos
        handoff.send_released();
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        log!("RELEASED enviado · epoch_ms={ms} — mirada toma la pantalla");
        // Ventana para que mirada agarre master y flipee su FB antes de que
        // soltemos el nuestro. mirada presenta en ~40-100 ms; 300 ms es margen.
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    // Sin handoff (tope de tiempo / señal): cerrar el fd suelta el master.
    drop(a);
    drop(b);
    drop(card);
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
    blend: f32,
    scene: Scene,
    bg: (u8, u8, u8),
    overlay: Option<(&[String], f32)>,
    can_flip: &mut bool,
    frame_dt: Duration,
) -> Result<(), String> {
    let frame_start = Instant::now();
    paint_into(card, surf, w, h, t, blend, scene, bg, overlay)?;
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

/// Mapea el dumb buffer de la superficie y pinta la escena en él. Para `Splash`,
/// `blend` es el fade-out a BG y `t` el tiempo de la animación; para `Greeter`,
/// `blend` es la aparición de la tarjeta desde BG (`t` no se usa).
fn paint_into(
    card: &Card,
    s: &mut Surface,
    w: usize,
    h: usize,
    t: u64,
    blend: f32,
    scene: Scene,
    bg: (u8, u8, u8),
    overlay: Option<(&[String], f32)>,
) -> Result<(), String> {
    let pitch = s.db.pitch() as usize;
    let mut map = card
        .map_dumb_buffer(&mut s.db)
        .map_err(|e| format!("map_dumb_buffer: {e}"))?;
    match scene {
        Scene::Splash => render::paint_frame(map.as_mut(), w, h, pitch, t, blend),
        Scene::Chakana => crate::image::blit_chakana(map.as_mut(), w, h, pitch, t, bg, blend),
        Scene::Image(img) => crate::image::blit_fit(map.as_mut(), w, h, pitch, img, bg, blend),
        Scene::Greeter => render::paint_greeter(map.as_mut(), w, h, pitch, blend),
    }
    // Panel de logs encima de la escena (si está revelándose/visible).
    if let Some((lines, reveal)) = overlay {
        crate::logs::render_panel(map.as_mut(), w, h, pitch, lines, reveal);
    }
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
