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
use smithay::reexports::wayland_server::{Display, ListeningSocket};
use smithay::utils::{
    DeviceFd, IsAlive, Logical, Physical, Point, Rectangle, Scale, Size, Transform, SERIAL_COUNTER,
};

use auth_core::SessionTicket;
use mirada_brain::{BodyEvent, CtlReply, CtlRequest, Rect, ZoneFrac};

use crate::{
    combo_string, send_frames_surface_tree, App, BodyMode, ClientState, DragGrab, DragMode,
    Setup,
};

/// El `DrmCompositor` concreto para la salida (un solo GPU).
type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

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
    compositor: Compositor,
    renderer: GlesRenderer,
    /// Contexto `libinput` — se suspende y reanuda al conmutar de VT.
    libinput: Libinput,
    /// `false` mientras la sesión está cedida a otra VT — no se compone.
    active: bool,
    /// `true` entre que se encola un page-flip y llega su VBlank.
    pending_flip: bool,
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
    /// Ruta del wallpaper configurado (`None` = fondo de color sólido).
    wallpaper_path: Option<String>,
    /// Modo de ajuste de la imagen (stretch/fit/fill/center/tile).
    wallpaper_fit: mirada_brain::WallpaperFit,
    /// Wallpaper ya decodificado y compuesto al tamaño de la salida, con el
    /// tamaño para el que se construyó. Se (re)arma perezosamente cuando
    /// cambia el tamaño, la ruta o el modo. `None` si no hay ruta o la
    /// imagen no carga.
    wallpaper: Option<(MemoryRenderBuffer, (i32, i32))>,
    /// Árbol del menú raíz (de la config), con submenús anidados.
    menu_entries: Vec<crate::menu::MenuNode>,
    /// Menú raíz abierto, si lo hay (click derecho sobre el fondo).
    root_menu: Option<crate::menu::RootMenu>,
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
    /// Compone el cursor y las ventanas y, si hubo cambios, encola el cuadro.
    fn render(&mut self) {
        if !self.active {
            return; // la sesión está en otra VT — no tocamos la GPU
        }
        if self.pending_flip {
            return; // aún esperamos el VBlank del cuadro anterior
        }
        let output_h = self.app.output_size.1;

        // Paso 1 · refresca los búferes del marco de cada ventana — su
        // tamaño (sigue al contenido) y su color (según el foco). Cada
        // `SolidColorBuffer` sube su contador de daño sólo si algo cambió.
        let dec = self.app.decorations;
        for w in &mut self.app.windows {
            if !w.visible || w.is_shell {
                continue; // el shell no lleva marco
            }
            let tb = crate::titlebar_for(w, dec.titlebar_height);
            let (x, y) = crate::render_loc(w, output_h, dec.titlebar_height);
            let (sw, sh) = crate::surface_px_size(w).unwrap_or((w.size.0, w.size.1 - tb));
            // El marco envuelve barra de título + superficie.
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

        // Snapshot del menú raíz (sin retener préstamos de `self` dentro del
        // bloque de elementos, donde se toma `&mut self.renderer`): las
        // columnas abiertas en cascada, ya colocadas y con resaltado resuelto.
        let menu_render: Option<Vec<crate::menu::MenuColView>> = self.root_menu.as_ref().map(|m| {
            let (px, py) = self.app.pointer_loc;
            m.render(px.round() as i32, py.round() as i32)
        });
        let menu_hl_color = rgba_f32(self.app.decorations.border_focus);

        // Snapshot de las zonas de arrastre: sólo durante un arrastre Move/Tile.
        // `(rects en px, índice resaltado bajo el puntero)`.
        let zone_overlay: Option<(Vec<Rect>, Option<usize>)> = {
            let m = self.app.drag.as_ref().map(|d| d.mode);
            if matches!(m, Some(DragMode::Move) | Some(DragMode::Tile)) && !self.zones.is_empty() {
                let wr = self.work_rect();
                let rects = self.zones.iter().map(|z| z.to_rect(wr)).collect();
                Some((rects, self.drag_zone))
            } else {
                None
            }
        };

        // Paso 2 · arma los elementos — lista front-to-back (índice 0 =
        // encima): el cursor, y por cada ventana su marco sobre su
        // superficie. Las flotantes van antes que las teseladas.
        let elements: Vec<Frame<GlesRenderer>> = {
            let mut out: Vec<Frame<GlesRenderer>> = Vec::new();

            // El cursor — la superficie que pidió el cliente (la «I» del
            // texto, una mano…), o el cuadrado por defecto si pidió un
            // cursor con nombre y no hay tema. `Hidden` no pinta nada.
            let (cx, cy) = self.app.pointer_loc;
            match &self.app.cursor_status {
                CursorImageStatus::Hidden => {}
                CursorImageStatus::Surface(surface) if surface.alive() => {
                    let (hx, hy) = crate::cursor_hotspot(surface);
                    let loc = (cx.round() as i32 - hx, cy.round() as i32 - hy);
                    for el in render_elements_from_surface_tree(
                        &mut self.renderer,
                        surface,
                        loc,
                        1.0,
                        1.0,
                        Kind::Cursor,
                    ) {
                        out.push(Frame::Window(el));
                    }
                }
                _ => {
                    let cursor_rect = Rectangle::new(
                        Point::<i32, Physical>::from((cx.round() as i32, cy.round() as i32)),
                        Size::<i32, Physical>::from((CURSOR_SIZE, CURSOR_SIZE)),
                    );
                    out.push(Frame::Solid(SolidColorRenderElement::new(
                        self.cursor_id.clone(),
                        cursor_rect,
                        CommitCounter::default(),
                        CURSOR_COLOR,
                        Kind::Cursor,
                    )));
                }
            }

            // HUD del preset activo: panel discreto arriba al centro mientras
            // dura la ventana de feedback (HUD_DURATION). Después del deadline
            // el siguiente cuadro lo borra (damasca el área). Va bajo el
            // cursor pero sobre el resto. Se rasteriza fresco en cada cuadro
            // — son ~90 rasterizaciones durante la vida del HUD; los búferes
            // son del tamaño del texto, sin caché.
            if let Some(deadline) = self.preset_hud_until {
                if Instant::now() >= deadline {
                    self.preset_hud_until = None;
                } else if let Some(tr) = &self.text {
                    if !self.preset_hud_label.is_empty() {
                        if let Some(r) =
                            tr.rasterize(&self.preset_hud_label, HUD_TEXT_PX, HUD_TEXT_COLOR)
                        {
                            let tw = r.width;
                            let th = r.height;
                            let panel_w = tw + 2 * HUD_PAD;
                            let panel_h = th.max(HUD_TEXT_PX as i32) + 2 * HUD_PAD;
                            let (ow, _oh) = self.app.output_size;
                            let panel_x = ((ow as i32 - panel_w) / 2).max(0);
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
                                out.push(Frame::Text(el));
                            }
                            let mut bg = SolidColorBuffer::default();
                            bg.update((panel_w, panel_h), HUD_BG);
                            out.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                                &bg,
                                (panel_x, panel_y),
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            )));
                        }
                    }
                }
            }

            // Zonas de arrastre (drag-to-zone): mientras se arrastra, se pintan
            // tenues sobre las ventanas; la que está bajo el puntero, más fuerte
            // (es donde aterrizará al soltar). Bajo el cursor/menú.
            if let Some((rects, hot)) = &zone_overlay {
                let acc = self.app.decorations.border_focus;
                let fill = |a: f32| {
                    [
                        acc[0] as f32 / 255.0,
                        acc[1] as f32 / 255.0,
                        acc[2] as f32 / 255.0,
                        a,
                    ]
                };
                for (i, r) in rects.iter().enumerate() {
                    let color = if Some(i) == *hot { fill(0.40) } else { fill(0.16) };
                    let mut buf = SolidColorBuffer::default();
                    buf.update((r.w, r.h), color);
                    out.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &buf,
                        (r.x, r.y),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )));
                }
            }

            // Menú raíz (openbox) — bajo el cursor, sobre todo lo demás. Una
            // columna por nivel de submenú abierto. Se recorren de la más
            // profunda a la raíz (la lista es front-to-back: lo primero queda
            // arriba), así una columna hija solapada queda sobre su padre.
            // Dentro de cada columna: texto, luego resaltado, luego fondo.
            if let Some(cols) = &menu_render {
                if self.text_cache.len() > 256 {
                    self.text_cache.clear();
                }
                for col in cols.iter().rev() {
                    // Texto de cada fila (rasterizado, con la caché). A los
                    // submenús se les agrega un indicador `›`.
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
                                out.push(Frame::Text(el));
                            }
                        }
                    }
                    // Resaltado de las filas activas (hover / submenú abierto).
                    for row in &col.rows {
                        if row.highlighted {
                            let mut hl = SolidColorBuffer::default();
                            hl.update((col.w, crate::menu::ITEM_H), menu_hl_color);
                            out.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                                &hl,
                                (row.x, row.y),
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            )));
                        }
                    }
                    // Fondo de la columna.
                    let mut bg = SolidColorBuffer::default();
                    bg.update((col.w, col.h), MENU_BG);
                    out.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &bg,
                        (col.x, col.y),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )));
                }
            }

            // Pista de revelado del dock autoescondido: una franja fina en el
            // borde anclado mientras está oculto, para indicar dónde acercar el
            // puntero para revelarlo.
            if crate::shell_dock().autohide && self.app.shell_hidden {
                let (ow, oh) = self.app.output_size;
                if ow > 0 && oh > 0 {
                    let dock = crate::shell_dock();
                    let limite = if dock.anchor.es_horizontal() { oh } else { ow };
                    let t = dock.thickness.clamp(1, limite.max(1));
                    let (bx, by, bw, bh) =
                        crate::shell_reveal_band(dock.anchor, ow, oh, t, crate::SHELL_REVEAL_BAND);
                    let mut band = SolidColorBuffer::default();
                    band.update((bw, bh), menu_hl_color);
                    out.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &band,
                        (bx, by),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )));
                }
            }

            // Layer surfaces (waybar, swaybg…): los Overlay/Top van encima
            // de las ventanas; los Bottom/Background, debajo. Front-to-back.
            let (over_layers, under_layers) =
                crate::layer_render_elements(self.app.output.as_ref(), &mut self.renderer);
            for el in over_layers {
                out.push(Frame::Window(el));
            }

            // El shell va sobre todo; luego las flotantes; luego las
            // teseladas. Dentro de cada grupo, la enfocada se pinta encima
            // (raise-on-focus). `sort_by_key` es estable: respeta el orden de
            // apertura entre las no enfocadas.
            let mut shown: Vec<_> = self.app.windows.iter().filter(|w| w.visible).collect();
            shown.sort_by_key(|w| (!w.is_shell, !w.floating, !w.focused));
            let tbh = self.app.decorations.titlebar_height;
            for w in &shown {
                let tb = crate::titlebar_for(w, tbh);
                let (x, y) = crate::render_loc(w, output_h, tbh); // pos de la superficie
                let (sw, sh) =
                    crate::surface_px_size(w).unwrap_or((w.size.0, (w.size.1 - tb).max(1)));
                // El rect decorado envuelve barra de título + superficie.
                let dec_y = y - tb;
                let dec_h = sh + tb;

                if tb > 0 {
                    // Barra de título real: una franja arriba de la superficie,
                    // coloreada por el foco, con el título a la izquierda.
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
                                out.push(Frame::Text(el));
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
                    out.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &bar,
                        (x, dec_y),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )));
                } else if w.focused && !w.is_shell && !w.title.is_empty() {
                    // Sin barra (titlebar_height = 0): el viejo comportamiento,
                    // el título de la enfocada superpuesto sobre su superficie.
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
                            out.push(Frame::Text(el));
                        }
                    }
                }
                // El marco, alrededor de la decoración completa (barra +
                // superficie) — el shell no lleva, y se omite si el grosor es 0.
                if !w.is_shell && self.app.decorations.border_width > 0 {
                    let rects = border_rects(x, dec_y, sw, dec_h, self.app.decorations.border_width);
                    for (buf, (bx, by, _, _)) in w.borders.iter().zip(rects) {
                        out.push(Frame::Solid(SolidColorRenderElement::from_buffer(
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
                    out.push(Frame::Window(el));
                }
            }

            // Layer surfaces de fondo (Bottom/Background) — debajo de todo.
            for el in under_layers {
                out.push(Frame::Window(el));
            }

            // Wallpaper — al fondo de todo (la lista es front-to-back, así que
            // va último). Se (re)arma perezosamente al tamaño de la salida.
            if let Some(path) = self.wallpaper_path.clone() {
                let size = (
                    self.app.output_size.0 as i32,
                    self.app.output_size.1 as i32,
                );
                let stale = self
                    .wallpaper
                    .as_ref()
                    .map(|(_, s)| *s != size)
                    .unwrap_or(true);
                if stale && size.0 > 0 && size.1 > 0 {
                    self.wallpaper = load_wallpaper(&path, self.wallpaper_fit, size.0, size.1)
                        .map(|b| (b, size));
                }
                if let Some((buf, _)) = &self.wallpaper {
                    if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                        &mut self.renderer,
                        (0.0, 0.0),
                        buf,
                        None,
                        None,
                        None,
                        Kind::Unspecified,
                    ) {
                        out.push(Frame::Text(el));
                    }
                }
            }
            out
        };
        match self.compositor.render_frame::<_, _>(
            &mut self.renderer,
            &elements,
            CLEAR_COLOR,
            FrameFlags::DEFAULT,
        ) {
            Ok(result) => {
                if !result.is_empty {
                    match self.compositor.queue_frame(()) {
                        Ok(()) => self.pending_flip = true,
                        Err(e) => eprintln!("mirada-compositor · queue_frame: {e}"),
                    }
                }
            }
            Err(e) => eprintln!("mirada-compositor · render_frame: {e}"),
        }
        // Avisa a cada cliente de que puede dibujar el siguiente cuadro.
        let time = self.start.elapsed().as_millis() as u32;
        for w in &self.app.windows {
            send_frames_surface_tree(&w.surface, time);
        }
        if let Some(output) = self.app.output.clone() {
            for layer in smithay::desktop::layer_map_for_output(&output).layers() {
                send_frames_surface_tree(layer.wl_surface(), time);
            }
        }
        // También a la superficie del cursor, por si es un cursor animado.
        if let CursorImageStatus::Surface(surface) = &self.app.cursor_status {
            if surface.alive() {
                send_frames_surface_tree(surface, time);
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
    /// el estado del compositor y repinta.
    fn resume_session(&mut self) {
        if self.libinput.resume().is_err() {
            eprintln!("mirada-compositor · libinput.resume falló.");
        }
        if let Err(e) = self.drm.activate(false) {
            eprintln!("mirada-compositor · drm.activate falló: {e}");
        }
        if let Err(e) = self.compositor.reset_state() {
            eprintln!("mirada-compositor · compositor.reset_state falló: {e}");
        }
        self.active = true;
        self.pending_flip = false;
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
            let new_wp = self.app.config_wallpaper_path();
            let new_fit = self.app.config_wallpaper_fit();
            if new_wp != self.wallpaper_path || new_fit != self.wallpaper_fit {
                self.wallpaper_path = new_wp;
                self.wallpaper_fit = new_fit;
                self.wallpaper = None; // se rearma en el próximo render
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
                let (mut x, mut y) = self.app.pointer_loc;
                x = (x + event.delta_x()).clamp(0.0, self.output_size.0);
                y = (y + event.delta_y()).clamp(0.0, self.output_size.1);
                self.app.pointer_loc = (x, y);
                if let Some(m) = self.root_menu.as_mut() {
                    m.update_hover(x.round() as i32, y.round() as i32);
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
                self.app.pointer_loc = (
                    pos.x.clamp(0.0, self.output_size.0),
                    pos.y.clamp(0.0, self.output_size.1),
                );
                if let Some(m) = self.root_menu.as_mut() {
                    let (x, y) = self.app.pointer_loc;
                    m.update_hover(x.round() as i32, y.round() as i32);
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
                    let res = if button == BTN_LEFT {
                        self.root_menu
                            .as_mut()
                            .unwrap()
                            .click(x.round() as i32, y.round() as i32)
                    } else {
                        ClickResult::Close
                    };
                    match res {
                        ClickResult::Launch(cmd) => {
                            self.root_menu = None;
                            self.app.spawn_user(&cmd);
                        }
                        ClickResult::Stay => {}
                        ClickResult::Close => self.root_menu = None,
                    }
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
                        self.root_menu = Some(crate::menu::RootMenu::open(
                            x.round() as i32,
                            y.round() as i32,
                            self.menu_entries.clone(),
                            self.output_size.0 as i32,
                            self.output_size.1 as i32,
                        ));
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
        let output_h = self.app.output_size.1;
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
        let output_h = self.app.output_size.1;
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

    /// El área de trabajo en px: la salida menos las reservas (dock/layers).
    /// Las zonas se escalan a ella, así no aterrizan bajo la barra.
    fn work_rect(&self) -> Rect {
        let (ow, oh) = self.app.output_size;
        let (top, bottom, left, right) = self.app.reserved;
        Rect::new(left, top, (ow - left - right).max(1), (oh - top - bottom).max(1))
    }

    /// El rect en píxeles de la zona `i` (fracciones escaladas al área útil).
    fn zone_rect(&self, i: usize) -> Option<Rect> {
        let wr = self.work_rect();
        self.zones.get(i).map(|z| z.to_rect(wr))
    }

    /// El índice de la zona de arrastre bajo `(x, y)`, si la hay.
    fn zone_at(&self, x: f64, y: f64) -> Option<usize> {
        let (ow, oh) = self.app.output_size;
        if ow == 0 || oh == 0 || self.zones.is_empty() {
            return None;
        }
        let wr = self.work_rect();
        let (xi, yi) = (x.round() as i32, y.round() as i32);
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

    // 4 · Elegir la salida conectada: conector + CRTC + modo.
    println!("[4/8] eligiendo salida …");
    let resources = drm
        .resource_handles()
        .map_err(|e| format!("no pude leer los recursos DRM: {e}"))?;
    let mut chosen = None;
    for &conn_handle in resources.connectors() {
        let conn = match drm.get_connector(conn_handle, false) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if conn.state() != ConnectorState::Connected {
            continue;
        }
        let name = format!("{:?}-{}", conn.interface(), conn.interface_id());
        // Registra todos los modos del panel — diagnóstico.
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
        // nativo del panel. La marca PREFERRED no es fiable: a veces
        // señala un modo menor.
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
        let crtc = conn
            .encoders()
            .iter()
            .filter_map(|enc| drm.get_encoder(*enc).ok())
            .find_map(|enc| resources.filter_crtcs(enc.possible_crtcs()).into_iter().next());
        if let Some(crtc) = crtc {
            let (w, h) = mode.size();
            println!("      salida «{name}» · {w}×{h} · CRTC {crtc:?}");
            chosen = Some((conn_handle, crtc, mode, name));
            break;
        }
    }
    let (conn_handle, crtc, mode, out_name) =
        chosen.ok_or("ninguna salida conectada con CRTC disponible")?;
    let (mode_w, mode_h) = mode.size();

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

    // 6 · Superficie DRM + DrmCompositor de la salida.
    println!("[6/8] creando la superficie DRM y el compositor …");
    let surface = drm
        .create_surface(crtc, mode, &[conn_handle])
        .map_err(|e| format!("create_surface falló: {e}"))?;
    let allocator =
        GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let exporter = GbmFramebufferExporter::new(gbm.clone(), None);
    let renderer_formats = renderer.dmabuf_formats();
    let mode_source = OutputModeSource::Static {
        size: Size::from((mode_w as i32, mode_h as i32)),
        scale: Scale::from(1.0),
        transform: Transform::Normal,
    };
    let compositor: Compositor = DrmCompositor::new(
        mode_source,
        surface,
        None,
        allocator,
        exporter,
        [Fourcc::Argb8888, Fourcc::Xrgb8888],
        renderer_formats,
        drm.cursor_size(),
        Some(gbm.clone()),
    )
    .map_err(|e| format!("DrmCompositor::new falló: {e}"))?;
    println!("      compositor de «{out_name}» listo.");

    // 7 · El estado Wayland (Cerebro, teclado, keymap, control).
    println!("[7/8] armando el estado Wayland …");
    let Setup { mut display, mut app, watches, ctl } =
        crate::build_app(greeter)?;
    // Con el renderer ya creado, anuncia dmabuf — sin esto las apps que
    // pintan por GPU (GPUI, navegadores acelerados) no pueden conectarse.
    crate::announce_dmabuf(&mut app, &display.handle(), &renderer);
    // La salida del Cerebro = el modo del monitor.
    let ev = app.body.add_output(0, mode_w as i32, mode_h as i32);
    app.brain_feed(ev);
    app.output_size = (mode_w as i32, mode_h as i32);
    // El puntero arranca en el centro de la pantalla.
    app.pointer_loc = (mode_w as f64 / 2.0, mode_h as f64 / 2.0);
    // Anuncia el monitor en el protocolo Wayland — los clientes lo exigen.
    app.output = Some(crate::announce_output(
        &display.handle(),
        &out_name,
        mode_w as i32,
        mode_h as i32,
        mode.vrefresh() as i32 * 1000,
    ));

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

    // VBlank: el page-flip terminó.
    handle
        .insert_source(drm_notifier, |event, _meta, state| match event {
            DrmEvent::VBlank(_crtc) => {
                if let Err(e) = state.compositor.frame_submitted() {
                    eprintln!("mirada-compositor · frame_submitted: {e}");
                }
                state.pending_flip = false;
            }
            DrmEvent::Error(e) => eprintln!("mirada-compositor · DRM: {e}"),
        })
        .map_err(|e| format!("insert drm: {e}"))?;

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
                    let _ = state
                        .display
                        .handle()
                        .insert_client(stream, Arc::new(ClientState::default()));
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
    let wallpaper_path = app.config_wallpaper_path();
    let wallpaper_fit = app.config_wallpaper_fit();
    let menu_entries = app.config_menu();
    let zones = app.config_zones();
    // Lista de presets: el 0 es `config.zones`, luego los de `zone_presets`.
    let mut zone_presets = vec![zones.clone()];
    zone_presets.extend(app.config_zone_presets());
    let mut state = DrmState {
        app,
        session: session.clone(),
        display,
        drm,
        compositor,
        renderer,
        libinput: libinput_handle,
        active: true,
        pending_flip: false,
        watches,
        ctl,
        start: Instant::now(),
        last_windows: 0,
        cursor_id: Id::new(),
        last_pointer_window: None,
        output_size: (mode_w as f64, mode_h as f64),
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
        wallpaper_path,
        wallpaper_fit,
        wallpaper: None,
        menu_entries,
        root_menu: None,
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
