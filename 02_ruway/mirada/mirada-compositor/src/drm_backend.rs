//! `drm_backend` — el Cuerpo del compositor sobre **DRM/KMS**, sin
//! sesión gráfica anfitriona: corre directo sobre una TTY.
//!
//! Por fases, para verificarlo en hardware real paso a paso:
//!
//! - **Fase 1 — bring-up**: sesión (`libseat`), GPU, dispositivo DRM,
//!   enumerar salidas.
//! - **Fase 2a — pipeline de render** (esto): GBM, EGL y `GlesRenderer`,
//!   con un `DrmCompositor` para la salida conectada y un test que pinta
//!   la pantalla de colores unos segundos. Confirma que EGL, GBM, el
//!   *modeset* y el *page-flip* funcionan.
//! - **Fase 2b** (siguiente): el bucle Wayland completo — clientes,
//!   `libinput`, composición real de ventanas.
//!
//! Todo con logs para diagnosticar sin el hardware delante.

use std::error::Error;
use std::time::{Duration, Instant};

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::Session;
use smithay::backend::udev;
use smithay::output::OutputModeSource;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::connector::State as ConnectorState;
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::{DeviceFd, Scale, Size, Transform};

/// El `DrmCompositor` concreto para una salida (un solo GPU, `()` de
/// datos de usuario por cuadro).
type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

/// El estado del test de la fase 2a: lo comparten los callbacks de `calloop`.
struct TestState {
    compositor: Compositor,
    renderer: GlesRenderer,
    /// Cuántos cuadros se han pintado.
    frames: u32,
    /// Inicio del test, para un tope por tiempo (anti-cuelgue).
    start: Instant,
}

impl TestState {
    /// Pinta un cuadro: limpia la pantalla a un color que va cambiando
    /// (para que siempre haya daño y el *page-flip* no se salte) y lo
    /// encola para el siguiente VBlank.
    fn render(&mut self) {
        // Un ciclo lento por rojo → verde → azul.
        let phase = (self.frames / 60) % 3;
        let t = (self.frames % 60) as f32 / 60.0;
        let color = match phase {
            0 => [t, 0.0, 1.0 - t, 1.0],
            1 => [1.0 - t, t, 0.0, 1.0],
            _ => [0.0, 1.0 - t, t, 1.0],
        };
        let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
        match self
            .compositor
            .render_frame::<_, _>(&mut self.renderer, &elements, color, FrameFlags::DEFAULT)
        {
            Ok(result) => {
                if !result.is_empty {
                    if let Err(e) = self.compositor.queue_frame(()) {
                        eprintln!("      error al encolar el cuadro: {e}");
                    }
                }
                self.frames += 1;
            }
            Err(e) => eprintln!("      error pintando el cuadro: {e}"),
        }
    }
}

