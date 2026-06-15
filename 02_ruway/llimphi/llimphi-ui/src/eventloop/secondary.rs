// secondary.rs — Gestión de ventanas OS secundarias (opt-in, multiventana).
// Path APARTE del de la primaria: comparten modelo (vive en `self.state`) y
// `Hal`/`Renderer`, pero cada secundaria lleva su surface + caches. Sin
// overlay ni foco (la config no los necesita); se puede ampliar luego.

use super::super::*;

impl<A: App> Runtime<A> {
    /// Aplica un Msg al modelo (que vive en la primaria) e invalida + repinta
    /// TODAS las ventanas. Es el camino de cualquier evento de una secundaria,
    /// así un cambio hecho en la config se refleja al toque en el reproductor
    /// (y viceversa, vía los ticks que pasan por `user_event`).
    pub(super) fn dispatch_model(&mut self, msg: A::Msg) {
        if let Some(prim) = self.state.as_mut() {
            let model = prim.model.take().expect("model");
            prim.model = Some(A::update(model, msg, &self.handle));
            prim.last_render = None;
            prim.window.request_redraw();
        }
        // OJO: NO repintamos las secundarias acá. `dispatch_model` corre en
        // cada Msg (incluido el tick ~33 fps), y repintar una secundaria por
        // tick serializaba dos `acquire()` de swapchain en Wayland FIFO →
        // ralentización y cuelgue. Cada secundaria se repinta sola al
        // interactuar con ella (`handle_secondary_event` llama
        // `render_secondary` tras un cambio) y en su `RedrawRequested` del
        // compositor (expose/resize). El modelo igual quedó actualizado, así
        // que el próximo repintado de la secundaria refleja el cambio.
    }

    /// Despacha un Msg y repinta la secundaria `idx` en el acto (si sigue
    /// viva). El camino de los eventos de una secundaria: como su
    /// `request_redraw` no dispara `RedrawRequested` en algunos compositores,
    /// la pintamos directo tras el cambio.
    pub(super) fn dispatch_and_render_secondary(&mut self, idx: usize, msg: A::Msg) {
        self.dispatch_model(msg);
        if idx < self.secondaries.len() {
            self.render_secondary(idx);
        }
    }

