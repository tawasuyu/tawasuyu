// eventloop/mod.rs — Bucle Elm sobre winit: núcleo del runtime llimphi-ui.
//
// Este módulo actúa como organizador: declara los submódulos por responsabilidad
// e implementa los tres puntos de entrada del `ApplicationHandler` de winit
// (`resumed`, `user_event`, `about_to_wait`). El handler de `window_event`
// delega a `input` (primaria) o `secondary` según el `WindowId`.
//
// Submódulos:
//  - helpers   — funciones puras (hit-test helpers, selección, layout builders)
//  - input     — manejo de todos los WindowEvent de la ventana primaria
//  - redraw    — ciclo mount → layout → paint → GPU → present
//  - secondary — gestión de ventanas OS secundarias (opt-in)
//  - a11y_rt   — integración AccessKit en tiempo de ejecución

mod a11y_rt;
mod helpers;
mod input;
mod redraw;
mod secondary;

use super::*;

pub(crate) fn build_window_attributes<A: App>() -> WindowAttributes {
    let (w, h) = A::initial_size();
    let attrs = WindowAttributes::default()
        .with_title(A::title())
        .with_inner_size(LogicalSize::new(w, h));
    // En Linux, `with_name` del trait de Wayland mapea al `app_id` del
    // xdg-toplevel — lo que el compositor (`mirada-compositor`) usa para
    // reconocer ventanas especiales (greeter, launcher…).
    #[cfg(all(target_os = "linux", not(target_os = "android")))]
    {
        if let Some(id) = A::app_id() {
            use llimphi_hal::winit::platform::wayland::WindowAttributesExtWayland;
            return attrs.with_name(id, "");
        }
    }
    attrs
}

/// Empuja al adapter AccessKit el árbol del último frame pintado. Llamar tras
/// guardar `state.last_render`. `update_if_active` no construye el árbol si no
/// hay tecnología asistiva activa (coste cero en ese caso). Pública sólo
/// dentro del crate; las tests no la necesitan.
fn push_a11y_tree<A: App>(state: &mut RuntimeState<A>) {
    let Some(cache) = state.last_render.as_ref() else {
        return;
    };
    // El foco que tenemos es un id opaco u64 (`focusable`); el árbol AccessKit
    // necesita el índice del MountedNode. Resolvemos buscando.
    let focused_idx = state.focused.and_then(|fid| {
        cache
            .mounted
            .nodes
            .iter()
            .position(|n| n.focusable == Some(fid))
    });
    let app_name = A::window_title(state.model.as_ref().expect("model"))
        .unwrap_or_else(|| String::from("Llimphi"));
    let tree_id = state.a11y_tree_id;
    state.a11y_adapter.update_if_active(|| {
        crate::a11y::build_tree(&cache.mounted, &cache.computed, focused_idx, &app_name, tree_id)
    });
}

