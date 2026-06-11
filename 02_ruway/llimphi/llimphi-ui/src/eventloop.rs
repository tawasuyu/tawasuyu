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

/// Mapea el [`Cursor`](llimphi_compositor::Cursor) llimphi-native (resuelto por
/// el hit-test de hover) a `winit::window::CursorIcon`. `None` → flecha default.
/// Mantiene al compositor winit-free: la traducción vive sólo en el runtime.
fn to_winit_cursor(c: Option<llimphi_compositor::Cursor>) -> llimphi_hal::winit::window::CursorIcon {
    use llimphi_compositor::Cursor as C;
    use llimphi_hal::winit::window::CursorIcon as I;
    match c {
        None | Some(C::Default) => I::Default,
        Some(C::Pointer) => I::Pointer,
        Some(C::Text) => I::Text,
        Some(C::Crosshair) => I::Crosshair,
        Some(C::Move) => I::Move,
        Some(C::Grab) => I::Grab,
        Some(C::Grabbing) => I::Grabbing,
        Some(C::NotAllowed) => I::NotAllowed,
        Some(C::Wait) => I::Wait,
        Some(C::Progress) => I::Progress,
        Some(C::Help) => I::Help,
        Some(C::ColResize) => I::ColResize,
        Some(C::RowResize) => I::RowResize,
        Some(C::EwResize) => I::EwResize,
        Some(C::NsResize) => I::NsResize,
        Some(C::NeswResize) => I::NeswResize,
        Some(C::NwseResize) => I::NwseResize,
        Some(C::ZoomIn) => I::ZoomIn,
        Some(C::ZoomOut) => I::ZoomOut,
    }
}

/// Resuelve el handler de **escala** (pinch-to-zoom) bajo el punto `(x, y)`
/// contra el cache del último frame (overlay con prioridad, igual que clicks).
/// Devuelve `(handler, focal_x, focal_y)` con el punto focal ya en coordenadas
/// **locales** al rect del nodo. `None` si no hay nodo `on_scale` bajo el
/// cursor. Compartido por el camino Ctrl+rueda y el de `PinchGesture`.
fn scale_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<(ScaleFn<Msg>, f32, f32)> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    hit_test_scale(m, c, x, y).and_then(|i| {
        let node = &m.nodes[i];
        node.on_scale.clone().map(|h| {
            let (fx, fy) = c
                .get(node.id)
                .map(|r| (x - r.x, y - r.y))
                .unwrap_or((0.0, 0.0));
            (h, fx, fy)
        })
    })
}

/// Resuelve el handler de **rotación** (trackpad) bajo `(x, y)` contra el
/// cache del último frame (overlay con prioridad). Espejo de
/// [`scale_hit_from_cache`]. Devuelve `(handler, focal_x, focal_y)` con el
/// punto focal local al rect del nodo. `None` si no hay nodo `on_rotate`.
fn rotate_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<(RotateFn<Msg>, f32, f32)> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    hit_test_rotate(m, c, x, y).and_then(|i| {
        let node = &m.nodes[i];
        node.on_rotate.clone().map(|h| {
            let (fx, fy) = c
                .get(node.id)
                .map(|r| (x - r.x, y - r.y))
                .unwrap_or((0.0, 0.0));
            (h, fx, fy)
        })
    })
}

/// Resuelve el handler de **doble-tap** bajo `(x, y)` contra el cache del
/// último frame (overlay con prioridad). Elige la variante `_at` (con focal
/// local) si está, o el `Msg` directo. `None` si no hay nodo con doble-tap.
fn double_tap_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<GestureResolved<Msg>> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    hit_test_double_tap(m, c, x, y).and_then(|i| {
        let node = &m.nodes[i];
        let (rx, ry, rw, rh) = c.get(node.id).map(|r| (r.x, r.y, r.w, r.h)).unwrap_or_default();
        if let Some(h) = node.on_double_tap_at.clone() {
            Some(GestureResolved::At(h, x - rx, y - ry, rw, rh))
        } else {
            node.on_double_tap.clone().map(GestureResolved::Direct)
        }
    })
}

