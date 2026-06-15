use super::*;

impl DrmState {
    /// Procesa un evento de `libinput`: teclado y puntero.
    pub(super) fn handle_input(&mut self, event: InputEvent<LibinputInputBackend>) {
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
                        let sym = handle.modified_sym();
                        // Conmutar de VT (Ctrl+Alt+Fn o XF86Switch_VT_n). Lo
                        // aplica el backend tras el evento (sólo él tiene la
                        // sesión). Se chequea a nivel de keysym, antes del
                        // combo, porque según el keymap no llega como «Fn».
                        if let Some(vt) = crate::vt_target(mods, sym) {
                            st.pending_vt = Some(vt);
                            return FilterResult::Intercept(());
                        }
                        if let Some(combo) = combo_string(mods, sym) {
                            if crate::is_escape_hatch(&combo) {
                                eprintln!(
                                    "mirada-compositor · salida de emergencia ({combo})."
                                );
                                st.running = false;
                                return FilterResult::Intercept(());
                            }
                            if st.grabs.contains(&combo) {
                                st.pending_keybind = Some(combo);
                                return FilterResult::Intercept(());
                            }
                            // Diagnóstico opt-in (`MIRADA_DEBUG_KEYS=1`): un
                            // combo con modificador que NO está en los grabs se
                            // reenvía al cliente (de ahí que «Alt+Tab» escriba un
                            // tab si el keymap no lo tiene). Útil para depurar
                            // atajos sin flujo, ruidoso en uso normal.
                            if combo.contains('+') && st.debug_keys {
                                eprintln!(
                                    "mirada-compositor · tecla no interceptada «{combo}» (grabs={})",
                                    st.grabs.len()
                                );
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
                let (x0, y0) = self.app.pointer_loc;
                // Pre-acotado al bounding box: descarta los outliers extremos
                // sin hacer rondas innecesarias en `clamp_to_outputs`.
                let x = (x0 + event.delta_x()).clamp(0.0, self.output_size.0);
                let y = (y0 + event.delta_y()).clamp(0.0, self.output_size.1);
                // Proyectado al output más cercano si cayó en zona muerta.
                let (x, y) = self.clamp_to_outputs(x, y);
                self.app.pointer_loc = (x, y);
                if self.root_menu.is_some() {
                    // El menú vive en coords locales a su salida. Si esa salida
                    // se desenchufó mientras estaba abierto, el idx queda viejo:
                    // cerramos el menú en vez de indexar fuera de rango.
                    let idx = self.menu_output_idx.unwrap_or(Self::PRIMARY);
                    let Some(r) = self.outputs.get(idx).map(|o| o.rect) else {
                        self.root_menu = None;
                        self.menu_output_idx = None;
                        return;
                    };
                    let lx = x.round() as i32 - r.x;
                    let ly = y.round() as i32 - r.y;
                    self.root_menu.as_mut().unwrap().update_hover(lx, ly);
                    return; // con el menú abierto, el puntero lo navega
                }
                self.app.update_shell_autohide(x, y);
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
                let x = pos.x.clamp(0.0, self.output_size.0);
                let y = pos.y.clamp(0.0, self.output_size.1);
                self.app.pointer_loc = self.clamp_to_outputs(x, y);
                if self.root_menu.is_some() {
                    let (x, y) = self.app.pointer_loc;
                    let idx = self.menu_output_idx.unwrap_or(Self::PRIMARY);
                    let Some(r) = self.outputs.get(idx).map(|o| o.rect) else {
                        self.root_menu = None;
                        self.menu_output_idx = None;
                        return;
                    };
                    let lx = x.round() as i32 - r.x;
                    let ly = y.round() as i32 - r.y;
                    self.root_menu.as_mut().unwrap().update_hover(lx, ly);
                    return; // con el menú abierto, el puntero lo navega
                }
                let (x, y) = self.app.pointer_loc;
                self.app.update_shell_autohide(x, y);
                if !self.drag_update() {
                    self.pointer_motion(time);
                }
            }

            // --- Puntero: botones ----------------------------------------
            InputEvent::PointerButton { event } => {
                let pressed = event.state() == ButtonState::Pressed;
                let button = event.button_code();

                // Menú raíz abierto: el botón se lo come el menú. Click
                // izquierdo sobre una hoja la lanza y cierra; sobre una
                // fila-submenú la abre y sigue; click derecho o fuera cierra.
                // (Sólo al apretar; soltar no hace nada.)
                if pressed && self.root_menu.is_some() {
                    use crate::menu::ClickResult;
                    let (x, y) = self.app.pointer_loc;
                    let idx = self.menu_output_idx.unwrap_or(Self::PRIMARY);
                    let Some(r) = self.outputs.get(idx).map(|o| o.rect) else {
                        self.root_menu = None;
                        self.menu_output_idx = None;
                        return;
                    };
                    let lx = x.round() as i32 - r.x;
                    let ly = y.round() as i32 - r.y;
                    let res = if button == BTN_LEFT {
                        self.root_menu.as_mut().unwrap().click(lx, ly)
                    } else {
                        ClickResult::Close
                    };
                    match res {
                        ClickResult::Launch(cmd) => {
                            self.root_menu = None;
                            self.menu_output_idx = None;
                            self.app.spawn_user(&cmd);
                        }
                        ClickResult::Stay => {}
                        ClickResult::Close => {
                            self.root_menu = None;
                            self.menu_output_idx = None;
                        }
                    }
                    // El click cambió el menú (abrió submenú o lo cerró):
                    // daño para screencopy. Grueso pero raro.
                    crate::screencopy::danar_todo(&mut self.app);
                    return; // el menú captura el botón
                }

                // Click DERECHO sobre el fondo (sin ventana ni `Super`): abre el
                // menú raíz, si hay entradas configuradas. No aplica en greeter.
                if pressed
                    && button == BTN_RIGHT
                    && !self.menu_entries.is_empty()
                    && self.app.mode != BodyMode::Greeter
                {
                    let super_held = self
                        .app
                        .keyboard
                        .as_ref()
                        .is_some_and(|kb| kb.modifier_state().logo);
                    let (x, y) = self.app.pointer_loc;
                    if !super_held && self.window_at(x, y).is_none() {
                        // El menú vive en el monitor donde se hizo el click; su
                        // origen y su rect de acotamiento son **locales** a ese
                        // monitor — así no se sale del borde de su pantalla.
                        let idx = self.output_at_point(x.round() as i32, y.round() as i32);
                        // Sin salida real (0 monitores) no hay dónde anclar el
                        // menú: no lo abrimos en vez de indexar fuera de rango.
                        let Some(r) = self.outputs.get(idx).map(|o| o.rect) else {
                            return;
                        };
                        self.menu_output_idx = Some(idx);
                        self.root_menu = Some(crate::menu::RootMenu::open(
                            x.round() as i32 - r.x,
                            y.round() as i32 - r.y,
                            self.menu_entries.clone(),
                            r.w,
                            r.h,
                        ));
                        // El menú aparece en pantalla: daño para screencopy.
                        crate::screencopy::danar_todo(&mut self.app);
                        return; // el botón abrió el menú, no va al cliente
                    }
                }

                // ¿Empieza un arrastre? `Super`+botón sobre una ventana:
                // izquierdo mueve, derecho redimensiona. En modo greeter no
                // hay arrastre: el login está clavado a pantalla completa.
                if pressed && self.app.drag.is_none() && self.app.mode != BodyMode::Greeter {
                    let super_held = self
                        .app
                        .keyboard
                        .as_ref()
                        .is_some_and(|kb| kb.modifier_state().logo);
                    // `Super`+izquierdo arrastra: una flotante se mueve, una
                    // teselada se reordena (swap). `Super`+derecho redimensiona
                    // (flotando la ventana si estaba teselada).
                    let (x, y) = self.app.pointer_loc;
                    let hit = self.window_at(x, y);
                    let mode = match (button, hit) {
                        (BTN_LEFT, Some(i)) if super_held => Some(if self.app.windows[i].floating {
                            DragMode::Move
                        } else {
                            DragMode::Tile
                        }),
                        (BTN_RIGHT, Some(_)) if super_held => Some(DragMode::Resize),
                        _ => None,
                    };
                    if let (Some(mode), Some(i)) = (mode, hit) {
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

                // Click izquierdo sobre la BARRA DE TÍTULO (sin `Super`): arranca
                // un arrastre Move — saca la ventana de su tile y la lleva
                // flotante, lista para aterrizar en una zona (drag-to-zone) o
                // quedar overflow. La barra deja de ser chrome inerte.
                if pressed
                    && button == BTN_LEFT
                    && self.app.drag.is_none()
                    && self.app.mode != BodyMode::Greeter
                {
                    let (x, y) = self.app.pointer_loc;
                    if let Some(i) = self.titlebar_at(x, y) {
                        let (id, loc, size) = {
                            let w = &self.app.windows[i];
                            (w.id, w.loc, w.size)
                        };
                        self.app.drag = Some(DragGrab {
                            id,
                            mode: DragMode::Move,
                            start_pointer: (x, y),
                            start_rect: (loc.0, loc.1, size.0, size.1),
                        });
                        let ev = self.app.body.clicked(id); // enfoca la agarrada
                        self.app.brain_feed(ev);
                        return; // el arrastre captura el botón
                    }
                }

                // Durante un arrastre los botones no llegan al cliente;
                // soltar cualquiera lo termina. Si se soltó sobre una zona
                // (drag-to-zone), la ventana aterriza en ese rect (flotante);
                // si no, queda flotando donde cayó (overflow, ya aplicado por
                // el último drag_update).
                if self.app.drag.is_some() {
                    if !pressed {
                        let mode = self.app.drag.as_ref().map(|d| d.mode);
                        let id = self.app.drag.as_ref().map(|d| d.id);
                        let zone = self.drag_zone.take();
                        let (px, py) = self.app.pointer_loc;
                        self.app.drag = None;
                        if let (Some(mode), Some(id)) = (mode, id) {
                            if matches!(mode, DragMode::Move | DragMode::Tile) {
                                match zone.and_then(|zi| self.zone_rect(zi)) {
                                    // Sobre una zona: aterriza ahí (flotante posicional).
                                    Some(rect) => {
                                        self.app.brain_feed(BodyEvent::WindowFloatTo { id, rect })
                                    }
                                    // Sin zona: si cayó sobre una tesela, el Cerebro
                                    // la devuelve al mosaico; sobre vacío, sigue
                                    // flotando. Antes acá no pasaba nada → una
                                    // ventana movida no volvía nunca al tile.
                                    None => self.app.brain_feed(BodyEvent::WindowDragged {
                                        id,
                                        x: px as i32,
                                        y: py as i32,
                                    }),
                                }
                            }
                        }
                    }
                    return;
                }

                // Click sobre una barra que acepta teclado (cabezal de shuma):
                // le damos el foco de teclado para poder escribir en el drawer.
                // (El click en sí llega al cliente vía pointer.button de abajo,
                // porque el motion ya enfocó el puntero en esa layer.)
                if pressed {
                    let (x, y) = self.app.pointer_loc;
                    if let Some(surf) = self.app.keyboard_focusable_layer_under(x, y) {
                        if let Some(kb) = self.app.keyboard.clone() {
                            kb.set_focus(&mut self.app, Some(surf), SERIAL_COUNTER.next_serial());
                        }
                    } else if button == BTN_LEFT {
                        // Foco-al-click: la ventana clickeada pide el foco al
                        // Cerebro (que la pinta encima). Independiente del
                        // foco-sigue-ratón; el click sigue llegando al cliente.
                        if let Some(i) = self.window_at(x, y) {
                            if !self.app.windows[i].is_shell {
                                let id = self.app.windows[i].id;
                                let ev = self.app.body.clicked(id);
                                self.app.brain_feed(ev);
                            }
                        }
                    }
                }

                // Botón normal: a la ventana (o layer) bajo el puntero.
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
    pub(super) fn pointer_motion(&mut self, time: u32) {
        let Some(pointer) = self.app.pointer.clone() else {
            return;
        };
        let (x, y) = self.app.pointer_loc;

        // Las capas Overlay/Top (las barras de `pata`) están por encima de las
        // ventanas: el puntero va ahí primero. Sin esto, los clicks sólo llegaban
        // a las ventanas y las barras quedaban muertas al mouse.
        if let Some((surface, loc)) = self.app.layer_under(x, y) {
            pointer.motion(
                &mut self.app,
                Some((surface, loc)),
                &MotionEvent {
                    location: Point::from((x, y)),
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                },
            );
            pointer.frame(&mut self.app);
            // El cliente del layer pondría su propio cursor; por ahora, el default.
            self.app.cursor_status = CursorImageStatus::default_named();
            // Dejamos de sobrevolar cualquier ventana.
            self.last_pointer_window = None;
            return;
        }

        let hit = self.window_at(x, y);
        let focus = hit.map(|i| {
            let w = &self.app.windows[i];
            let (lx, ly) =
                crate::render_loc(w, self.app.output_size.1, self.app.decorations.titlebar_height);
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
        // corresponda — el Cerebro para las teseladas, mirada mismo para
        // el shell (que no vive en el Cerebro). PERO si una layer reclama teclado
        // Exclusive (el drawer Quake de pata abierto), no le robamos el foco al
        // mover el mouse sobre una ventana: seguís escribiendo en el drawer.
        let exclusive_layer = self.app.exclusive_layer_surface().is_some();
        let hovered = hit.map(|i| self.app.windows[i].id);
        if hovered != self.last_pointer_window {
            self.last_pointer_window = hovered;
            match hit {
                _ if exclusive_layer => {}
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
    pub(super) fn drag_update(&mut self) -> bool {
        let Some(drag) = self.app.drag.as_ref() else {
            return false;
        };
        let mode = drag.mode;
        let (spx, spy) = drag.start_pointer;
        let (sx, sy, sw, sh) = drag.start_rect;
        let id = drag.id;

        let (px, py) = self.app.pointer_loc;
        // Drag-to-zone: resalta la zona bajo el puntero (Move/Tile, no Resize).
        // Sobre una zona, la ventana aterrizará ahí al soltar.
        self.drag_zone = if mode == DragMode::Resize { None } else { self.zone_at(px, py) };
        // Arrastre de una teselada: el Cerebro la intercambia con la tesela
        // bajo el puntero — no flota, sólo reordena el stack. Pero si está
        // sobre una zona, suprimimos el swap (se resolverá al soltar).
        if mode == DragMode::Tile {
            if self.drag_zone.is_none() {
                self.app
                    .brain_feed(BodyEvent::WindowDragged { id, x: px as i32, y: py as i32 });
            }
            return true;
        }
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
            DragMode::Tile => unreachable!("Tile se maneja arriba"),
        };
        self.app.brain_feed(BodyEvent::WindowFloatTo { id, rect });
        true
    }

    /// El índice de la ventana visible bajo el punto `(x, y)`, si la hay
    /// — en orden front-to-back (el shell gana a las flotantes, y éstas a
    /// las teseladas).
    pub(super) fn window_at(&self, x: f64, y: f64) -> Option<usize> {
        let mut idx: Vec<usize> = (0..self.app.windows.len())
            .filter(|&i| self.app.windows[i].visible)
            .collect();
        idx.sort_by_key(|&i| {
            let w = &self.app.windows[i];
            (!w.is_shell, !w.floating, !w.focused)
        });
        // `output_h` se usa para anclar el shell al borde inferior; el shell
        // vive en la primaria, así que usamos su altura, no la total. Sin
        // monitores (todos desconectados) no hay ventana que golpear.
        let Some(primary) = self.outputs.get(Self::PRIMARY) else {
            return None;
        };
        let output_h = primary.rect.h;
        let tbh = self.app.decorations.titlebar_height;
        idx.into_iter().find(|&i| {
            let w = &self.app.windows[i];
            let tb = crate::titlebar_for(w, tbh);
            let (lx, ly) = crate::render_loc(w, output_h, tbh);
            let (sw, sh) = crate::surface_px_size(w).unwrap_or((w.size.0, (w.size.1 - tb).max(1)));
            // Impacto sobre la SUPERFICIE (la barra de título es chrome inerte
            // en este MVP: no captura el puntero hacia el cliente).
            x >= lx as f64 && y >= ly as f64 && x < (lx + sw) as f64 && y < (ly + sh) as f64
        })
    }

    /// El índice de la ventana cuya **barra de título** está bajo `(x, y)`, si
    /// la hay (front-to-back). Permite agarrar la ventana por su barra para
    /// arrastrarla (sin `Super`).
    pub(super) fn titlebar_at(&self, x: f64, y: f64) -> Option<usize> {
        let tbh = self.app.decorations.titlebar_height;
        if tbh <= 0 {
            return None;
        }
        // `output_h` se usa para anclar el shell al borde inferior; el shell
        // vive en la primaria, así que usamos su altura, no la total. Sin
        // monitores (todos desconectados) no hay ventana que golpear.
        let Some(primary) = self.outputs.get(Self::PRIMARY) else {
            return None;
        };
        let output_h = primary.rect.h;
        let mut idx: Vec<usize> = (0..self.app.windows.len())
            .filter(|&i| self.app.windows[i].visible)
            .collect();
        idx.sort_by_key(|&i| {
            let w = &self.app.windows[i];
            (!w.is_shell, !w.floating, !w.focused)
        });
        idx.into_iter().find(|&i| {
            let w = &self.app.windows[i];
            let tb = crate::titlebar_for(w, tbh);
            if tb == 0 {
                return false;
            }
            let (lx, ly) = crate::render_loc(w, output_h, tbh);
            let (sw, _) = crate::surface_px_size(w).unwrap_or((w.size.0, (w.size.1 - tb).max(1)));
            let top = ly - tb;
            x >= lx as f64 && y >= top as f64 && x < (lx + sw) as f64 && y < (top + tb) as f64
        })
    }
}
