// input.rs — Manejo de todos los eventos de entrada de la ventana primaria.
// Cubre: teclado, mouse (click/drag/rueda/gestos), IME, drop de archivos,
// resize, DPI. Cada variante de `WindowEvent` queda en su sección.

use super::super::*;
use super::helpers::*;

/// Resultado de un hit-test de click izquierdo. Tupla interna para extraer
/// handlers del cache antes de mutar el modelo (suelta el borrow).
pub(super) type HitInfo<M> = (
    Option<DragFn<M>>,
    Option<DragAtFn<M>>,
    Option<DragVelocityFn<M>>,
    Option<u64>,
    Option<M>,
    Option<ClickAtFn<M>>,
    Option<(f32, f32, f32, f32)>,
);

/// Extrae el `HitInfo` de click izquierdo para un árbol montado dado.
pub(super) fn lookup_click_hit<Msg: Clone>(
    m: &Mounted<Msg>,
    c: &ComputedLayout,
    cx: f32,
    cy: f32,
) -> Option<HitInfo<Msg>> {
    hit_test_click(m, c, cx, cy).map(|i| {
        let node = &m.nodes[i];
        let rect = c.get(node.id).map(|r| (r.x, r.y, r.w, r.h));
        (
            node.drag.clone(),
            node.drag_at.clone(),
            node.drag_velocity.clone(),
            node.drag_payload,
            node.on_click.clone(),
            node.on_click_at.clone(),
            rect,
        )
    })
}