/// Arranca el Cuerpo sobre DRM/KMS — fases 1 y 2a.
pub fn run() -> Result<(), Box<dyn Error>> {
    println!("mirada-compositor · backend DRM — fases 1 (bring-up) y 2a (render).");
    println!("──────────────────────────────────────────────────");

    // 1 · Sesión.
    println!("[1/7] abriendo la sesión (libseat) …");
    let (mut session, _notifier) = LibSeatSession::new().map_err(|e| {
        format!(
            "no pude abrir la sesión libseat: {e}\n       \
             ¿estás en una TTY de verdad (Ctrl+Alt+F3), con `seatd` o `logind`?"
        )
    })?;
    let seat_name = session.seat();
    println!("      sesión abierta · seat «{seat_name}»");

    // 2 · GPU primaria.
    println!("[2/7] buscando la GPU primaria …");
    let gpu = udev::primary_gpu(&seat_name)
        .map_err(|e| format!("error consultando udev: {e}"))?
        .ok_or("no encontré ninguna GPU — ¿existe algún /dev/dri/card*?")?;
    println!("      GPU primaria: {}", gpu.display());

    // 3 · Dispositivo DRM.
    println!("[3/7] abriendo el dispositivo DRM …");
    let fd = session
        .open(&gpu, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK)
        .map_err(|e| format!("no pude abrir {}: {e}", gpu.display()))?;
    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (mut drm, drm_notifier) =
        DrmDevice::new(drm_fd.clone(), true).map_err(|e| format!("DrmDevice::new falló: {e}"))?;
    println!("      dispositivo DRM listo.");

    // 4 · Elegir la salida conectada: conector + CRTC + modo.
    println!("[4/7] eligiendo salida (conector + CRTC + modo) …");
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
        let Some(&mode) = conn.modes().first() else {
            println!("      «{name}» sin modos — la salto");
            continue;
        };
        // Un CRTC capaz de gobernar este conector, vía sus encoders.
        let crtc = conn
            .encoders()
            .iter()
            .filter_map(|enc| drm.get_encoder(*enc).ok())
            .find_map(|enc| resources.filter_crtcs(enc.possible_crtcs()).into_iter().next());
        match crtc {
            Some(crtc) => {
                let (w, h) = mode.size();
                println!("      salida «{name}» · {w}×{h} · CRTC {crtc:?}");
                chosen = Some((conn_handle, crtc, mode, name));
                break;
            }
            None => println!("      «{name}» sin CRTC libre — la salto"),
        }
    }
    let (conn_handle, crtc, mode, out_name) =
        chosen.ok_or("ninguna salida conectada con CRTC disponible")?;

    // 5 · GBM + EGL + GlesRenderer.
    println!("[5/7] inicializando GBM + EGL + GlesRenderer …");
    let gbm = GbmDevice::new(drm_fd.clone()).map_err(|e| format!("GbmDevice::new falló: {e}"))?;
    let egl_display =
        unsafe { EGLDisplay::new(gbm.clone()) }.map_err(|e| format!("EGLDisplay::new falló: {e}"))?;
    let egl_context =
        EGLContext::new(&egl_display).map_err(|e| format!("EGLContext::new falló: {e}"))?;
    let renderer =
        unsafe { GlesRenderer::new(egl_context) }.map_err(|e| format!("GlesRenderer falló: {e}"))?;
    println!("      renderer GLES listo.");

    // 6 · La superficie DRM y el DrmCompositor de esta salida.
    println!("[6/7] creando la superficie DRM y el compositor …");
    let surface = drm
        .create_surface(crtc, mode, &[conn_handle])
        .map_err(|e| format!("create_surface falló: {e}"))?;
    let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let exporter = GbmFramebufferExporter::new(gbm.clone(), None);
    let renderer_formats = renderer.dmabuf_formats();
    let (mw, mh) = mode.size();
    let mode_source = OutputModeSource::Static {
        size: Size::from((mw as i32, mh as i32)),
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

    // 7 · Bucle de prueba: pinta colores ~6 s, sincronizado al VBlank.
    println!("[7/7] test de pintado — la pantalla debería cambiar de color …");
    let mut event_loop: EventLoop<TestState> =
        EventLoop::try_new().map_err(|e| format!("calloop falló: {e}"))?;
    event_loop
        .handle()
        .insert_source(drm_notifier, |event, _meta, state| match event {
            DrmEvent::VBlank(_crtc) => {
                if let Err(e) = state.compositor.frame_submitted() {
                    eprintln!("      frame_submitted error: {e}");
                }
                state.render();
            }
            DrmEvent::Error(e) => eprintln!("      DRM error: {e}"),
        })
        .map_err(|e| format!("no pude registrar el DRM en calloop: {e}"))?;

    let mut state = TestState {
        compositor,
        renderer,
        frames: 0,
        start: Instant::now(),
    };
    // Primer cuadro: arranca la cadena render → VBlank → render.
    state.render();

    let signal = event_loop.get_signal();
    event_loop
        .run(Some(Duration::from_millis(16)), &mut state, |state| {
            // Tope: ~6 s de cuadros, o 10 s de reloj (anti-cuelgue si no
            // llegaran los VBlank).
            if state.frames >= 360 || state.start.elapsed() > Duration::from_secs(10) {
                signal.stop();
            }
        })
        .map_err(|e| format!("el bucle de eventos falló: {e}"))?;

    println!("──────────────────────────────────────────────────");
    println!("mirada-compositor · fase 2a completada — {} cuadros pintados.", state.frames);
    println!("   Si viste la pantalla cambiar de color, EGL/GBM/modeset/page-flip");
    println!("   funcionan. Copia estos logs y seguimos con la fase 2b (clientes).");
    Ok(())
}