impl<A: App> ApplicationHandler<UserEvent<A::Msg>> for Runtime<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = event_loop
            .create_window(build_window_attributes::<A>())
            .expect("create window");
        let window = Arc::new(window);
        // IME opt-in: sólo se habilita si la app lo pide (ver `App::ime_allowed`).
        // Con IME activo el texto compuesto llega por `WindowEvent::Ime`.
        if A::ime_allowed() {
            window.set_ime_allowed(true);
        }
        // Adapter AccessKit: lo creamos ANTES del primer redraw, conectado al
        // EventLoopProxy del runtime. El adapter emitirá `accesskit_winit::Event`
        // (Initial tree requested, ActionRequested, deactivated) — nuestro
        // `From<accesskit_winit::Event> for UserEvent<Msg>` los rutea como
        // `UserEvent::A11y(...)` para que entren por el mismo `user_event`.
        let a11y_proxy: EventLoopProxy<UserEvent<A::Msg>> =
            match &self.handle.inner {
                HandleInner::Real(p) => p.clone(),
                HandleInner::Test => unreachable!("resumed sin event loop real"),
                HandleInner::Lifted(_) => unreachable!("el runtime nunca corre con un handle lifteado"),
            };
        let a11y_adapter =
            accesskit_winit::Adapter::with_event_loop_proxy(event_loop, &window, a11y_proxy);
        let a11y_tree_id = accesskit::TreeId(uuid::Uuid::new_v4());
        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        let surface = WinitSurface::new(&hal, window.clone()).expect("surface");
        let renderer = Renderer::new(&hal).expect("renderer");
        let overlay_compositor = llimphi_hal::OverlayCompositor::new(&hal.device);
        let blur_compositor = llimphi_hal::BlurCompositor::new(&hal.device);
        let typesetter = llimphi_text::Typesetter::new();
        window.request_redraw();
        self.state = Some(RuntimeState {
            window,
            hal,
            surface,
            renderer,
            scene: vello::Scene::new(),
            overlay_compositor,
            blur_compositor,
            model: Some(A::init(&self.handle)),
            cursor: PhysicalPosition::new(0.0, 0.0),
            modifiers: Modifiers::default(),
            typesetter,
            layout: LayoutTree::new(),
            overlay_layout: LayoutTree::new(),
            last_render: None,
            hovered: None,
            drag: None,
            focused: None,
            last_title: None,
            anim_registry: llimphi_compositor::AnimRegistry::new(),
            size_anim_registry: llimphi_compositor::SizeAnimRegistry::new(),
            hero_registry: llimphi_compositor::HeroRegistry::new(),
            ripple_registry: llimphi_compositor::RippleRegistry::new(),
            last_tap: None,
            pending_long_press: None,
            retained: None,
            selection: None,
            a11y_adapter,
            a11y_tree_id,
        });
        // Sincroniza el factor de escala inicial (el de la ventana recién
        // creada) ANTES del primer render: así una app que dependa del DPI
        // (p. ej. `devicePixelRatio` en puriy) ya lo tiene correcto en su
        // primera pasada, sin esperar a un ScaleFactorChanged.
        if let Some(state) = self.state.as_mut() {
            let scale = state.window.scale_factor();
            if let Some(msg) = A::on_scale_factor(state.model.as_ref().expect("model"), scale) {
                let model = state.model.take().expect("model");
                state.model = Some(A::update(model, msg, &self.handle));
                state.last_render = None;
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent<A::Msg>) {
        match event {
            UserEvent::Quit => event_loop.exit(),
            UserEvent::Msg(msg) => {
                // Un Msg del canal (Handle::dispatch, ticks periódicos, trabajo
                // de fondo) muta el modelo compartido y repinta TODAS las
                // ventanas — así un cambio se refleja tanto en la primaria como
                // en las secundarias (config) sin importar de dónde vino.
                self.dispatch_model(msg);
            }
            UserEvent::OpenWindow { key, title, width, height } => {
                self.open_secondary(event_loop, key, title, width, height);
            }
            UserEvent::CloseWindow { key } => {
                if let Some(pos) = self.secondaries.iter().position(|s| s.key == key) {
                    // Drop de la SecondaryState → se destruye la ventana/surface.
                    self.secondaries.remove(pos);
                }
            }
            UserEvent::A11y(ev) => {
                self.handle_a11y_event(ev);
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        // ¿El evento es de una ventana secundaria? Lo atiende su handler
        // dedicado (path aparte: la primaria queda 100% intacta).
        if let Some(idx) = self.secondaries.iter().position(|s| s.window.id() == _id) {
            self.handle_secondary_event(idx, event);
            return;
        }
        self.handle_primary_window_event(event_loop, event);
    }

    /// Se ejecuta tras procesar los eventos de cada vuelta, justo antes de que
    /// el loop se duerma. Es donde vence el **long-press**: si hay uno armado y
    /// ya pasó su `deadline` (el botón siguió apretado y quieto), se dispara su
    /// `Msg`. Mientras quede uno pendiente, ponemos `WaitUntil(deadline)` para
    /// que winit nos despierte a tiempo (con `ControlFlow::Wait` el loop dormiría
    /// indefinidamente sin un evento que lo despierte). Sin long-press armado,
    /// volvemos a `Wait` (no dejar un `WaitUntil` viejo: con un deadline pasado
    /// el loop spinearía). Las animaciones implícitas no usan el control flow
    /// (piden frames con `request_redraw`), así que esto no las afecta.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match state.pending_long_press.as_ref() {
            Some(p) => {
                if std::time::Instant::now() >= p.deadline {
                    let handler = state.pending_long_press.take().expect("pending").handler;
                    event_loop.set_control_flow(ControlFlow::Wait);
                    if let Some(msg) = handler.invoke() {
                        let model = state.model.take().expect("model");
                        state.model = Some(A::update(model, msg, &self.handle));
                        state.last_render = None;
                        state.window.request_redraw();
                    }
                } else {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(p.deadline));
                }
            }
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}
