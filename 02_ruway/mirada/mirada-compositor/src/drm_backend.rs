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
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, InputEvent, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::{render_elements, Id, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{ImportAll, ImportDma, ImportMem};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev;
use smithay::input::keyboard::FilterResult;
use smithay::input::pointer::{AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent};
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
    /// (búferes RGBA rasterizados — títulos, menú).
    Frame<R> where R: ImportAll + ImportMem;
    Window = WaylandSurfaceRenderElement<R>,
    Solid = SolidColorRenderElement,
    Text = MemoryRenderBufferRenderElement<R>,
}

/// Color de fondo del escritorio cuando no hay nada que lo tape.
const CLEAR_COLOR: [f32; 4] = [0.05, 0.05, 0.08, 1.0];

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

/// Fondo default cuando el usuario no configura `wallpaper_path`: un gradient
/// vertical sobrio (noche profunda → púrpura → azul-medianoche) generado
/// runtime, sin bytes embebidos en el binario. La idea es que arrancar mirada
/// "vacío" no se sienta como una pantalla muerta — un par de stops bastan
/// para que el escritorio respire.
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
    use mirada_brain::WallpaperFit;
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
    /// Contexto `libinput` — se suspende y reanuda al conmutar de VT.
    libinput: Libinput,
    /// `false` mientras la sesión está cedida a otra VT — no se compone.
    active: bool,
    /// Vigías de los archivos de config recargables en caliente.
    watches: crate::ConfigWatches,
    ctl: Option<crate::CtlServer>,
    /// Inicio del compositor — base de tiempos para los frame-callbacks.
    start: Instant,
    /// Nº de ventanas en el último `tick` — para registrar los cambios.
    last_windows: usize,
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
    /// Zonas de arrastre activas (fracciones del área útil) — el preset actual.
    zones: Vec<ZoneFrac>,
    /// Todos los presets de zonas (preset 0 = `config.zones`, luego
    /// `config.zone_presets`). `mirada-ctl cycle-zones` avanza entre ellos.
    zone_presets: Vec<Vec<ZoneFrac>>,
    /// Índice del preset activo dentro de [`Self::zone_presets`].
    active_preset: usize,
    /// Índice de la zona resaltada bajo el puntero durante un arrastre.
    drag_zone: Option<usize>,
    /// Instante hasta el que pintar el HUD del preset activo (al ciclar
    /// zonas). `None` = sin HUD. Se setea en cada ciclo y se respeta hasta
    /// que el reloj de `start` lo supera; el siguiente tick (~60 Hz) lo
    /// borra y damasca su área para el siguiente flip.
    preset_hud_until: Option<Instant>,
    /// Etiqueta cacheada del HUD (la última que se rasterizó). Permite
    /// reusar `text_cache` sin recomputar el texto en cada cuadro.
    preset_hud_label: String,
}

impl DrmState {
    /// Índice de la salida primaria en [`Self::outputs`]. Hoy hard-coded a 0
    /// (la primera descubierta); a futuro será configurable.
    const PRIMARY: usize = 0;

    /// Encuentra el índice de la salida cuyo CRTC es `crtc`, si existe.
    fn output_index_by_crtc(
        &self,
        crtc: smithay::reexports::drm::control::crtc::Handle,
    ) -> Option<usize> {
        self.outputs.iter().position(|o| o.crtc == crtc)
    }

    /// Índice de la salida que contiene el punto global `(gx, gy)`. Si el
    /// punto cae en zona muerta entre rects (puede pasar con salidas de
    /// distinto tamaño dispuestas side-by-side), devuelve la primaria.
    fn output_at_point(&self, gx: i32, gy: i32) -> usize {
        self.outputs
            .iter()
            .position(|o| {
                gx >= o.rect.x
                    && gy >= o.rect.y
                    && gx < o.rect.x + o.rect.w
                    && gy < o.rect.y + o.rect.h
            })
            .unwrap_or(Self::PRIMARY)
    }

    /// Acota un punto al **interior de algún output**, respetando la geometría
    /// real. Si `(x, y)` ya cae sobre un output, se devuelve tal cual; si no
    /// (zona muerta entre rects de distinto tamaño), se proyecta al borde
    /// del output euclídeamente más cercano. Sin esto, el cursor podía
    /// quedar atrapado en zonas que ninguna salida pinta — el usuario lo ve
    /// como un cursor fantasma sobre fondo negro.
    fn clamp_to_outputs(&self, x: f64, y: f64) -> (f64, f64) {
        let xi = x.round() as i32;
        let yi = y.round() as i32;
        if self.outputs.iter().any(|o| {
            xi >= o.rect.x
                && yi >= o.rect.y
                && xi < o.rect.x + o.rect.w
                && yi < o.rect.y + o.rect.h
        }) {
            return (x, y);
        }
        // El menor cuadrado-distancia al rect proyecta `(x, y)` al borde.
        let Some(first) = self.outputs.first() else {
            return (x, y); // sin monitores conectados: nada a lo que recortar
        };
        let mut best = (first.rect, f64::INFINITY);
        for o in &self.outputs {
            let r = o.rect;
            if r.w <= 0 || r.h <= 0 {
                continue;
            }
            let cx = x.clamp(r.x as f64, (r.x + r.w - 1) as f64);
            let cy = y.clamp(r.y as f64, (r.y + r.h - 1) as f64);
            let d = (x - cx).powi(2) + (y - cy).powi(2);
            if d < best.1 {
                best = (r, d);
            }
        }
        let r = best.0;
        (
            x.clamp(r.x as f64, (r.x + r.w - 1) as f64),
            y.clamp(r.y as f64, (r.y + r.h - 1) as f64),
        )
    }

    /// El área útil (rect menos reservas) **de una salida concreta**: a las
    /// layers exclusivas de su `layer_map` se le suma, sólo en la primaria,
    /// la franja del shell (pata). Devuelve rect en coords globales — el
    /// teselado y las zonas de arrastre lo usan como dominio efectivo.
    fn output_work_rect(&self, idx: usize) -> Rect {
        // `output_at_point` cae a `PRIMARY` cuando el punto no toca ninguna
        // salida — incluido el caso de 0 monitores, donde `outputs` está
        // vacío. Y un `idx` de antes de un desenchufe puede quedar fuera de
        // rango. En ambos casos el dominio de zonas degenera al tamaño lógico,
        // sin reservas: no hay panic ni salida a la que recortar.
        let Some(o) = self.outputs.get(idx) else {
            return Rect::new(0, 0, self.output_size.0 as i32, self.output_size.1 as i32);
        };
        // Layers exclusivas de ESTA salida: la zona "no exclusiva" da los
        // insets directos.
        let z = smithay::desktop::layer_map_for_output(&o.output).non_exclusive_zone();
        let mut top = z.loc.y.max(0);
        let mut left = z.loc.x.max(0);
        let mut right = (o.rect.w - (z.loc.x + z.size.w)).max(0);
        let mut bottom = (o.rect.h - (z.loc.y + z.size.h)).max(0);
        // El dock del shell (pata) sólo se descuenta en la primaria.
        if idx == Self::PRIMARY {
            let (rt, rb, rl, rr) = self.app.reserved;
            // Las reservas del shell ya están sumadas en `self.app.reserved`
            // (recompute_reservations las publica), pero `app.reserved`
            // incluye también las de layer-shell (no podemos restarlas
            // limpiamente). Tomamos `max` para no doble-contar: la mayor
            // gana, y el shell (que es la suma) cubre los dos casos.
            top = top.max(rt);
            bottom = bottom.max(rb);
            left = left.max(rl);
            right = right.max(rr);
        }
        Rect::new(
            o.rect.x + left,
            o.rect.y + top,
            (o.rect.w - left - right).max(1),
            (o.rect.h - top - bottom).max(1),
        )
    }

    /// Compone un cuadro por cada salida y avisa a los clientes una sola vez.
    /// Si una salida tiene su `pending_flip` puesto, se saltea hasta el
    /// próximo VBlank. Refresca los búferes de marco una vez al principio.
    fn render(&mut self) {
        if !self.active {
            return; // la sesión está en otra VT — no tocamos la GPU
        }
        self.refresh_window_borders();
        for i in 0..self.outputs.len() {
            self.render_output(i);
        }
        self.send_frames_to_clients();
    }