impl<A: App> Runtime<A> {
    /// Maneja un `WindowEvent` de la ventana primaria. La delegación a
    /// secciones internas sigue el orden de frecuencia (más común primero).
    pub(super) fn handle_primary_window_event(
        &mut self,
        event_loop: &llimphi_hal::winit::event_loop::ActiveEventLoop,
        event: llimphi_hal::winit::event::WindowEvent,
    ) {
        use llimphi_hal::winit::event::WindowEvent;
        let Some(state) = self.state.as_mut() else {
            return;
        };
        // Cada window_event debe pasar primero por el adapter AccessKit para
        // que las tecnologías asistivas se enteren del estado real de la
        // ventana (focus_change del SO, cursor moves, etc.). El adapter
        // no consume el evento — lo despacha aparte vía el EventLoopProxy.
        state.a11y_adapter.process_event(&state.window, &event);
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
                self.handle_cursor_moved(position);
            }
            WindowEvent::ModifiersChanged(mods) => {
                let Some(state) = self.state.as_mut() else { return };
                state.modifiers = mods.state().into();
            }
            WindowEvent::Ime(ime) if A::ime_allowed() => {
                self.handle_ime(ime);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_keyboard_input(event);
            }
            WindowEvent::DroppedFile(path) => {
                // Un evento por archivo (winit los entrega serializados); si
                // el usuario suelta varios, el bucle re-entra y aplicamos
                // updates en orden.
                let Some(state) = self.state.as_mut() else { return };
                if let Some(msg) = A::on_file_drop(state.model.as_ref().expect("model"), path) {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_mouse_wheel(delta);
            }
            WindowEvent::PinchGesture { delta, phase, .. } => {
                self.handle_pinch_gesture(delta, phase);
            }
            WindowEvent::RotationGesture { delta, phase, .. } => {
                self.handle_rotation_gesture(delta, phase);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.handle_left_press();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Middle,
                ..
            } => {
                self.handle_middle_press();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                self.handle_right_press();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.handle_left_release();
            }
            WindowEvent::RedrawRequested => {
                super::redraw::handle_redraw::<A>(
                    self.state.as_mut().expect("state en redraw"),
                    &self.handle,
                );
            }
            _ => {}
        }
    }

    // ── Cursor moved ─────────────────────────────────────────────────────────

    fn handle_cursor_moved(
        &mut self,
        position: llimphi_hal::winit::dpi::PhysicalPosition<f64>,
    ) {
        let Some(state) = self.state.as_mut() else { return };
        let prev_cursor = state.cursor;
        state.cursor = position;
        // Selección de texto en curso: extender el foco al punto actual.
        if let Some(tsel) = state.selection.filter(|s| s.dragging) {
            let info = state
                .last_render
                .as_ref()
                .and_then(|c| selectable_by_key(c, tsel.key));
            if let Some((spec, (rx, ry, rw, _rh))) = info {
                let layout = build_selectable_layout(&mut state.typesetter, &spec, rw);
                let lx = position.x as f32 - rx;
                let ly = position.y as f32 - ry;
                let new_sel = tsel.sel.extend_to_point(&layout, lx, ly);
                state.selection = Some(TextSelection { sel: new_sel, ..tsel });
                state.last_render = None;
                state.window.request_redraw();
            }
        }
        // Long-press armado: si el cursor se alejó del origen del press
        // más que el umbral, el gesto pasó a drag/scroll → cancelar.
        if let Some(p) = state.pending_long_press.as_ref() {
            let dx = position.x - p.origin.x;
            let dy = position.y - p.origin.y;
            if (dx * dx + dy * dy).sqrt() > LONG_PRESS_MOVE_CANCEL {
                state.pending_long_press = None;
            }
        }
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
                    DragHandlerKind::Velocity(h) => {
                        // Durante Move, vx=vy=0 — la velocidad sólo
                        // tiene sentido al End. Acá registramos el
                        // sample para esa medición.
                        let now = std::time::Instant::now();
                        drag.samples.push_back((now, dx as f64, dy as f64));
                        while drag.samples.len() > VELOCITY_MAX_SAMPLES {
                            drag.samples.pop_front();
                        }
                        h(DragPhase::Move, dx, dy, 0.0, 0.0)
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
            // Además del repintado (para el `hover_fill`), en la
            // transición de hover dispatcheamos dos Msgs: el
            // `on_pointer_leave` del nodo que abandonamos y el
            // `on_pointer_enter` del recién hovereado. Es lo que
            // permite, p.ej., cambiar de menú con el mouse, abrir un
            // submenú al pasar por encima, o cerrar un tooltip/drawer
            // al salir. Extraemos los Msg en un scope para soltar el
            // borrow del cache antes de mutar el modelo.
            let mut enter_msg: Option<A::Msg> = None;
            let mut leave_msg: Option<A::Msg> = None;
            let mut hovered_changed = false;
            let mut new_hovered: Option<usize> = state.hovered;
            // Forma del cursor del nodo recién hovereado (sólo se
            // resuelve en la transición; ver `to_winit_cursor`).
            let mut new_cursor: Option<Cursor> = None;
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
                    // El nodo que dejamos (índice persistente previo)
                    // dispara su leave; el nuevo, su enter. Ambos se
                    // resuelven contra el árbol montado actual, igual
                    // que el resto del hit-test de hover.
                    leave_msg = state
                        .hovered
                        .and_then(|i| mounted.nodes.get(i))
                        .and_then(|n| n.on_pointer_leave.clone());
                    enter_msg = new_hover
                        .and_then(|i| mounted.nodes.get(i))
                        .and_then(|n| n.on_pointer_enter.clone());
                    new_cursor = hit_test_cursor(
                        mounted,
                        computed,
                        position.x as f32,
                        position.y as f32,
                    );
                }
                new_hovered = new_hover;
            }
            state.hovered = new_hovered;
            if hovered_changed {
                state.window.set_cursor(to_winit_cursor(new_cursor));
                // Invalidar el cache de paint retenido: el `hover_fill`
                // del nodo viejo está pintado en `state.scene` y el del
                // nuevo no — un cache hit re-presentaría el frame stale.
                state.last_render = None;
                state.window.request_redraw();
            }
            // Despachamos leave antes que enter: el orden natural de
            // la transición es "salgo de A, luego entro en B".
            for msg in [leave_msg, enter_msg].into_iter().flatten() {
                let model = state.model.take().expect("model");
                state.model = Some(A::update(model, msg, &self.handle));
                // El estado cambió → invalidamos el cache para
                // re-render (p.ej. el submenú que se abre/cierra).
                state.last_render = None;
            }
            let _ = prev_cursor;
        }
    }

    // ── IME ──────────────────────────────────────────────────────────────────

    fn handle_ime(&mut self, ime: llimphi_hal::winit::event::Ime) {
        use llimphi_hal::winit::event::Ime;
        let Some(state) = self.state.as_mut() else { return };
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

    // ── Teclado ───────────────────────────────────────────────────────────────

    fn handle_keyboard_input(
        &mut self,
        event: llimphi_hal::winit::event::KeyEvent,
    ) {
        let Some(state) = self.state.as_mut() else { return };
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
        // Ctrl/Cmd+C copia la selección de texto activa (fuera del
        // editor). Consume el evento sólo si había algo seleccionado;
        // si no, cae al `on_key` normal (apps que usan Ctrl+C para lo
        // suyo lo siguen recibiendo).
        if event.state == ElementState::Pressed
            && (state.modifiers.ctrl || state.modifiers.meta)
            && key_is_char(&event.logical_key, 'c')
        {
            if let Some(tsel) = state.selection {
                let text = state
                    .last_render
                    .as_ref()
                    .and_then(|c| selectable_by_key(c, tsel.key))
                    .and_then(|(spec, _)| {
                        spec.content.get(tsel.sel.text_range()).map(str::to_string)
                    });
                if let Some(t) = text.filter(|t| !t.is_empty()) {
                    copy_to_clipboard(&t);
                    return;
                }
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

    // ── Rueda del mouse ───────────────────────────────────────────────────────

    fn handle_mouse_wheel(
        &mut self,
        delta: MouseScrollDelta,
    ) {
        let Some(state) = self.state.as_mut() else { return };
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
        // Ctrl+rueda = **pinch-to-zoom sintético** (camino universal de
        // desktop: Wayland/Windows no emiten el gesto de pinch del
        // trackpad). Si hay un nodo `on_scale` bajo el cursor, el zoom
        // lo consume ANTES que el scroll/`on_wheel`. Factor
        // multiplicativo incremental `1.1^(-dy)`: rueda hacia arriba
        // (`dy<0`) agranda, hacia abajo achica. El punto focal es el
        // cursor (en coords locales al nodo), para zoomear "hacia el
        // cursor". Sin nodo zoomeable, Ctrl+rueda cae al `on_wheel`
        // global como siempre (browsers, etc. lo siguen recibiendo).
        if state.modifiers.ctrl {
            let scale_hit = state
                .last_render
                .as_ref()
                .and_then(|cache| scale_hit_from_cache(cache, cursor.0, cursor.1));
            if let Some((h, fx, fy)) = scale_hit {
                let factor = 1.1_f32.powf(-wd.y);
                if let Some(msg) = h(GesturePhase::Update, factor, fx, fy) {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
                return;
            }
        }
        // Primero: ¿hay nodos con `on_scroll` bajo el cursor? Se
        // arma la **cadena** (front→back) y se invoca cada handler
        // en orden; el primero que devuelva `Some` consume el
        // evento. Un scroll interno que llegó al extremo de su eje
        // puede devolver `None` para dejar el sobrante al ancestro
        // scrollable más cercano (scroll anidado). Si todos
        // devuelven `None`, cae al `on_wheel` global.
        // El overlay tiene prioridad, igual que con clicks. Se
        // extraen los handlers en un scope para soltar el borrow
        // del cache antes de mutar el modelo.
        let scroll_chain: Vec<ScrollFn<A::Msg>> =
            if let Some(cache) = state.last_render.as_ref() {
                let (m, c) = match cache.overlay.as_ref() {
                    Some(ov) => (&ov.mounted, &ov.computed),
                    None => (&cache.mounted, &cache.computed),
                };
                hit_test_scroll_chain(m, c, cursor.0, cursor.1)
                    .into_iter()
                    .filter_map(|i| m.nodes[i].on_scroll.clone())
                    .collect()
            } else {
                Vec::new()
            };
        let mut msg: Option<A::Msg> = None;
        for h in &scroll_chain {
            if let Some(m) = h(wd.x, wd.y) {
                msg = Some(m);
                break;
            }
        }
        if msg.is_none() {
            msg = A::on_wheel(
                state.model.as_ref().expect("model"),
                wd,
                cursor,
                state.modifiers,
            );
        }
        if let Some(msg) = msg {
            let model = state.model.take().expect("model");
            state.model = Some(A::update(model, msg, &self.handle));
            state.last_render = None;
            state.window.request_redraw();
        }
    }

    // ── Gestos trackpad ───────────────────────────────────────────────────────

    fn handle_pinch_gesture(
        &mut self,
        delta: f64,
        phase: llimphi_hal::winit::event::TouchPhase,
    ) {
        use llimphi_hal::winit::event::TouchPhase;
        let Some(state) = self.state.as_mut() else { return };
        // Pinch del trackpad (winit lo emite **sólo en macOS/iOS**; en
        // Wayland/Windows el zoom va por Ctrl+rueda, arriba). `delta` es
        // el cambio de escala incremental (p. ej. 0.01 = +1%); lo
        // mapeamos al mismo `on_scale` que Ctrl+rueda, con factor
        // multiplicativo `1.0 + delta`. La fase de winit se traduce a
        // la de gesto (Begin/Update/End) para que el handler pueda, p.
        // ej., abrir/cerrar un estado de zoom en vivo.
        let cursor = (state.cursor.x as f32, state.cursor.y as f32);
        let gphase = match phase {
            TouchPhase::Started => GesturePhase::Begin,
            TouchPhase::Moved => GesturePhase::Update,
            TouchPhase::Ended | TouchPhase::Cancelled => GesturePhase::End,
        };
        // `delta` puede venir NaN según la doc de winit; en ese caso
        // (o fuera de Update) el factor neutro es 1.0.
        let factor = if gphase == GesturePhase::Update && delta.is_finite() {
            (1.0 + delta) as f32
        } else {
            1.0
        };
        let scale_hit = state
            .last_render
            .as_ref()
            .and_then(|cache| scale_hit_from_cache(cache, cursor.0, cursor.1));
        if let Some((h, fx, fy)) = scale_hit {
            if let Some(msg) = h(gphase, factor, fx, fy) {
                let model = state.model.take().expect("model");
                state.model = Some(A::update(model, msg, &self.handle));
                state.last_render = None;
                state.window.request_redraw();
            }
        }
    }

    fn handle_rotation_gesture(
        &mut self,
        delta: f32,
        phase: llimphi_hal::winit::event::TouchPhase,
    ) {
        use llimphi_hal::winit::event::TouchPhase;
        let Some(state) = self.state.as_mut() else { return };
        // Rotación de dos dedos en el trackpad (winit la emite **sólo
        // en macOS**). `delta` viene en **grados**; lo convertimos a
        // radianes para el handler (positivo = horario). La fase de
        // winit se traduce a la de gesto (Begin/Update/End). No hay
        // camino universal por teclado/rueda como sí lo tiene el zoom.
        let cursor = (state.cursor.x as f32, state.cursor.y as f32);
        let gphase = match phase {
            TouchPhase::Started => GesturePhase::Begin,
            TouchPhase::Moved => GesturePhase::Update,
            TouchPhase::Ended | TouchPhase::Cancelled => GesturePhase::End,
        };
        let delta_rad = if gphase == GesturePhase::Update && delta.is_finite() {
            delta.to_radians()
        } else {
            0.0
        };
        let rotate_hit = state
            .last_render
            .as_ref()
            .and_then(|cache| rotate_hit_from_cache(cache, cursor.0, cursor.1));
        if let Some((h, fx, fy)) = rotate_hit {
            if let Some(msg) = h(gphase, delta_rad, fx, fy) {
                let model = state.model.take().expect("model");
                state.model = Some(A::update(model, msg, &self.handle));
                state.last_render = None;
                state.window.request_redraw();
            }
        }
    }

    // ── Click izquierdo ───────────────────────────────────────────────────────

    fn handle_left_press(&mut self) {
        let Some(state) = self.state.as_mut() else { return };
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
        // ── Selección de texto fuera del editor (aditiva) ──────────
        // Si el press cae sobre un nodo `View::selectable`, arranca una
        // selección (anchor = punto). Un press en cualquier otro lado
        // limpia la selección activa. Aditivo: un label seleccionable
        // sin `on_click`/drag no choca con el camino de abajo.
        let sel_hit = state.last_render.as_ref().and_then(|c| {
            selectable_hit_from_cache(c, cursor.x as f32, cursor.y as f32)
        });
        if let Some((key, spec, (rx, ry, rw, _rh))) = sel_hit {
            let layout = build_selectable_layout(&mut state.typesetter, &spec, rw);
            let lx = cursor.x as f32 - rx;
            let ly = cursor.y as f32 - ry;
            let sel = llimphi_text::parley::Selection::from_point(&layout, lx, ly);
            state.selection = Some(TextSelection { key, sel, dragging: true });
            state.last_render = None;
            state.window.request_redraw();
        } else if state.selection.is_some() {
            state.selection = None;
            state.last_render = None;
            state.window.request_redraw();
        }
        // ── Arena de gestos (aditiva): doble-tap + long-press ──────
        // Se resuelven con su propio hit-test y NO tocan el camino de
        // click/drag de abajo (intacto). El árbitro real es el tiempo:
        // doble-tap = dos presses cercanos dentro de una ventana;
        // long-press = un press que sobrevive ~500 ms quieto (lo vence
        // `about_to_wait`, lo cancela el movimiento/release).
        let now = std::time::Instant::now();
        // Doble-tap: si este press cae sobre un nodo con doble-tap y
        // hubo un tap previo cercano y a tiempo, dispará. Si no, este
        // press queda registrado como "primer tap".
        if let Some(resolved) =
            state.last_render.as_ref().and_then(|c| {
                double_tap_hit_from_cache(c, cursor.x as f32, cursor.y as f32)
            })
        {
            let qualifies = double_tap_qualifies(state.last_tap, now, cursor);
            if qualifies {
                state.last_tap = None; // consumido; un 3er tap no re-dispara
                if let Some(msg) = resolved.invoke() {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
            } else {
                state.last_tap = Some((now, cursor));
            }
        }
        // Long-press: si el press cae sobre un nodo con long-press,
        // armalo. `about_to_wait` lo dispara al vencer; CursorMoved lo
        // cancela si el cursor se aleja; el release lo cancela siempre.
        if let Some(handler) = state.last_render.as_ref().and_then(|c| {
            long_press_hit_from_cache(c, cursor.x as f32, cursor.y as f32)
        }) {
            state.pending_long_press = Some(PendingLongPress {
                deadline: now + LONG_PRESS_DELAY,
                origin: cursor,
                handler,
            });
        }
        // Ripple/InkWell: si el press cae sobre un nodo con ripple,
        // dispará la salpicadura desde el punto. Aditivo — no toca el
        // camino click/drag de abajo; un botón con `on_click` +
        // `.ripple(...)` recibe ambos.
        if let Some((rp, lx, ly)) = state
            .last_render
            .as_ref()
            .and_then(|c| ripple_hit_from_cache(c, cursor.x as f32, cursor.y as f32))
        {
            state
                .ripple_registry
                .trigger(rp.key, lx, ly, rp.color, rp.duration, now);
            // El ripple se pinta sobre la scene; invalidamos el cache
            // de paint para que el próximo redraw lo dibuje (sin esto,
            // un cache hit re-presentaría el frame sin la salpicadura).
            state.last_render = None;
            state.window.request_redraw();
        }
        // Con overlay activo, los clicks van EXCLUSIVAMENTE a él.
        // Si el cursor cae sobre un nodo del overlay sin handler,
        // el click se descarta — la convención de "scrim que
        // dismissa" pide que la app meta su propio fondo
        // clicable con `on_click = DismissOverlay`.
        let idx_and_action: Option<HitInfo<A::Msg>> = if let Some(cache) =
            state.last_render.as_ref()
        {
            if let Some(ov) = cache.overlay.as_ref() {
                lookup_click_hit(&ov.mounted, &ov.computed, cursor.x as f32, cursor.y as f32)
            } else {
                lookup_click_hit(&cache.mounted, &cache.computed, cursor.x as f32, cursor.y as f32)
            }
        } else {
            let (w, h) = state.surface.size();
            // Mismo resolve de LayoutBuilder que el redraw, para que el
            // hit-test del fallback vea los hijos producidos por builders.
            let view = resolve_layout_builders::<A>(
                state.model.as_ref().expect("model"),
                (w as f32, h as f32),
                &mut state.typesetter,
            );
            let model_ref = state.model.as_ref().expect("model");
            let overlay_view = A::view_overlay(model_ref);
            let mut layout = LayoutTree::new();
            let mounted: Mounted<A::Msg> = mount(&mut layout, view);
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
                lookup_click_hit(&omounted, &ocomp, cursor.x as f32, cursor.y as f32)
            } else {
                lookup_click_hit(&mounted, &computed, cursor.x as f32, cursor.y as f32)
            }
        };
        // Prioridad: drag_velocity > drag_at + on_click_at > drag >
        // on_click_at > on_click. `drag_velocity` gana exclusivo
        // (no convive con on_click_at) — el caller que lo elige
        // suele querer un fling físico puro (lienzos pan-and-fling).
        if let Some((_, _, Some(handler_v), payload, _, _, _)) = &idx_and_action {
            state.drag = Some(DragState {
                handler: DragHandlerKind::Velocity(handler_v.clone()),
                last_cursor: cursor,
                payload: *payload,
                samples: std::collections::VecDeque::with_capacity(VELOCITY_MAX_SAMPLES),
            });
            state.window.request_redraw();
        } else if let Some((_, Some(handler_at), _, payload, _, click_at, Some((ox, oy, rw, rh)))) =
            &idx_and_action
        {
            // drag_at + on_click_at COEXISTEN: el press dispara
            // on_click_at (si está) y arranca un drag rastreado con
            // la posición inicial. Diseño pensado para canvas
            // elements que necesitan select-on-press + move-on-drag.
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
                samples: std::collections::VecDeque::new(),
            });
            state.window.request_redraw();
        } else if let Some((Some(handler), _, _, payload, _, _, _)) = &idx_and_action {
            // `drag` simple (sin _at, sin velocity) mantiene la
            // semántica antigua: gana exclusivo sobre on_click.
            state.drag = Some(DragState {
                handler: DragHandlerKind::Delta(handler.clone()),
                last_cursor: cursor,
                payload: *payload,
                samples: std::collections::VecDeque::new(),
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
        } else if let Some((_, _, _, _, _, Some(handler), Some((ox, oy, rw, rh)))) =
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
        } else if let Some((_, _, _, _, Some(msg), _, _)) = idx_and_action {
            let model = state.model.take().expect("model");
            state.model = Some(A::update(model, msg, &self.handle));
            state.last_render = None;
            state.window.request_redraw();
        }
    }

    // ── Click medio ───────────────────────────────────────────────────────────

    fn handle_middle_press(&mut self) {
        let Some(state) = self.state.as_mut() else { return };
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

    // ── Click derecho ─────────────────────────────────────────────────────────

    fn handle_right_press(&mut self) {
        let Some(state) = self.state.as_mut() else { return };
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

    // ── Release izquierdo ─────────────────────────────────────────────────────

    fn handle_left_release(&mut self) {
        let Some(state) = self.state.as_mut() else { return };
        // El botón se soltó antes de vencer el long-press → no era un
        // long-press (fue un click/drag); cancelá el gesto armado.
        state.pending_long_press = None;
        // Fin del arrastre de selección: la selección queda viva (para
        // Ctrl/Cmd+C) pero deja de extenderse con el cursor.
        if let Some(tsel) = state.selection.as_mut() {
            tsel.dragging = false;
        }
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
                DragHandlerKind::Velocity(h) => {
                    let (vx, vy) =
                        compute_drag_velocity(&drag.samples, std::time::Instant::now());
                    h(DragPhase::End, 0.0, 0.0, vx, vy)
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
}
