//! `zwlr_screencopy_v1` — captura de pantalla, implementada a mano.
//!
//! smithay 0.7 no trae la lógica de servidor de este protocolo (sólo los
//! bindings generados del XML, vía `wayland-protocols-wlr`), así que el
//! dispatch vive acá. Primera rebanada: `capture_output` y
//! `capture_output_region` sobre buffers `wl_shm`, copia one-shot. Tercera
//! rebanada: **dmabuf zero-copy** — además del `wl_shm` se anuncia
//! `linux_dmabuf`; si el cliente trae un buffer dmabuf (lo que prefieren
//! xdg-desktop-portal-wlr / PipeWire para el screencast), la captura se hace
//! GPU→GPU (`blit` del framebuffer compuesto al dmabuf del cliente), sin el
//! `ReadPixels`+memcpy del camino shm. Segunda rebanada: **daño real** para
//! `copy_with_damage` — la captura queda retenida hasta que la salida tenga
//! daño genuino (commits de clientes, re-teselados, foco, cierre de ventanas)
//! y el evento `damage` reporta el extents acumulado, no el frame entero;
//! es lo que permite a wf-recorder grabar sin re-capturar cuadros idénticos.
//! Granularidad: la celda de la ventana (no los rects finos del commit);
//! lo que no se puede acotar (layer surfaces, menú raíz, cambio de modo)
//! daña todo. El cursor NO acumula daño porque tampoco entra en la captura
//! (`overlay_cursor` sigue pendiente).
//!
//! El global nace **gateado por ejecutable** (`Permisos.screencopy_denylist`),
//! igual que clipboard / virtual-keyboard / foreign-toplevel-list: al denegado
//! no se le anuncia el global — frontera física, no tabla eludible. Es la
//! capacidad más sensible de las cuatro: leer los píxeles de la pantalla.
//!
//! Flujo: el cliente pide un frame → le anunciamos formato/tamaño/stride
//! (`Xrgb8888`, el ancho de la salida) → el cliente trae su buffer con `copy`
//! → la captura queda **pendiente de la próxima composición** de esa salida;
//! el backend (winit o DRM) llama [`servir`] con el framebuffer recién
//! compuesto y ahí se hace el `ReadPixels` y se responde `flags`+`ready`.

use std::sync::atomic::{AtomicBool, Ordering};

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer as _, Fourcc};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, GlesTarget, GlesTexture};
use smithay::backend::renderer::utils::draw_render_elements;
use smithay::backend::renderer::{
    Bind, Blit, Color32F, ExportMem, Frame as _, Offscreen, Renderer, TextureFilter, TextureMapping,
};
use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm};
use smithay::reexports::wayland_server::{
    backend::{ClientId, GlobalId},
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Buffer as BufferCoord, Logical, Monotonic, Physical, Rectangle, Size, Transform};
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::shm::with_buffer_contents_mut;

use crate::App;

/// Formato único que se anuncia a los clientes. `Xrgb8888` es el estándar de
/// facto de los screenshots wlroots; en GL se lee con `BGRA_EXT`
/// (`GL_EXT_read_format_bgra`, presente en todo Mesa).
const FORMATO: wl_shm::Format = wl_shm::Format::Xrgb8888;
const FOURCC: Fourcc = Fourcc::Xrgb8888;

/// Versión del global que se anuncia. v3 = `capture_output_region` +
/// `buffer_done` + `linux_dmabuf` (anunciamos shm **y** dmabuf; el cliente
/// elige el tipo de buffer al pedir `copy`).
const VERSION: u32 = 3;

/// Datos del global: el filtro de visibilidad por cliente (la frontera).
pub struct ScreencopyGlobalData {
    filtro: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// El estado del protocolo — sólo retiene el global para mantenerlo vivo.
pub struct ScreencopyState {
    _global: GlobalId,
}

impl ScreencopyState {
    /// Crea el global `zwlr_screencopy_manager_v1`, gateado por `filtro`
    /// desde su nacimiento: el cliente que no pase el filtro no lo ve.
    pub fn new<F>(dh: &DisplayHandle, filtro: F) -> Self
    where
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = dh.create_global::<App, ZwlrScreencopyManagerV1, _>(
            VERSION,
            ScreencopyGlobalData {
                filtro: Box::new(filtro),
            },
        );
        Self { _global: global }
    }
}

