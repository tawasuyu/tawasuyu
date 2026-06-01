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
        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        let surface = WinitSurface::new(&hal, window.clone()).expect("surface");
        let renderer = Renderer::new(&hal).expect("renderer");
        let typesetter = llimphi_text::Typesetter::new();
        window.request_redraw();
        self.state = Some(RuntimeState {
            window,
            hal,
            surface,
            renderer,
            scene: vello::Scene::new(),
            model: Some(A::init(&self.handle)),
            cursor: PhysicalPosition::new(0.0, 0.0),
            modifiers: Modifiers::default(),
            typesetter,
            layout: LayoutTree::new(),
            overlay_layout: LayoutTree::new(),
            last_render: None,
            drag: None,
        });
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent<A::Msg>) {
        match event {
            UserEvent::Quit => event_loop.exit(),
            UserEvent::Msg(msg) => {
                let Some(state) = self.state.as_mut() else {
                    return;
                };
                let model = state.model.take().expect("model");
                state.model = Some(A::update(model, msg, &self.handle));
                state.last_render = None; // model cambió → cache obsoleto
                state.window.request_redraw();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
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
                    if let Some(cache) = state.last_render.as_ref() {
                        let (mounted, computed, prev_idx) = match cache.overlay.as_ref() {
                            Some(ov) => (&ov.mounted, &ov.computed, ov.hover_idx),
                            None => (&cache.mounted, &cache.computed, cache.hover_idx),
                        };
                        let new_hover = hit_test_hover(
                            mounted,
                            computed,
                            position.x as f32,
                            position.y as f32,
                        );
                        if new_hover != prev_idx {
                            hovered_changed = true;
                            enter_msg = new_hover
                                .and_then(|i| mounted.nodes.get(i))
                                .and_then(|n| n.on_pointer_enter.clone());
                        }
                    }
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
            WindowEvent::KeyboardInput { event, .. } => {
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
                if let Some(msg) = A::on_wheel(
                    state.model.as_ref().expect("model"),
                    wd,
                    cursor,
                    state.modifiers,
                ) {
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
                state.scene.reset();
                paint(
                    &mut state.scene,
                    &mounted,
                    &computed,
                    &mut state.typesetter,
                    hover_idx,
                    drop_hover_idx,
                );
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
                if let Err(e) = state.renderer.render(
                    &state.hal,
                    &state.scene,
                    &frame,
                    palette::css::BLACK,
                ) {
                    eprintln!("render error: {e}");
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
                    any_gpu |= paint_gpu(
                        &ov.mounted,
                        &ov.computed,
                        &state.hal.device,
                        &state.hal.queue,
                        &mut gpu_encoder,
                        frame.view(),
                        viewport,
                    );
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
