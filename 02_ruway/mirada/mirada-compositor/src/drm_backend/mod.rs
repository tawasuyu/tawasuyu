//! `drm_backend` — el Cuerpo del compositor sobre **DRM/KMS**, sin
//! sesión gráfica anfitriona: corre directo sobre una TTY, como tu
//! escritorio de verdad.
//!
//! Construido por fases para verificarlo en hardware paso a paso:
//!
//! - **Fase 1 — bring-up**: sesión (`libseat`), GPU, dispositivo DRM,
//!   enumerar salidas.
//! - **Fase 2a — pipeline de render**: GBM, EGL y `GlesRenderer`, con un
//!   `DrmCompositor` para la salida conectada.
//! - **Fase 2b — bucle Wayland** (esto): un bucle `calloop` que atiende
//!   a los clientes Wayland, el teclado (`libinput`) y el VBlank, y
//!   compone las ventanas de verdad. Aquí `mirada-compositor --drm` ya
//!   es un escritorio funcionando.
//!
//! Todo con logs para diagnosticar sin el hardware delante.

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputEvent, KeyState,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::utils::RescaleRenderElement;
use smithay::backend::renderer::element::{render_elements, Id, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{ImportAll, ImportDma, ImportMem};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev;
use smithay::input::keyboard::FilterResult;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent, RelativeMotionEvent,
};
use smithay::output::OutputModeSource;
use smithay::reexports::calloop::channel::{channel as ticket_channel, Event as TicketEvent};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{EventLoop, Interest, Mode as CalloopMode, PostAction};
use smithay::reexports::drm::control::connector::State as ConnectorState;
use smithay::reexports::drm::control::{Device as ControlDevice, ModeTypeFlags};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::{Display, DisplayHandle, ListeningSocket};
use smithay::utils::{
    DeviceFd, IsAlive, Logical, Physical, Point, Rectangle, Scale, Size, Transform, SERIAL_COUNTER,
};

use auth_core::SessionTicket;
use mirada_brain::{BodyEvent, CtlReply, CtlRequest, Rect, ZoneFrac};

use crate::{
    combo_string, send_frames_surface_tree, App, BodyMode, ClientState, DragGrab, DragMode,
    Setup,
};

mod outputs;
mod render;
mod sesion;
mod input;

/// El `DrmCompositor` concreto para una salida (un solo GPU). Hay uno por
/// cada conector activo en multi-monitor.
type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

/// Una salida física activa: su conector + CRTC + `DrmCompositor` propio +
/// el [`smithay::output::Output`] anunciado en Wayland + su posición en el
/// escritorio global (multi-monitor). La primera de la lista es la
/// **primaria**: ahí van layer-shell, tiling, menú, zonas y HUD; las
/// secundarias renderizan sólo wallpaper hasta tener distribución global.
struct OutputCtx {
    /// Identidad **estable** de esta salida ante el Cerebro (el `OutputId` con
    /// que se registró). No es el índice en [`DrmState::outputs`]: ese cambia al
    /// reordenar por `(order, name)`, pero este id sigue siendo el mismo monitor
    /// físico toda su vida, así que reservas y geometría (`OutputMoved`) le
    /// llegan al monitor correcto aunque la lista se haya reordenado.
    id: u32,
    /// Nombre legible (`DP-1`, `HDMI-A-1`, …) — sale del conector DRM.
    name: String,
    /// El `Output` smithay (vive mientras el compositor corre).
    output: smithay::output::Output,
    /// El CRTC al que está atada esta salida — clave de routing del VBlank.
    crtc: smithay::reexports::drm::control::crtc::Handle,
    /// El `DrmCompositor` que pinta esta salida.
    compositor: Compositor,
    /// Rect en coordenadas del escritorio global. `(rect.x, rect.y)` es la
    /// esquina superior-izquierda en el espacio común; `(rect.w, rect.h)`
    /// es el tamaño nativo del modo.
    rect: Rect,
    /// Refresco en mHz (lo guardamos para futura reconfiguración / hotplug).
    #[allow(dead_code)]
    refresh_mhz: i32,
    /// Wallpaper ya compuesto al tamaño de **esta** salida; `None` se rearma
    /// perezosamente en el próximo render.
    wallpaper: Option<(MemoryRenderBuffer, (i32, i32))>,
    /// Ruta del wallpaper a usar en esta salida (`None` = fondo de color
    /// sólido). Resuelta de la config: el override por nombre de
    /// [`mirada_brain::OutputOverride`] gana; si no hay, cae al global.
    wallpaper_path: Option<String>,
    /// Modo de ajuste del wallpaper en esta salida — análogo a
    /// [`Self::wallpaper_path`]: por-salida si hay override, global si no.
    wallpaper_fit: mirada_brain::WallpaperFit,
    /// `true` entre que esta salida encola un page-flip y llega su VBlank.
    pending_flip: bool,
}

render_elements! {
    /// Lo que el backend DRM compone en un cuadro: superficies de cliente,
    /// rectángulos de color sólido (cursor, marcos) y etiquetas de texto
    /// (búferes RGBA rasterizados — títulos, menú). `ScaledWindow` es una
    /// superficie de cliente **reescalada** (miniatura viva de la vista
    /// espacial): el `scale` de `render_elements_from_surface_tree` es la escala
    /// de salida, no un resize, así que para achicar de verdad la envolvemos en
    /// un `RescaleRenderElement`.
    Frame<R> where R: ImportAll + ImportMem;
    Window = WaylandSurfaceRenderElement<R>,
    Solid = SolidColorRenderElement,
    Text = MemoryRenderBufferRenderElement<R>,
    ScaledWindow = RescaleRenderElement<WaylandSurfaceRenderElement<R>>,
    ScaledText = RescaleRenderElement<MemoryRenderBufferRenderElement<R>>,
}

/// Color de fondo del escritorio cuando no hay nada que lo tape. **Debe ser
/// idéntico al `BG` de `arje-splash` (18,18,24)** para que el handoff de Fase 2
/// no muestre un escalón de color: el splash funde a ese fondo común y mirada
/// limpia al mismo, así el gap (mientras el greeter aún no pintó) se ve como el
/// fondo común y no como un pseudonegro. Ver `SDD-ARRANQUE-SIN-PARPADEO.md`.
const CLEAR_COLOR: [f32; 4] = [18.0 / 255.0, 18.0 / 255.0, 24.0 / 255.0, 1.0];

/// Lado del cursor de software, en píxeles.
const CURSOR_SIZE: i32 = 12;