/// Lo que el frame sabe de sí mismo desde que se pidió la captura.
struct FrameMeta {
    output: Output,
    /// Región a copiar, en píxeles del buffer de la salida.
    rect: Rectangle<i32, BufferCoord>,
}

/// User data de cada `zwlr_screencopy_frame_v1`.
pub struct ScreencopyFrameData {
    /// `None` si la captura nació inválida (salida desaparecida, región
    /// vacía) — ya se le envió `failed` y los requests se ignoran.
    meta: Option<FrameMeta>,
    /// Un frame se copia una sola vez (`already_used` si reincide).
    copiado: AtomicBool,
}

/// Una captura aceptada, esperando la próxima composición de su salida.
/// Las `copy` planas se sirven en la composición que sigue; las
/// `copy_with_damage` quedan retenidas hasta acumular daño real.
pub struct PendingScreencopy {
    pub frame: ZwlrScreencopyFrameV1,
    pub buffer: WlBuffer,
    pub output: Output,
    rect: Rectangle<i32, BufferCoord>,
    con_damage: bool,
    /// Daño acumulado desde que se aceptó la copia, en coordenadas GLOBALES
    /// del compositor (el espacio de `ManagedWindow::loc`). Se guarda el
    /// extents — un solo rect que envuelve todo, igual que wlroots, que
    /// también emite un único evento `damage` con la envolvente.
    danio_global: Option<Rectangle<i32, Logical>>,
    /// Daño que no se puede acotar a un rect (layer surfaces, menú raíz,
    /// cambio de modo de la salida): sirve la captura con daño total.
    danio_todo: bool,
    /// El daño ya traducido al frame del cliente — lo fija [`tomar_capturas`]
    /// en el drenado (es quien conoce el origen de la salida) para que
    /// [`copiar_una`] lo emita sin re-traducir.
    danio_frame: Option<Rectangle<i32, BufferCoord>>,
}

