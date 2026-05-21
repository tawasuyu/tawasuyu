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
use smithay::backend::input::{InputEvent, KeyState, KeyboardKeyEvent};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev;
use smithay::input::keyboard::FilterResult;
use smithay::output::OutputModeSource;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{EventLoop, Interest, Mode as CalloopMode, PostAction};
use smithay::reexports::drm::control::connector::State as ConnectorState;
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::{Display, ListeningSocket};
use smithay::utils::{DeviceFd, Scale, Size, Transform, SERIAL_COUNTER};

use mirada_brain::{CtlReply, Keymap};

use crate::{combo_string, send_frames_surface_tree, App, Brain, ClientState, Setup};

/// El `DrmCompositor` concreto para la salida (un solo GPU).
type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

/// Color de fondo del escritorio cuando no hay nada que lo tape.
const CLEAR_COLOR: [f32; 4] = [0.05, 0.05, 0.08, 1.0];

/// El estado del bucle DRM — lo comparten todos los callbacks de `calloop`.
struct DrmState {
    app: App,
    display: Display<App>,
    compositor: Compositor,
    renderer: GlesRenderer,
    /// `true` entre que se encola un page-flip y llega su VBlank.
    pending_flip: bool,
    keymap_path: Option<std::path::PathBuf>,
    keymap_watch: Option<mirada_brain::KeymapWatch>,
    ctl: Option<crate::CtlServer>,
    /// Inicio del compositor — base de tiempos para los frame-callbacks.
    start: Instant,
}

impl DrmState {
    /// Compone las ventanas y, si hubo cambios, encola el cuadro.
    fn render(&mut self) {
        if self.pending_flip {
            return; // aún esperamos el VBlank del cuadro anterior
        }
        // Elementos a pintar: las flotantes primero (lista front-to-back).
        let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = {
            let mut shown: Vec<_> = self.app.windows.iter().filter(|w| w.visible).collect();
            shown.sort_by_key(|w| !w.floating);
            shown
                .iter()
                .flat_map(|w| {
                    render_elements_from_surface_tree(
                        &mut self.renderer,
                        &w.surface,
                        w.loc,
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )
                })
                .collect()
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
    }

    /// Tarea periódica: Cerebro enlazado, recarga del keymap, API de
    /// control, composición y vaciado hacia los clientes.
    fn tick(&mut self) {
        self.app.brain_poll();

        if self.keymap_watch.as_ref().is_some_and(|w| w.changed()) {
            if let Some(path) = &self.keymap_path {
                match Keymap::load(path) {
                    Ok(km) => {
                        let cmd = if let Brain::Embedded(d) = &mut self.app.brain {
                            Some(d.set_keymap(km))
                        } else {
                            None
                        };
                        if let Some(cmd) = cmd {
                            self.app.apply_commands(vec![cmd]);
                        }
                        println!("mirada-compositor · keymap recargado.");
                    }
                    Err(e) => eprintln!("mirada-compositor · keymap inválido: {e}"),
                }
            }
        }

        if let Some(ctl) = &self.ctl {
            while let Some(mut conn) = ctl.poll() {
                let reply = match conn.read_request() {
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

    /// Procesa un evento de `libinput` — por ahora, sólo el teclado.
    fn handle_input(&mut self, event: InputEvent<LibinputInputBackend>) {
        let InputEvent::Keyboard { event } = event else {
            return; // puntero/táctil: pendiente
        };
        let Some(keyboard) = self.app.keyboard.clone() else {
            return;
        };
        let code = event.key_code();
        let key_state = event.state();
        let pressed = key_state == KeyState::Pressed;
        let time = self.start.elapsed().as_millis() as u32;
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
                if let Some(combo) = combo_string(mods, handle.modified_sym()) {
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
    }
}

/// Arranca el Cuerpo sobre DRM/KMS — fases 1, 2a y 2b.
pub fn run() -> Result<(), Box<dyn Error>> {
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
        let Some(&mode) = conn.modes().first() else {
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
    let Setup { mut display, mut app, keymap_path, keymap_watch, ctl } = crate::build_app()?;
    // La salida del Cerebro = el modo del monitor.
    let ev = app.body.add_output(0, mode_w as i32, mode_h as i32);
    app.brain_feed(ev);

    // El socket Wayland por el que se conectan los clientes.
    let listener = ListeningSocket::bind_auto("wayland", 1..32)?;
    let socket_name = listener
        .socket_name()
        .and_then(|s| s.to_str())
        .unwrap_or("wayland-?")
        .to_string();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    println!("      escuchando en WAYLAND_DISPLAY={socket_name}");

    // 8 · El bucle `calloop`: VBlank, teclado, clientes y un timer.
    println!("[8/8] montando el bucle de eventos …");
    let mut event_loop: EventLoop<DrmState> =
        EventLoop::try_new().map_err(|e| format!("calloop falló: {e}"))?;
    let handle = event_loop.handle();

    // Sesión: pausa/activación al cambiar de VT.
    handle
        .insert_source(session_notifier, |event, _, _state| match event {
            SessionEvent::PauseSession => println!("mirada-compositor · sesión en pausa."),
            SessionEvent::ActivateSession => println!("mirada-compositor · sesión activa."),
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

    // Teclado y ratón vía libinput.
    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|()| "libinput: no pude asignar el seat")?;
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

    println!("──────────────────────────────────────────────────");
    println!("mirada-compositor · escritorio en marcha sobre «{out_name}».");
    println!("   Lanza un cliente:  WAYLAND_DISPLAY={socket_name} foot");
    println!("   Salir: Super+Shift+e.");

    let mut state = DrmState {
        app,
        display,
        compositor,
        renderer,
        pending_flip: false,
        keymap_path,
        keymap_watch,
        ctl,
        start: Instant::now(),
    };

    let signal = event_loop.get_signal();
    event_loop
        .run(None, &mut state, |state| {
            if !state.app.running {
                signal.stop();
            }
        })
        .map_err(|e| format!("el bucle de eventos falló: {e}"))?;

    println!("mirada-compositor · adiós.");
    Ok(())
}