/// Color del cursor — un cuadrado casi blanco, opaco.
const CURSOR_COLOR: [f32; 4] = [0.95, 0.95, 0.97, 1.0];

/// Alto de las etiquetas de título, en píxeles.
const TITLE_PX: f32 = 16.0;

/// Alto del texto de las filas del menú raíz, en píxeles.
const MENU_TEXT_PX: f32 = 14.0;

/// Color de fondo del menú raíz (RGBA `0..=1`, casi opaco).
const MENU_BG: [f32; 4] = [0.11, 0.11, 0.14, 0.97];

/// Color del texto de las filas del menú raíz (RGBA `0..=255`).
const MENU_TEXT_COLOR: [u8; 4] = [228, 228, 234, 255];


/// Color RGBA de las etiquetas de título — casi blanco.
const TITLE_COLOR: [u8; 4] = [230, 230, 235, 255];

/// Alto del texto del HUD de presets de zonas, en píxeles.
const HUD_TEXT_PX: f32 = 18.0;

/// Color RGBA (`0..=255`) del texto del HUD de presets.
const HUD_TEXT_COLOR: [u8; 4] = [240, 240, 246, 255];

/// Color del fondo del HUD de presets (RGBA `0..=1`, casi opaco).
const HUD_BG: [f32; 4] = [0.10, 0.10, 0.14, 0.85];

/// Margen interno del HUD entre el texto y el borde de su panel, en píxeles.
const HUD_PAD: i32 = 12;

/// Distancia del HUD al borde superior de la salida, en píxeles.
const HUD_TOP: i32 = 40;

/// Duración del HUD del preset activo al ciclar zonas.
const HUD_DURATION: Duration = Duration::from_millis(1500);

/// Lado mínimo de una ventana al redimensionarla con el ratón.
const MIN_WINDOW: i32 = 120;

/// Convierte un color RGBA en `0..=255` (como viaja por el protocolo y la
/// config) al `[f32; 4]` normalizado que consume el renderer.
fn rgba_f32(c: [u8; 4]) -> [f32; 4] {
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        c[3] as f32 / 255.0,
    ]
}

/// Los 4 rectángulos `(x, y, w, h)` del marco de grosor `bw` de una ventana
/// cuyo contenido ocupa `(sx, sy, sw, sh)`. El marco va *hacia adentro*
/// (pisa el borde de la superficie), así nunca se solapa con el de la
/// ventana vecina: arriba, abajo, izquierda, derecha.
fn border_rects(sx: i32, sy: i32, sw: i32, sh: i32, bw: i32) -> [(i32, i32, i32, i32); 4] {
    let side_h = (sh - 2 * bw).max(0);
    [
        (sx, sy, sw, bw),
        (sx, sy + sh - bw, sw, bw),
        (sx, sy + bw, bw, side_h),
        (sx + sw - bw, sy + bw, bw, side_h),
    ]
}

/// Gradiente vertical sobrio (noche profunda → púrpura → azul-medianoche),
/// generado runtime. Es el **fallback** cuando el wallpaper de marca no
/// decodifica; el default real sin config es [`make_marca_wallpaper`].
fn make_default_wallpaper(w: i32, h: i32) -> MemoryRenderBuffer {
    let w_u = w as usize;
    let h_u = h as usize;
    let mut bgra = vec![0u8; w_u * h_u * 4];
    let stops: [(f32, [u8; 3]); 3] = [
        (0.0, [0x0a, 0x0e, 0x22]),
        (0.55, [0x1b, 0x1a, 0x3e]),
        (1.0, [0x2a, 0x1c, 0x4a]),
    ];
    let denom = (h_u.saturating_sub(1)).max(1) as f32;
    for y in 0..h_u {
        let t = y as f32 / denom;
        let (r, g, b) = mezcla_stops(&stops, t);
        let row = &mut bgra[y * w_u * 4..(y + 1) * w_u * 4];
        for x in 0..w_u {
            let i = x * 4;
            row[i] = b;
            row[i + 1] = g;
            row[i + 2] = r;
            row[i + 3] = 255;
        }
    }
    MemoryRenderBuffer::from_slice(
        &bgra,
        Fourcc::Argb8888,
        (w, h),
        1,
        Transform::Normal,
        None,
    )
}

/// Buffer de color sólido `w×h` (BGRA opaco).
fn make_solid_wallpaper(rgb: [u8; 3], w: i32, h: i32) -> MemoryRenderBuffer {
    let (w_u, h_u) = (w.max(1) as usize, h.max(1) as usize);
    let mut bgra = vec![0u8; w_u * h_u * 4];
    for px in bgra.chunks_exact_mut(4) {
        px[0] = rgb[2];
        px[1] = rgb[1];
        px[2] = rgb[0];
        px[3] = 255;
    }
    MemoryRenderBuffer::from_slice(&bgra, Fourcc::Argb8888, (w, h), 1, Transform::Normal, None)
}

/// Gradiente vertical `w×h` (BGRA opaco) a partir de stops RGB equiespaciados.
/// Con menos de 2 stops cae al gradiente sobrio por defecto.
fn make_gradient_wallpaper(stops_rgb: &[[u8; 3]], w: i32, h: i32) -> MemoryRenderBuffer {
    if stops_rgb.len() < 2 {
        return make_default_wallpaper(w, h);
    }
    let (w_u, h_u) = (w.max(1) as usize, h.max(1) as usize);
    let n = stops_rgb.len();
    let stops: Vec<(f32, [u8; 3])> = stops_rgb
        .iter()
        .enumerate()
        .map(|(i, c)| (i as f32 / (n - 1) as f32, *c))
        .collect();
    let mut bgra = vec![0u8; w_u * h_u * 4];
    let denom = (h_u.saturating_sub(1)).max(1) as f32;
    for y in 0..h_u {
        let (r, g, b) = mezcla_stops(&stops, y as f32 / denom);
        let row = &mut bgra[y * w_u * 4..(y + 1) * w_u * 4];
        for x in 0..w_u {
            let i = x * 4;
            row[i] = b;
            row[i + 1] = g;
            row[i + 2] = r;
            row[i + 3] = 255;
        }
    }
    MemoryRenderBuffer::from_slice(&bgra, Fourcc::Argb8888, (w, h), 1, Transform::Normal, None)
}

/// Fondo **procedural** `w×h` (BGRA opaco): delega en `mirada-procedural`. Seed
/// fijo → determinista por tamaño (mismo patrón en cada arranque/monitor).
fn make_procedural_wallpaper(
    pattern: mirada_procedural::Pattern,
    palette: &[[u8; 3]],
    w: i32,
    h: i32,
) -> MemoryRenderBuffer {
    let bgra = mirada_procedural::generate_bgra(pattern, palette, w.max(1) as u32, h.max(1) as u32, 1);
    MemoryRenderBuffer::from_slice(&bgra, Fourcc::Argb8888, (w, h), 1, Transform::Normal, None)
}

