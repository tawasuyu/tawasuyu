// Bucle principal del backend winit (nested).
use crate::*;
use std::time::Instant;
use std::sync::Arc;
use smithay::reexports::wayland_server::ListeningSocket;
use smithay::reexports::winit::platform::pump_events::PumpStatus;
use smithay::backend::winit::{self, WinitEvent};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::draw_render_elements;
use smithay::backend::input::{InputEvent, KeyState, KeyboardKeyEvent};
use smithay::utils::{Rectangle, Transform, SERIAL_COUNTER};
use smithay::backend::renderer::{Color32F, Frame, Renderer};
use smithay::desktop::layer_map_for_output;
use auth_core::SessionTicket;

/// El backend `winit`: corre anidado dentro de una sesión gráfica.
pub(crate) fn run_winit(greeter: bool) -> Result<(), Box<dyn std::error::Error>> {
    let Setup {
        mut display,
        app: mut state,
        watches,
        ctl,
    } = build_app(greeter)?;
    let keyboard = state.keyboard.clone().expect("teclado inicializado");

    // El backend gráfico va primero. winit abre la ventana del compositor
    // dentro de tu sesión gráfica anfitriona, y para encontrarla lee
    // `WAYLAND_DISPLAY` / `DISPLAY` del entorno. Si publicáramos antes
    // nuestro propio socket en `WAYLAND_DISPLAY`, winit intentaría
    // anidarse en nosotros mismos —un socket que aún no atiende a nadie—
    // y se quedaría colgado para siempre.
    let (mut backend, mut winit) = match winit::init::<GlesRenderer>() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("mirada-compositor · no pude abrir la ventana: {e}");
            eprintln!(
                "   El backend `winit` necesita una sesión gráfica anfitriona\n   \
                 (X11 o Wayland) donde dibujar la ventana del compositor.\n   \
                 Aquí no hay ninguna: DISPLAY='{}', WAYLAND_DISPLAY='{}',\n   \
                 XDG_SESSION_TYPE='{}'.\n   \
                 Lánzalo desde un escritorio gráfico, o desde un servidor X\n   \
                 virtual (Xvfb) al que te conectes por VNC.",
                std::env::var("DISPLAY").unwrap_or_default(),
                std::env::var("WAYLAND_DISPLAY").unwrap_or_default(),
                std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "tty".into()),
            );
            return Err(e.into());
        }
    };

    // Ahora sí, nuestro propio socket Wayland — y `WAYLAND_DISPLAY` se
    // publica *después* de winit, sólo para los clientes que lancemos
    // como procesos hijos.
    let listener = ListeningSocket::bind_auto("wayland", 1..32)?;
    let socket_name = listener
        .socket_name()
        .and_then(|s| s.to_str())
        .unwrap_or("wayland-?")
        .to_string();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    println!("mirada-compositor · escuchando en WAYLAND_DISPLAY={socket_name}");
    println!("   lanza un cliente:  WAYLAND_DISPLAY={socket_name} foot");

    let start = Instant::now();
    let mut clients = Vec::new();

    // Con el renderer ya creado, anuncia dmabuf (clientes con GPU).
    announce_dmabuf(&mut state, &display.handle(), backend.renderer());

    // Salida inicial = el tamaño de la ventana winit.
    let win_size = backend.window_size();
    // Backend winit (single-output nested): 100 % nativo y sin transform —
    // los overrides per-output sólo aplican en el backend DRM.
    state.output = Some(announce_output(
        &display.handle(),
        "winit",
        win_size.w,
        win_size.h,
        60_000,
        120,
        Transform::Normal,
    ));
    {
        let ev = state.body.add_output(0, win_size.w, win_size.h);
        state.brain_feed(ev);
        state.output_size = (win_size.w, win_size.h);
    }

    // Modo greeter (DM anidado — útil para iterar la UI del login):
    // lanza el greeter y recibe su tiquet por un canal que el bucle sondea.
    let greeter_rx = if state.mode == BodyMode::Greeter {
        let (tx, rx) = std::sync::mpsc::channel::<SessionTicket>();
        spawn_greeter(move |ticket| {
            let _ = tx.send(ticket);
        })?;
        Some(rx)
    } else {
        None
    };

    while state.running {
        // 1 · Eventos del backend (teclado, redimensión, cierre).
        let status = winit.dispatch_new_events(|event| match event {
            WinitEvent::CloseRequested => state.running = false,
            WinitEvent::Resized { size, .. } => {
                state.output_changed(size.w, size.h);
            }
            WinitEvent::Input(InputEvent::Keyboard { event }) => {
                let code = event.key_code();
                let key_state = event.state();
                let pressed = key_state == KeyState::Pressed;
                let time = start.elapsed().as_millis() as u32;
                keyboard.clone().input::<(), _>(
                    &mut state,
                    code,
                    key_state,
                    SERIAL_COUNTER.next_serial(),
                    time,
                    |st, mods, handle| {
                        if !pressed {
                            return FilterResult::Forward;
                        }
                        if let Some(combo) = combo_string(mods, handle.modified_sym()) {
                            if is_escape_hatch(&combo) {
                                eprintln!("mirada-compositor · salida de emergencia ({combo}).");
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
                if let Some(combo) = state.pending_keybind.take() {
                    let ev = state.body.keybind(combo);
                    state.brain_feed(ev);
                }
            }
            _ => {}
        });
        if let PumpStatus::Exit(_) = status {
            break;
        }

        // 2 · Comandos de un Cerebro enlazado.
        state.brain_poll();

        // 2 bis · El tiquet del greeter (modo DM): dispara el traspaso.
        if let Some(rx) = &greeter_rx {
            while let Ok(ticket) = rx.try_recv() {
                state.complete_greeter_handoff(ticket);
            }
        }

        // 2 ter · Recarga en caliente de keymap/config/reglas si cambiaron.
        // (El backend winit anidado no cachea menú/wallpaper/fuente, así que
        // ignora si la config cambió — sólo importa en el backend DRM.)
        let _ = watches.poll(&mut state);

        // 2 quater · Peticiones del API de control (mirada-ctl).
        if let Some(ctl) = &ctl {
            while let Some(mut conn) = ctl.poll() {
                let reply = match conn.read_request() {
                    Ok(Some(req)) => state.serve_ctl(req),
                    Ok(None) => continue,
                    Err(e) => CtlReply::Error(format!("{e}")),
                };
                let _ = conn.reply(&reply);
            }
        }

        // 3 · Composición de las superficies en sus rectángulos.
        let size = backend.window_size();
        let damage: Rectangle<i32, smithay::utils::Physical> = Rectangle::from_size(size);
        // Etiquetado para poder saltar el frame entero (sin panic) si una
        // operación de GPU falla — paridad con el manejo del backend DRM.
        'frame: {
            let (renderer, mut framebuffer) = match backend.bind() {
                Ok(rf) => rf,
                Err(e) => {
                    eprintln!("mirada-compositor · bind del backbuffer winit falló ({e}); salteo el frame.");
                    break 'frame;
                }
            };
            // Orden de pintado: la lista de elementos va front-to-back
            // (índice 0 = encima): el shell primero —va sobre todo—, luego
            // las flotantes, luego las teseladas. `sort_by_key` es estable:
            // dentro de cada grupo se respeta el orden de apertura.
            let output_h = state.output_size.1;
            // Layer surfaces (waybar, swaybg…): overlay/top van ENCIMA de
            // las ventanas, bottom/background DEBAJO. La lista es front-to-back.
            let (over_layers, under_layers) =
                layer_render_elements(state.output.as_ref(), renderer);
            let mut shown: Vec<&ManagedWindow> =
                state.windows.iter().filter(|w| w.visible).collect();
            shown.sort_by_key(|w| (!w.is_shell, !w.floating));
            // El backend winit anidado no pinta decoración; pasa el alto de
            // barra para que la superficie quede donde el DRM la pondría.
            let tbh = state.decorations.titlebar_height;
            let window_elems = shown
                .iter()
                .filter(|w| buffer_render_sano(&w.surface))
                .flat_map(|w| {
                render_elements_from_surface_tree(
                    renderer,
                    &w.surface,
                    render_loc(w, output_h, tbh),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                )
            });
            let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = over_layers
                .into_iter()
                .chain(window_elems)
                .chain(under_layers)
                .collect();
            let mut frame = match renderer.render(&mut framebuffer, size, Transform::Flipped180) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("mirada-compositor · render winit falló ({e}); salteo el frame.");
                    break 'frame;
                }
            };
            if let Err(e) = frame.clear(Color32F::new(0.05, 0.05, 0.08, 1.0), &[damage]) {
                eprintln!("mirada-compositor · clear winit falló ({e}); salteo el frame.");
                break 'frame;
            }
            if let Err(e) = draw_render_elements(&mut frame, 1.0, &elements, &[damage]) {
                eprintln!("mirada-compositor · draw winit falló ({e}); salteo el frame.");
                break 'frame;
            }
            if let Err(e) = frame.finish() {
                eprintln!("mirada-compositor · finish winit falló ({e}); salteo el frame.");
                break 'frame;
            }

            // Capturas screencopy pendientes: el backbuffer recién compuesto
            // sigue bindeado — se leen los píxeles antes del submit.
            if !state.pending_screencopy.is_empty() {
                if let Some(out) = state.output.clone() {
                    // Salida única del backend winit: origen global (0,0).
                    let capturas = screencopy::tomar_capturas(&mut state, &out, (0, 0));
                    // El backbuffer real de la `EGLSurface` se lee bottom-up.
                    screencopy::servir(renderer, &framebuffer, capturas, false);
                }
            }
        }

        // 4 · Callbacks de frame + clientes nuevos + flush.
        let time = start.elapsed().as_millis() as u32;
        for w in &mut state.windows {
            w.frame_tick = w.frame_tick.wrapping_add(1);
            // Las capas dormidas (zoom-Z) no reciben frame callbacks: el
            // cliente bloquea su bucle y deja de pintar a ciegas.
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
        if let Some(output) = state.output.clone() {
            for layer in layer_map_for_output(&output).layers() {
                send_frames_surface_tree(layer.wl_surface(), time);
            }
        }
        if let Some(stream) = listener.accept()? {
            // El PID del cliente, de las credenciales del socket (`SO_PEERCRED`) —
            // el linaje de las constelaciones (best-effort: `None` si no se leen).
            let pid = peer_pid(&stream);
            match display
                .handle()
                .insert_client(stream, Arc::new(ClientState::with_pid(pid)))
            {
                Ok(client) => clients.push(client),
                Err(e) => eprintln!("mirada-compositor · no pude registrar un cliente winit ({e})."),
            }
        }
        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;

        if let Err(e) = backend.submit(Some(&[damage])) {
            eprintln!("mirada-compositor · submit winit falló ({e}); sigo al próximo frame.");
        }
    }

    // Sesión ajena pendiente (handoff por `exec`): en anidado no hay DRM
    // que ceder, pero soltamos la ventana del host y cedemos igual.
    if let Some((cmd, user)) = state.pending_session.take() {
        drop(backend);
        exec_session(&cmd, user.as_ref());
    }

    println!("mirada-compositor · adiós.");
    Ok(())
}
