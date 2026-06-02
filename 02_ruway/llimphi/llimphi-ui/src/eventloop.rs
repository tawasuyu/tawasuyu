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
        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        let surface = WinitSurface::new(&hal, window.clone()).expect("surface");
        let renderer = Renderer::new(&hal).expect("renderer");
        let overlay_compositor = llimphi_hal::OverlayCompositor::new(&hal.device);
        let typesetter = llimphi_text::Typesetter::new();
        window.request_redraw();
        self.state = Some(RuntimeState {
            window,
            hal,
            surface,
            renderer,
            scene: vello::Scene::new(),
            overlay_compositor,
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
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.surface.resize(size.width, size.height);
                // La app puede reaccionar al nuevo viewport (emitir un
                // evento `resize`, recalcular layout, etc.). El update se
                // corre tras reconfigurar la surface; el cache se invalida
                // para repintar con el tamaño nuevo.
                if let Some(msg) =
                    A::on_resize(state.model.as_ref().expect("model"), size.width, size.height)
                {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                }
                state.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // El DPI de la ventana cambió (movida a otro monitor, escalado
                // del sistema). winit envía un Resized aparte para el nuevo
                // tamaño físico; aquí sólo propagamos el factor.
                if let Some(msg) =
                    A::on_scale_factor(state.model.as_ref().expect("model"), scale_factor)
                {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                }
                state.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let prev_cursor = state.cursor;
                state.cursor = position;
                // Drag activo: dispatchear delta al handler + actualizar
                // tracking del drop target hovereado (solo si hay payload).
                if let Some(drag) = state.drag.as_mut() {
                    let dx = (position.x - drag.last_cursor.x) as f32;
                    let dy = (position.y - drag.last_cursor.y) as f32;
                    drag.last_cursor = position;
                    let payload_active = drag.payload.is_some();
                    let mut need_redraw = false;
                    if dx != 0.0 || dy != 0.0 {
                        let msg_opt = match &drag.handler {
                            DragHandlerKind::Delta(h) => h(DragPhase::Move, dx, dy),
                            DragHandlerKind::DeltaAt(h, lx0, ly0) => {
                                h(DragPhase::Move, dx, dy, *lx0, *ly0)
                            }
                        };
                        if let Some(msg) = msg_opt {
                            let model = state.model.take().expect("model");
                            state.model = Some(A::update(model, msg, &self.handle));
                            // Durante drag NO invalidamos el cache —
                            // queda válido para el próximo Move.
                            need_redraw = true;
                        }
                    }
                    if payload_active {
                        if let Some(cache) = state.last_render.as_mut() {
                            let new_drop = hit_test_drop(
                                &cache.mounted,
                                &cache.computed,
                                position.x as f32,
                                position.y as f32,
                            );
                            if new_drop != cache.drop_hover_idx {
                                cache.drop_hover_idx = new_drop;
                                need_redraw = true;
                            }
                        }
                    }
                    if need_redraw {
                        state.window.request_redraw();
                    }
                } else {
                    // Sin drag: chequear hover. Si hay overlay, el
                    // hover-test va contra él; el árbol principal queda
                    // congelado mientras el overlay esté arriba.
                    //
                    // Además del repintado (para el `hover_fill`), si el
                    // nodo recién hovereado declara un `on_pointer_enter`,
                    // lo dispatcheamos: es lo que permite, p.ej., cambiar
                    // de menú con el mouse o abrir un submenú al pasar por
                    // encima. Extraemos el Msg en un scope para soltar el
                    // borrow del cache antes de mutar el modelo.
                    let mut enter_msg: Option<A::Msg> = None;
                    let mut hovered_changed = false;
                    let mut new_hovered: Option<usize> = state.hovered;
                    if let Some(cache) = state.last_render.as_ref() {
                        let (mounted, computed) = match cache.overlay.as_ref() {
                            Some(ov) => (&ov.mounted, &ov.computed),
                            None => (&cache.mounted, &cache.computed),
                        };
                        let new_hover = hit_test_hover(
                            mounted,
                            computed,
                            position.x as f32,
                            position.y as f32,
                        );
                        // Comparamos contra el hover PERSISTENTE (state.hovered),
                        // no contra el del cache: el render recomputa el del cache
                        // al cursor actual cada cuadro, así que en una app que
                        // re-renderiza sin parar la transición de hover se perdería
                        // (y el hover-switch de menús no andaría). Ver `hovered`.
                        if new_hover != state.hovered {
                            hovered_changed = true;
                            enter_msg = new_hover
                                .and_then(|i| mounted.nodes.get(i))
                                .and_then(|n| n.on_pointer_enter.clone());
                        }
                        new_hovered = new_hover;
                    }
                    state.hovered = new_hovered;
                    if hovered_changed {
                        state.window.request_redraw();
                    }
                    if let Some(msg) = enter_msg {
                        let model = state.model.take().expect("model");
                        state.model = Some(A::update(model, msg, &self.handle));
                        // El estado cambió → invalidamos el cache para
                        // re-render (p.ej. el submenú que se abre).
                        state.last_render = None;
                    }
                    let _ = prev_cursor;
                }
            }
            WindowEvent::ModifiersChanged(mods) => {
                state.modifiers = mods.state().into();
            }
            WindowEvent::Ime(ime) if A::ime_allowed() => {
                use llimphi_hal::winit::event::Ime;
                let ev = match ime {
                    Ime::Enabled => ImeEvent::Enabled,
                    Ime::Preedit(text, cursor) => ImeEvent::Preedit { text, cursor },
                    Ime::Commit(text) => ImeEvent::Commit(text),
                    Ime::Disabled => ImeEvent::Disabled,
                };
                if let Some(msg) = A::on_ime(state.model.as_ref().expect("model"), &ev) {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Tab / Shift+Tab mueven el foco entre nodos `focusable`,
                // que administra el runtime. Sólo intercepta si hay
                // enfocables y en Pressed; si no, cae al `on_key` normal
                // (apps que usan Tab para otra cosa lo siguen recibiendo).
                let is_tab = event.state == ElementState::Pressed
                    && matches!(event.logical_key, Key::Named(NamedKey::Tab));
                if is_tab {
                    let order = state
                        .last_render
                        .as_ref()
                        .map(|c| focus_order(&c.mounted, &c.computed))
                        .unwrap_or_default();
                    if !order.is_empty() {
                        let next = next_focus(&order, state.focused, state.modifiers.shift);
                        state.focused = next;
                        if let Some(msg) =
                            A::on_focus(state.model.as_ref().expect("model"), next)
                        {
                            let model = state.model.take().expect("model");
                            state.model = Some(A::update(model, msg, &self.handle));
                        }
                        state.last_render = None;
                        state.window.request_redraw();
                        return;
                    }
                }
                let ev = KeyEvent {
                    key: event.logical_key.clone(),
                    state: match event.state {
                        ElementState::Pressed => KeyState::Pressed,
                        ElementState::Released => KeyState::Released,
                    },
                    text: event.text.as_ref().map(|t| t.to_string()),
                    modifiers: state.modifiers,
                    repeat: event.repeat,
                };
                if let Some(msg) = A::on_key(state.model.as_ref().expect("model"), &ev) {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
            }
            WindowEvent::DroppedFile(path) => {
                // Un evento por archivo (winit los entrega serializados); si
                // el usuario suelta varios, el bucle re-entra y aplicamos
                // updates en orden.
                if let Some(msg) = A::on_file_drop(state.model.as_ref().expect("model"), path) {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Convención winit: LineDelta es líneas; PixelDelta es
                // píxeles físicos (touchpads). En CSS y aquí, positivo
                // (rueda hacia adelante / dos dedos arriba) = scroll
                // hacia arriba, así que invertimos `y` para que el
                // contenido "siga al dedo" en y positivo. `x` queda
                // como llega.
                let wd = match delta {
                    MouseScrollDelta::LineDelta(x, y) => WheelDelta { x, y: -y },
                    MouseScrollDelta::PixelDelta(p) => WheelDelta {
                        x: (p.x as f32) / 20.0,
                        y: -(p.y as f32) / 20.0,
                    },
                };
                let cursor = (state.cursor.x as f32, state.cursor.y as f32);
                // Primero: ¿hay un nodo con `on_scroll` bajo el cursor? Si
                // consume el evento (`Some`), no cae al `on_wheel` global.
                // El overlay tiene prioridad, igual que con clicks. Se
                // extrae el handler en un scope para soltar el borrow del
                // cache antes de mutar el modelo.
                let scroll_handler: Option<ScrollFn<A::Msg>> =
                    if let Some(cache) = state.last_render.as_ref() {
                        let (m, c) = match cache.overlay.as_ref() {
                            Some(ov) => (&ov.mounted, &ov.computed),
                            None => (&cache.mounted, &cache.computed),
                        };
                        hit_test_scroll(m, c, cursor.0, cursor.1)
                            .and_then(|i| m.nodes[i].on_scroll.clone())
                    } else {
                        None
                    };
                let msg = match scroll_handler {
                    Some(h) => h(wd.x, wd.y),
                    None => A::on_wheel(
                        state.model.as_ref().expect("model"),
                        wd,
                        cursor,
                        state.modifiers,
                    ),
                };
                if let Some(msg) = msg {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                // Hit-test contra el cache del último redraw (siempre
                // representa lo visible). Fallback raro: cache vacío.
                let cursor = state.cursor;
                // Click-to-focus: si el click cae sobre un nodo enfocable,
                // el runtime le da el foco ANTES de procesar la acción de
                // click. Extraemos el id en un scope (suelta el borrow del
                // cache) y recién después mutamos el foco/modelo.
                let focus_hit = state
                    .last_render
                    .as_ref()
                    .and_then(|cache| {
                        let (m, c) = match cache.overlay.as_ref() {
                            Some(ov) => (&ov.mounted, &ov.computed),
                            None => (&cache.mounted, &cache.computed),
                        };
                        hit_test_focusable(m, c, cursor.x as f32, cursor.y as f32)
                    });
                if focus_hit.is_some() && focus_hit != state.focused {
                    state.focused = focus_hit;
                    if let Some(msg) =
                        A::on_focus(state.model.as_ref().expect("model"), focus_hit)
                    {
                        let model = state.model.take().expect("model");
                        state.model = Some(A::update(model, msg, &self.handle));
                    }
                    state.last_render = None;
                }
                // Tupla: (drag_fn, drag_at_fn, payload, on_click_msg,
                //         on_click_at_handler, rect: (x, y, w, h))
                type HitInfo<M> = (
                    Option<DragFn<M>>,
                    Option<DragAtFn<M>>,
                    Option<u64>,
                    Option<M>,
                    Option<ClickAtFn<M>>,
                    Option<(f32, f32, f32, f32)>,
                );
                let lookup_hit = |m: &Mounted<A::Msg>, c: &ComputedLayout| -> Option<HitInfo<A::Msg>> {
                    hit_test_click(m, c, cursor.x as f32, cursor.y as f32).map(|i| {
                        let node = &m.nodes[i];
                        let rect = c.get(node.id).map(|r| (r.x, r.y, r.w, r.h));
                        (
                            node.drag.clone(),
                            node.drag_at.clone(),
                            node.drag_payload,
                            node.on_click.clone(),
                            node.on_click_at.clone(),
                            rect,
                        )
                    })
                };
                // Con overlay activo, los clicks van EXCLUSIVAMENTE a él.
                // Si el cursor cae sobre un nodo del overlay sin handler,
                // el click se descarta — la convención de "scrim que
                // dismissa" pide que la app meta su propio fondo
                // clicable con `on_click = DismissOverlay`.
                let idx_and_action: Option<HitInfo<A::Msg>> = if let Some(cache) =
                    state.last_render.as_ref()
                {
                    if let Some(ov) = cache.overlay.as_ref() {
                        lookup_hit(&ov.mounted, &ov.computed)
                    } else {
                        lookup_hit(&cache.mounted, &cache.computed)
                    }
                } else {
                    let model_ref = state.model.as_ref().expect("model");
                    let view = A::view(model_ref);
                    let overlay_view = A::view_overlay(model_ref);
                    let mut layout = LayoutTree::new();
                    let mounted: Mounted<A::Msg> = mount(&mut layout, view);
                    let (w, h) = state.surface.size();
                    let ts = &mut state.typesetter;
                    let computed = {
                        let tmap = &mounted.text_measures;
                        layout
                            .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                                match tmap.get(&nid) {
                                    Some(tm) => measure_text_node(ts, tm, known, avail),
                                    None => llimphi_layout::taffy::Size::ZERO,
                                }
                            })
                            .expect("layout")
                    };
                    if let Some(ov) = overlay_view {
                        let mut olay = LayoutTree::new();
                        let omounted: Mounted<A::Msg> = mount(&mut olay, ov);
                        let ocomp = {
                            let tmap = &omounted.text_measures;
                            olay
                                .compute_with_measure(omounted.root, (w as f32, h as f32), |nid, known, avail| {
                                    match tmap.get(&nid) {
                                        Some(tm) => measure_text_node(ts, tm, known, avail),
                                        None => llimphi_layout::taffy::Size::ZERO,
                                    }
                                })
                                .expect("layout overlay")
                        };
                        lookup_hit(&omounted, &ocomp)
                    } else {
                        lookup_hit(&mounted, &computed)
                    }
                };
                // drag_at + on_click_at COEXISTEN: el press dispara
                // on_click_at (si está) y arranca un drag rastreado con la
                // posición inicial. Diseño pensado para canvas elements
                // que necesitan select-on-press + move-on-drag.
                //
                // En cambio, `drag` simple (sin _at) mantiene la semántica
                // antigua: gana exclusivo sobre on_click.
                if let Some((_, Some(handler_at), payload, _, click_at, Some((ox, oy, rw, rh)))) =
                    &idx_and_action
                {
                    let lx0 = cursor.x as f32 - ox;
                    let ly0 = cursor.y as f32 - oy;
                    // Disparar on_click_at en el press (si también está).
                    if let Some(click_at_h) = click_at {
                        if let Some(msg) = click_at_h(lx0, ly0, *rw, *rh) {
                            let model = state.model.take().expect("model");
                            state.model = Some(A::update(model, msg, &self.handle));
                            state.last_render = None;
                        }
                    }
                    state.drag = Some(DragState {
                        handler: DragHandlerKind::DeltaAt(handler_at.clone(), lx0, ly0),
                        last_cursor: cursor,
                        payload: *payload,
                    });
                    state.window.request_redraw();
                } else if let Some((Some(handler), _, payload, _, _, _)) = &idx_and_action {
                    state.drag = Some(DragState {
                        handler: DragHandlerKind::Delta(handler.clone()),
                        last_cursor: cursor,
                        payload: *payload,
                    });
                    // Si hay payload, repintar para que el drop target
                    // bajo cursor (si lo hay) se ilumine de entrada.
                    if payload.is_some() {
                        if let Some(cache) = state.last_render.as_mut() {
                            let new_drop = hit_test_drop(
                                &cache.mounted,
                                &cache.computed,
                                cursor.x as f32,
                                cursor.y as f32,
                            );
                            if new_drop != cache.drop_hover_idx {
                                cache.drop_hover_idx = new_drop;
                                state.window.request_redraw();
                            }
                        }
                    }
                } else if let Some((_, _, _, _, Some(handler), Some((ox, oy, rw, rh)))) =
                    &idx_and_action
                {
                    // on_click_at gana sobre on_click si ambos existen.
                    let lx = cursor.x as f32 - ox;
                    let ly = cursor.y as f32 - oy;
                    if let Some(msg) = handler(lx, ly, *rw, *rh) {
                        let model = state.model.take().expect("model");
                        state.model = Some(A::update(model, msg, &self.handle));
                        state.last_render = None;
                        state.window.request_redraw();
                    }
                } else if let Some((_, _, _, Some(msg), _, _)) = idx_and_action {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Middle,
                ..
            } => {
                // Middle-click: dispatcha `on_middle_click` del nodo
                // bajo cursor si lo declaró. La capa overlay tiene
                // prioridad (mismo razonamiento que el left/right click).
                let cursor = state.cursor;
                let lookup =
                    |m: &Mounted<A::Msg>, c: &ComputedLayout| -> Option<A::Msg> {
                        hit_test_middle_click(m, c, cursor.x as f32, cursor.y as f32)
                            .and_then(|i| m.nodes[i].on_middle_click.clone())
                    };
                let msg = if let Some(cache) = state.last_render.as_ref() {
                    if let Some(ov) = cache.overlay.as_ref() {
                        lookup(&ov.mounted, &ov.computed)
                    } else {
                        lookup(&cache.mounted, &cache.computed)
                    }
                } else {
                    None
                };
                if let Some(msg) = msg {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                // Right-click: dispatcheamos `on_right_click` o
                // `on_right_click_at` del nodo bajo cursor. La capa
                // overlay tiene prioridad (mismo razonamiento que el
                // left-click). Nodos sin handler de right-click no
                // reaccionan — no "filtramos" al left.
                let cursor = state.cursor;
                let lookup =
                    |m: &Mounted<A::Msg>, c: &ComputedLayout| -> Option<(Option<A::Msg>, Option<ClickAtFn<A::Msg>>, (f32, f32, f32, f32))> {
                        hit_test_right_click(m, c, cursor.x as f32, cursor.y as f32).map(|i| {
                            let node = &m.nodes[i];
                            let rect = c
                                .get(node.id)
                                .map(|r| (r.x, r.y, r.w, r.h))
                                .unwrap_or((0.0, 0.0, 0.0, 0.0));
                            (
                                node.on_right_click.clone(),
                                node.on_right_click_at.clone(),
                                rect,
                            )
                        })
                    };
                let hit = if let Some(cache) = state.last_render.as_ref() {
                    if let Some(ov) = cache.overlay.as_ref() {
                        lookup(&ov.mounted, &ov.computed)
                    } else {
                        lookup(&cache.mounted, &cache.computed)
                    }
                } else {
                    None
                };
                if let Some((msg_opt, at_opt, (ox, oy, rw, rh))) = hit {
                    let msg = if let Some(handler) = at_opt {
                        handler(
                            cursor.x as f32 - ox,
                            cursor.y as f32 - oy,
                            rw,
                            rh,
                        )
                    } else {
                        msg_opt
                    };
                    if let Some(msg) = msg {
                        let model = state.model.take().expect("model");
                        state.model = Some(A::update(model, msg, &self.handle));
                        state.last_render = None;
                        state.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(drag) = state.drag.take() {
                    let cursor = state.cursor;
                    // 1. Drop: si hay payload + drop target bajo cursor,
                    //    invocamos su handler. El Msg resultante se aplica
                    //    ANTES del End del drag — la convención es "drop
                    //    primero, cleanup del drag después".
                    if let Some(payload) = drag.payload {
                        if let Some(cache) = state.last_render.as_ref() {
                            if let Some(idx) = hit_test_drop(
                                &cache.mounted,
                                &cache.computed,
                                cursor.x as f32,
                                cursor.y as f32,
                            ) {
                                if let Some(drop_h) =
                                    cache.mounted.nodes[idx].on_drop.clone()
                                {
                                    if let Some(msg) = (drop_h)(payload) {
                                        let model = state.model.take().expect("model");
                                        state.model = Some(A::update(model, msg, &self.handle));
                                    }
                                }
                            }
                        }
                    }
                    // 2. Cierre del drag.
                    let end_msg = match &drag.handler {
                        DragHandlerKind::Delta(h) => h(DragPhase::End, 0.0, 0.0),
                        DragHandlerKind::DeltaAt(h, lx0, ly0) => {
                            h(DragPhase::End, 0.0, 0.0, *lx0, *ly0)
                        }
                    };
                    if let Some(msg) = end_msg {
                        let model = state.model.take().expect("model");
                        state.model = Some(A::update(model, msg, &self.handle));
                    }
                    // Cache invalidado siempre — hover/drop pueden cambiar
                    // y el modelo posiblemente mutó.
                    state.last_render = None;
                    state.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                // Título dinámico (App::window_title): si cambió respecto del
                // último aplicado, se lo pasamos a winit. Barato: una
                // comparación de String por frame, set_title sólo en el cambio.
                if let Some(t) = A::window_title(state.model.as_ref().expect("model")) {
                    if state.last_title.as_deref() != Some(t.as_str()) {
                        state.window.set_title(&t);
                        state.last_title = Some(t);
                    }
                }
                // Posicioná la ventana de candidatos del IME junto al caret
                // (sólo con IME activo y si la app reporta el área).
                if A::ime_allowed() {
                    if let Some((x, y, w, h)) =
                        A::ime_cursor_area(state.model.as_ref().expect("model"))
                    {
                        state.window.set_ime_cursor_area(
                            PhysicalPosition::new(x as f64, y as f64),
                            llimphi_hal::winit::dpi::PhysicalSize::new(
                                w.max(1.0) as u32,
                                h.max(1.0) as u32,
                            ),
                        );
                    }
                }
                let frame = match state.surface.acquire() {
                    Ok(f) => f,
                    Err(_) => {
                        let (w, h) = state.surface.size();
                        state.surface.resize(w, h);
                        state.window.request_redraw();
                        return;
                    }
                };
                let (w, h) = frame.size();
                let model_ref = state.model.as_ref().expect("model");
                let view = A::view(model_ref);
                let overlay_view = A::view_overlay(model_ref);
                // Reusamos los árboles de layout del runtime: `clear()` +
                // `mount` evita re-allocar el slotmap de taffy por frame.
                state.layout.clear();
                let mounted: Mounted<A::Msg> = mount(&mut state.layout, view);
                let computed = {
                    let ts = &mut state.typesetter;
                    let tmap = &mounted.text_measures;
                    state
                        .layout
                        .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                            match tmap.get(&nid) {
                                Some(tm) => measure_text_node(ts, tm, known, avail),
                                None => llimphi_layout::taffy::Size::ZERO,
                            }
                        })
                        .expect("layout")
                };
                // Mount + layout del overlay en un árbol aparte. Lo
                // computamos con el mismo tamaño de viewport para que
                // un scrim a percent(1.0) cubra toda la pantalla.
                let overlay_built = if let Some(v) = overlay_view {
                    state.overlay_layout.clear();
                    let omounted: Mounted<A::Msg> = mount(&mut state.overlay_layout, v);
                    let ocomputed = {
                        let ts = &mut state.typesetter;
                        let tmap = &omounted.text_measures;
                        state
                            .overlay_layout
                            .compute_with_measure(omounted.root, (w as f32, h as f32), |nid, known, avail| {
                                match tmap.get(&nid) {
                                    Some(tm) => measure_text_node(ts, tm, known, avail),
                                    None => llimphi_layout::taffy::Size::ZERO,
                                }
                            })
                            .expect("layout overlay")
                    };
                    let ohover = hit_test_hover(
                        &omounted,
                        &ocomputed,
                        state.cursor.x as f32,
                        state.cursor.y as f32,
                    );
                    Some(OverlayCache {
                        mounted: omounted,
                        computed: ocomputed,
                        hover_idx: ohover,
                    })
                } else {
                    None
                };
                // Hover en el main solo si NO hay overlay — durante un
                // menú abierto, el fondo no debe reaccionar al ratón.
                let hover_idx = if overlay_built.is_some() {
                    None
                } else {
                    hit_test_hover(
                        &mounted,
                        &computed,
                        state.cursor.x as f32,
                        state.cursor.y as f32,
                    )
                };
                // Drop hover sólo si hay drag activo con payload (un
                // drag bloquea el overlay; rara combinación pero la
                // resolvemos a favor del drag).
                let drop_hover_idx = state
                    .drag
                    .as_ref()
                    .and_then(|d| d.payload.map(|_| ()))
                    .and_then(|_| {
                        hit_test_drop(
                            &mounted,
                            &computed,
                            state.cursor.x as f32,
                            state.cursor.y as f32,
                        )
                    });
                // Z-order del overlay sobre contenido `gpu_paint`: si el
                // árbol principal tiene painters gpu (p. ej. el video de
                // media) Y hay un overlay activo, el overlay NO va en la
                // escena principal (quedaría debajo del blit gpu). Se
                // rasteriza aparte sobre fondo transparente y se compone con
                // alpha DESPUÉS del pase gpu. Sin gpu o sin overlay, el camino
                // de siempre (overlay en la escena principal) — coste cero.
                let composite_overlay =
                    overlay_built.is_some() && has_gpu_painter(&mounted);

                state.scene.reset();
                paint(
                    &mut state.scene,
                    &mounted,
                    &computed,
                    &mut state.typesetter,
                    hover_idx,
                    drop_hover_idx,
                );
                if !composite_overlay {
                    if let Some(ov) = overlay_built.as_ref() {
                        paint(
                            &mut state.scene,
                            &ov.mounted,
                            &ov.computed,
                            &mut state.typesetter,
                            ov.hover_idx,
                            None,
                        );
                    }
                }
                if let Err(e) = state.renderer.render(
                    &state.hal,
                    &state.scene,
                    &frame,
                    palette::css::BLACK,
                ) {
                    eprintln!("render error: {e}");
                }
                let (vw, vh) = frame.size();
                // Capa de overlay aparte (camino composite): vello la
                // rasteriza con fondo transparente en `frame.overlay_view()`.
                // Se renderiza ANTES del pase gpu para que el blit del
                // compositor (en `gpu_encoder`) la lea ya escrita.
                if composite_overlay {
                    if let Some(ov) = overlay_built.as_ref() {
                        state.scene.reset();
                        paint(
                            &mut state.scene,
                            &ov.mounted,
                            &ov.computed,
                            &mut state.typesetter,
                            ov.hover_idx,
                            None,
                        );
                        if let Err(e) = state.renderer.render_to_view(
                            &state.hal,
                            &state.scene,
                            frame.overlay_view(),
                            vw,
                            vh,
                            palette::css::TRANSPARENT,
                        ) {
                            eprintln!("render overlay error: {e}");
                        }
                    }
                }
                // Pasada GPU directo (Fase 1 del SDD §"GPU directo wgpu"):
                // si algún View del main o del overlay registró un
                // `gpu_painter`, ejecutamos todos sus callbacks contra un
                // único `CommandEncoder`, encima de lo que vello acaba de
                // pintar sobre la intermediate. Submitimos antes del
                // present para que el blit al swapchain incluya las
                // primitivas GPU. Si nadie usó el hook, no se crea ni
                // submitea nada — coste cero.
                let mut gpu_encoder = state.hal.device.create_command_encoder(
                    &llimphi_hal::wgpu::CommandEncoderDescriptor {
                        label: Some("llimphi-ui-gpu-paint"),
                    },
                );
                let viewport = frame.size();
                let mut any_gpu = paint_gpu(
                    &mounted,
                    &computed,
                    &state.hal.device,
                    &state.hal.queue,
                    &mut gpu_encoder,
                    frame.view(),
                    viewport,
                );
                if let Some(ov) = overlay_built.as_ref() {
                    // En el camino composite, los painters gpu del overlay van
                    // sobre SU textura; si no, sobre la intermedia.
                    let target = if composite_overlay {
                        frame.overlay_view()
                    } else {
                        frame.view()
                    };
                    any_gpu |= paint_gpu(
                        &ov.mounted,
                        &ov.computed,
                        &state.hal.device,
                        &state.hal.queue,
                        &mut gpu_encoder,
                        target,
                        viewport,
                    );
                }
                // Composición alpha del overlay SOBRE la intermedia (que ya
                // tiene UI + video). Último pase del encoder → corre después
                // del blit del video. Garantiza menús por encima del video.
                if composite_overlay {
                    state.overlay_compositor.composite(
                        &state.hal.device,
                        &mut gpu_encoder,
                        frame.view(),
                        frame.overlay_view(),
                    );
                    any_gpu = true;
                }
                if any_gpu {
                    state
                        .hal
                        .queue
                        .submit(std::iter::once(gpu_encoder.finish()));
                }
                state.surface.present(frame, &state.hal);
                state.last_render = Some(RenderCache {
                    mounted,
                    computed,
                    hover_idx,
                    drop_hover_idx,
                    overlay: overlay_built,
                });
            }
            _ => {}
        }
    }
}

// ── Ventanas secundarias (multiventana, opt-in) ──────────────────────────────
// Path APARTE del de la primaria: comparten modelo (vive en `self.state`) y
// `Hal`/`Renderer`, pero cada secundaria lleva su surface + caches. Sin
// overlay ni foco (la config no los necesita); se puede ampliar luego.
impl<A: App> Runtime<A> {
    /// Aplica un Msg al modelo (que vive en la primaria) e invalida + repinta
    /// TODAS las ventanas. Es el camino de cualquier evento de una secundaria,
    /// así un cambio hecho en la config se refleja al toque en el reproductor
    /// (y viceversa, vía los ticks que pasan por `user_event`).
    fn dispatch_model(&mut self, msg: A::Msg) {
        if let Some(prim) = self.state.as_mut() {
            let model = prim.model.take().expect("model");
            prim.model = Some(A::update(model, msg, &self.handle));
            prim.last_render = None;
            prim.window.request_redraw();
        }
        // Repintamos las secundarias YA, de forma directa, en vez de pedir un
        // redraw: en algunos compositores (Wayland) `request_redraw()` sobre
        // una ventana secundaria no dispara `RedrawRequested`, así que el
        // contenido quedaba congelado en el primer frame y su cache de
        // hit-test (`last_render`) en `None` (los clicks no pegaban en nada).
        // Como `dispatch_model` corre en cada Msg (incluido el tick ~33 fps),
        // esto mantiene cada secundaria viva y su cache fresco.
        for i in 0..self.secondaries.len() {
            self.render_secondary(i);
        }
    }

    /// Crea una ventana OS secundaria (o enfoca la existente con esa key). Toma
    /// el `Hal` de la primaria — no levanta un segundo device GPU.
    fn open_secondary(
        &mut self,
        event_loop: &ActiveEventLoop,
        key: u64,
        title: String,
        width: u32,
        height: u32,
    ) {
        if let Some(sec) = self.secondaries.iter().find(|s| s.key == key) {
            sec.window.focus_window();
            return;
        }
        let Some(prim) = self.state.as_ref() else {
            return; // no hay primaria todavía (no debería pasar)
        };
        let attrs = WindowAttributes::default()
            .with_title(title)
            .with_inner_size(LogicalSize::new(width, height));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("open_window: no pude crear la ventana: {e}");
                return;
            }
        };
        let surface = match WinitSurface::new(&prim.hal, window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("open_window: no pude crear la surface: {e}");
                return;
            }
        };
        window.request_redraw();
        self.secondaries.push(SecondaryState {
            key,
            window,
            surface,
            scene: vello::Scene::new(),
            typesetter: llimphi_text::Typesetter::new(),
            layout: LayoutTree::new(),
            cursor: PhysicalPosition::new(0.0, 0.0),
            modifiers: Modifiers::default(),
            last_render: None,
            hovered: None,
            drag: None,
            last_title: None,
        });
    }

    /// Pinta la ventana secundaria `idx` con `A::secondary_view`. Reusa el
    /// `Hal`/`Renderer` de la primaria; camino simple (sin overlay ni
    /// composite gpu de menús), pero soporta `gpu_paint` por si el contenido
    /// lo usa.
    fn render_secondary(&mut self, idx: usize) {
        let key = self.secondaries[idx].key;
        let Some(prim) = self.state.as_mut() else {
            return;
        };
        // Título dinámico de la secundaria.
        if let Some(t) = A::secondary_title(prim.model.as_ref().expect("model"), key) {
            let sec = &mut self.secondaries[idx];
            if sec.last_title.as_deref() != Some(t.as_str()) {
                sec.window.set_title(&t);
                sec.last_title = Some(t);
            }
        }
        let view = A::secondary_view(prim.model.as_ref().expect("model"), key)
            .unwrap_or_else(|| View::new(Default::default()));
        let hal = &prim.hal;
        let renderer = &mut prim.renderer;
        let sec = &mut self.secondaries[idx];

        let frame = match sec.surface.acquire() {
            Ok(f) => f,
            Err(_) => {
                let (w, h) = sec.surface.size();
                sec.surface.resize(w, h);
                sec.window.request_redraw();
                return;
            }
        };
        let (w, h) = frame.size();
        sec.layout.clear();
        let mounted: Mounted<A::Msg> = mount(&mut sec.layout, view);
        let computed = {
            let ts = &mut sec.typesetter;
            let tmap = &mounted.text_measures;
            sec.layout
                .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(ts, tm, known, avail),
                        None => llimphi_layout::taffy::Size::ZERO,
                    }
                })
                .expect("layout secundario")
        };
        let hover_idx = hit_test_hover(&mounted, &computed, sec.cursor.x as f32, sec.cursor.y as f32);
        let drop_hover_idx = sec
            .drag
            .as_ref()
            .and_then(|d| d.payload)
            .and_then(|_| hit_test_drop(&mounted, &computed, sec.cursor.x as f32, sec.cursor.y as f32));
        sec.scene.reset();
        paint(
            &mut sec.scene,
            &mounted,
            &computed,
            &mut sec.typesetter,
            hover_idx,
            drop_hover_idx,
        );
        if let Err(e) = renderer.render(hal, &sec.scene, &frame, palette::css::BLACK) {
            eprintln!("render secundario error: {e}");
        }
        // gpu_paint del contenido de la secundaria (si lo hubiera).
        let mut enc = hal
            .device
            .create_command_encoder(&llimphi_hal::wgpu::CommandEncoderDescriptor {
                label: Some("llimphi-ui-sec-gpu"),
            });
        let viewport = frame.size();
        let any = paint_gpu(
            &mounted,
            &computed,
            &hal.device,
            &hal.queue,
            &mut enc,
            frame.view(),
            viewport,
        );
        if any {
            hal.queue.submit(std::iter::once(enc.finish()));
        }
        sec.surface.present(frame, hal);
        let _ = (hover_idx, drop_hover_idx); // se usaron al pintar; no se cachean
        sec.last_render = Some(SecRenderCache { mounted, computed });
    }

    /// Atiende un evento de la ventana secundaria `idx`. Subconjunto de lo que
    /// hace la primaria (sin overlay/foco/IME): render, resize, cierre, hover,
    /// click, drag, teclado y rueda — suficiente para un panel de config.
    fn handle_secondary_event(&mut self, idx: usize, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                let key = self.secondaries[idx].key;
                let msg = self
                    .state
                    .as_ref()
                    .and_then(|p| A::on_secondary_close(p.model.as_ref().expect("model"), key));
                self.secondaries.remove(idx);
                if let Some(msg) = msg {
                    self.dispatch_model(msg);
                }
            }
            WindowEvent::Resized(size) => {
                let sec = &mut self.secondaries[idx];
                sec.surface.resize(size.width, size.height);
                sec.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.secondaries[idx].window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.render_secondary(idx);
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.secondaries[idx].modifiers = mods.state().into();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let mut drag_msg: Option<A::Msg> = None;
                let mut redraw = false;
                {
                    let sec = &mut self.secondaries[idx];
                    sec.cursor = position;
                    if let Some(drag) = sec.drag.as_mut() {
                        let dx = (position.x - drag.last_cursor.x) as f32;
                        let dy = (position.y - drag.last_cursor.y) as f32;
                        drag.last_cursor = position;
                        if dx != 0.0 || dy != 0.0 {
                            drag_msg = match &drag.handler {
                                DragHandlerKind::Delta(h) => h(DragPhase::Move, dx, dy),
                                DragHandlerKind::DeltaAt(h, lx0, ly0) => {
                                    h(DragPhase::Move, dx, dy, *lx0, *ly0)
                                }
                            };
                        }
                        redraw = true;
                    } else {
                        let new_hover = sec.last_render.as_ref().and_then(|c| {
                            hit_test_hover(&c.mounted, &c.computed, position.x as f32, position.y as f32)
                        });
                        if new_hover != sec.hovered {
                            sec.hovered = new_hover;
                            redraw = true;
                        }
                    }
                }
                if let Some(msg) = drag_msg {
                    self.dispatch_model(msg);
                } else if redraw {
                    self.secondaries[idx].window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                type SecHit<M> = (
                    Option<DragFn<M>>,
                    Option<DragAtFn<M>>,
                    Option<u64>,
                    Option<M>,
                    Option<ClickAtFn<M>>,
                    Option<(f32, f32, f32, f32)>,
                );
                let cursor = self.secondaries[idx].cursor;
                let hit: Option<SecHit<A::Msg>> = {
                    let sec = &self.secondaries[idx];
                    sec.last_render.as_ref().and_then(|c| {
                        hit_test_click(&c.mounted, &c.computed, cursor.x as f32, cursor.y as f32).map(
                            |i| {
                                let node = &c.mounted.nodes[i];
                                let rect = c.computed.get(node.id).map(|r| (r.x, r.y, r.w, r.h));
                                (
                                    node.drag.clone(),
                                    node.drag_at.clone(),
                                    node.drag_payload,
                                    node.on_click.clone(),
                                    node.on_click_at.clone(),
                                    rect,
                                )
                            },
                        )
                    })
                };
                // Misma prioridad que la primaria: drag_at + on_click_at, luego
                // drag simple, luego on_click_at, luego on_click.
                match hit {
                    Some((_, Some(handler_at), payload, _, click_at, Some((ox, oy, rw, rh)))) => {
                        let lx0 = cursor.x as f32 - ox;
                        let ly0 = cursor.y as f32 - oy;
                        if let Some(h) = click_at {
                            if let Some(msg) = h(lx0, ly0, rw, rh) {
                                self.dispatch_model(msg);
                            }
                        }
                        self.secondaries[idx].drag = Some(DragState {
                            handler: DragHandlerKind::DeltaAt(handler_at, lx0, ly0),
                            last_cursor: cursor,
                            payload,
                        });
                        self.secondaries[idx].window.request_redraw();
                    }
                    Some((Some(handler), _, payload, _, _, _)) => {
                        self.secondaries[idx].drag = Some(DragState {
                            handler: DragHandlerKind::Delta(handler),
                            last_cursor: cursor,
                            payload,
                        });
                        self.secondaries[idx].window.request_redraw();
                    }
                    Some((_, _, _, _, Some(handler), Some((ox, oy, rw, rh)))) => {
                        let lx = cursor.x as f32 - ox;
                        let ly = cursor.y as f32 - oy;
                        if let Some(msg) = handler(lx, ly, rw, rh) {
                            self.dispatch_model(msg);
                        }
                    }
                    Some((_, _, _, Some(msg), _, _)) => {
                        self.dispatch_model(msg);
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                let cursor = self.secondaries[idx].cursor;
                let drag = self.secondaries[idx].drag.take();
                if let Some(drag) = drag {
                    // Drop primero (si hay payload + target), luego End.
                    if let Some(payload) = drag.payload {
                        let drop_h = self.secondaries[idx].last_render.as_ref().and_then(|c| {
                            hit_test_drop(&c.mounted, &c.computed, cursor.x as f32, cursor.y as f32)
                                .and_then(|i| c.mounted.nodes[i].on_drop.clone())
                        });
                        if let Some(h) = drop_h {
                            if let Some(msg) = h(payload) {
                                self.dispatch_model(msg);
                            }
                        }
                    }
                    let end_msg = match &drag.handler {
                        DragHandlerKind::Delta(h) => h(DragPhase::End, 0.0, 0.0),
                        DragHandlerKind::DeltaAt(h, lx0, ly0) => h(DragPhase::End, 0.0, 0.0, *lx0, *ly0),
                    };
                    if let Some(msg) = end_msg {
                        self.dispatch_model(msg);
                    }
                    self.secondaries[idx].last_render = None;
                    self.secondaries[idx].window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let wd = match delta {
                    MouseScrollDelta::LineDelta(x, y) => WheelDelta { x, y: -y },
                    MouseScrollDelta::PixelDelta(p) => WheelDelta {
                        x: (p.x as f32) / 20.0,
                        y: -(p.y as f32) / 20.0,
                    },
                };
                let cursor = self.secondaries[idx].cursor;
                let handler = {
                    let sec = &self.secondaries[idx];
                    sec.last_render.as_ref().and_then(|c| {
                        hit_test_scroll(&c.mounted, &c.computed, cursor.x as f32, cursor.y as f32)
                            .and_then(|i| c.mounted.nodes[i].on_scroll.clone())
                    })
                };
                if let Some(msg) = handler.and_then(|h| h(wd.x, wd.y)) {
                    self.dispatch_model(msg);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let ev = KeyEvent {
                    key: event.logical_key.clone(),
                    state: match event.state {
                        ElementState::Pressed => KeyState::Pressed,
                        ElementState::Released => KeyState::Released,
                    },
                    text: event.text.as_ref().map(|t| t.to_string()),
                    modifiers: self.secondaries[idx].modifiers,
                    repeat: event.repeat,
                };
                let msg = self
                    .state
                    .as_ref()
                    .and_then(|p| A::on_key(p.model.as_ref().expect("model"), &ev));
                if let Some(msg) = msg {
                    self.dispatch_model(msg);
                }
            }
            _ => {}
        }
    }
}