/// Interpola entre stops `(t, rgb)` para `t in 0..1`. Lineal por tramo.
fn mezcla_stops(stops: &[(f32, [u8; 3])], t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let mut prev = stops[0];
    for s in &stops[1..] {
        if t <= s.0 {
            let span = (s.0 - prev.0).max(1e-6);
            let u = ((t - prev.0) / span).clamp(0.0, 1.0);
            let lerp = |a: u8, b: u8| -> u8 {
                (a as f32 + (b as f32 - a as f32) * u).round().clamp(0.0, 255.0) as u8
            };
            return (
                lerp(prev.1[0], s.1[0]),
                lerp(prev.1[1], s.1[1]),
                lerp(prev.1[2], s.1[2]),
            );
        }
        prev = *s;
    }
    let last = stops.last().unwrap().1;
    (last[0], last[1], last[2])
}

/// Decodifica el wallpaper de `path` y lo compone en un buffer del tamaño
/// de la salida (`w` × `h`) según `fit`. El resto del buffer queda en negro
/// opaco (BGRA `[0, 0, 0, 255]`). Devuelve `None` si la imagen no abre o el
/// tamaño es degenerado. El formato es `Argb8888` (little-endian → bytes
/// `[B, G, R, A]` en memoria); el wallpaper es opaco, así que la
/// premultiplicación por alfa es identidad.
fn load_wallpaper(
    path: &str,
    fit: mirada_brain::WallpaperFit,
    w: i32,
    h: i32,
) -> Option<MemoryRenderBuffer> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let img = match image::open(path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("mirada-compositor · wallpaper «{path}» no carga ({e}); fondo sólido.");
            return None;
        }
    };
    compose_wallpaper(&img, fit, w, h)
}

/// El wallpaper de **marca** como fondo por defecto (embebido en el crate
/// `marca`, con override por disco). Cae a `None` si los bytes no decodifican —
/// el llamador usa entonces el gradiente sobrio de `make_default_wallpaper`.
fn make_marca_wallpaper(
    fit: mirada_brain::WallpaperFit,
    w: i32,
    h: i32,
) -> Option<MemoryRenderBuffer> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let bytes = marca::wallpaper();
    let img = match image::load_from_memory(&bytes) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("mirada-compositor · wallpaper de marca no decodifica ({e}); gradiente.");
            return None;
        }
    };
    compose_wallpaper(&img, fit, w, h)
}

/// Compone una imagen ya decodificada en un buffer del tamaño de la salida
/// (`w`×`h`) según `fit`; el resto queda en negro opaco. Núcleo compartido por
/// `load_wallpaper` (desde ruta) y `make_marca_wallpaper` (bytes embebidos).
fn compose_wallpaper(
    img: &image::DynamicImage,
    fit: mirada_brain::WallpaperFit,
    w: i32,
    h: i32,
) -> Option<MemoryRenderBuffer> {
    use mirada_brain::WallpaperFit;
    let rgba = img.to_rgba8();
    let sw = rgba.width() as i32;
    let sh = rgba.height() as i32;
    if sw <= 0 || sh <= 0 {
        return None;
    }

    // Lienzo negro opaco del tamaño de la salida.
    let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];
    for px in bgra.chunks_exact_mut(4) {
        px[3] = 255;
    }

    match fit {
        WallpaperFit::Tile => {
            // Repetir desde la esquina superior-izquierda en tamaño nativo.
            let src = rgba.as_raw();
            let sw_u = sw as usize;
            let sh_u = sh as usize;
            for y in 0..(h as usize) {
                let sy = y % sh_u;
                let src_row = &src[sy * sw_u * 4..(sy + 1) * sw_u * 4];
                let dst_row = &mut bgra[y * (w as usize) * 4..(y + 1) * (w as usize) * 4];
                for x in 0..(w as usize) {
                    let sx = x % sw_u;
                    let si = sx * 4;
                    let di = x * 4;
                    dst_row[di] = src_row[si + 2]; // B
                    dst_row[di + 1] = src_row[si + 1]; // G
                    dst_row[di + 2] = src_row[si]; // R
                    dst_row[di + 3] = 255;
                }
            }
        }
        _ => {
            let (dx, dy, dw, dh) = mirada_brain::wallpaper_dst_rect(fit, sw, sh, w, h);
            // Para Stretch/Fit/Fill escalamos la imagen al rect que cae en
            // pantalla; para Center la dejamos a tamaño nativo. En todos los
            // casos el "lienzo" donde se pintará es de tamaño `(dw, dh)` y
            // luego se composita con offset `(dx, dy)` clipeando al destino.
            let (img_pixels, iw, ih) = if matches!(fit, WallpaperFit::Center) {
                (rgba.as_raw().clone(), sw, sh)
            } else if dw <= 0 || dh <= 0 {
                return None;
            } else {
                let scaled = image::imageops::resize(
                    &rgba,
                    dw as u32,
                    dh as u32,
                    image::imageops::FilterType::Triangle,
                );
                (scaled.into_raw(), dw, dh)
            };
            paste_rgba_into_bgra(&mut bgra, w, h, &img_pixels, iw, ih, dx, dy);
        }
    }

    Some(MemoryRenderBuffer::from_slice(
        &bgra,
        Fourcc::Argb8888,
        (w, h),
        1,
        Transform::Normal,
        None,
    ))
}

/// Pega un bloque RGBA de `(iw, ih)` en un lienzo BGRA `(dw, dh)` con esquina
/// superior-izquierda en `(dx, dy)`. Filas y columnas fuera del destino se
/// clipean. Alfa se fuerza opaco.
fn paste_rgba_into_bgra(
    dst: &mut [u8],
    dw: i32,
    dh: i32,
    src: &[u8],
    iw: i32,
    ih: i32,
    dx: i32,
    dy: i32,
) {
    if iw <= 0 || ih <= 0 || dw <= 0 || dh <= 0 {
        return;
    }
    let row_dst = (dw as usize) * 4;
    let row_src = (iw as usize) * 4;
    // Recorta horizontalmente: rango de columnas de la fuente que cae en pantalla.
    let x0 = dx.max(0);
    let x1 = (dx + iw).min(dw);
    if x1 <= x0 {
        return;
    }
    let src_x0 = (x0 - dx) as usize;
    let copy_w = (x1 - x0) as usize;
    for y in 0..ih {
        let ty = dy + y;
        if ty < 0 || ty >= dh {
            continue;
        }
        let src_row = &src[(y as usize) * row_src..(y as usize + 1) * row_src];
        let dst_row = &mut dst[(ty as usize) * row_dst..(ty as usize + 1) * row_dst];
        for k in 0..copy_w {
            let si = (src_x0 + k) * 4;
            let di = (x0 as usize + k) * 4;
            dst_row[di] = src_row[si + 2]; // B
            dst_row[di + 1] = src_row[si + 1]; // G
            dst_row[di + 2] = src_row[si]; // R
            dst_row[di + 3] = 255;
        }
    }
}

