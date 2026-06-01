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
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::{render_elements, Id, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{ImportAll, ImportDma};
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
use mirada_brain::{BodyEvent, CtlReply, Keymap, Rect};

use crate::{
    combo_string, send_frames_surface_tree, App, BodyMode, Brain, ClientState, DragGrab, DragMode,
    Setup,
};

/// El `DrmCompositor` concreto para la salida (un solo GPU).
type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

render_elements! {
    /// Lo que el backend DRM compone en un cuadro: superficies de cliente
    /// y rectángulos de color sólido (el cursor y los marcos de ventana).
    Frame<R> where R: ImportAll;
    Window = WaylandSurfaceRenderElement<R>,
    Solid = SolidColorRenderElement,
}

/// Color de fondo del escritorio cuando no hay nada que lo tape.
const CLEAR_COLOR: [f32; 4] = [0.05, 0.05, 0.08, 1.0];

/// Lado del cursor de software, en píxeles.
const CURSOR_SIZE: i32 = 12;

/// Color del cursor — un cuadrado casi blanco, opaco.
const CURSOR_COLOR: [f32; 4] = [0.95, 0.95, 0.97, 1.0];

/// Lado mínimo de una ventana al redimensionarla con el ratón.
const MIN_WINDOW: i32 = 120;

/// Grosor del marco de una ventana, en píxeles.
const BORDER_WIDTH: i32 = 2;

/// Color del marco de la ventana enfocada — un azul que resalta.
const BORDER_FOCUS: [f32; 4] = [0.36, 0.56, 0.92, 1.0];

/// Color del marco de las ventanas sin foco — gris discreto.
const BORDER_NORMAL: [f32; 4] = [0.22, 0.22, 0.27, 1.0];

/// Los 4 rectángulos `(x, y, w, h)` del marco de una ventana cuyo
/// contenido ocupa `(sx, sy, sw, sh)`. El marco va *hacia adentro* (pisa
/// el borde de la superficie), así nunca se solapa con el de la ventana
/// vecina: arriba, abajo, izquierda, derecha.
fn border_rects(sx: i32, sy: i32, sw: i32, sh: i32) -> [(i32, i32, i32, i32); 4] {
    let bw = BORDER_WIDTH;
    let side_h = (sh - 2 * bw).max(0);
    [
        (sx, sy, sw, bw),
        (sx, sy + sh - bw, sw, bw),
        (sx, sy + bw, bw, side_h),
        (sx + sw - bw, sy + bw, bw, side_h),
    ]
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
    keymap_path: Option<std::path::PathBuf>,
    keymap_watch: Option<mirada_brain::KeymapWatch>,
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
        for w in &mut self.app.windows {
            if !w.visible || w.is_shell {
                continue; // el shell no lleva marco
            }
            let (x, y) = crate::render_loc(w, output_h);
            let (sw, sh) = crate::surface_px_size(w).unwrap_or(w.size);
            let color = if w.focused { BORDER_FOCUS } else { BORDER_NORMAL };
            let rects = border_rects(x, y, sw, sh);
            for (buf, (_, _, bw, bh)) in w.borders.iter_mut().zip(rects) {
                buf.update((bw, bh), color);
            }
        }

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

            // El shell va sobre todo; luego las flotantes; luego las
            // teseladas. `sort_by_key` es estable: respeta el orden de
            // apertura dentro de cada grupo.
            let mut shown: Vec<_> = self.app.windows.iter().filter(|w| w.visible).collect();
            shown.sort_by_key(|w| (!w.is_shell, !w.floating));
            for w in &shown {
                let (x, y) = crate::render_loc(w, output_h);
                let (sw, sh) = crate::surface_px_size(w).unwrap_or(w.size);
                // El marco, encima de la propia superficie de la ventana
                // — el shell no lleva.
                if !w.is_shell {
                    let rects = border_rects(x, y, sw, sh);
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
                        if let Some(combo) = combo_string(mods, handle.modified_sym()) {
                            if crate::is_escape_hatch(&combo) {
                                eprintln!(
                                    "mirada-compositor · salida de emergencia ({combo})."
                                );
                                st.running = false;
                                return FilterResult::Intercept(());
                            }
                            // Ctrl+Alt+Fn: conmutar de VT. Lo aplica el
                            // backend tras el evento (sólo él tiene la sesión).
                            if let Some(vt) = crate::vt_from_combo(&combo) {
                                st.pending_vt = Some(vt);
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
                if !self.drag_update() {
                    self.pointer_motion(time);
                }
            }

            // --- Puntero: botones ----------------------------------------
            InputEvent::PointerButton { event } => {
                let pressed = event.state() == ButtonState::Pressed;
                let button = event.button_code();

                // ¿Empieza un arrastre? `Super`+botón sobre una ventana:
                // izquierdo mueve, derecho redimensiona.
                if pressed && self.app.drag.is_none() {
                    let super_held = self
                        .app
                        .keyboard
                        .as_ref()
                        .is_some_and(|kb| kb.modifier_state().logo);
                    let mode = match button {
                        BTN_LEFT if super_held => Some(DragMode::Move),
                        BTN_RIGHT if super_held => Some(DragMode::Resize),
                        _ => None,
                    };
                    if let Some(mode) = mode {
                        let (x, y) = self.app.pointer_loc;
                        if let Some(i) = self.window_at(x, y) {
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
                }

                // Durante un arrastre los botones no llegan al cliente;
                // soltar cualquiera lo termina.
                if self.app.drag.is_some() {
                    if !pressed {
                        self.app.drag = None;
                    }
                    return;
                }

                // Botón normal: a la ventana bajo el puntero.
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
        let hit = self.window_at(x, y);
        let focus = hit.map(|i| {
            let w = &self.app.windows[i];
            let (lx, ly) = crate::render_loc(w, self.app.output_size.1);
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
        // el shell (que no vive en el Cerebro).
        let hovered = hit.map(|i| self.app.windows[i].id);
        if hovered != self.last_pointer_window {
            self.last_pointer_window = hovered;
            match hit {
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
            (!w.is_shell, !w.floating)
        });
        let output_h = self.app.output_size.1;
        idx.into_iter().find(|&i| {
            let w = &self.app.windows[i];
            let (lx, ly) = crate::render_loc(w, output_h);
            let (sw, sh) = crate::surface_px_size(w).unwrap_or(w.size);
            x >= lx as f64 && y >= ly as f64 && x < (lx + sw) as f64 && y < (ly + sh) as f64
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
    let Setup { mut display, mut app, keymap_path, keymap_watch, ctl } =
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
    let _wl_output = crate::announce_output(
        &display.handle(),
        &out_name,
        mode_w as i32,
        mode_h as i32,
        mode.vrefresh() as i32 * 1000,
    );

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
        crate::spawn_autostart(None);
        // App de arranque: si `MIRADA_STARTUP` trae un comando, se lanza
        // como hijo (hereda `WAYLAND_DISPLAY`) — cómodo para probar sin
        // saltar de VT.
        if let Ok(cmd) = std::env::var("MIRADA_STARTUP") {
            crate::spawn_command(&cmd, None);
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
        keymap_path,
        keymap_watch,
        ctl,
        start: Instant::now(),
        last_windows: 0,
        cursor_id: Id::new(),
        last_pointer_window: None,
        output_size: (mode_w as f64, mode_h as f64),
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