    /// Si el puntero global cae sobre `rect`, emite el cursor en coordenadas
    /// **locales** a `rect`. Si el cliente publicó una superficie de cursor,
    /// usa esa; si no, el cuadrado por defecto. `Hidden` o puntero fuera del
    /// rect no emiten nada.
    fn emit_cursor(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        let (cx, cy) = self.app.pointer_loc;
        let (cxi, cyi) = (cx.round() as i32, cy.round() as i32);
        if cxi < rect.x || cyi < rect.y || cxi >= rect.x + rect.w || cyi >= rect.y + rect.h {
            return;
        }
        match &self.app.cursor_status {
            CursorImageStatus::Hidden => {}
            CursorImageStatus::Surface(surface) if surface.alive() => {
                let (hx, hy) = crate::cursor_hotspot(surface);
                let loc = (cxi - rect.x - hx, cyi - rect.y - hy);
                for el in render_elements_from_surface_tree(
                    &mut self.renderer,
                    surface,
                    loc,
                    1.0,
                    1.0,
                    Kind::Cursor,
                ) {
                    into.push(Frame::Window(el));
                }
            }
            _ => {
                let cursor_rect = Rectangle::new(
                    Point::<i32, Physical>::from((cxi - rect.x, cyi - rect.y)),
                    Size::<i32, Physical>::from((CURSOR_SIZE, CURSOR_SIZE)),
                );
                into.push(Frame::Solid(SolidColorRenderElement::new(
                    self.cursor_id.clone(),
                    cursor_rect,
                    CommitCounter::default(),
                    CURSOR_COLOR,
                    Kind::Cursor,
                )));
            }
        }
    }