/// Rasteriza un título a un `MemoryRenderBuffer` (Argb8888); si no rasteriza
/// nada, devuelve un búfer 1×1 transparente. Lo cachea el llamador.
fn title_buffer(tr: &crate::text::TextRenderer, title: &str) -> MemoryRenderBuffer {
    match tr.rasterize(title, TITLE_PX, TITLE_COLOR) {
        Some(r) => MemoryRenderBuffer::from_slice(
            &r.rgba,
            Fourcc::Argb8888,
            (r.width, r.height),
            1,
            Transform::Normal,
            None,
        ),
        None => MemoryRenderBuffer::from_slice(
            &[0u8; 4],
            Fourcc::Argb8888,
            (1, 1),
            1,
            Transform::Normal,
            None,
        ),
    }
}

/// Códigos de botón de `<linux/input-event-codes.h>`.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// El estado del bucle DRM — lo comparten todos los callbacks de `calloop`.
struct DrmState {
    app: App,
    /// La sesión libseat — se conserva para conmutar de VT (`Ctrl+Alt+Fn`).
    session: LibSeatSession,
    display: Display<App>,
    /// El dispositivo DRM — se conserva para pausarlo y reactivarlo al
    /// conmutar de VT.
    drm: DrmDevice,
    /// Recursos para crear `OutputCtx` nuevos en caliente (hotplug). Se
    /// guardan acá porque la creación inicial los necesita y el handler de
    /// `UdevEvent::Changed` también, dentro del bucle de eventos.
    gbm: GbmDevice<DrmDeviceFd>,
    allocator: GbmAllocator<DrmDeviceFd>,
    exporter: GbmFramebufferExporter<DrmDeviceFd>,
    renderer_formats: smithay::backend::allocator::format::FormatSet,
    /// Handle al display Wayland — `announce_output` lo necesita al crear
    /// el `wl_output` global de cada monitor nuevo.
    dh: DisplayHandle,
    /// Salidas físicas activas: una por conector. `outputs[0]` es la
    /// primaria — soporta layer-shell, tiling, menú, zonas, HUD.
    outputs: Vec<OutputCtx>,
    renderer: GlesRenderer,
    /// Sombra bajo cada ventana (capas translúcidas, sin shader). Gateada por
    /// la env `MIRADA_SHADOW` mientras se verifica en pantalla — así no toca el
    /// default de nadie hasta confirmarla.
    shadows_on: bool,
    /// Contexto `libinput` — se suspende y reanuda al conmutar de VT.
    libinput: Libinput,
    /// `false` mientras la sesión está cedida a otra VT — no se compone.
    active: bool,
    /// Vigías de los archivos de config recargables en caliente.
    watches: crate::ConfigWatches,
    ctl: Option<crate::CtlServer>,
    /// Próximo `OutputId` a asignar a un monitor recién enchufado. Monótono y
    /// **nunca reusado** (a diferencia de `outputs.len()`, que reciclaba ids
    /// tras un desenchufe y los hacía colisionar): así cada salida tiene una
    /// identidad estable de por vida ante el Cerebro.
    next_output_id: u32,
    /// Inicio del compositor — base de tiempos para los frame-callbacks.
    start: Instant,
    /// Nº de ventanas en el último `tick` — para registrar los cambios.
    last_windows: usize,
    /// Último escritorio activo visto — para detectar el cambio y disparar el
    /// slide de transición (modo `Hyprland`/`Prezi`).
    last_active_ws: usize,
    /// Última salida enfocada vista — para NO confundir un cambio de monitor
    /// enfocado (mover el mouse entre pantallas) con un cambio de escritorio.
    /// Sin esto, cruzar el mouse a otro monitor (que muestra otro escritorio)
    /// disparaba el slide en cada cruce → parpadeo «los contenidos se cambian».
    last_focused_output: usize,
    /// Slide de escritorios en curso: `(ms de inicio, signo de dirección)`.
    /// `None` = sin transición. El signo: +1 desliza desde la derecha (fuiste a
    /// un escritorio mayor), -1 desde la izquierda.
    ws_slide: Option<(u32, f32)>,
    /// Animación de zoom de la vista espacial (Prezi): `(ms de inicio, abriendo)`.
    /// `abriendo=true` = zoom-OUT (del escritorio activo al mosaico); `false` =
    /// zoom-IN de cierre. `None` = sin animación (abierta-quieta o cerrada).
    overview_anim: Option<(u32, bool)>,
    /// Valor de `overview_open` del tick anterior — para detectar el flanco de
    /// apertura y arrancar la animación de zoom-out.
    prev_overview_open: bool,
    /// Rects en pantalla de cada tile de la vista espacial (Prezi) `(escritorio,
    /// rect)` — poblado al pintar, usado para el hit-test del click. (El flag
    /// `overview_open` vive en `App` para togglearlo desde el filtro de teclado.)
    overview_tiles: Vec<(usize, Rect)>,
    /// Fondo automático (slideshow): imágenes de la carpeta, índice actual, ms
    /// del próximo cambio, y la carpeta cacheada (para recargar si cambia).
    wp_images: Vec<std::path::PathBuf>,
    wp_index: usize,
    wp_next_switch_ms: u32,
    wp_dir: String,
    /// Identidad estable del cursor de software — el seguimiento de daño
    /// la usa para no recomponer todo cuando el cursor sólo se mueve.
    cursor_id: Id,
    /// Ventana sobre la que estaba el puntero — para el foco-sigue-ratón.
    last_pointer_window: Option<u64>,
    /// Tamaño de la salida, en píxeles — los topes del puntero.
    output_size: (f64, f64),
    /// Renderizador de texto (etiquetas de título/menú). `None` si no se
    /// encontró ninguna fuente — entonces no se pintan etiquetas.
    text: Option<crate::text::TextRenderer>,
    /// Caché de etiquetas ya rasterizadas, por (texto, color) → búfer subido.
    /// Evita re-rasterizar y re-subir la textura en cada cuadro.
    text_cache: std::collections::HashMap<(String, [u8; 4]), MemoryRenderBuffer>,
    /// Árbol del menú raíz (de la config), con submenús anidados.
    menu_entries: Vec<crate::menu::MenuNode>,
    /// Menú raíz abierto, si lo hay (click derecho sobre el fondo). Sus
    /// coordenadas son **locales** a la salida [`Self::menu_output_idx`]:
    /// el menú se abre en el monitor donde se hizo right-click.
    root_menu: Option<crate::menu::RootMenu>,
    /// Índice de la salida en la que vive el menú raíz abierto. `None` =
    /// no hay menú.
    menu_output_idx: Option<usize>,
    /// Ventana objetivo del menú **contextual** abierto (click derecho en el
    /// titlebar). `None` = el menú abierto es el raíz (del fondo), no de ventana.
    menu_window: Option<u64>,
    /// Último click izquierdo sobre una barra de título: `(id, instante)`. Sirve
    /// para detectar el **doble-click** (maximiza) antes de arrancar un arrastre.
    last_titlebar_click: Option<(u64, Instant)>,
    /// Zonas de arrastre activas (fracciones del área útil) — el preset actual.
    zones: Vec<ZoneFrac>,
    /// Todos los presets de zonas (preset 0 = `config.zones`, luego
    /// `config.zone_presets`). `mirada-ctl cycle-zones` avanza entre ellos.
    zone_presets: Vec<Vec<ZoneFrac>>,
    /// Índice del preset activo dentro de [`Self::zone_presets`].
    active_preset: usize,
    /// Rect destino (global) del drag-to-zone resaltado bajo el puntero durante
    /// un arrastre. `None` = sin snap (la ventana cae libre). Lo calcula
    /// [`Self::zone_at`] (snap por borde estilo KDE).
    drag_zone: Option<Rect>,
    /// Instante hasta el que pintar el HUD del preset activo (al ciclar
    /// zonas). `None` = sin HUD. Se setea en cada ciclo y se respeta hasta
    /// que el reloj de `start` lo supera; el siguiente tick (~60 Hz) lo
    /// borra y damasca su área para el siguiente flip.
    preset_hud_until: Option<Instant>,
    /// Etiqueta cacheada del HUD (la última que se rasterizó). Permite
    /// reusar `text_cache` sin recomputar el texto en cada cuadro.
    preset_hud_label: String,
}