impl PendingScreencopy {
    /// `true` si la captura espera daño (`copy_with_damage`) — los sitios
    /// que acumulan daño lo usan para saltarse el trabajo si nadie escucha.
    pub fn con_damage(&self) -> bool {
        self.con_damage
    }
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyGlobalData> for App {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &ScreencopyGlobalData,
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &ScreencopyGlobalData) -> bool {
        (global_data.filtro)(&client)
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for App {
    fn request(
        state: &mut Self,
        _client: &Client,
        _mgr: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput { frame, output, .. } => {
                iniciar_frame(state, frame, &output, None, data_init);
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                output,
                x,
                y,
                width,
                height,
                ..
            } => {
                iniciar_frame(state, frame, &output, Some((x, y, width, height)), data_init);
            }
            zwlr_screencopy_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

/// Inicializa un frame de captura: resuelve la salida, recorta la región y
/// anuncia el buffer que el cliente debe traer. Si algo no cierra (salida
/// muerta, región vacía) el frame nace fallido.
fn iniciar_frame(
    _state: &mut App,
    frame: New<ZwlrScreencopyFrameV1>,
    output: &smithay::reexports::wayland_server::protocol::wl_output::WlOutput,
    region: Option<(i32, i32, i32, i32)>,
    data_init: &mut DataInit<'_, App>,
) {
    let salida = Output::from_resource(output);
    let modo = salida.as_ref().and_then(|o| o.current_mode());
    let rect = match (&salida, modo) {
        (Some(_), Some(modo)) => {
            let pantalla: Rectangle<i32, BufferCoord> =
                Rectangle::from_size((modo.size.w, modo.size.h).into());
            let pedido = match region {
                // La región llega en coordenadas lógicas de la salida; a
                // escala 1 y sin transform coinciden con las del buffer.
                Some((x, y, w, h)) => Rectangle::new((x, y).into(), (w, h).into()),
                None => pantalla,
            };
            pedido.intersection(pantalla)
        }
        _ => None,
    };

    match (salida, rect) {
        (Some(output), Some(rect)) if !rect.is_empty() => {
            let f = data_init.init(
                frame,
                ScreencopyFrameData {
                    meta: Some(FrameMeta { output, rect }),
                    copiado: AtomicBool::new(false),
                },
            );
            f.buffer(
                FORMATO,
                rect.size.w as u32,
                rect.size.h as u32,
                rect.size.w as u32 * 4,
            );
            if f.version() >= 3 {
                // También ofrecemos dmabuf (mismo fourcc/tamaño): el cliente que
                // lo soporte captura GPU→GPU. `format` es el fourcc DRM crudo.
                f.linux_dmabuf(FOURCC as u32, rect.size.w as u32, rect.size.h as u32);
                f.buffer_done();
            }
        }
        _ => {
            let f = data_init.init(
                frame,
                ScreencopyFrameData {
                    meta: None,
                    copiado: AtomicBool::new(true),
                },
            );
            f.failed();
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameData> for App {
    fn request(
        state: &mut Self,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &ScreencopyFrameData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => {
                aceptar_copia(state, frame, data, buffer, false);
            }
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => {
                aceptar_copia(state, frame, data, buffer, true);
            }
            zwlr_screencopy_frame_v1::Request::Destroy => {
                state.pending_screencopy.retain(|p| p.frame != *frame);
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        frame: &ZwlrScreencopyFrameV1,
        _data: &ScreencopyFrameData,
    ) {
        state.pending_screencopy.retain(|p| p.frame != *frame);
    }
}

/// Valida el buffer que trajo el cliente y encola la captura para la próxima
/// composición de su salida. Los errores de uso son errores de protocolo
/// (el spec los tipifica); un buffer no-shm o con otra geometría es
/// `invalid_buffer`.
fn aceptar_copia(
    state: &mut App,
    frame: &ZwlrScreencopyFrameV1,
    data: &ScreencopyFrameData,
    buffer: WlBuffer,
    con_damage: bool,
) {
    let Some(meta) = &data.meta else {
        return; // nació fallido; el cliente va a destruirlo
    };
    if data.copiado.swap(true, Ordering::SeqCst) {
        frame.post_error(
            zwlr_screencopy_frame_v1::Error::AlreadyUsed,
            "el frame ya se copió una vez",
        );
        return;
    }
    let (w, h) = (meta.rect.size.w, meta.rect.size.h);
    // El cliente puede traer un dmabuf (zero-copy GPU→GPU) o un `wl_shm`. En
    // ambos casos exigimos el mismo fourcc y geometría que anunciamos.
    let valido = if let Ok(dmabuf) = get_dmabuf(&buffer) {
        dmabuf.format().code == FOURCC && dmabuf.width() == w as u32 && dmabuf.height() == h as u32
    } else {
        matches!(
            with_buffer_contents_mut(&buffer, |_ptr, len, datos| {
                datos.format == FORMATO
                    && datos.width == w
                    && datos.height == h
                    && datos.stride == w * 4
                    && (datos.offset as usize) + (h as usize * datos.stride as usize) <= len
            }),
            Ok(true)
        )
    };
    if !valido {
        frame.post_error(
            zwlr_screencopy_frame_v1::Error::InvalidBuffer,
            format!("se esperaba dmabuf o wl_shm {FORMATO:?} de {w}×{h}"),
        );
        return;
    }
    // Tope anti-agotamiento: un cliente malicioso podría crear capturas sin
    // límite —sobre todo `copy_with_damage`, que esperan daño retenidas en la
    // cola— y agotar la memoria del compositor. Pasado el tope, la captura nueva
    // falla (el cliente reintenta) en vez de hacer crecer la cola sin freno. Un
    // grabador legítimo tiene ~1-2 capturas en vuelo; 64 es holgado.
    const MAX_PENDIENTES: usize = 64;
    if state.pending_screencopy.len() >= MAX_PENDIENTES {
        frame.failed();
        return;
    }
    state.pending_screencopy.push(PendingScreencopy {
        frame: frame.clone(),
        buffer,
        output: meta.output.clone(),
        rect: meta.rect,
        con_damage,
        danio_global: None,
        danio_todo: false,
        danio_frame: None,
    });
}

/// Acumula daño (en coordenadas globales del compositor) en todas las
/// capturas `copy_with_damage` que esperan. Las `copy` planas no lo
/// necesitan: se sirven en la próxima composición incondicionalmente.
pub fn danar(app: &mut App, rect: Rectangle<i32, Logical>) {
    if rect.is_empty() {
        return;
    }
    for p in app.pending_screencopy.iter_mut().filter(|p| p.con_damage) {
        p.danio_global = Some(match p.danio_global {
            Some(acc) => acc.merge(rect),
            None => rect,
        });
    }
}

/// Daño total: para cambios que no se pueden acotar a un rect (layer
/// surfaces, menú raíz, cambio de modo de la salida).
pub fn danar_todo(app: &mut App) {
    for p in app.pending_screencopy.iter_mut().filter(|p| p.con_damage) {
        p.danio_todo = true;
    }
}

/// Traduce el daño global acumulado al espacio del frame del cliente
/// (origen = esquina del rect capturado), recortado al rect. `None` =
/// todavía no hay daño visible dentro de la captura. Función pura para
/// poder testearla sin recursos wayland.
fn danio_en_frame(
    danio_global: Option<Rectangle<i32, Logical>>,
    danio_todo: bool,
    rect: Rectangle<i32, BufferCoord>,
    origen: (i32, i32),
) -> Option<Rectangle<i32, BufferCoord>> {
    if danio_todo {
        return Some(Rectangle::from_size(rect.size));
    }
    let g = danio_global?;
    // global → local de la salida (restar el origen) → local del frame
    // (restar la esquina del rect). A escala 1 y sin transform, las
    // coordenadas lógicas coinciden con las del buffer.
    let local: Rectangle<i32, BufferCoord> = Rectangle::new(
        (
            g.loc.x - origen.0 - rect.loc.x,
            g.loc.y - origen.1 - rect.loc.y,
        )
            .into(),
        (g.size.w, g.size.h).into(),
    );
    local.intersection(Rectangle::from_size(rect.size))
}

/// Saca de la cola las capturas de `output` que ya pueden servirse — las
/// `copy` planas siempre; las `copy_with_damage` sólo si acumularon daño
/// visible dentro de su rect (las demás siguen esperando en la cola). El
/// backend pasa las drenadas a [`servir`] con el framebuffer recién
/// compuesto. `origen` es la esquina de la salida en coordenadas globales
/// del compositor ((0,0) en winit; el rect de la salida en DRM).
pub fn tomar_capturas(
    app: &mut App,
    output: &Output,
    origen: (i32, i32),
) -> Vec<PendingScreencopy> {
    let todas = std::mem::take(&mut app.pending_screencopy);
    let (mut mias, resto): (Vec<_>, Vec<_>) = todas.into_iter().partition(|p| {
        p.output == *output
            && (!p.con_damage
                || danio_en_frame(p.danio_global, p.danio_todo, p.rect, origen).is_some())
    });
    app.pending_screencopy = resto;
    for p in &mut mias {
        if p.con_damage {
            p.danio_frame = danio_en_frame(p.danio_global, p.danio_todo, p.rect, origen);
        }
    }
    mias
}

/// Sirve las capturas desde un target ya compuesto (el backbuffer winit, o el
/// offscreen del backend DRM): `ReadPixels` región por región y los eventos
/// `flags`+`ready` (o `failed` si la GPU no quiso).
pub fn servir(
    renderer: &mut GlesRenderer,
    target: &GlesTarget<'_>,
    capturas: Vec<PendingScreencopy>,
) {
    for c in capturas {
        match copiar_una(renderer, target, &c) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("mirada-compositor · screencopy falló: {e}");
                c.frame.failed();
            }
        }
    }
}

/// Copia una captura del target al buffer del cliente (dmabuf o `wl_shm`) y
/// responde con `flags`+`damage`+`ready`. El tipo de buffer se resuelve acá:
/// dmabuf → `blit` GPU→GPU; si no, `ReadPixels`+memcpy a shm.
fn copiar_una(
    renderer: &mut GlesRenderer,
    target: &GlesTarget<'_>,
    c: &PendingScreencopy,
) -> Result<(), String> {
    // `cloned()` corta el préstamo de `c.buffer` ya: lo que sigue sólo toca `c`
    // por valores `Copy` (el rect) o re-lee `c.buffer` en el path shm.
    let dmabuf = get_dmabuf(&c.buffer).ok().cloned();
    let flipped = match &dmabuf {
        Some(d) => copiar_a_dmabuf(renderer, target, c.rect, d)?,
        None => copiar_a_shm(renderer, target, c.rect, &c.buffer)?,
    };

    // El cliente endereza con el flag — grim y wf-recorder lo honran.
    c.frame.flags(if flipped {
        zwlr_screencopy_frame_v1::Flags::YInvert
    } else {
        zwlr_screencopy_frame_v1::Flags::empty()
    });
    if c.con_damage {
        // El extents del daño acumulado mientras la captura esperaba,
        // traducido al frame por `tomar_capturas`. El fallback a daño total
        // sólo dispara si alguien llamó a `servir` sin pasar por el drenado.
        let d = c
            .danio_frame
            .unwrap_or_else(|| Rectangle::from_size(c.rect.size));
        c.frame
            .damage(d.loc.x as u32, d.loc.y as u32, d.size.w as u32, d.size.h as u32);
    }
    let ahora: std::time::Duration =
        smithay::utils::Clock::<Monotonic>::new().now().into();
    c.frame.ready(
        (ahora.as_secs() >> 32) as u32,
        (ahora.as_secs() & 0xffff_ffff) as u32,
        ahora.subsec_nanos(),
    );
    Ok(())
}

/// Camino `wl_shm`: `ReadPixels` del target a CPU y memcpy al buffer del cliente.
/// Devuelve si el resultado quedó y-invertido (`ReadPixels` lee de abajo-arriba).
fn copiar_a_shm(
    renderer: &mut GlesRenderer,
    target: &GlesTarget<'_>,
    rect: Rectangle<i32, BufferCoord>,
    buffer: &WlBuffer,
) -> Result<bool, String> {
    let mapping = renderer
        .copy_framebuffer(target, rect, FOURCC)
        .map_err(|e| format!("copy_framebuffer: {e}"))?;
    let bytes = renderer
        .map_texture(&mapping)
        .map_err(|e| format!("map_texture: {e}"))?;
    let (w, h) = (rect.size.w as usize, rect.size.h as usize);
    if bytes.len() < w * h * 4 {
        return Err(format!("mapping corto: {} < {}", bytes.len(), w * h * 4));
    }
    let escrito = with_buffer_contents_mut(buffer, |ptr, len, datos| {
        let desde = datos.offset as usize;
        // Re-chequeo en el punto de uso: `aceptar_copia` ya validó el buffer,
        // pero el `unsafe` de abajo no debe depender de eso. Un `desde + n` que
        // se salga del mapeo (cliente raro, pool mutado) aborta la copia en vez
        // de escribir fuera de rango.
        let Some(fin) = desde.checked_add(w * h * 4).filter(|&fin| fin <= len) else {
            return false;
        };
        let destino = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        destino[desde..fin].copy_from_slice(&bytes[..w * h * 4]);
        true
    })
    .map_err(|e| format!("buffer shm: {e:?}"))?;
    if !escrito {
        return Err("buffer shm: offset+tamaño fuera del mapeo".into());
    }
    Ok(mapping.flipped())
}

/// Camino **dmabuf zero-copy**: enlaza el dmabuf del cliente como framebuffer y
/// `blit`ea la región del target compuesto adentro —sin tocar la CPU—.
///
/// Orientación: el framebuffer compuesto es bottom-up (origen abajo-izquierda,
/// GL); un blit recto dejaría el dmabuf invertido respecto al origen
/// arriba-izquierda del `wl_buffer`. En vez de delegar el arreglo al flag
/// `YInvert` —que clientes dmabuf como wf-recorder **ignoran**, dejando el video
/// de cabeza— volteamos en el propio blit: el rect de destino lleva altura
/// **negativa** (`glBlitFramebuffer` con `dstY0 > dstY1` invierte la copia), así
/// el dmabuf queda físicamente top-down. Por eso devolvemos `false` (sin
/// YInvert): correcto tanto si el cliente honra el flag como si lo ignora.
///
/// Si el renderer no puede enlazar el dmabuf (modifier no soportado), propaga el
/// error y [`servir`] le manda `failed` al cliente (que reintenta con shm).
fn copiar_a_dmabuf(
    renderer: &mut GlesRenderer,
    target: &GlesTarget<'_>,
    rect: Rectangle<i32, BufferCoord>,
    dmabuf: &Dmabuf,
) -> Result<bool, String> {
    let mut dst = dmabuf.clone();
    let mut dst_fb = renderer
        .bind(&mut dst)
        .map_err(|e| format!("bind dmabuf: {e}"))?;
    // A escala 1 sin transform las coordenadas de buffer y físicas coinciden.
    let src: Rectangle<i32, Physical> =
        Rectangle::new((rect.loc.x, rect.loc.y).into(), (rect.size.w, rect.size.h).into());
    // Destino con Y invertido: loc.y = alto, size.h = -alto → glBlitFramebuffer
    // recibe dstY0 = h, dstY1 = 0 y voltea verticalmente al copiar.
    let dst_rect: Rectangle<i32, Physical> =
        Rectangle::new((0, rect.size.h).into(), (rect.size.w, -rect.size.h).into());
    renderer
        .blit(target, &mut dst_fb, src, dst_rect, TextureFilter::Linear)
        .map_err(|e| format!("blit dmabuf: {e}"))?;
    Ok(false)
}

/// Sirve capturas en el backend DRM: re-compone los mismos elementos del
/// frame en un offscreen (el framebuffer real vive dentro del
/// `DrmCompositor`, inaccesible) y copia de ahí.
pub fn servir_offscreen<E>(
    renderer: &mut GlesRenderer,
    size: (i32, i32),
    elements: &[E],
    clear: Color32F,
    capturas: Vec<PendingScreencopy>,
) where
    E: smithay::backend::renderer::element::RenderElement<GlesRenderer>,
{
    let fallar_todas = |capturas: Vec<PendingScreencopy>, e: String| {
        eprintln!("mirada-compositor · screencopy offscreen falló: {e}");
        for c in capturas {
            c.frame.failed();
        }
    };
    let buffer_size: Size<i32, BufferCoord> = (size.0, size.1).into();
    let mut tex = match Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, buffer_size)
    {
        Ok(t) => t,
        Err(e) => return fallar_todas(capturas, format!("create_buffer: {e}")),
    };
    let mut target = match renderer.bind(&mut tex) {
        Ok(t) => t,
        Err(e) => return fallar_todas(capturas, format!("bind: {e}")),
    };
    let fisico: Size<i32, smithay::utils::Physical> = (size.0, size.1).into();
    let damage = [Rectangle::from_size(fisico)];
    {
        let mut frame = match renderer.render(&mut target, fisico, Transform::Normal) {
            Ok(f) => f,
            Err(e) => return fallar_todas(capturas, format!("render: {e}")),
        };
        if let Err(e) = frame
            .clear(clear, &damage)
            .map_err(|e| format!("clear: {e}"))
            .and_then(|()| {
                draw_render_elements(&mut frame, 1.0, elements, &damage)
                    .map(|_| ())
                    .map_err(|e| format!("draw: {e}"))
            })
        {
            return fallar_todas(capturas, e);
        }
        match frame.finish() {
            Ok(_) => {}
            Err(e) => return fallar_todas(capturas, format!("finish: {e}")),
        }
    }
    servir(renderer, &target, capturas);
}

/// Renderiza `elements` en un **offscreen** `size` y devuelve sus píxeles RGBA
/// (en realidad bytes `Xrgb8888` = `[B,G,R,X]`, listos para subir como
/// `Argb8888`), corregidos de orientación (el framebuffer GL es bottom-up). Lo
/// usa la vista espacial para sacar una miniatura VIVA de un escritorio y luego
/// **rotarla en CPU** ([`crate::text::rotate_buffer`]) — la única forma de
/// rotar contenido vivo a un ángulo libre, ya que los elementos GL no rotan. La
/// transparencia de las esquinas la pone la rotación, así que acá el contenido
/// puede ir opaco (`Xrgb8888`, el camino probado de screencopy). `None` si algún
/// paso de GPU falla (el llamante cae al esquema rotado).
pub fn render_elements_offscreen<E>(
    renderer: &mut GlesRenderer,
    size: (i32, i32),
    elements: &[E],
) -> Option<Vec<u8>>
where
    E: smithay::backend::renderer::element::RenderElement<GlesRenderer>,
{
    if size.0 <= 0 || size.1 <= 0 {
        return None;
    }
    // Reporta el PRIMER fallo (con el paso) una sola vez — si no, spamearía a
    // 60fps porque falla cada frame. Sirve para saber qué primitiva de GPU no
    // está disponible y poder cazarlo sin volar el log.
    fn log_once(paso: &str, e: &dyn std::fmt::Display) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static L: AtomicBool = AtomicBool::new(false);
        if !L.swap(true, Ordering::Relaxed) {
            eprintln!("mirada-compositor · prezi offscreen falló en {paso}: {e}");
        }
    }
    macro_rules! paso {
        ($paso:literal, $e:expr) => {
            match $e {
                Ok(v) => v,
                Err(e) => {
                    log_once($paso, &e);
                    return None;
                }
            }
        };
    }
    let buffer_size: Size<i32, BufferCoord> = (size.0, size.1).into();
    let mut tex = paso!(
        "create_buffer",
        Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, buffer_size)
    );
    let mut target = paso!("bind", renderer.bind(&mut tex));
    let fisico: Size<i32, smithay::utils::Physical> = (size.0, size.1).into();
    let damage = [Rectangle::from_size(fisico)];
    {
        let mut frame = paso!("render", renderer.render(&mut target, fisico, Transform::Normal));
        paso!("clear", frame.clear(Color32F::TRANSPARENT, &damage));
        paso!("draw", draw_render_elements(&mut frame, 1.0, elements, &damage));
        paso!("finish", frame.finish());
    }
    let rect: Rectangle<i32, BufferCoord> = Rectangle::from_size(buffer_size);
    let mapping = paso!("copy_framebuffer", renderer.copy_framebuffer(&target, rect, FOURCC));
    let bytes = paso!("map_texture", renderer.map_texture(&mapping));
    let (w, h) = (size.0 as usize, size.1 as usize);
    if bytes.len() < w * h * 4 {
        return None;
    }
    let mut out = bytes[..w * h * 4].to_vec();
    if mapping.flipped() {
        // El framebuffer GL es bottom-up: invertimos las filas para que la
        // miniatura quede con el origen arriba-izquierda como el resto.
        let row = w * 4;
        for y in 0..h / 2 {
            let (a, b) = (y * row, (h - 1 - y) * row);
            for k in 0..row {
                out.swap(a + k, b + k);
            }
        }
    }
    Some(out)
}

/// Como [`render_elements_offscreen`] pero el dibujo lo hace un **closure** con
/// acceso crudo al [`GlesFrame`] — para componer con `render_texture_from_to`
/// pasando texturas EXTRAÍDAS a mano (la vista espacial rotada lo necesita: el
/// dibujo por render-elements no encontraba la textura de la superficie en este
/// contexto, pero pasándola explícita sí). Limpia con `clear` (poné el fondo del
/// tile, opaco). Devuelve los píxeles `Xrgb8888` corregidos de orientación, o
/// `None` si la GPU falla.
pub fn render_offscreen_drawing<F>(
    renderer: &mut GlesRenderer,
    size: (i32, i32),
    clear: Color32F,
    draw: F,
) -> Option<Vec<u8>>
where
    F: FnOnce(&mut GlesFrame<'_, '_>) -> Result<(), GlesError>,
{
    if size.0 <= 0 || size.1 <= 0 {
        return None;
    }
    let buffer_size: Size<i32, BufferCoord> = (size.0, size.1).into();
    let mut tex = Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, buffer_size).ok()?;
    let mut target = renderer.bind(&mut tex).ok()?;
    let fisico: Size<i32, smithay::utils::Physical> = (size.0, size.1).into();
    let damage = [Rectangle::from_size(fisico)];
    {
        let mut frame = renderer.render(&mut target, fisico, Transform::Normal).ok()?;
        frame.clear(clear, &damage).ok()?;
        draw(&mut frame).ok()?;
        let _ = frame.finish().ok()?;
    }
    let rect: Rectangle<i32, BufferCoord> = Rectangle::from_size(buffer_size);
    let mapping = renderer.copy_framebuffer(&target, rect, FOURCC).ok()?;
    let bytes = renderer.map_texture(&mapping).ok()?;
    let (w, h) = (size.0 as usize, size.1 as usize);
    if bytes.len() < w * h * 4 {
        return None;
    }
    let mut out = bytes[..w * h * 4].to_vec();
    if mapping.flipped() {
        let row = w * 4;
        for y in 0..h / 2 {
            let (a, b) = (y * row, (h - 1 - y) * row);
            for k in 0..row {
                out.swap(a + k, b + k);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r<K>(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, K> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn sin_danio_no_se_sirve() {
        assert_eq!(danio_en_frame(None, false, r(0, 0, 800, 600), (0, 0)), None);
    }

    #[test]
    fn danio_todo_cubre_el_frame() {
        assert_eq!(
            danio_en_frame(Some(r(10, 10, 5, 5)), true, r(0, 0, 800, 600), (0, 0)),
            Some(r(0, 0, 800, 600)),
        );
        // … incluso sin rect acumulado (p. ej. cambio de modo).
        assert_eq!(
            danio_en_frame(None, true, r(0, 0, 800, 600), (0, 0)),
            Some(r(0, 0, 800, 600)),
        );
    }

    #[test]
    fn danio_global_se_traduce_al_frame() {
        // Captura del output entero en (0,0): global == frame.
        assert_eq!(
            danio_en_frame(Some(r(100, 50, 200, 100)), false, r(0, 0, 800, 600), (0, 0)),
            Some(r(100, 50, 200, 100)),
        );
        // Salida secundaria en x=1920: la misma ventana global cae 1920 a la izquierda.
        assert_eq!(
            danio_en_frame(Some(r(2020, 50, 200, 100)), false, r(0, 0, 800, 600), (1920, 0)),
            Some(r(100, 50, 200, 100)),
        );
        // Captura de región: además se resta la esquina del rect.
        assert_eq!(
            danio_en_frame(Some(r(100, 50, 200, 100)), false, r(80, 40, 400, 300), (0, 0)),
            Some(r(20, 10, 200, 100)),
        );
    }

    #[test]
    fn danio_fuera_del_rect_no_despierta_la_captura() {
        // Ventana dañada en la otra salida: no intersecta este frame.
        assert_eq!(
            danio_en_frame(Some(r(2020, 50, 200, 100)), false, r(0, 0, 800, 600), (0, 0)),
            None,
        );
        // Dentro de la salida pero fuera de la región capturada.
        assert_eq!(
            danio_en_frame(Some(r(500, 500, 50, 50)), false, r(0, 0, 100, 100), (0, 0)),
            None,
        );
    }

    #[test]
    fn danio_se_recorta_al_frame() {
        // Daño que desborda la captura: se recorta a sus límites.
        assert_eq!(
            danio_en_frame(Some(r(-50, -50, 200, 200)), false, r(0, 0, 100, 100), (0, 0)),
            Some(r(0, 0, 100, 100)),
        );
    }
}