/// Como [`double_tap_hit_from_cache`] pero para **long-press**.
fn long_press_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<GestureResolved<Msg>> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    hit_test_long_press(m, c, x, y).and_then(|i| {
        let node = &m.nodes[i];
        let (rx, ry, rw, rh) = c.get(node.id).map(|r| (r.x, r.y, r.w, r.h)).unwrap_or_default();
        if let Some(h) = node.on_long_press_at.clone() {
            Some(GestureResolved::At(h, x - rx, y - ry, rw, rh))
        } else {
            node.on_long_press.clone().map(GestureResolved::Direct)
        }
    })
}

/// Resuelve el **ripple** bajo `(x, y)` contra el cache del último frame
/// (overlay con prioridad). Devuelve `(Ripple, lx, ly)`: la config de la onda
/// + el punto del tap relativo al rect del nodo. `None` si no hay nodo ripple.
fn ripple_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<(llimphi_compositor::Ripple, f32, f32)> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    hit_test_ripple(m, c, x, y).and_then(|i| {
        let node = &m.nodes[i];
        node.ripple.map(|rp| {
            let (rx, ry) = c.get(node.id).map(|r| (r.x, r.y)).unwrap_or_default();
            (rp, x - rx, y - ry)
        })
    })
}

// ── Selección de texto fuera del editor (ver `View::selectable`) ──

/// Rect absoluto de un nodo: `(x, y, w, h)`.
type AbsRect = (f32, f32, f32, f32);

/// `true` si el `TextSpec` es de texto **uniforme** (sin `runs`/`spans`): los
/// únicos que la selección fuera-del-editor soporta. Los multicolor/RichText
/// son del editor y se ignoran.
fn spec_is_uniform(spec: &llimphi_compositor::TextSpec) -> bool {
    spec.runs.is_none() && spec.spans.is_none()
}

/// Bajo `(x, y)`, el nodo de texto seleccionable más al frente: su key, su
/// `TextSpec` clonado y su rect absoluto. `None` si no hay texto seleccionable
/// uniforme ahí.
fn selectable_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<(u64, llimphi_compositor::TextSpec, AbsRect)> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    let i = hit_test_selectable(m, c, x, y)?;
    let node = &m.nodes[i];
    let key = node.text_select_key?;
    let spec = node.text.as_ref()?;
    if !spec_is_uniform(spec) {
        return None;
    }
    let r = c.get(node.id)?;
    Some((key, spec.clone(), (r.x, r.y, r.w, r.h)))
}

/// Busca el nodo seleccionable por su `key` estable (para extender el drag o
/// pintar el resaltado en frames posteriores, cuando el `NodeId` ya cambió).
/// Recorre el overlay y el árbol principal. `None` si la key ya no está.
fn selectable_by_key<Msg>(
    cache: &RenderCache<Msg>,
    key: u64,
) -> Option<(llimphi_compositor::TextSpec, AbsRect)> {
    let trees = [
        cache.overlay.as_ref().map(|ov| (&ov.mounted, &ov.computed)),
        Some((&cache.mounted, &cache.computed)),
    ];
    trees
        .into_iter()
        .flatten()
        .find_map(|(m, c)| selectable_node_in(m, c, key))
}

/// Busca en un árbol montado concreto el nodo de texto seleccionable con esa
/// `key` y devuelve su `TextSpec` clonado + rect. Lo usa tanto la búsqueda por
/// cache como el pintado del resaltado en el redraw (que tiene el `Mounted`
/// del frame a mano, no un `RenderCache`).
fn selectable_node_in<Msg>(
    m: &Mounted<Msg>,
    c: &ComputedLayout,
    key: u64,
) -> Option<(llimphi_compositor::TextSpec, AbsRect)> {
    for node in &m.nodes {
        if node.text_select_key == Some(key) {
            let spec = node.text.as_ref()?;
            if !spec_is_uniform(spec) {
                return None;
            }
            let r = c.get(node.id)?;
            return Some((spec.clone(), (r.x, r.y, r.w, r.h)));
        }
    }
    None
}

/// Reconstruye el `parley::Layout` de un nodo de texto, idéntico al que pinta
/// el render (misma ruta cacheada `Typesetter::layout`), para hit-testear y
/// medir la selección. El ancho de wrap es el del rect del nodo.
fn build_selectable_layout(
    ts: &mut llimphi_text::Typesetter,
    spec: &llimphi_compositor::TextSpec,
    width: f32,
) -> llimphi_text::parley::Layout<()> {
    ts.layout(
        &spec.content,
        spec.size_px,
        Some(width),
        spec.alignment,
        spec.line_height,
        spec.italic,
        spec.font_family.as_deref(),
        spec.weight,
        spec.underline,
        spec.strikethrough,
    )
}