    /// Crea una ventana OS secundaria (o enfoca la existente con esa key). Toma
    /// el `Hal` de la primaria — no levanta un segundo device GPU.
    pub(super) fn open_secondary(
        &mut self,
        event_loop: &llimphi_hal::winit::event_loop::ActiveEventLoop,
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
        let attrs = llimphi_hal::winit::window::WindowAttributes::default()
            .with_title(title)
            .with_inner_size(llimphi_hal::winit::dpi::LogicalSize::new(width, height));
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
            cursor: llimphi_hal::winit::dpi::PhysicalPosition::new(0.0, 0.0),
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
    pub(super) fn render_secondary(&mut self, idx: usize) {
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
    pub(super) fn handle_secondary_event(
        &mut self,
        idx: usize,
        event: llimphi_hal::winit::event::WindowEvent,
    ) {
        use llimphi_hal::winit::event::WindowEvent;
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
                self.secondaries[idx].surface.resize(size.width, size.height);
                self.render_secondary(idx);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.render_secondary(idx);
            }
            WindowEvent::RedrawRequested => {
                self.render_secondary(idx);
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.secondaries[idx].modifiers = mods.state().into();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let mut drag_msg: Option<A::Msg> = None;
                let mut move_call: Option<(ClickAtFn<A::Msg>, f32, f32, f32, f32)> = None;
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
                                DragHandlerKind::Velocity(h) => {
                                    let now = std::time::Instant::now();
                                    drag.samples.push_back((now, dx as f64, dy as f64));
                                    while drag.samples.len() > VELOCITY_MAX_SAMPLES {
                                        drag.samples.pop_front();
                                    }
                                    h(DragPhase::Move, dx, dy, 0.0, 0.0)
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
                        // Movimiento posicional (on_pointer_move_at) en cada move.
                        move_call = sec.last_render.as_ref().and_then(|c| {
                            let i = hit_test_pointer_move(
                                &c.mounted,
                                &c.computed,
                                position.x as f32,
                                position.y as f32,
                            )?;
                            let node = &c.mounted.nodes[i];
                            let h = node.on_pointer_move_at.clone()?;
                            let r = c.computed.get(node.id)?;
                            Some((h, position.x as f32 - r.x, position.y as f32 - r.y, r.w, r.h))
                        });
                    }
                }
                let move_msg = move_call.and_then(|(h, lx, ly, w, hh)| h(lx, ly, w, hh));
                if let Some(msg) = drag_msg.or(move_msg) {
                    self.dispatch_and_render_secondary(idx, msg);
                } else if redraw {
                    self.render_secondary(idx);
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
                    Option<DragVelocityFn<M>>,
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
                                    node.drag_velocity.clone(),
                                    node.drag_payload,
                                    node.on_click.clone(),
                                    node.on_click_at.clone(),
                                    rect,
                                )
                            },
                        )
                    })
                };
                // Misma prioridad que la primaria: drag_velocity > drag_at +
                // on_click_at > drag simple > on_click_at > on_click.
                match hit {
                    Some((_, _, Some(handler_v), payload, _, _, _)) => {
                        self.secondaries[idx].drag = Some(DragState {
                            handler: DragHandlerKind::Velocity(handler_v),
                            last_cursor: cursor,
                            payload,
                            samples: std::collections::VecDeque::with_capacity(VELOCITY_MAX_SAMPLES),
                        });
                        self.render_secondary(idx);
                    }
                    Some((_, Some(handler_at), _, payload, _, click_at, Some((ox, oy, rw, rh)))) => {
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
                            samples: std::collections::VecDeque::new(),
                        });
                        self.render_secondary(idx);
                    }
                    Some((Some(handler), _, _, payload, _, _, _)) => {
                        self.secondaries[idx].drag = Some(DragState {
                            handler: DragHandlerKind::Delta(handler),
                            last_cursor: cursor,
                            payload,
                            samples: std::collections::VecDeque::new(),
                        });
                        self.render_secondary(idx);
                    }
                    Some((_, _, _, _, _, Some(handler), Some((ox, oy, rw, rh)))) => {
                        let lx = cursor.x as f32 - ox;
                        let ly = cursor.y as f32 - oy;
                        if let Some(msg) = handler(lx, ly, rw, rh) {
                            self.dispatch_and_render_secondary(idx, msg);
                        }
                    }
                    Some((_, _, _, _, Some(msg), _, _)) => {
                        self.dispatch_and_render_secondary(idx, msg);
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
                        DragHandlerKind::Velocity(h) => {
                            let (vx, vy) =
                                compute_drag_velocity(&drag.samples, std::time::Instant::now());
                            h(DragPhase::End, 0.0, 0.0, vx, vy)
                        }
                    };
                    if let Some(msg) = end_msg {
                        self.dispatch_model(msg);
                    }
                    self.render_secondary(idx);
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
                let chain: Vec<ScrollFn<A::Msg>> = {
                    let sec = &self.secondaries[idx];
                    sec.last_render
                        .as_ref()
                        .map(|c| {
                            hit_test_scroll_chain(
                                &c.mounted,
                                &c.computed,
                                cursor.x as f32,
                                cursor.y as f32,
                            )
                            .into_iter()
                            .filter_map(|i| c.mounted.nodes[i].on_scroll.clone())
                            .collect()
                        })
                        .unwrap_or_default()
                };
                let mut msg: Option<A::Msg> = None;
                for h in &chain {
                    if let Some(m) = h(wd.x, wd.y) {
                        msg = Some(m);
                        break;
                    }
                }
                if let Some(msg) = msg {
                    self.dispatch_and_render_secondary(idx, msg);
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
                    self.dispatch_and_render_secondary(idx, msg);
                }
            }
            _ => {}
        }
    }
}