/// Arranca el Cuerpo sobre DRM/KMS — fases 1, 2a y 2b. Con `greeter`,
/// el compositor nace en modo DM: ver [`BodyMode`].
pub fn run(greeter: bool) -> Result<(), Box<dyn Error>> {
    println!("mirada-compositor · backend DRM.");
    println!("──────────────────────────────────────────────────");

    // 1 · Sesión.
    println!("[1/8] abriendo la sesión (libseat) …");
    let (mut session, session_notifier) = LibSeatSession::new().map_err(|e| {
        format!(
            "no pude abrir la sesión libseat: {e}\n       \
             ¿estás en una TTY de verdad (Ctrl+Alt+F3), con `seatd` o `logind`?"
        )
    })?;
    let seat_name = session.seat();
    println!("      sesión abierta · seat «{seat_name}»");

    // 2 · GPU primaria.
    println!("[2/8] buscando la GPU primaria …");
    let gpu = udev::primary_gpu(&seat_name)
        .map_err(|e| format!("error consultando udev: {e}"))?
        .ok_or("no encontré ninguna GPU — ¿existe algún /dev/dri/card*?")?;
    println!("      GPU primaria: {}", gpu.display());

    // Handoff sin parpadeo (Fase 2 del SDD-ARRANQUE-SIN-PARPADEO): si arje-splash
    // está mostrando el splash del arranque y tiene el DRM master, le pedimos la
    // pantalla y esperamos su fade-out + `RELEASED` ANTES de abrir el device.
    // El orden es crítico: el seat manager (seatd/logind) hace `SET_MASTER` al
    // `session.open()` de un nodo DRM si la sesión está activa. Si abrimos
    // mientras el splash todavía tiene el master (lo tomó directo, fuera del
    // seat manager), ese `SET_MASTER` choca y el fd queda **sin master para
    // siempre** → el modeset de `DrmDevice::new` cae con `EACCES`. Abriendo
    // después del `RELEASED`, el master ya está libre y seatd lo concede limpio.
    // Best-effort: sin splash, `esperar_release` vuelve enseguida y seguimos.
    crate::handoff::esperar_release_del_splash();

    // 3 · Dispositivo DRM (con el master ya libre tras el handoff).
    println!("[3/8] abriendo el dispositivo DRM …");
    let fd = session
        .open(&gpu, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK)
        .map_err(|e| format!("no pude abrir {}: {e}", gpu.display()))?;

    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));
    // Red de seguridad ante una carrera residual: si tras el `RELEASED` el
    // kernel todavía está finalizando la liberación del master del splash, el
    // primer `DrmDevice::new` puede ver `EACCES`. Reintentamos con backoff
    // corto; sin splash (arranque a mano) entra al primer intento.
    let (mut drm, drm_notifier) = {
        let mut intento = 0u32;
        loop {
            match DrmDevice::new(drm_fd.clone(), true) {
                Ok(par) => break par,
                Err(_) if intento < 20 => {
                    if intento == 0 {
                        println!("      esperando el master del splash …");
                    }
                    intento += 1;
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(e) => return Err(format!("DrmDevice::new falló: {e}").into()),
            }
        }
    };
    println!("      dispositivo DRM listo.");

    // 4 · Enumerar TODAS las salidas conectadas: conector + CRTC + modo.
    println!("[4/8] enumerando salidas …");
    let resources = drm
        .resource_handles()
        .map_err(|e| format!("no pude leer los recursos DRM: {e}"))?;
    use smithay::reexports::drm::control::{crtc, Mode as DrmMode};
    let mut chosen: Vec<(
        smithay::reexports::drm::control::connector::Handle,
        crtc::Handle,
        DrmMode,
        String,
    )> = Vec::new();
    // Pool de CRTCs ya tomados — un CRTC no se reparte entre dos salidas.
    let mut used_crtcs: Vec<crtc::Handle> = Vec::new();
    for &conn_handle in resources.connectors() {
        let conn = match drm.get_connector(conn_handle, false) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if conn.state() != ConnectorState::Connected {
            continue;
        }
        let name = format!("{:?}-{}", conn.interface(), conn.interface_id());
        for m in conn.modes() {
            let (mw, mh) = m.size();
            let pref = if m.mode_type().contains(ModeTypeFlags::PREFERRED) {
                " [PREFERRED]"
            } else {
                ""
            };
            eprintln!("      modo de «{name}»: {mw}×{mh} @ {} Hz{pref}", m.vrefresh());
        }
        // Elige el modo de mayor área (a igualdad, mayor refresco) — el
        // nativo del panel.
        let mode = conn
            .modes()
            .iter()
            .max_by_key(|m| {
                let (mw, mh) = m.size();
                (mw as u32 * mh as u32, m.vrefresh())
            })
            .copied();
        let Some(mode) = mode else {
            continue;
        };
        // El primer CRTC compatible que no esté ya tomado.
        let crtc_choice = conn
            .encoders()
            .iter()
            .filter_map(|enc| drm.get_encoder(*enc).ok())
            .find_map(|enc| {
                resources
                    .filter_crtcs(enc.possible_crtcs())
                    .into_iter()
                    .find(|c| !used_crtcs.contains(c))
            });
        if let Some(crtc_h) = crtc_choice {
            let (w, h) = mode.size();
            println!("      salida «{name}» · {w}×{h} · CRTC {crtc_h:?}");
            used_crtcs.push(crtc_h);
            chosen.push((conn_handle, crtc_h, mode, name));
        } else {
            eprintln!("      salida «{name}» sin CRTC libre — se ignora");
        }
    }
    if chosen.is_empty() {
        return Err("ninguna salida conectada con CRTC disponible".into());
    }
    // El nombre de la primaria se decide tras ordenar por (order, name) más
    // abajo — no por orden de discovery. Se captura justo después del sort.

    // 5 · GBM + EGL + GlesRenderer.
    println!("[5/8] inicializando GBM + EGL + GlesRenderer …");
    let gbm = GbmDevice::new(drm_fd.clone()).map_err(|e| format!("GbmDevice::new falló: {e}"))?;
    let egl_display =
        unsafe { EGLDisplay::new(gbm.clone()) }.map_err(|e| format!("EGLDisplay::new falló: {e}"))?;
    let egl_context =
        EGLContext::new(&egl_display).map_err(|e| format!("EGLContext::new falló: {e}"))?;
    let renderer =
        unsafe { GlesRenderer::new(egl_context) }.map_err(|e| format!("GlesRenderer falló: {e}"))?;
    println!("      renderer GLES listo.");

    // 6 · Superficie DRM + DrmCompositor por cada salida descubierta.
    // El renderer GLES se comparte (un solo EGLContext sobre la GPU); cada
    // salida tiene su propio DrmSurface + DrmCompositor.
    println!("[6/8] creando la superficie DRM y compositors ({}) …", chosen.len());
    let allocator =
        GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let exporter = GbmFramebufferExporter::new(gbm.clone(), None);
    let renderer_formats = renderer.dmabuf_formats();

    // 7 · El estado Wayland (Cerebro, teclado, keymap, control).
    println!("[7/8] armando el estado Wayland …");
    let Setup { mut display, mut app, watches, ctl } =
        crate::build_app(greeter)?;
    // Con el renderer ya creado, anuncia dmabuf — sin esto las apps que
    // pintan por GPU (GPUI, navegadores acelerados) no pueden conectarse.
    crate::announce_dmabuf(&mut app, &display.handle(), &renderer);

    // Ordenar las salidas según la config: `(order, name)` ascendente. La de
    // menor `order` queda **primaria** (origen `(0,0)` y todas las features
    // gated por primaria — layer-shell, tiling, HUD — viven ahí). Sin
    // overrides todas son `order=0` y el desempate por nombre da un orden
    // alfabético estable y reproducible (no el azar de discovery).
    chosen.sort_by(|a, b| {
        let oa = app.config_output_order_for(&a.3);
        let ob = app.config_output_order_for(&b.3);
        oa.cmp(&ob).then_with(|| a.3.cmp(&b.3))
    });
    // Primaria = la primera tras el sort.
    let out_name = chosen[0].3.clone();

    // Disponer los rects globales en la dirección que pide la config
    // (horizontal o vertical). El cálculo lo hace `mirada-layout` —cero
    // geometría a mano— y es la misma función que el día de mañana usará
    // wawa cuando enumere scanouts.
    let tamanos: Vec<(i32, i32)> = chosen
        .iter()
        .map(|(_, _, mode, _)| {
            let (w, h) = mode.size();
            (w as i32, h as i32)
        })
        .collect();
    let disp = app.config_output_disposition();
    let rects = mirada_brain::disponer(&tamanos, disp);

    // Construir un OutputCtx por cada salida en el orden ya decidido.
    let mut output_ctxs: Vec<OutputCtx> = Vec::with_capacity(chosen.len());
    for (i, (conn_handle, crtc_h, mode, name)) in chosen.iter().cloned().enumerate() {
        let (w, h) = mode.size();
        let surface = drm
            .create_surface(crtc_h, mode, &[conn_handle])
            .map_err(|e| format!("create_surface[{name}] falló: {e}"))?;
        // Scale/transform por salida — overrides de la config o defaults.
        // Para `OutputModeSource::Static` (geometría DRM) hace falta el
        // factor decimal; el announce al protocolo usa el enum de smithay
        // (Integer/Fractional) para no romper clientes viejos.
        let scale_120 = app.config_output_scale_120_for(&name);
        let transform = app.config_output_transform_for(&name);
        let scale_f64 = (if scale_120 > 0 { scale_120 } else { 120 }) as f64 / 120.0;
        let mode_source = OutputModeSource::Static {
            size: Size::from((w as i32, h as i32)),
            scale: Scale::from(scale_f64),
            transform,
        };
        let comp: Compositor = DrmCompositor::new(
            mode_source,
            surface,
            None,
            allocator.clone(),
            exporter.clone(),
            [Fourcc::Argb8888, Fourcc::Xrgb8888],
            renderer_formats.clone(),
            drm.cursor_size(),
            Some(gbm.clone()),
        )
        .map_err(|e| format!("DrmCompositor::new[{name}] falló: {e}"))?;
        let refresh_mhz = mode.vrefresh() as i32 * 1000;
        let smithay_out = crate::announce_output(
            &display.handle(),
            &name,
            w as i32,
            h as i32,
            refresh_mhz,
            scale_120,
            transform,
        );
        // Brain: cada salida es un id incremental con su tamaño local. El id
        // (= índice inicial, ya en orden ordenado) es ESTABLE: lo guardamos en
        // el `OutputCtx` para direccionar reservas/geometría aunque luego se
        // reordene la lista.
        let id = i as u32;
        let ev = app.body.add_output(id, w as i32, h as i32);
        app.brain_feed(ev);
        let rect = rects[i];
        // El Cuerpo es la fuente única de la posición global: se la fijamos al
        // Cerebro explícitamente (no que la reconstruya por orden de aparición).
        let ev = app.body.move_output(id, rect.x, rect.y);
        app.brain_feed(ev);
        // Wallpaper resuelto por salida (override por nombre o global).
        let wp_path = app.config_wallpaper_path_for(&name);
        let wp_fit = app.config_wallpaper_fit_for(&name);
        println!("      compositor de «{name}» listo · rect global {rect:?}");
        output_ctxs.push(OutputCtx {
            id,
            name,
            output: smithay_out,
            crtc: crtc_h,
            compositor: comp,
            rect,
            refresh_mhz,
            wallpaper: None,
            wallpaper_path: wp_path,
            wallpaper_fit: wp_fit,
            pending_flip: false,
        });
    }

    // Espacio global del escritorio: union de todos los rects ya dispuestos.
    let env = mirada_brain::envolvente(&rects);
    let total_w = env.w.max(1);
    let total_h = env.h.max(1);
    app.output_size = (total_w, total_h);
    // El puntero arranca centrado en la salida primaria (no en el centro
    // del escritorio global) — más predecible cuando hay varios monitores.
    let primary_rect = output_ctxs[0].rect;
    app.pointer_loc = (
        (primary_rect.x + primary_rect.w / 2) as f64,
        (primary_rect.y + primary_rect.h / 2) as f64,
    );
    // Primary `smithay::Output` para layer-shell, xdg-output handlers, etc.
    app.output = Some(output_ctxs[0].output.clone());
    app.outputs = output_ctxs.iter().map(|c| c.output.clone()).collect();
    // `output_ids[i]` = id estable del monitor en `outputs[i]` (mismo orden):
    // las reservas se direccionan por id, no por índice, así sobreviven a un
    // reordenamiento por hotplug.
    app.output_ids = output_ctxs.iter().map(|c| c.id).collect();

    // El socket Wayland por el que se conectan los clientes.
    let listener = ListeningSocket::bind_auto("wayland", 1..32)?;
    let socket_name = listener
        .socket_name()
        .and_then(|s| s.to_str())
        .unwrap_or("wayland-?")
        .to_string();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    println!("      escuchando en WAYLAND_DISPLAY={socket_name}");

    // Modo DM: lanza el greeter y recibe su tiquet por un canal de
    // `calloop`. Modo normal: autoarranque + `MIRADA_STARTUP`.
    let greeter_rx = if app.mode == BodyMode::Greeter {
        let (tx, rx) = ticket_channel::<SessionTicket>();
        let stdin = crate::spawn_greeter(move |ticket| {
            let _ = tx.send(ticket);
        })?;
        app.greeter_stdin = Some(stdin);
        Some(rx)
    } else {
        // Autoarranque: los programas de `~/.config/mirada/autostart`.
        // Modo no-greeter: ya corremos con la identidad del usuario, sin
        // traspaso, así que no hay entorno de sesión que inyectar.
        crate::spawn_autostart(None, &[]);
        // App de arranque: si `MIRADA_STARTUP` trae un comando, se lanza
        // como hijo (hereda `WAYLAND_DISPLAY`) — cómodo para probar sin
        // saltar de VT.
        if let Ok(cmd) = std::env::var("MIRADA_STARTUP") {
            crate::spawn_command(&cmd, None, &[]);
        }
        None
    };

    // 8 · El bucle `calloop`: VBlank, teclado, clientes y un timer.
    println!("[8/8] montando el bucle de eventos …");
    let mut event_loop: EventLoop<DrmState> =
        EventLoop::try_new().map_err(|e| format!("calloop falló: {e}"))?;
    let handle = event_loop.handle();

    // Sesión: pausa/activación al conmutar de VT.
    handle
        .insert_source(session_notifier, |event, _, state: &mut DrmState| match event {
            SessionEvent::PauseSession => state.pause_session(),
            SessionEvent::ActivateSession => state.resume_session(),
        })
        .map_err(|e| format!("insert session: {e}"))?;

    // VBlank: el page-flip de un CRTC terminó. Rutearlo a la salida que lo
    // posee. Si llega para un CRTC desconocido (no debería en estado estable)
    // se ignora silenciosamente.
    handle
        .insert_source(drm_notifier, |event, _meta, state| match event {
            DrmEvent::VBlank(crtc) => {
                if let Some(idx) = state.output_index_by_crtc(crtc) {
                    let ctx = &mut state.outputs[idx];
                    if let Err(e) = ctx.compositor.frame_submitted() {
                        eprintln!(
                            "mirada-compositor · frame_submitted[{}]: {e}",
                            ctx.name
                        );
                    }
                    ctx.pending_flip = false;
                }
            }
            DrmEvent::Error(e) => eprintln!("mirada-compositor · DRM: {e}"),
        })
        .map_err(|e| format!("insert drm: {e}"))?;

    // Hotplug: udev notifica cuando un monitor se conecta/desconecta o
    // cuando aparece/desaparece una GPU. En `Changed` reenumeramos los
    // conectores y reconciliamos `outputs` (crear OutputCtx para los
    // recién enchufados, dropear los desenchufados). Los eventos
    // `Added`/`Removed` de GPU completa se logean: el compositor hoy
    // sirve una sola GPU, cambiar la primaria pide reiniciar.
    let udev = match smithay::backend::udev::UdevBackend::new(&seat_name) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("mirada-compositor · UdevBackend no arranca ({e}); sin hotplug");
            return Err(format!("insert drm: udev: {e}").into());
        }
    };
    handle
        .insert_source(udev, |event, _meta, state| {
            use smithay::backend::udev::UdevEvent;
            match event {
                UdevEvent::Added { device_id, path } => {
                    eprintln!(
                        "mirada-compositor · hotplug · GPU añadida: {} ({device_id:?}); ignorada (multi-GPU pendiente)",
                        path.display()
                    );
                }
                UdevEvent::Changed { device_id: _ } => state.detect_connector_changes(),
                UdevEvent::Removed { device_id, .. } => {
                    eprintln!(
                        "mirada-compositor · hotplug · GPU retirada ({device_id:?}); ignorada (multi-GPU pendiente)"
                    );
                }
            }
        })
        .map_err(|e| format!("insert udev: {e}"))?;

    // Teclado y ratón vía libinput. Guardamos un clon del contexto (es
    // un manejador con contador de referencias) para suspenderlo y
    // reanudarlo al conmutar de VT.
    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|()| "libinput: no pude asignar el seat")?;
    let libinput_handle = libinput.clone();
    handle
        .insert_source(LibinputInputBackend::new(libinput), |event, _meta, state| {
            state.handle_input(event);
        })
        .map_err(|e| format!("insert libinput: {e}"))?;

    // Clientes Wayland nuevos.
    handle
        .insert_source(
            Generic::new(listener, Interest::READ, CalloopMode::Level),
            |_readiness, listener, state| {
                while let Some(stream) = listener.accept()? {
                    eprintln!("mirada-compositor · cliente Wayland conectado.");
                    // PID del cliente para el linaje de las constelaciones.
                    let pid = crate::peer_pid(&stream);
                    let _ = state
                        .display
                        .handle()
                        .insert_client(stream, Arc::new(ClientState::with_pid(pid)));
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| format!("insert socket: {e}"))?;

    // Peticiones de los clientes ya conectados.
    let poll_fd = display.backend().poll_fd().try_clone_to_owned()?;
    handle
        .insert_source(
            Generic::new(poll_fd, Interest::READ, CalloopMode::Level),
            |_readiness, _fd, state| {
                let DrmState { display, app, .. } = state;
                if let Err(e) = display.dispatch_clients(app) {
                    eprintln!("mirada-compositor · dispatch: {e}");
                }
                let _ = display.flush_clients();
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| format!("insert display: {e}"))?;

    // Timer de composición + tareas — ~60 Hz.
    handle
        .insert_source(Timer::immediate(), |_instant, _meta, state| {
            state.tick();
            TimeoutAction::ToDuration(Duration::from_millis(16))
        })
        .map_err(|e| format!("insert timer: {e}"))?;

    // Tiquet del greeter (modo DM): al llegar, el traspaso a la sesión.
    // El hilo lector del greeter despierta el bucle por este canal.
    if let Some(rx) = greeter_rx {
        handle
            .insert_source(rx, |event, _, state: &mut DrmState| {
                if let TicketEvent::Msg(ticket) = event {
                    state.app.complete_greeter_handoff(ticket);
                }
            })
            .map_err(|e| format!("insert greeter: {e}"))?;
    }

    // Tope de tiempo opcional: `MIRADA_DRM_TIMEOUT=<segundos>` cierra el
    // compositor solo (0 o sin definir = sin tope). El teclado ya
    // funciona — `Super+Shift+e` o `Ctrl+C` son la salida normal.
    let timeout_secs: u64 = std::env::var("MIRADA_DRM_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    println!("──────────────────────────────────────────────────");
    println!("mirada-compositor · escritorio en marcha sobre «{out_name}».");
    println!("   Lanza un cliente:  WAYLAND_DISPLAY={socket_name} foot");
    println!("   Salir: Super+Shift+e  ·  o Ctrl+C en esta TTY.");
    if timeout_secs > 0 {
        println!("   Se cerrará solo a los {timeout_secs}s (MIRADA_DRM_TIMEOUT=0 lo quita).");
    }

    let font_path = app.config_font_path();
    let menu_entries = app.config_menu();
    let zones = app.config_zones();
    // Drag-to-zone estilo KDE: el snap se calcula por proximidad al borde
    // (esquinas→cuartos, arriba→maximizar/mitad-sup, abajo→mitad-inf,
    // izq/der→mitades). Siempre activo, no depende de la lista de zonas.
    println!("mirada-compositor · drag-to-zone: snap por bordes activo (KDE)");
    // Lista de presets: el 0 es `config.zones`, luego los de `zone_presets`.
    let mut zone_presets = vec![zones.clone()];
    zone_presets.extend(app.config_zone_presets());
    let dh = display.handle();
    // Próximo id a repartir en hotplug = uno más que el último inicial.
    let next_output_id = output_ctxs.len() as u32;
    let mut state = DrmState {
        app,
        session: session.clone(),
        display,
        drm,
        gbm: gbm.clone(),
        allocator: allocator.clone(),
        exporter: exporter.clone(),
        renderer_formats: renderer_formats.clone(),
        dh,
        outputs: output_ctxs,
        renderer,
        libinput: libinput_handle,
        active: true,
        watches,
        ctl,
        next_output_id,
        start: Instant::now(),
        last_windows: 0,
        last_active_ws: 0,
        last_focused_output: 0,
        shadows_on: std::env::var_os("MIRADA_SHADOW").is_some(),
        ws_slide: None,
        overview_anim: None,
        prev_overview_open: false,
        overview_tiles: Vec::new(),
        wp_images: Vec::new(),
        wp_index: 0,
        wp_next_switch_ms: 0,
        wp_dir: String::new(),
        cursor_id: Id::new(),
        last_pointer_window: None,
        output_size: (total_w as f64, total_h as f64),
        text: {
            let t = crate::text::TextRenderer::system(font_path.as_deref());
            if t.is_some() {
                println!("mirada-compositor · fuente de etiquetas cargada.");
            } else {
                eprintln!("mirada-compositor · sin fuente para etiquetas; no pinto títulos.");
            }
            t
        },
        text_cache: std::collections::HashMap::new(),
        menu_entries,
        root_menu: None,
        menu_output_idx: None,
        menu_window: None,
        last_titlebar_click: None,
        zones,
        zone_presets,
        active_preset: 0,
        drag_zone: None,
        preset_hud_until: None,
        preset_hud_label: String::new(),
    };

    let signal = event_loop.get_signal();
    event_loop
        .run(None, &mut state, |state| {
            let timed_out =
                timeout_secs > 0 && state.start.elapsed() > Duration::from_secs(timeout_secs);
            if !state.app.running || timed_out {
                if timed_out {
                    println!("mirada-compositor · tope de tiempo — cerrando.");
                }
                signal.stop();
            }
        })
        .map_err(|e| format!("el bucle de eventos falló: {e}"))?;

    // Sesión ajena pendiente: soltamos TODO —`drop(state)` cierra el
    // dispositivo DRM y `drop(event_loop)` libera el último clon de la
    // sesión libseat (cede el seat)— y recién entonces ejecutamos el otro
    // compositor, que ya puede tomar la GPU.
    let pending = state.app.pending_session.take();
    drop(state);
    drop(event_loop);
    if let Some((cmd, user)) = pending {
        crate::exec_session(&cmd, user.as_ref());
    }

    println!("mirada-compositor · adiós.");
    Ok(())
}