    /// Emite todas las ventanas visibles cuya posición global intersecta `rect`,
    /// traducidas a coordenadas locales a `rect`. Incluye marcos, barras de
    /// título y el árbol de superficie del cliente, en orden front-to-back
    /// (`shell` arriba > flotantes > teseladas). Se saltea ventanas que no
    /// caen sobre `rect` para no malgastar trabajo del compositor.
    fn emit_windows(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        let mut shown: Vec<_> = self.app.windows.iter().filter(|w| w.visible).collect();
        shown.sort_by_key(|w| (!w.is_shell, !w.floating, !w.focused));
        let tbh = self.app.decorations.titlebar_height;
        // `render_loc` necesita el alto de la "salida lógica" sólo para anclar
        // el shell al borde inferior. En multi-monitor esa salida es la
        // primaria (output 0); el shell vive ahí.
        let primary_h = self.outputs[Self::PRIMARY].rect.h;
        for w in &shown {
            let tb = crate::titlebar_for(w, tbh);
            let (gx, gy) = crate::render_loc(w, primary_h, tbh);
            let (sw, sh) = crate::surface_px_size(w).unwrap_or((w.size.0, (w.size.1 - tb).max(1)));
            // Rect decorado en coords globales (incluye barra + superficie).
            let gxd = gx;
            let gyd = gy - tb;
            let gwd = sw;
            let ghd = sh + tb;
            // Filtrar por intersección con `rect`.
            if gxd + gwd <= rect.x
                || gyd + ghd <= rect.y
                || gxd >= rect.x + rect.w
                || gyd >= rect.y + rect.h
            {
                continue;
            }
            // Posición local de la superficie y de la decoración.
            let x = gx - rect.x;
            let y = gy - rect.y;
            let dec_y = y - tb;
            let dec_h = sh + tb;

            if tb > 0 {
                if let Some(tr) = &self.text {
                    if !w.title.is_empty() {
                        if self.text_cache.len() > 256 {
                            self.text_cache.clear();
                        }
                        let buf = self
                            .text_cache
                            .entry((w.title.clone(), TITLE_COLOR))
                            .or_insert_with(|| title_buffer(tr, &w.title));
                        let ty = dec_y + (tb - TITLE_PX as i32) / 2;
                        if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                            &mut self.renderer,
                            ((x + 8) as f64, ty as f64),
                            buf,
                            None,
                            None,
                            None,
                            Kind::Unspecified,
                        ) {
                            into.push(Frame::Text(el));
                        }
                    }
                }
                let color = rgba_f32(if w.focused {
                    self.app.decorations.border_focus
                } else {
                    self.app.decorations.border_normal
                });
                let mut bar = SolidColorBuffer::default();
                bar.update((sw, tb), color);
                into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                    &bar,
                    (x, dec_y),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                )));
            } else if w.focused && !w.is_shell && !w.title.is_empty() {
                if let Some(tr) = &self.text {
                    if self.text_cache.len() > 256 {
                        self.text_cache.clear();
                    }
                    let buf = self
                        .text_cache
                        .entry((w.title.clone(), TITLE_COLOR))
                        .or_insert_with(|| title_buffer(tr, &w.title));
                    if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                        &mut self.renderer,
                        ((x + 6) as f64, (y + 4) as f64),
                        buf,
                        None,
                        None,
                        None,
                        Kind::Unspecified,
                    ) {
                        into.push(Frame::Text(el));
                    }
                }
            }
            if !w.is_shell && self.app.decorations.border_width > 0 {
                let rects = border_rects(x, dec_y, sw, dec_h, self.app.decorations.border_width);
                for (buf, (bx, by, _, _)) in w.borders.iter().zip(rects) {
                    into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        buf,
                        (bx, by),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )));
                }
            }
            for el in render_elements_from_surface_tree(
                &mut self.renderer,
                &w.surface,
                (x, y),
                1.0,
                1.0,
                Kind::Unspecified,
            ) {
                into.push(Frame::Window(el));
            }
        }
    }

    /// Emite el HUD del preset activo en la salida `rect` — un panel discreto
    /// arriba al centro de la salida (no del escritorio global) mientras dure
    /// la ventana de feedback. Si el deadline pasó, limpia el estado. Llamar
    /// sólo en la salida dueña del HUD (hoy la primaria).
    fn emit_hud(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        let Some(deadline) = self.preset_hud_until else {
            return;
        };
        if Instant::now() >= deadline {
            self.preset_hud_until = None;
            return;
        }
        let Some(tr) = &self.text else { return };
        if self.preset_hud_label.is_empty() {
            return;
        }
        let Some(r) = tr.rasterize(&self.preset_hud_label, HUD_TEXT_PX, HUD_TEXT_COLOR) else {
            return;
        };
        let tw = r.width;
        let th = r.height;
        let panel_w = tw + 2 * HUD_PAD;
        let panel_h = th.max(HUD_TEXT_PX as i32) + 2 * HUD_PAD;
        // Centra el panel en el ancho de la salida (no del escritorio total),
        // en coords locales — el frame de esta salida arranca en (0,0).
        let panel_x = ((rect.w - panel_w) / 2).max(0);
        let panel_y = HUD_TOP;
        let tx = panel_x + (panel_w - tw) / 2;
        let ty = panel_y + (panel_h - th) / 2;
        let buf = MemoryRenderBuffer::from_slice(
            &r.rgba,
            Fourcc::Argb8888,
            (tw, th),
            1,
            Transform::Normal,
            None,
        );
        if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
            &mut self.renderer,
            (tx as f64, ty as f64),
            &buf,
            None,
            None,
            None,
            Kind::Unspecified,
        ) {
            into.push(Frame::Text(el));
        }
        let mut bg = SolidColorBuffer::default();
        bg.update((panel_w, panel_h), HUD_BG);
        into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
            &bg,
            (panel_x, panel_y),
            1.0,
            1.0,
            Kind::Unspecified,
        )));
    }

    /// Emite el overlay de zonas de arrastre (drag-to-zone) — visible sólo
    /// durante un arrastre Move/Tile. Las zonas se escalan al monitor bajo
    /// el puntero y se emiten traducidas a coords locales de `rect`. Si las
    /// zonas no caen sobre `rect` (drag en otro monitor), no emite nada.
    fn emit_zone_overlay(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        let drag_mode = self.app.drag.as_ref().map(|d| d.mode);
        if !matches!(drag_mode, Some(DragMode::Move) | Some(DragMode::Tile)) {
            return;
        }
        if self.zones.is_empty() {
            return;
        }
        let wr = self.work_rect();
        // Las zonas son del monitor bajo el puntero — si ese no es esta
        // salida, no las pintamos acá.
        if wr.x + wr.w <= rect.x
            || wr.y + wr.h <= rect.y
            || wr.x >= rect.x + rect.w
            || wr.y >= rect.y + rect.h
        {
            return;
        }
        let acc = self.app.decorations.border_focus;
        let fill = |a: f32| {
            [
                acc[0] as f32 / 255.0,
                acc[1] as f32 / 255.0,
                acc[2] as f32 / 255.0,
                a,
            ]
        };
        for (i, z) in self.zones.iter().enumerate() {
            let r = z.to_rect(wr);
            let color = if Some(i) == self.drag_zone { fill(0.40) } else { fill(0.16) };
            let mut buf = SolidColorBuffer::default();
            buf.update((r.w, r.h), color);
            into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                &buf,
                (r.x - rect.x, r.y - rect.y),
                1.0,
                1.0,
                Kind::Unspecified,
            )));
        }
    }

    /// Emite el menú raíz en `rect` si esta salida es la dueña del menú.
    /// El menú vive en **coords locales** de su salida (se abrió ahí), así
    /// que las posiciones de las columnas no necesitan traducción.
    fn emit_menu(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        let Some(m) = self.root_menu.as_ref() else { return };
        // El menú se rasteriza con el puntero **local** a su salida — así
        // resaltado y hover apuntan a la fila correcta.
        let (px, py) = self.app.pointer_loc;
        let cols = m.render(px.round() as i32 - rect.x, py.round() as i32 - rect.y);
        let menu_hl_color = rgba_f32(self.app.decorations.border_focus);
        if self.text_cache.len() > 256 {
            self.text_cache.clear();
        }
        for col in cols.iter().rev() {
            // Texto (caché).
            if let Some(tr) = &self.text {
                for row in &col.rows {
                    let label = if row.submenu {
                        format!("{}   ›", row.label)
                    } else {
                        row.label.clone()
                    };
                    let buf = self
                        .text_cache
                        .entry((label.clone(), MENU_TEXT_COLOR))
                        .or_insert_with(|| {
                            match tr.rasterize(&label, MENU_TEXT_PX, MENU_TEXT_COLOR) {
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
                        });
                    let ty = row.y + (crate::menu::ITEM_H - MENU_TEXT_PX as i32) / 2;
                    if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                        &mut self.renderer,
                        ((row.x + 10) as f64, ty as f64),
                        buf,
                        None,
                        None,
                        None,
                        Kind::Unspecified,
                    ) {
                        into.push(Frame::Text(el));
                    }
                }
            }
            // Resaltado.
            for row in &col.rows {
                if row.highlighted {
                    let mut hl = SolidColorBuffer::default();
                    hl.update((col.w, crate::menu::ITEM_H), menu_hl_color);
                    into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &hl,
                        (row.x, row.y),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )));
                }
            }
            // Fondo.
            let mut bg = SolidColorBuffer::default();
            bg.update((col.w, col.h), MENU_BG);
            into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                &bg,
                (col.x, col.y),
                1.0,
                1.0,
                Kind::Unspecified,
            )));
        }
    }

    /// Emite la pista de revelado del dock autoescondido — una franja fina en
    /// el borde anclado mientras está oculto. Sólo en la salida donde vive
    /// el shell (primaria).
    fn emit_reveal_band(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        if !(crate::shell_dock().autohide && self.app.shell_hidden) {
            return;
        }
        let (ow, oh) = (rect.w, rect.h);
        if ow <= 0 || oh <= 0 {
            return;
        }
        let dock = crate::shell_dock();
        let limite = if dock.anchor.es_horizontal() { oh } else { ow };
        let t = dock.thickness.clamp(1, limite.max(1));
        let (bx, by, bw, bh) =
            crate::shell_reveal_band(dock.anchor, ow, oh, t, crate::SHELL_REVEAL_BAND);
        let menu_hl_color = rgba_f32(self.app.decorations.border_focus);
        let mut band = SolidColorBuffer::default();
        band.update((bw, bh), menu_hl_color);
        into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
            &band,
            (bx, by),
            1.0,
            1.0,
            Kind::Unspecified,
        )));
    }

    /// Emite el wallpaper de la salida `idx` al fondo (rearmándolo si quedó
    /// stale). Cada salida tiene su propio búfer escalado, su propia ruta y
    /// su propio modo de ajuste — un override por nombre puede pintarle un
    /// fondo distinto a cada monitor.
    fn emit_wallpaper(&mut self, idx: usize, into: &mut Vec<Frame<GlesRenderer>>) {
        let ctx = &mut self.outputs[idx];
        let path = ctx.wallpaper_path.clone();
        let fit = ctx.wallpaper_fit;
        let size = (ctx.rect.w, ctx.rect.h);
        if size.0 <= 0 || size.1 <= 0 {
            return;
        }
        let stale = ctx
            .wallpaper
            .as_ref()
            .map(|(_, s)| *s != size)
            .unwrap_or(true);
        if stale {
            ctx.wallpaper = match path {
                Some(p) => load_wallpaper(&p, fit, size.0, size.1).map(|b| (b, size)),
                None => Some((make_default_wallpaper(size.0, size.1), size)),
            };
        }
        let Some((buf, _)) = &ctx.wallpaper else {
            return;
        };
        if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
            &mut self.renderer,
            (0.0, 0.0),
            buf,
            None,
            None,
            None,
            Kind::Unspecified,
        ) {
            into.push(Frame::Text(el));
        }
    }

    /// Refresca los búferes de marco (color por foco) de todas las ventanas
    /// visibles. Es estado global que no depende de cuál salida se está
    /// rindiendo — se hace una vez al inicio de [`Self::render`].
    fn refresh_window_borders(&mut self) {
        let dec = self.app.decorations;
        let Some(primary) = self.outputs.get(Self::PRIMARY) else {
            return; // sin monitores: no hay bordes que recalcular
        };
        let output_h = primary.rect.h;
        for w in &mut self.app.windows {
            if !w.visible || w.is_shell {
                continue;
            }
            let tb = crate::titlebar_for(w, dec.titlebar_height);
            let (x, y) = crate::render_loc(w, output_h, dec.titlebar_height);
            let (sw, sh) = crate::surface_px_size(w).unwrap_or((w.size.0, w.size.1 - tb));
            let (x, y, sh) = (x, y - tb, sh + tb);
            let color = rgba_f32(if w.focused {
                dec.border_focus
            } else {
                dec.border_normal
            });
            let rects = border_rects(x, y, sw, sh, dec.border_width);
            for (buf, (_, _, bw, bh)) in w.borders.iter_mut().zip(rects) {
                buf.update((bw, bh), color);
            }
        }
    }

    /// Avisa a cada cliente (ventanas, layers de la primaria, cursor) de
    /// que puede dibujar el siguiente cuadro. Se llama una sola vez por
    /// `render`, no por salida.
    fn send_frames_to_clients(&mut self) {
        let time = self.start.elapsed().as_millis() as u32;
        for w in &mut self.app.windows {
            w.frame_tick = w.frame_tick.wrapping_add(1);
            // Las capas dormidas (zoom-Z) no reciben frame callbacks.
            if w.suspended {
                continue;
            }
            // Throttle de fondo: 1 de cada `frame_divisor` vblanks.
            let div = w.frame_divisor.max(1);
            if div > 1 && w.frame_tick % div != 0 {
                continue;
            }
            send_frames_surface_tree(&w.surface, time);
        }
        // Layers de TODAS las salidas — un cliente puede tener barras en
        // distintos monitores, cada una con su frame-callback propio.
        for output in self.app.outputs.clone() {
            for layer in smithay::desktop::layer_map_for_output(&output).layers() {
                send_frames_surface_tree(layer.wl_surface(), time);
            }
        }
        if let CursorImageStatus::Surface(surface) = &self.app.cursor_status {
            if surface.alive() {
                send_frames_surface_tree(surface, time);
            }
        }
    }

    /// Render unificado de una salida. Cada feature decide si pertenece o
    /// no a esta salida (gates por dueño) — el cursor en la del puntero,
    /// HUD/layer-shell/reveal-band en primaria, menú y zonas en la salida
    /// donde se inició la acción, ventanas y wallpaper en todas.
    fn render_output(&mut self, idx: usize) {
        if self.outputs[idx].pending_flip {
            return;
        }
        let rect = self.outputs[idx].rect;
        let is_primary = idx == Self::PRIMARY;
        let owns_menu = self.menu_output_idx == Some(idx);

        let elements: Vec<Frame<GlesRenderer>> = {
            let mut out: Vec<Frame<GlesRenderer>> = Vec::new();

            // 1. Cursor (si el puntero cae sobre esta salida).
            self.emit_cursor(rect, &mut out);

            // 2. HUD del preset (primaria por ahora; centrado en su rect).
            if is_primary {
                self.emit_hud(rect, &mut out);
            }

            // 3. Zonas de arrastre — en la salida bajo el puntero durante
            //    un drag (helper filtra por intersección de work-rect).
            self.emit_zone_overlay(rect, &mut out);

            // 4. Menú raíz — sólo en la salida donde se abrió.
            if owns_menu {
                self.emit_menu(rect, &mut out);
            }

            // 5. Pista de revelado del dock autoescondido — primaria, el
            //    shell vive ahí.
            if is_primary {
                self.emit_reveal_band(rect, &mut out);
            }

            // 6. Layer surfaces (waybar, swaybg…) de **esta** salida —
            //    smithay las cuelga por output, así que un layer mapeado a
            //    una secundaria con `output_hint` aparece donde toca.
            let output_for_layers = self.outputs[idx].output.clone();
            let (over_layers, under_layers) =
                crate::layer_render_elements(Some(&output_for_layers), &mut self.renderer);
            for el in over_layers {
                out.push(Frame::Window(el));
            }
            // 7. Ventanas entre layers Overlay/Top y Bottom/Background.
            self.emit_windows(rect, &mut out);
            for el in under_layers {
                out.push(Frame::Window(el));
            }

            // 8. Wallpaper al fondo (por salida).
            self.emit_wallpaper(idx, &mut out);

            out
        };

        let ctx = &mut self.outputs[idx];
        match ctx.compositor.render_frame::<_, _>(
            &mut self.renderer,
            &elements,
            CLEAR_COLOR,
            FrameFlags::DEFAULT,
        ) {
            Ok(result) => {
                if !result.is_empty {
                    match ctx.compositor.queue_frame(()) {
                        Ok(()) => ctx.pending_flip = true,
                        Err(e) => eprintln!(
                            "mirada-compositor · queue_frame[{}]: {e}",
                            ctx.name
                        ),
                    }
                }
            }
            Err(e) => eprintln!(
                "mirada-compositor · render_frame[{}]: {e}",
                ctx.name
            ),
        }

        // Capturas screencopy pendientes de esta salida: el framebuffer real
        // vive dentro del DrmCompositor, así que se re-componen los mismos
        // elementos en un offscreen y se copia de ahí.
        if !self.app.pending_screencopy.is_empty() {
            let output = self.outputs[idx].output.clone();
            let capturas =
                crate::screencopy::tomar_capturas(&mut self.app, &output, (rect.x, rect.y));
            if !capturas.is_empty() {
                crate::screencopy::servir_offscreen(
                    &mut self.renderer,
                    (rect.w, rect.h),
                    &elements,
                    CLEAR_COLOR.into(),
                    capturas,
                );
            }
        }
    }

    /// La sesión se cede a otra VT (`Ctrl+Alt+Fn`): suelta la GPU y deja
    /// de leer el ratón y el teclado, para no chocar con quien ahora
    /// manda en la pantalla.
    fn pause_session(&mut self) {
        self.active = false;
        self.drm.pause();
        self.libinput.suspend();
        println!("mirada-compositor · sesión cedida a otra VT.");
    }

    /// La sesión vuelve a esta VT: recupera la GPU y la entrada, reinicia
    /// el estado de cada compositor y repinta.
    fn resume_session(&mut self) {
        if self.libinput.resume().is_err() {
            eprintln!("mirada-compositor · libinput.resume falló.");
        }
        if let Err(e) = self.drm.activate(false) {
            eprintln!("mirada-compositor · drm.activate falló: {e}");
        }
        for ctx in &mut self.outputs {
            if let Err(e) = ctx.compositor.reset_state() {
                eprintln!(
                    "mirada-compositor · compositor.reset_state[{}]: {e}",
                    ctx.name
                );
            }
            ctx.pending_flip = false;
        }
        self.active = true;
        self.render();
        println!("mirada-compositor · sesión recuperada.");
    }

    /// Tarea periódica: Cerebro enlazado, recarga del keymap, API de
    /// control, composición y vaciado hacia los clientes.
    fn tick(&mut self) {
        self.app.brain_poll();

        let n = self.app.windows.len();
        if n != self.last_windows {
            eprintln!("mirada-compositor · ventanas en pantalla: {n}");
            self.last_windows = n;
        }

        // Recarga en caliente de keymap/config/reglas si cambiaron en disco.
        // Si la config general cambió, refresca las cachés que el Cuerpo deriva
        // de ella (menú raíz, wallpaper, fuente de etiquetas) — el Cerebro ya
        // aplicó teselado/decoración/foco.
        if self.watches.poll(&mut self.app) {
            self.menu_entries = self.app.config_menu();
            // Reconstruye los presets de zonas y reacota el activo.
            let mut presets = vec![self.app.config_zones()];
            presets.extend(self.app.config_zone_presets());
            self.zone_presets = presets;
            if self.active_preset >= self.zone_presets.len() {
                self.active_preset = 0;
            }
            self.zones = self.zone_presets.get(self.active_preset).cloned().unwrap_or_default();
            self.root_menu = None; // un menú abierto puede quedar obsoleto
            self.menu_output_idx = None;
            // Config nueva (wallpaper, fuente, menú): todo puede repintarse.
            crate::screencopy::danar_todo(&mut self.app);
            // Refresca el wallpaper por salida: cada `OutputCtx` resuelve su
            // ruta y su `fit` por nombre del conector (override o global).
            for ctx in &mut self.outputs {
                let new_wp = self.app.config_wallpaper_path_for(&ctx.name);
                let new_fit = self.app.config_wallpaper_fit_for(&ctx.name);
                if new_wp != ctx.wallpaper_path || new_fit != ctx.wallpaper_fit {
                    ctx.wallpaper_path = new_wp;
                    ctx.wallpaper_fit = new_fit;
                    ctx.wallpaper = None; // se rearma en el próximo render
                }
            }
            self.text = crate::text::TextRenderer::system(self.app.config_font_path().as_deref());
        }

        if let Some(ctl) = &self.ctl {
            while let Some(mut conn) = ctl.poll() {
                let reply = match conn.read_request() {
                    // El ciclo de zonas es estado del Cuerpo (DRM): lo atendemos
                    // acá, no en el Cerebro. Avanza al siguiente preset.
                    Ok(Some(CtlRequest::CycleZones)) => {
                        if !self.zone_presets.is_empty() {
                            self.active_preset =
                                (self.active_preset + 1) % self.zone_presets.len();
                            self.zones = self.zone_presets[self.active_preset].clone();
                            self.preset_hud_label = format!(
                                "Zonas · {}/{}",
                                self.active_preset + 1,
                                self.zone_presets.len()
                            );
                            self.preset_hud_until = Some(Instant::now() + HUD_DURATION);
                        }
                        CtlReply::Ok
                    }
                    Ok(Some(req)) => self.app.serve_ctl(req),
                    Ok(None) => continue,
                    Err(e) => CtlReply::Error(format!("{e}")),
                };
                let _ = conn.reply(&reply);
            }
        }

        self.render();
        let _ = self.display.flush_clients();
    }

    /// Procesa un evento de `libinput`: teclado y puntero.
    fn handle_input(&mut self, event: InputEvent<LibinputInputBackend>) {
        let time = self.start.elapsed().as_millis() as u32;
        match event {
            // --- Teclado: intercepta los atajos del Cerebro --------------
            InputEvent::Keyboard { event } => {
                let Some(keyboard) = self.app.keyboard.clone() else {
                    return;
                };
                let code = event.key_code();
                let key_state = event.state();
                let pressed = key_state == KeyState::Pressed;
                keyboard.input::<(), _>(
                    &mut self.app,
                    code,
                    key_state,
                    SERIAL_COUNTER.next_serial(),
                    time,
                    |st, mods, handle| {
                        if !pressed {
                            return FilterResult::Forward;
                        }
                        let sym = handle.modified_sym();
                        // Conmutar de VT (Ctrl+Alt+Fn o XF86Switch_VT_n). Lo
                        // aplica el backend tras el evento (sólo él tiene la
                        // sesión). Se chequea a nivel de keysym, antes del
                        // combo, porque según el keymap no llega como «Fn».
                        if let Some(vt) = crate::vt_target(mods, sym) {
                            st.pending_vt = Some(vt);
                            return FilterResult::Intercept(());
                        }
                        if let Some(combo) = combo_string(mods, sym) {
                            if crate::is_escape_hatch(&combo) {
                                eprintln!(
                                    "mirada-compositor · salida de emergencia ({combo})."
                                );
                                st.running = false;
                                return FilterResult::Intercept(());
                            }
                            if st.grabs.contains(&combo) {
                                st.pending_keybind = Some(combo);
                                return FilterResult::Intercept(());
                            }
                        }
                        FilterResult::Forward
                    },
                );
                if let Some(combo) = self.app.pending_keybind.take() {
                    let ev = self.app.body.keybind(combo);
                    self.app.brain_feed(ev);
                }
                if let Some(vt) = self.app.pending_vt.take() {
                    if let Err(e) = self.session.change_vt(vt) {
                        eprintln!("mirada-compositor · no pude conmutar a VT{vt}: {e}");
                    }
                }
            }

            // --- Puntero: movimiento relativo (ratón, touchpad) ----------
            InputEvent::PointerMotion { event } => {
                let (x0, y0) = self.app.pointer_loc;
                // Pre-acotado al bounding box: descarta los outliers extremos
                // sin hacer rondas innecesarias en `clamp_to_outputs`.
                let x = (x0 + event.delta_x()).clamp(0.0, self.output_size.0);
                let y = (y0 + event.delta_y()).clamp(0.0, self.output_size.1);
                // Proyectado al output más cercano si cayó en zona muerta.
                let (x, y) = self.clamp_to_outputs(x, y);
                self.app.pointer_loc = (x, y);
                if self.root_menu.is_some() {
                    // El menú vive en coords locales a su salida. Si esa salida
                    // se desenchufó mientras estaba abierto, el idx queda viejo:
                    // cerramos el menú en vez de indexar fuera de rango.
                    let idx = self.menu_output_idx.unwrap_or(Self::PRIMARY);
                    let Some(r) = self.outputs.get(idx).map(|o| o.rect) else {
                        self.root_menu = None;
                        self.menu_output_idx = None;
                        return;
                    };
                    let lx = x.round() as i32 - r.x;
                    let ly = y.round() as i32 - r.y;
                    self.root_menu.as_mut().unwrap().update_hover(lx, ly);
                    return; // con el menú abierto, el puntero lo navega
                }
                self.app.update_shell_autohide(x, y);
                if !self.drag_update() {
                    self.pointer_motion(time);
                }
            }

            // --- Puntero: movimiento absoluto (táctil, tableta) ----------
            InputEvent::PointerMotionAbsolute { event } => {
                let space = Size::<i32, Logical>::from((
                    self.output_size.0 as i32,
                    self.output_size.1 as i32,
                ));
                let pos = event.position_transformed(space);
                let x = pos.x.clamp(0.0, self.output_size.0);
                let y = pos.y.clamp(0.0, self.output_size.1);
                self.app.pointer_loc = self.clamp_to_outputs(x, y);
                if self.root_menu.is_some() {
                    let (x, y) = self.app.pointer_loc;
                    let idx = self.menu_output_idx.unwrap_or(Self::PRIMARY);
                    let Some(r) = self.outputs.get(idx).map(|o| o.rect) else {
                        self.root_menu = None;
                        self.menu_output_idx = None;
                        return;
                    };
                    let lx = x.round() as i32 - r.x;
                    let ly = y.round() as i32 - r.y;
                    self.root_menu.as_mut().unwrap().update_hover(lx, ly);
                    return; // con el menú abierto, el puntero lo navega
                }
                let (x, y) = self.app.pointer_loc;
                self.app.update_shell_autohide(x, y);
                if !self.drag_update() {
                    self.pointer_motion(time);
                }
            }

            // --- Puntero: botones ----------------------------------------
            InputEvent::PointerButton { event } => {
                let pressed = event.state() == ButtonState::Pressed;
                let button = event.button_code();

                // Menú raíz abierto: el botón se lo come el menú. Click
                // izquierdo sobre una hoja la lanza y cierra; sobre una
                // fila-submenú la abre y sigue; click derecho o fuera cierra.
                // (Sólo al apretar; soltar no hace nada.)
                if pressed && self.root_menu.is_some() {
                    use crate::menu::ClickResult;
                    let (x, y) = self.app.pointer_loc;
                    let idx = self.menu_output_idx.unwrap_or(Self::PRIMARY);
                    let Some(r) = self.outputs.get(idx).map(|o| o.rect) else {
                        self.root_menu = None;
                        self.menu_output_idx = None;
                        return;
                    };
                    let lx = x.round() as i32 - r.x;
                    let ly = y.round() as i32 - r.y;
                    let res = if button == BTN_LEFT {
                        self.root_menu.as_mut().unwrap().click(lx, ly)
                    } else {
                        ClickResult::Close
                    };
                    match res {
                        ClickResult::Launch(cmd) => {
                            self.root_menu = None;
                            self.menu_output_idx = None;
                            self.app.spawn_user(&cmd);
                        }
                        ClickResult::Stay => {}
                        ClickResult::Close => {
                            self.root_menu = None;
                            self.menu_output_idx = None;
                        }
                    }
                    // El click cambió el menú (abrió submenú o lo cerró):
                    // daño para screencopy. Grueso pero raro.
                    crate::screencopy::danar_todo(&mut self.app);
                    return; // el menú captura el botón
                }

                // Click DERECHO sobre el fondo (sin ventana ni `Super`): abre el
                // menú raíz, si hay entradas configuradas. No aplica en greeter.
                if pressed
                    && button == BTN_RIGHT
                    && !self.menu_entries.is_empty()
                    && self.app.mode != BodyMode::Greeter
                {
                    let super_held = self
                        .app
                        .keyboard
                        .as_ref()
                        .is_some_and(|kb| kb.modifier_state().logo);
                    let (x, y) = self.app.pointer_loc;
                    if !super_held && self.window_at(x, y).is_none() {
                        // El menú vive en el monitor donde se hizo el click; su
                        // origen y su rect de acotamiento son **locales** a ese
                        // monitor — así no se sale del borde de su pantalla.
                        let idx = self.output_at_point(x.round() as i32, y.round() as i32);
                        // Sin salida real (0 monitores) no hay dónde anclar el
                        // menú: no lo abrimos en vez de indexar fuera de rango.
                        let Some(r) = self.outputs.get(idx).map(|o| o.rect) else {
                            return;
                        };
                        self.menu_output_idx = Some(idx);
                        self.root_menu = Some(crate::menu::RootMenu::open(
                            x.round() as i32 - r.x,
                            y.round() as i32 - r.y,
                            self.menu_entries.clone(),
                            r.w,
                            r.h,
                        ));
                        // El menú aparece en pantalla: daño para screencopy.
                        crate::screencopy::danar_todo(&mut self.app);
                        return; // el botón abrió el menú, no va al cliente
                    }
                }

                // ¿Empieza un arrastre? `Super`+botón sobre una ventana:
                // izquierdo mueve, derecho redimensiona. En modo greeter no
                // hay arrastre: el login está clavado a pantalla completa.
                if pressed && self.app.drag.is_none() && self.app.mode != BodyMode::Greeter {
                    let super_held = self
                        .app
                        .keyboard
                        .as_ref()
                        .is_some_and(|kb| kb.modifier_state().logo);
                    // `Super`+izquierdo arrastra: una flotante se mueve, una
                    // teselada se reordena (swap). `Super`+derecho redimensiona
                    // (flotando la ventana si estaba teselada).
                    let (x, y) = self.app.pointer_loc;
                    let hit = self.window_at(x, y);
                    let mode = match (button, hit) {
                        (BTN_LEFT, Some(i)) if super_held => Some(if self.app.windows[i].floating {
                            DragMode::Move
                        } else {
                            DragMode::Tile
                        }),
                        (BTN_RIGHT, Some(_)) if super_held => Some(DragMode::Resize),
                        _ => None,
                    };
                    if let (Some(mode), Some(i)) = (mode, hit) {
                        let w = &self.app.windows[i];
                        let grab = DragGrab {
                            id: w.id,
                            mode,
                            start_pointer: (x, y),
                            start_rect: (w.loc.0, w.loc.1, w.size.0, w.size.1),
                        };
                        self.app.drag = Some(grab);
                        return; // el arrastre captura el botón
                    }
                }

                // Click izquierdo sobre la BARRA DE TÍTULO (sin `Super`): arranca
                // un arrastre Move — saca la ventana de su tile y la lleva
                // flotante, lista para aterrizar en una zona (drag-to-zone) o
                // quedar overflow. La barra deja de ser chrome inerte.
                if pressed
                    && button == BTN_LEFT
                    && self.app.drag.is_none()
                    && self.app.mode != BodyMode::Greeter
                {
                    let (x, y) = self.app.pointer_loc;
                    if let Some(i) = self.titlebar_at(x, y) {
                        let (id, loc, size) = {
                            let w = &self.app.windows[i];
                            (w.id, w.loc, w.size)
                        };
                        self.app.drag = Some(DragGrab {
                            id,
                            mode: DragMode::Move,
                            start_pointer: (x, y),
                            start_rect: (loc.0, loc.1, size.0, size.1),
                        });
                        let ev = self.app.body.clicked(id); // enfoca la agarrada
                        self.app.brain_feed(ev);
                        return; // el arrastre captura el botón
                    }
                }

                // Durante un arrastre los botones no llegan al cliente;
                // soltar cualquiera lo termina. Si se soltó sobre una zona
                // (drag-to-zone), la ventana aterriza en ese rect (flotante);
                // si no, queda flotando donde cayó (overflow, ya aplicado por
                // el último drag_update).
                if self.app.drag.is_some() {
                    if !pressed {
                        let mode = self.app.drag.as_ref().map(|d| d.mode);
                        let id = self.app.drag.as_ref().map(|d| d.id);
                        let zone = self.drag_zone.take();
                        self.app.drag = None;
                        if let (Some(mode), Some(id), Some(zi)) = (mode, id, zone) {
                            if matches!(mode, DragMode::Move | DragMode::Tile) {
                                if let Some(rect) = self.zone_rect(zi) {
                                    self.app.brain_feed(BodyEvent::WindowFloatTo { id, rect });
                                }
                            }
                        }
                    }
                    return;
                }

                // Click sobre una barra que acepta teclado (cabezal de shuma):
                // le damos el foco de teclado para poder escribir en el drawer.
                // (El click en sí llega al cliente vía pointer.button de abajo,
                // porque el motion ya enfocó el puntero en esa layer.)
                if pressed {
                    let (x, y) = self.app.pointer_loc;
                    if let Some(surf) = self.app.keyboard_focusable_layer_under(x, y) {
                        if let Some(kb) = self.app.keyboard.clone() {
                            kb.set_focus(&mut self.app, Some(surf), SERIAL_COUNTER.next_serial());
                        }
                    } else if button == BTN_LEFT {
                        // Foco-al-click: la ventana clickeada pide el foco al
                        // Cerebro (que la pinta encima). Independiente del
                        // foco-sigue-ratón; el click sigue llegando al cliente.
                        if let Some(i) = self.window_at(x, y) {
                            if !self.app.windows[i].is_shell {
                                let id = self.app.windows[i].id;
                                let ev = self.app.body.clicked(id);
                                self.app.brain_feed(ev);
                            }
                        }
                    }
                }

                // Botón normal: a la ventana (o layer) bajo el puntero.
                let Some(pointer) = self.app.pointer.clone() else {
                    return;
                };
                pointer.button(
                    &mut self.app,
                    &ButtonEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                        button,
                        state: event.state(),
                    },
                );
                pointer.frame(&mut self.app);
            }

            // --- Puntero: rueda / desplazamiento -------------------------
            InputEvent::PointerAxis { event } => {
                let Some(pointer) = self.app.pointer.clone() else {
                    return;
                };
                let source = event.source();
                let mut frame = AxisFrame::new(time).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    match event.amount(axis) {
                        Some(v) if v != 0.0 => frame = frame.value(axis, v),
                        Some(_) if source == AxisSource::Finger => {
                            frame = frame.stop(axis);
                        }
                        _ => {}
                    }
                    if let Some(d) = event.amount_v120(axis) {
                        frame = frame.v120(axis, d as i32);
                    }
                }
                pointer.axis(&mut self.app, frame);
                pointer.frame(&mut self.app);
            }

            _ => {} // otros dispositivos: aún no
        }
    }

    /// Reenvía el puntero a la ventana que tiene debajo y, si esa ventana
    /// cambió, aplica el foco-sigue-ratón avisando al Cerebro.
    fn pointer_motion(&mut self, time: u32) {
        let Some(pointer) = self.app.pointer.clone() else {
            return;
        };
        let (x, y) = self.app.pointer_loc;

        // Las capas Overlay/Top (las barras de `pata`) están por encima de las
        // ventanas: el puntero va ahí primero. Sin esto, los clicks sólo llegaban
        // a las ventanas y las barras quedaban muertas al mouse.
        if let Some((surface, loc)) = self.app.layer_under(x, y) {
            pointer.motion(
                &mut self.app,
                Some((surface, loc)),
                &MotionEvent {
                    location: Point::from((x, y)),
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                },
            );
            pointer.frame(&mut self.app);
            // El cliente del layer pondría su propio cursor; por ahora, el default.
            self.app.cursor_status = CursorImageStatus::default_named();
            // Dejamos de sobrevolar cualquier ventana.
            self.last_pointer_window = None;
            return;
        }

        let hit = self.window_at(x, y);
        let focus = hit.map(|i| {
            let w = &self.app.windows[i];
            let (lx, ly) =
                crate::render_loc(w, self.app.output_size.1, self.app.decorations.titlebar_height);
            (
                w.surface.clone(),
                Point::<f64, Logical>::from((lx as f64, ly as f64)),
            )
        });
        pointer.motion(
            &mut self.app,
            focus,
            &MotionEvent {
                location: Point::from((x, y)),
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(&mut self.app);

        // Sobre el escritorio pelado no manda ningún cliente: el cursor
        // vuelve al de por defecto (si no, se queda con la «I» del texto
        // de la última ventana).
        if hit.is_none() {
            self.app.cursor_status = CursorImageStatus::default_named();
        }

        // Foco-sigue-ratón: al pasar a otra ventana, que la enfoque quien
        // corresponda — el Cerebro para las teseladas, carmen mismo para
        // el shell (que no vive en el Cerebro). PERO si una layer reclama teclado
        // Exclusive (el drawer Quake de pata abierto), no le robamos el foco al
        // mover el mouse sobre una ventana: seguís escribiendo en el drawer.
        let exclusive_layer = self.app.exclusive_layer_surface().is_some();
        let hovered = hit.map(|i| self.app.windows[i].id);
        if hovered != self.last_pointer_window {
            self.last_pointer_window = hovered;
            match hit {
                _ if exclusive_layer => {}
                Some(i) if self.app.windows[i].is_shell => {
                    let surf = self.app.windows[i].surface.clone();
                    if let Some(kb) = self.app.keyboard.clone() {
                        kb.set_focus(&mut self.app, Some(surf), SERIAL_COUNTER.next_serial());
                    }
                }
                Some(i) => {
                    let id = self.app.windows[i].id;
                    let ev = self.app.body.pointer_enter(id);
                    self.app.brain_feed(ev);
                }
                None => {}
            }
        }
    }

    /// Si hay un arrastre en curso, recalcula el rectángulo de la ventana
    /// y se lo manda al Cerebro (que la hace flotar ahí). Devuelve `true`
    /// si consumió el movimiento — entonces el puntero no llega al cliente.
    fn drag_update(&mut self) -> bool {
        let Some(drag) = self.app.drag.as_ref() else {
            return false;
        };
        let mode = drag.mode;
        let (spx, spy) = drag.start_pointer;
        let (sx, sy, sw, sh) = drag.start_rect;
        let id = drag.id;

        let (px, py) = self.app.pointer_loc;
        // Drag-to-zone: resalta la zona bajo el puntero (Move/Tile, no Resize).
        // Sobre una zona, la ventana aterrizará ahí al soltar.
        self.drag_zone = if mode == DragMode::Resize { None } else { self.zone_at(px, py) };
        // Arrastre de una teselada: el Cerebro la intercambia con la tesela
        // bajo el puntero — no flota, sólo reordena el stack. Pero si está
        // sobre una zona, suprimimos el swap (se resolverá al soltar).
        if mode == DragMode::Tile {
            if self.drag_zone.is_none() {
                self.app
                    .brain_feed(BodyEvent::WindowDragged { id, x: px as i32, y: py as i32 });
            }
            return true;
        }
        let dx = (px - spx) as i32;
        let dy = (py - spy) as i32;
        let rect = match mode {
            DragMode::Move => Rect::new(sx + dx, sy + dy, sw, sh),
            DragMode::Resize => Rect::new(
                sx,
                sy,
                (sw + dx).max(MIN_WINDOW),
                (sh + dy).max(MIN_WINDOW),
            ),
            DragMode::Tile => unreachable!("Tile se maneja arriba"),
        };
        self.app.brain_feed(BodyEvent::WindowFloatTo { id, rect });
        true
    }

    /// El índice de la ventana visible bajo el punto `(x, y)`, si la hay
    /// — en orden front-to-back (el shell gana a las flotantes, y éstas a
    /// las teseladas).
    fn window_at(&self, x: f64, y: f64) -> Option<usize> {
        let mut idx: Vec<usize> = (0..self.app.windows.len())
            .filter(|&i| self.app.windows[i].visible)
            .collect();
        idx.sort_by_key(|&i| {
            let w = &self.app.windows[i];
            (!w.is_shell, !w.floating, !w.focused)
        });
        // `output_h` se usa para anclar el shell al borde inferior; el shell
        // vive en la primaria, así que usamos su altura, no la total. Sin
        // monitores (todos desconectados) no hay ventana que golpear.
        let Some(primary) = self.outputs.get(Self::PRIMARY) else {
            return None;
        };
        let output_h = primary.rect.h;
        let tbh = self.app.decorations.titlebar_height;
        idx.into_iter().find(|&i| {
            let w = &self.app.windows[i];
            let tb = crate::titlebar_for(w, tbh);
            let (lx, ly) = crate::render_loc(w, output_h, tbh);
            let (sw, sh) = crate::surface_px_size(w).unwrap_or((w.size.0, (w.size.1 - tb).max(1)));
            // Impacto sobre la SUPERFICIE (la barra de título es chrome inerte
            // en este MVP: no captura el puntero hacia el cliente).
            x >= lx as f64 && y >= ly as f64 && x < (lx + sw) as f64 && y < (ly + sh) as f64
        })
    }

    /// El índice de la ventana cuya **barra de título** está bajo `(x, y)`, si
    /// la hay (front-to-back). Permite agarrar la ventana por su barra para
    /// arrastrarla (sin `Super`).
    fn titlebar_at(&self, x: f64, y: f64) -> Option<usize> {
        let tbh = self.app.decorations.titlebar_height;
        if tbh <= 0 {
            return None;
        }
        // `output_h` se usa para anclar el shell al borde inferior; el shell
        // vive en la primaria, así que usamos su altura, no la total. Sin
        // monitores (todos desconectados) no hay ventana que golpear.
        let Some(primary) = self.outputs.get(Self::PRIMARY) else {
            return None;
        };
        let output_h = primary.rect.h;
        let mut idx: Vec<usize> = (0..self.app.windows.len())
            .filter(|&i| self.app.windows[i].visible)
            .collect();
        idx.sort_by_key(|&i| {
            let w = &self.app.windows[i];
            (!w.is_shell, !w.floating, !w.focused)
        });
        idx.into_iter().find(|&i| {
            let w = &self.app.windows[i];
            let tb = crate::titlebar_for(w, tbh);
            if tb == 0 {
                return false;
            }
            let (lx, ly) = crate::render_loc(w, output_h, tbh);
            let (sw, _) = crate::surface_px_size(w).unwrap_or((w.size.0, (w.size.1 - tb).max(1)));
            let top = ly - tb;
            x >= lx as f64 && y >= top as f64 && x < (lx + sw) as f64 && y < (top + tb) as f64
        })
    }

    /// Reenumera los conectores DRM y aplica las diferencias con
    /// [`Self::outputs`]: monitores recién enchufados se agregan, los que
    /// dejan de estar Connected se quitan. En cualquier cambio, se re-dispone
    /// la geometría global, se notifica al Brain y se rearman las reservas.
    fn detect_connector_changes(&mut self) {
        use smithay::reexports::drm::control::{connector, crtc};
        let resources = match self.drm.resource_handles() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("mirada-compositor · hotplug · no pude releer DRM: {e}");
                return;
            }
        };
        // Conectores Connected ahora, con su handle + nombre.
        let mut live: Vec<(connector::Handle, String)> = Vec::new();
        for &h in resources.connectors() {
            let Ok(c) = self.drm.get_connector(h, false) else {
                continue;
            };
            if c.state() == ConnectorState::Connected {
                live.push((h, format!("{:?}-{}", c.interface(), c.interface_id())));
            }
        }
        let live_names: std::collections::HashSet<&str> =
            live.iter().map(|(_, n)| n.as_str()).collect();
        let known_names: std::collections::HashSet<String> =
            self.outputs.iter().map(|o| o.name.clone()).collect();

        let mut changed = false;

        // 1 · Desenchufes — drop OutputCtx + remove_output al Brain.
        let to_remove: Vec<usize> = self
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, o)| !live_names.contains(o.name.as_str()))
            .map(|(i, _)| i)
            .collect();
        for &i in to_remove.iter().rev() {
            let name = self.outputs[i].name.clone();
            println!("mirada-compositor · hotplug · monitor «{name}» desenchufado");
            let ev = self.app.body.remove_output(i as u32);
            self.app.brain_feed(ev);
            // Drop del compositor + smithay::Output: la GPU libera recursos.
            let _ = self.outputs.remove(i);
            changed = true;
        }

        // El menú raíz se ancla a una salida por índice; tras un desenchufe ese
        // índice puede quedar viejo (fuera de rango o apuntando a otro monitor).
        // Lo cerramos para que no pinte con coords muertas ni indexe de más.
        if self
            .menu_output_idx
            .is_some_and(|i| i >= self.outputs.len())
        {
            self.root_menu = None;
            self.menu_output_idx = None;
        }

        // 2 · Enchufes — armar OutputCtx para cada conector nuevo.
        let used_crtcs: Vec<crtc::Handle> = self.outputs.iter().map(|o| o.crtc).collect();
        let mut taken: Vec<crtc::Handle> = used_crtcs.clone();
        for (conn_handle, name) in &live {
            if known_names.contains(name) {
                continue;
            }
            let Ok(conn) = self.drm.get_connector(*conn_handle, false) else {
                continue;
            };
            // Modo: el de mayor área (a igualdad, mayor refresco).
            let Some(mode) = conn
                .modes()
                .iter()
                .max_by_key(|m| {
                    let (w, h) = m.size();
                    (w as u32 * h as u32, m.vrefresh())
                })
                .copied()
            else {
                continue;
            };
            // CRTC libre compatible.
            let crtc_choice = conn
                .encoders()
                .iter()
                .filter_map(|enc| self.drm.get_encoder(*enc).ok())
                .find_map(|enc| {
                    resources
                        .filter_crtcs(enc.possible_crtcs())
                        .into_iter()
                        .find(|c| !taken.contains(c))
                });
            let Some(crtc_h) = crtc_choice else {
                eprintln!("mirada-compositor · hotplug · «{name}» sin CRTC libre — se ignora");
                continue;
            };
            taken.push(crtc_h);
            match self.armar_output_ctx(*conn_handle, crtc_h, mode, name.clone()) {
                Ok(ctx) => {
                    println!("mirada-compositor · hotplug · monitor «{}» enchufado", ctx.name);
                    let (w, h) = mode.size();
                    let ev = self.app.body.add_output(
                        self.outputs.len() as u32,
                        w as i32,
                        h as i32,
                    );
                    self.app.brain_feed(ev);
                    self.outputs.push(ctx);
                    changed = true;
                }
                Err(e) => eprintln!("mirada-compositor · hotplug · falló «{name}»: {e}"),
            }
        }

        if changed {
            self.redisponer_outputs();
        }
    }

    /// Crea un `OutputCtx` nuevo desde un conector recién enchufado: arma
    /// `DrmSurface` + `DrmCompositor` + `smithay::Output` con la escala y
    /// transformación que mande la config. Idéntica a la rama del discovery
    /// inicial — el día que haya que tocar uno hay que tocar el otro.
    fn armar_output_ctx(
        &mut self,
        conn_handle: smithay::reexports::drm::control::connector::Handle,
        crtc_h: smithay::reexports::drm::control::crtc::Handle,
        mode: smithay::reexports::drm::control::Mode,
        name: String,
    ) -> Result<OutputCtx, String> {
        let (w, h) = mode.size();
        let surface = self
            .drm
            .create_surface(crtc_h, mode, &[conn_handle])
            .map_err(|e| format!("create_surface: {e}"))?;
        let scale_120 = self.app.config_output_scale_120_for(&name);
        let transform = self.app.config_output_transform_for(&name);
        let scale_f64 = (if scale_120 > 0 { scale_120 } else { 120 }) as f64 / 120.0;
        let mode_source = OutputModeSource::Static {
            size: Size::from((w as i32, h as i32)),
            scale: Scale::from(scale_f64),
            transform,
        };
        let compositor: Compositor = DrmCompositor::new(
            mode_source,
            surface,
            None,
            self.allocator.clone(),
            self.exporter.clone(),
            [Fourcc::Argb8888, Fourcc::Xrgb8888],
            self.renderer_formats.clone(),
            self.drm.cursor_size(),
            Some(self.gbm.clone()),
        )
        .map_err(|e| format!("DrmCompositor::new: {e}"))?;
        let refresh_mhz = mode.vrefresh() as i32 * 1000;
        let smithay_out = crate::announce_output(
            &self.dh,
            &name,
            w as i32,
            h as i32,
            refresh_mhz,
            scale_120,
            transform,
        );
        let wp_path = self.app.config_wallpaper_path_for(&name);
        let wp_fit = self.app.config_wallpaper_fit_for(&name);
        Ok(OutputCtx {
            name,
            output: smithay_out,
            crtc: crtc_h,
            compositor,
            // rect lo fija `redisponer_outputs` después de añadir.
            rect: Rect::new(0, 0, w as i32, h as i32),
            refresh_mhz,
            wallpaper: None,
            wallpaper_path: wp_path,
            wallpaper_fit: wp_fit,
            pending_flip: false,
        })
    }

    /// Re-ordena las salidas por `(order, name)` de la config, recalcula sus
    /// rects globales con `mirada-layout::disponer`, actualiza el espacio
    /// total y resincroniza `app.outputs`/`app.output`/`app.output_size`,
    /// invalida wallpapers (rearmados al próximo render) y re-emite las
    /// reservas. Lo que NO toca: ventanas — el Brain decide a dónde van.
    fn redisponer_outputs(&mut self) {
        if self.outputs.is_empty() {
            self.app.outputs.clear();
            self.app.output = None;
            self.app.output_size = (1, 1);
            return;
        }
        // Sort por (order, name) — primaria queda en outputs[0].
        let app_ref = &self.app;
        self.outputs.sort_by(|a, b| {
            let oa = app_ref.config_output_order_for(&a.name);
            let ob = app_ref.config_output_order_for(&b.name);
            oa.cmp(&ob).then_with(|| a.name.cmp(&b.name))
        });
        let tamanos: Vec<(i32, i32)> = self.outputs.iter().map(|o| (o.rect.w, o.rect.h)).collect();
        let disp = self.app.config_output_disposition();
        let rects = mirada_brain::disponer(&tamanos, disp);
        for (ctx, r) in self.outputs.iter_mut().zip(rects.iter()) {
            ctx.rect = *r;
            ctx.wallpaper = None; // el tamaño global no cambia, pero la posición sí
        }
        let env = mirada_brain::envolvente(&rects);
        let total_w = env.w.max(1);
        let total_h = env.h.max(1);
        self.app.output_size = (total_w, total_h);
        self.output_size = (total_w as f64, total_h as f64);
        // Resincronizar el registro Wayland.
        self.app.outputs = self.outputs.iter().map(|c| c.output.clone()).collect();
        self.app.output = self.outputs.first().map(|c| c.output.clone());
        // Reposicionar el puntero al centro de la primaria si quedó fuera.
        let (px, py) = self.app.pointer_loc;
        let (px, py) = self.clamp_to_outputs(px, py);
        self.app.pointer_loc = (px, py);
        // Reservas y borders pueden cambiar con la nueva geometría.
        self.app.recompute_reservations();
    }

    /// Work rect del monitor bajo el puntero — el "lienzo" de zonas para
    /// arrastres. Multi-monitor: los zonas se escalan al monitor donde
    /// está la acción, no al desktop global.
    fn work_rect(&self) -> Rect {
        let (px, py) = self.app.pointer_loc;
        self.output_work_rect(self.output_at_point(px.round() as i32, py.round() as i32))
    }

    /// El rect en píxeles de la zona `i`, escalado al work-rect del
    /// monitor bajo el puntero. Devuelve coords globales.
    fn zone_rect(&self, i: usize) -> Option<Rect> {
        let wr = self.work_rect();
        self.zones.get(i).map(|z| z.to_rect(wr))
    }

    /// El índice de la zona de arrastre bajo `(x, y)`, si la hay. Las zonas
    /// se hit-testean contra el work-rect del monitor que contiene `(x,y)`.
    fn zone_at(&self, x: f64, y: f64) -> Option<usize> {
        if self.zones.is_empty() {
            return None;
        }
        let (xi, yi) = (x.round() as i32, y.round() as i32);
        let wr = self.output_work_rect(self.output_at_point(xi, yi));
        self.zones.iter().position(|z| {
            let r = z.to_rect(wr);
            xi >= r.x && yi >= r.y && xi < r.x + r.w && yi < r.y + r.h
        })
    }
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

    // 3 · Dispositivo DRM.
    println!("[3/8] abriendo el dispositivo DRM …");
    let fd = session
        .open(&gpu, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK)
        .map_err(|e| format!("no pude abrir {}: {e}", gpu.display()))?;
    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (mut drm, drm_notifier) =
        DrmDevice::new(drm_fd.clone(), true).map_err(|e| format!("DrmDevice::new falló: {e}"))?;
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
        // Brain: cada salida es un id incremental con su tamaño local.
        let ev = app.body.add_output(i as u32, w as i32, h as i32);
        app.brain_feed(ev);
        let rect = rects[i];
        // Wallpaper resuelto por salida (override por nombre o global).
        let wp_path = app.config_wallpaper_path_for(&name);
        let wp_fit = app.config_wallpaper_fit_for(&name);
        println!("      compositor de «{name}» listo · rect global {rect:?}");
        output_ctxs.push(OutputCtx {
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
        crate::spawn_greeter(move |ticket| {
            let _ = tx.send(ticket);
        })?;
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
    // Lista de presets: el 0 es `config.zones`, luego los de `zone_presets`.
    let mut zone_presets = vec![zones.clone()];
    zone_presets.extend(app.config_zone_presets());
    let dh = display.handle();
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
        start: Instant::now(),
        last_windows: 0,
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