/// `true` si la tecla lógica es el carácter `c` (case-insensitive). Para
/// atajos como Ctrl+C sin acoplarse a mayúsculas/minúsculas ni layout.
fn key_is_char(key: &Key, c: char) -> bool {
    matches!(
        key,
        Key::Character(s) if s.chars().next().map(|k| k.eq_ignore_ascii_case(&c)).unwrap_or(false)
    )
}

/// Copia texto al portapapeles del sistema (best-effort). Con la feature
/// `clipboard` usa `arboard`; sin backend (headless) o sin la feature es no-op
/// silencioso — nunca panica.
#[cfg(feature = "clipboard")]
fn copy_to_clipboard(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text.to_string());
    }
}

#[cfg(not(feature = "clipboard"))]
fn copy_to_clipboard(_text: &str) {}

/// Resuelve los [`View::layout_builder`] del árbol de la app en dos pasadas
/// (ver [`llimphi_compositor::expand_layout_builders`]). **Coste cero** cuando
/// ningún nodo usa el builder: devuelve el `view()` sin tocar tras un walk
/// barato. Cuando hay builders: monta el árbol (builders como hojas), computa
/// para conocer sus slots, y reconstruye un `view()` fresco expandiendo cada
/// builder con sus constraints reales. `viewport` en px físicos; `ts` para medir
/// texto igual que el compute principal. Lo llaman el redraw (vía cache) y el
/// fallback de press.
fn resolve_layout_builders<A: App>(
    model: &A::Model,
    viewport: (f32, f32),
    ts: &mut llimphi_text::Typesetter,
) -> View<A::Msg> {
    let view = A::view(model);
    if !has_layout_builder(&view) {
        return view;
    }
    // Pasada 1: montar (builders = hojas con su Style) y computar el layout.
    let mut l1 = LayoutTree::new();
    let m1: Mounted<A::Msg> = mount(&mut l1, view);
    let c1 = {
        let tmap = &m1.text_measures;
        l1.compute_with_measure(m1.root, viewport, |nid, known, avail| {
            match tmap.get(&nid) {
                Some(tm) => measure_text_node(ts, tm, known, avail),
                None => llimphi_layout::taffy::Size::ZERO,
            }
        })
        .expect("layout layout_builder pasada 1")
    };
    let cons = collect_builder_constraints(&m1, &c1);
    // Pasada 2: árbol fresco (mismo Model → misma estructura, mismo pre-orden de
    // builders) + expand con las constraints resueltas.
    expand_layout_builders(A::view(model), &cons)
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
                    // Además del repintado (para el `hover_fill`), si el
                    // nodo recién hovereado declara un `on_pointer_enter`,
                    // lo dispatcheamos: es lo que permite, p.ej., cambiar
                    // de menú con el mouse o abrir un submenú al pasar por
                    // encima. Extraemos el Msg en un scope para soltar el
                    // borrow del cache antes de mutar el modelo.
                    let mut enter_msg: Option<A::Msg> = None;
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
            WindowEvent::PinchGesture { delta, phase, .. } => {
                // Pinch del trackpad (winit lo emite **sólo en macOS/iOS**; en
                // Wayland/Windows el zoom va por Ctrl+rueda, arriba). `delta` es
                // el cambio de escala incremental (p. ej. 0.01 = +1%); lo
                // mapeamos al mismo `on_scale` que Ctrl+rueda, con factor
                // multiplicativo `1.0 + delta`. La fase de winit se traduce a
                // la de gesto (Begin/Update/End) para que el handler pueda, p.
                // ej., abrir/cerrar un estado de zoom en vivo.
                use llimphi_hal::winit::event::TouchPhase;
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
            WindowEvent::RotationGesture { delta, phase, .. } => {
                // Rotación de dos dedos en el trackpad (winit la emite **sólo
                // en macOS**). `delta` viene en **grados**; lo convertimos a
                // radianes para el handler (positivo = horario). La fase de
                // winit se traduce a la de gesto (Begin/Update/End). No hay
                // camino universal por teclado/rueda como sí lo tiene el zoom.
                use llimphi_hal::winit::event::TouchPhase;
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
                // Tupla: (drag_fn, drag_at_fn, drag_velocity_fn, payload,
                //         on_click_msg, on_click_at_handler,
                //         rect: (x, y, w, h))
                type HitInfo<M> = (
                    Option<DragFn<M>>,
                    Option<DragAtFn<M>>,
                    Option<DragVelocityFn<M>>,
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
                            node.drag_velocity.clone(),
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
                        lookup_hit(&omounted, &ocomp)
                    } else {
                        lookup_hit(&mounted, &computed)
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
            WindowEvent::RedrawRequested => {
                // **Retención de frame entero**. Si:
                //  (a) hay scene retenida del frame anterior (`retained`),
                //  (b) `last_render` SIGUE siendo `Some` — la invariante del
                //      runtime es que cualquier handler que muta visualmente
                //      pone `last_render = None`, así que `Some` ⇒ nadie tocó
                //      nada que afecte la pintura,
                //  (c) el frame retenido NO estaba animando ni ripplando
                //      (si lo estaba, el ticker NECESITA avanzarlo),
                //  (d) no hay overlay, drag, ni long-press en curso (camino
                //      conservador: esos casos suelen estar acoplados a
                //      cambios visuales que no atraviesan `last_render`),
                //  (e) el viewport sigue del mismo tamaño,
                // entonces `state.scene` ya tiene EXACTAMENTE lo que hay que
                // mostrar. Saltamos mount + layout + paint y solo hacemos un
                // render+present de la scene retenida. Cubre redraws espurios
                // (expose del compositor, refocus, el último frame de una anim
                // ya asentada). Si algo falla en el acquire, caemos al camino
                // completo (no es un error, sólo un viewport efímero).
                let cache_hit = state.last_render.is_some()
                    && state.drag.is_none()
                    && state.pending_long_press.is_none()
                    && state.retained.as_ref().is_some_and(|r| {
                        !r.animating
                            && !r.rippling
                            && !r.has_overlay
                            && (r.w, r.h) == state.surface.size()
                    });
                if cache_hit {
                    match state.surface.acquire() {
                        Ok(frame) => {
                            if state
                                .renderer
                                .render(&state.hal, &state.scene, &frame, palette::css::BLACK)
                                .is_ok()
                            {
                                state.surface.present(frame, &state.hal);
                                return;
                            }
                            // render falló → cae al camino completo
                        }
                        Err(_) => { /* surface efímera → camino completo */ }
                    }
                }
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
                // LayoutBuilder: resuelve los constructores diferidos en dos
                // pasadas (coste cero si no hay ninguno). Necesita el typesetter
                // para medir, así que va antes de tomar `model_ref` para el overlay.
                let mut view = resolve_layout_builders::<A>(
                    state.model.as_ref().expect("model"),
                    (w as f32, h as f32),
                    &mut state.typesetter,
                );
                // Animaciones implícitas de **tamaño** (`View::animated_size`):
                // reconcila el `View` tree y parcha `style.size` ANTES del
                // mount/layout. Así siblings/hijos reflowean suave (la
                // animación se ve en el layout cascade, no sólo en el rect del
                // nodo aislado). Coste cero sin nodos `animated_size`.
                let frame_now = std::time::Instant::now();
                let size_animating = llimphi_compositor::reconcile_size_anim(
                    &mut view,
                    &mut state.size_anim_registry,
                    frame_now,
                );
                let model_ref = state.model.as_ref().expect("model");
                let overlay_view = A::view_overlay(model_ref);
                // Reusamos los árboles de layout del runtime: `clear()` +
                // `mount` evita re-allocar el slotmap de taffy por frame.
                state.layout.clear();
                let mut mounted: Mounted<A::Msg> = mount(&mut state.layout, view);
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
                // Animaciones implícitas (`View::animated`): reconcilia el árbol
                // con el estado retenido DESPUÉS del layout y ANTES del paint —
                // interpola fill/radius de los nodos con `anim`. Si alguna sigue
                // viva pedimos otro frame al final (ticker autodetenido).
                let now = frame_now;
                let anim_active = state.anim_registry.reconcile(&mut mounted, now);
                // Heroes (`View::hero`): si la misma key cambió de rect entre
                // frames, escribe en `transform` la afín que "vuela" del rect
                // anterior al actual. Independiente del anim_registry — sólo
                // toca `transform`, que el paint ya respeta. Coste cero sin
                // nodos hero.
                let hero_active = state.hero_registry.reconcile(&mut mounted, &computed, now);
                // `size_animating` viene del reconcile previo al mount; lo
                // ORrijimos al `animating` global para que se pida el
                // próximo frame y el `retained.animating == true` invalide
                // la cache de retención (la siguiente pasada reconstruye con
                // el size interpolado).
                let animating = anim_active || hero_active || size_animating;
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
                // Animación de salida (fade-out). 1) Capturá la subescena de
                // cada nodo `exit` presente (snapshot para cuando desaparezca).
                // 2) Reproducí los fantasmas de los que ya se fueron, con
                // opacidad decreciente — por encima del contenido, debajo del
                // overlay. Coste cero si ningún nodo usa `animated_exit`.
                for (idx, end, key) in state.anim_registry.live_exit_nodes(&mounted) {
                    let (dur, easing) = {
                        let a = mounted.nodes[idx].anim.expect("nodo exit lleva anim");
                        (a.duration, a.easing)
                    };
                    let mut sub = vello::Scene::new();
                    paint_range(
                        &mut sub,
                        &mounted,
                        &computed,
                        &mut state.typesetter,
                        None,
                        None,
                        idx,
                        end,
                        vello::kurbo::Affine::IDENTITY,
                    );
                    state.anim_registry.store_live_exit(key, sub, dur, easing);
                }
                state
                    .anim_registry
                    .replay_ghosts(&mut state.scene, now, w as f32, h as f32);
                // Resaltado de la selección de texto activa (sobre el
                // contenido, bajo el overlay). Reconstruye el layout del nodo
                // seleccionado y pinta los rects de `parley::Selection` con un
                // tinte translúcido (deja leer el texto debajo).
                if let Some(tsel) = state.selection {
                    if let Some((spec, (rx, ry, rw, _rh))) =
                        selectable_node_in(&mounted, &computed, tsel.key)
                    {
                        let layout = build_selectable_layout(&mut state.typesetter, &spec, rw);
                        use vello::kurbo::{Affine, Rect};
                        use vello::peniko::{Color, Fill};
                        let hl = Color::from_rgba8(86, 148, 246, 80);
                        let scene = &mut state.scene;
                        tsel.sel.geometry_with(&layout, |bb, _line| {
                            let r = Rect::new(
                                rx as f64 + bb.x0,
                                ry as f64 + bb.y0,
                                rx as f64 + bb.x1,
                                ry as f64 + bb.y1,
                            );
                            scene.fill(Fill::NonZero, Affine::IDENTITY, hl, None, &r);
                        });
                    }
                }
                // Ripples/InkWell: las salpicaduras vivas se pintan sobre el
                // contenido (translúcidas, recortadas al nodo) y debajo del
                // overlay. Si alguna sigue viva, pide otro frame al final.
                let rippling =
                    state
                        .ripple_registry
                        .paint(&mut state.scene, &mounted, &computed, now);
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
                // Backdrop blur (Bloque 11): post-pasada Gauss separable sobre
                // la intermediate, restringida al rect de cada nodo
                // `.backdrop_blur(sigma)`. Sucede TRAS la rasterización vello
                // y ANTES de los `gpu_painter`/composite — los painters GPU
                // que se solapen con el blur ven el rect ya borroneado y se
                // dibujan encima nítidos. Coste cero sin nodos blur (loop
                // vacío + bandera `blurred` queda false).
                let backdrop_blurs =
                    llimphi_compositor::collect_backdrop_blurs(&mounted, &computed);
                let blurred = !backdrop_blurs.is_empty();
                for b in &backdrop_blurs {
                    state.blur_compositor.blur(
                        &state.hal.device,
                        &state.hal.queue,
                        &mut gpu_encoder,
                        frame.view(),
                        viewport,
                        b.rect,
                        b.sigma,
                    );
                }
                let mut any_gpu = blurred
                    | paint_gpu(
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
                // Ticker de animaciones implícitas: si quedó alguna en curso,
                // pedí el próximo frame. Cuando todas se asientan, `animating`
                // queda false y el loop de redraws se detiene solo (sin render
                // ocioso, sin spawn_periodic por animación).
                if animating || rippling {
                    state.window.request_redraw();
                }
                state.retained = Some(RetainedScene {
                    w,
                    h,
                    animating,
                    rippling,
                    has_overlay: overlay_built.is_some(),
                });
                state.last_render = Some(RenderCache {
                    mounted,
                    computed,
                    hover_idx,
                    drop_hover_idx,
                    overlay: overlay_built,
                });
                // AccessKit: tras un paint exitoso, empujamos el árbol al
                // adapter. `update_if_active` se salta el closure si no hay
                // tecnología asistiva escuchando — coste cero en ese caso.
                push_a11y_tree::<A>(state);
            }
            _ => {}
        }
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

// ── Ventanas secundarias (multiventana, opt-in) ──────────────────────────────
// Path APARTE del de la primaria: comparten modelo (vive en `self.state`) y
// `Hal`/`Renderer`, pero cada secundaria lleva su surface + caches. Sin
// overlay ni foco (la config no los necesita); se puede ampliar luego.
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

impl<A: App> Runtime<A> {
    /// Recibe un `accesskit_winit::Event` (ruteado vía `EventLoopProxy` como
    /// `UserEvent::A11y(...)`) y reacciona:
    /// - `InitialTreeRequested`: el lector pidió el árbol inicial → empujamos
    ///   uno desde `last_render` si lo hay, o pedimos un redraw que lo creará.
    /// - `ActionRequested(req)`: el lector quiere ejecutar una acción sobre un
    ///   `NodeId`. v1 soporta `Action::Focus` (mueve `state.focused` + dispara
    ///   `App::on_focus`) y `Action::Click` (ejecuta el `on_click` del nodo).
    /// - `AccessibilityDeactivated`: nada que hacer; el siguiente paint dejará
    ///   de construir trees (el `update_if_active` se autoinhibe).
    fn handle_a11y_event(&mut self, ev: accesskit_winit::Event) {
        use accesskit_winit::WindowEvent as AkWinEvent;
        let Some(state) = self.state.as_mut() else { return };
        match ev.window_event {
            AkWinEvent::InitialTreeRequested => {
                // Si ya pintamos un frame, ese mounted sirve para el árbol
                // inicial. Si no, forzamos un redraw — el path normal llamará
                // a `push_a11y_tree::<A>` al final.
                if state.last_render.is_some() {
                    push_a11y_tree::<A>(state);
                } else {
                    state.window.request_redraw();
                }
            }
            AkWinEvent::ActionRequested(req) => {
                let Some(idx) = crate::a11y::mounted_idx_for(req.target_node) else {
                    return;
                };
                let Some(cache) = state.last_render.as_ref() else {
                    return;
                };
                let Some(node) = cache.mounted.nodes.get(idx) else {
                    return;
                };
                match req.action {
                    accesskit::Action::Focus => {
                        // Si el nodo es focusable, movemos el foco a su id
                        // opaco; si no, lo limpiamos. La app recibe la
                        // transición vía `App::on_focus`.
                        let new_focus = node.focusable;
                        state.focused = new_focus;
                        let model = state.model.as_ref().expect("model");
                        if let Some(msg) = A::on_focus(model, new_focus) {
                            let m = state.model.take().expect("model");
                            state.model = Some(A::update(m, msg, &self.handle));
                        }
                        state.last_render = None;
                        state.window.request_redraw();
                    }
                    accesskit::Action::Click => {
                        // Sólo soportamos `on_click` (Msg directo) en v1. Los
                        // handlers `*_at` necesitan una posición sintética
                        // coherente que no tenemos — los ignoramos.
                        if let Some(msg) = node.on_click.clone() {
                            let m = state.model.take().expect("model");
                            state.model = Some(A::update(m, msg, &self.handle));
                            state.last_render = None;
                            state.window.request_redraw();
                        }
                    }
                    _ => {
                        // Otras acciones (Expand/Collapse/Increment/Decrement/
                        // SetValue/ScrollIntoView/etc.) se sumarán cuando un
                        // widget concreto lo pida — el modelo `SemanticsSpec`
                        // ya tiene los flags relevantes; solo falta cablear el
                        // efecto inverso (acción → mutación de Model).
                    }
                }
            }
            AkWinEvent::AccessibilityDeactivated => {}
        }
    }

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
    fn dispatch_and_render_secondary(&mut self, idx: usize, msg: A::Msg) {
        self.dispatch_model(msg);
        if idx < self.secondaries.len() {
            self.render_secondary(idx);
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
                    }
                }
                if let Some(msg) = drag_msg {
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
