//! `zwlr_screencopy_v1` — captura de pantalla, implementada a mano.
//!
//! smithay 0.7 no trae la lógica de servidor de este protocolo (sólo los
//! bindings generados del XML, vía `wayland-protocols-wlr`), así que el
//! dispatch vive acá. Primera rebanada: `capture_output` y
//! `capture_output_region` sobre buffers `wl_shm`, copia one-shot. Sin dmabuf
//! (el cliente cae a shm) y `copy_with_damage` se sirve como copia inmediata
//! con daño total — suficiente para screenshots (`grim`); la captura continua
//! eficiente (daño real) es la próxima rebanada.
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

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTarget, GlesTexture};
use smithay::backend::renderer::utils::draw_render_elements;
use smithay::backend::renderer::{
    Bind, Color32F, ExportMem, Frame as _, Offscreen, Renderer, TextureMapping,
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
use smithay::utils::{Buffer as BufferCoord, Monotonic, Rectangle, Size, Transform};
use smithay::wayland::shm::with_buffer_contents_mut;

use crate::App;

/// Formato único que se anuncia a los clientes. `Xrgb8888` es el estándar de
/// facto de los screenshots wlroots; en GL se lee con `BGRA_EXT`
/// (`GL_EXT_read_format_bgra`, presente en todo Mesa).
const FORMATO: wl_shm::Format = wl_shm::Format::Xrgb8888;
const FOURCC: Fourcc = Fourcc::Xrgb8888;

/// Versión del global que se anuncia. v3 = `capture_output_region` +
/// `buffer_done` (sin `linux_dmabuf`: no anunciar dmabuf es legal, el cliente
/// usa shm).
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
pub struct PendingScreencopy {
    pub frame: ZwlrScreencopyFrameV1,
    pub buffer: WlBuffer,
    pub output: Output,
    rect: Rectangle<i32, BufferCoord>,
    con_damage: bool,
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
    let valido = with_buffer_contents_mut(&buffer, |_ptr, len, datos| {
        datos.format == FORMATO
            && datos.width == w
            && datos.height == h
            && datos.stride == w * 4
            && (datos.offset as usize) + (h as usize * datos.stride as usize) <= len
    });
    if !matches!(valido, Ok(true)) {
        frame.post_error(
            zwlr_screencopy_frame_v1::Error::InvalidBuffer,
            format!("se esperaba wl_shm {FORMATO:?} de {w}×{h} con stride {}", w * 4),
        );
        return;
    }
    state.pending_screencopy.push(PendingScreencopy {
        frame: frame.clone(),
        buffer,
        output: meta.output.clone(),
        rect: meta.rect,
        con_damage,
    });
}

/// Saca de la cola las capturas que esperan a `output` — el backend las pasa
/// a [`servir`] con el framebuffer recién compuesto de esa salida.
pub fn tomar_capturas(app: &mut App, output: &Output) -> Vec<PendingScreencopy> {
    let todas = std::mem::take(&mut app.pending_screencopy);
    let (mias, resto): (Vec<_>, Vec<_>) = todas.into_iter().partition(|p| p.output == *output);
    app.pending_screencopy = resto;
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

/// Copia una captura del target al `wl_shm` del cliente y responde.
fn copiar_una(
    renderer: &mut GlesRenderer,
    target: &GlesTarget<'_>,
    c: &PendingScreencopy,
) -> Result<(), String> {
    let mapping = renderer
        .copy_framebuffer(target, c.rect, FOURCC)
        .map_err(|e| format!("copy_framebuffer: {e}"))?;
    let bytes = renderer
        .map_texture(&mapping)
        .map_err(|e| format!("map_texture: {e}"))?;
    let (w, h) = (c.rect.size.w as usize, c.rect.size.h as usize);
    if bytes.len() < w * h * 4 {
        return Err(format!("mapping corto: {} < {}", bytes.len(), w * h * 4));
    }
    with_buffer_contents_mut(&c.buffer, |ptr, len, datos| {
        let destino = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        let desde = datos.offset as usize;
        destino[desde..desde + w * h * 4].copy_from_slice(&bytes[..w * h * 4]);
    })
    .map_err(|e| format!("buffer shm: {e:?}"))?;

    // `GlesMapping` siempre sale con el origen abajo-izquierda (ReadPixels):
    // el cliente endereza con el flag — grim y wf-recorder lo honran.
    c.frame.flags(if mapping.flipped() {
        zwlr_screencopy_frame_v1::Flags::YInvert
    } else {
        zwlr_screencopy_frame_v1::Flags::empty()
    });
    if c.con_damage {
        // 1ª rebanada: copia inmediata con daño total (sin esperar daño real).
        c.frame
            .damage(0, 0, c.rect.size.w as u32, c.rect.size.h as u32);
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
