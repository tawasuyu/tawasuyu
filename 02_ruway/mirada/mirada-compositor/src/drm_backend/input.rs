use super::*;

impl DrmState {
    /// Procesa un evento de `libinput`: teclado y puntero.
    pub(super) fn handle_input(&mut self, event: InputEvent<LibinputInputBackend>) {
        let time = self.start.elapsed().as_millis() as u32;
        // Cualquier evento de entrada cuenta como actividad: reinicia el reloj de
        // inactividad y, si la pantalla estaba apagada por ocio, deja el pedido
        // de encenderla (lo consume el `tick`). Los eventos de no-input
        // (DeviceAdded, etc.) también — es inofensivo.
        self.app.idle_activity();
        if let Some(off) = self.app.pending_dpms.take() {
            self.set_dpms(off);
        }
        match event {
            // --- Tapa del laptop (SW_LID) --------------------------------
            // En arje no hay logind que maneje la tapa, así que lo hacemos acá.
            // Cerrar la tapa: bloquear + apagar la pantalla (DPMS off). El DPMS
            // off corta el render (render.rs: `if dpms_off { return }`), que es
            // la fuente principal de calor del compositor — clave para no
            // recalentar con la tapa cerrada en la mochila. arje todavía NO
            // suspende de verdad (`BusRequest::Suspend` es stub), así que esto es
            // lo seguro y efectivo hasta que exista suspend real. Abrir la tapa:
            // reanudar (encender); el lock queda hasta que el usuario autentique.
            // Una acción explícita del usuario → ignora inhibidores de inactividad.
            InputEvent::SwitchToggle { event } => {
                // Llamadas calificadas al trait de smithay: el tipo de evento de
                // libinput tiene un `switch()` inherente que devuelve el `Switch`
                // de libinput (otro tipo) y le ganaría a la resolución por método.
                if SwitchToggleEvent::switch(&event) == Some(Switch::Lid) {
                    match SwitchToggleEvent::state(&event) {
                        SwitchState::On => {
                            self.app.request_lock();
                            self.set_dpms(true);
                        }
                        SwitchState::Off => {
                            self.set_dpms(false);
                        }
                    }
                }
            }
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
                            // Cualquier combo (tecla real, con o sin modificador)
                            // significa que Super NO fue un tap solo: desarma la
                            // detección de la tecla Super sola. Un modificador
                            // pelado (Super) no produce combo, así que no desarma.
                            st.super_tap_armed = false;
                            if crate::is_escape_hatch(&combo) {
                                eprintln!(
                                    "mirada-compositor · salida de emergencia ({combo})."
                                );
                                st.running = false;
                                return FilterResult::Intercept(());
                            }
                            // Con un shell de credenciales arriba (login o lock)
                            // ninguna tecla manipula la sesión: switchers, overview
                            // y atajos quedan inertes y todo va al shell. En login
                            // los grabs ni se registran; en lock sí —de ahí el
                            // guard, o `Super+q` cerraría una ventana detrás del
                            // candado—. VT y salida de emergencia ya se atendieron.
                            if st.shell_activo() {
                                return FilterResult::Forward;
                            }
                            // Switchers visuales: Alt-Tab (ventanas) y Win-Tab
                            // (escritorios). Se manejan acá, NO por sus keybinds,
                            // para mostrar el overlay y confirmar al soltar el
                            // modificador. `combo_string` ordena Super+Ctrl+Shift+
                            // Alt, de ahí «Shift+Alt+Tab» / «Super+Shift+Tab».
                            use crate::switcher::SwitcherKind::{Windows, Workspaces};
                            match combo.as_str() {
                                "Alt+Tab" => {
                                    st.switcher_step = Some((Windows, true));
                                    return FilterResult::Intercept(());
                                }
                                "Shift+Alt+Tab" => {
                                    st.switcher_step = Some((Windows, false));
                                    return FilterResult::Intercept(());
                                }
                                "Super+Tab" => {
                                    // En modo **Prezi** el Win+Tab abre la VISTA
                                    // ESPACIAL viva (mosaico de escritorios con sus
                                    // ventanas reales a escala). En los demás modos
                                    // (Hyprland/Direct) usa el switcher de celdas +
                                    // slide de siempre.
                                    let prezi = st.config_workspace_switch_mode()
                                        == mirada_brain::WorkspaceSwitchMode::Prezi;
                                    if prezi {
                                        if st.brain_is_embedded() {
                                            st.overview_step(true); // el Cuerpo lo pinta
                                            return FilterResult::Intercept(());
                                        }
                                        // Enlazado + Prezi: la vista espacial la pinta
                                        // la APP (tiene los datos que el Cuerpo no ve en
                                        // linked). NO interceptamos con el switcher:
                                        // dejamos que «Super+Tab» caiga a los grabs y se
                                        // reenvíe como Keybind. Sin esto, Win+Tab hacía
                                        // el slide «sencillo» aunque el modo fuera Prezi.
                                        // Marcamos el Win+Tab en curso para, al soltar
                                        // Super, reenviarle a la app el commit (el
                                        // release sólo lo ve el Cuerpo).
                                        st.prezi_wintab_linked = true;
                                    } else {
                                        st.switcher_step = Some((Workspaces, true));
                                        return FilterResult::Intercept(());
                                    }
                                }
                                "Super+Shift+Tab" => {
                                    let prezi = st.config_workspace_switch_mode()
                                        == mirada_brain::WorkspaceSwitchMode::Prezi;
                                    if prezi {
                                        if st.brain_is_embedded() {
                                            st.overview_step(false);
                                            return FilterResult::Intercept(());
                                        }
                                        // Enlazado + Prezi: reenvío a la app (ver arriba).
                                        st.prezi_wintab_linked = true;
                                    } else {
                                        st.switcher_step = Some((Workspaces, false));
                                        return FilterResult::Intercept(());
                                    }
                                }
                                // Vista espacial (Prezi): con Cerebro EMBEBIDO la
                                // pinta el compositor (emit_overview), toggle
                                // local con Super+e (Esc cierra). Con Cerebro
                                // ENLAZADO el dueño externo (mirada-app) tiene su
                                // propio overview —con los datos de escritorios
                                // que el compositor no ve en linked—, así que NO
                                // interceptamos: el atajo cae a los grabs y se
                                // reenvía como `BodyEvent::Keybind`.
                                "Super+e" if st.brain_is_embedded() => {
                                    // Toggle: abre (con zoom-out) o pide cierre (con
                                    // zoom-in). No es Win+Tab, así que NO se cierra
                                    // al soltar Super — sólo con Super+e/Esc/click.
                                    if st.overview_open {
                                        st.overview_closing = true; // cierre animado
                                    } else {
                                        st.overview_open = true;
                                        st.overview_closing = false;
                                        st.overview_via_wintab = false;
                                        // Resaltado en el escritorio actual (se
                                        // navega con click; no se cierra al soltar
                                        // Super porque no es Win+Tab).
                                        st.overview_selected =
                                            st.workspace_overview().map_or(0, |(a, _)| a);
                                    }
                                    return FilterResult::Intercept(());
                                }
                                "Escape" if st.switcher.is_some() => {
                                    st.switcher_cancel = true;
                                    return FilterResult::Intercept(());
                                }
                                "Escape" if st.overview_open => {
                                    // Cancelar: cierra con zoom-in de vuelta al
                                    // escritorio ACTUAL (sin saltar).
                                    st.overview_selected =
                                        st.workspace_overview().map_or(0, |(a, _)| a);
                                    st.overview_closing = true;
                                    return FilterResult::Intercept(());
                                }
                                _ => {}
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
                // LEDs físicos del teclado: `smithay` recalculó el estado de
                // Bloq Mayús / Num / Despl (vía `led_state_changed`) al procesar
                // los modificadores; lo reflejamos en el teclado real con el que
                // se tipeó. Los dispositivos sin esos LEDs ignoran la orden.
                {
                    let mut device = event.device();
                    device.led_update(self.app.led_state.into());
                }
                if let Some(combo) = self.app.pending_keybind.take() {
                    let ev = self.app.body.keybind(combo);
                    self.app.brain_feed(ev);
                }
                if let Some(vt) = self.app.pending_vt.take() {
                    if let Err(e) = self.session.change_vt(vt) {
                        eprintln!("mirada-compositor · no pude conmutar a VT{vt}: {e}");
                    }
                }
                // Switchers visuales (Alt-Tab / Win-Tab): aplicar el paso pedido,
                // cancelar si Esc, y CONFIRMAR cuando se suelta el modificador del
                // switcher activo (el filtro no ve el release de un modificador,
                // así que lo chequeamos acá).
                if let Some((kind, forward)) = self.app.switcher_step.take() {
                    crate::switcher::advance(&mut self.app, kind, forward);
                }
                if self.app.switcher_cancel {
                    self.app.switcher_cancel = false;
                    crate::switcher::cancel(&mut self.app);
                }
                if let Some(kind) = self.app.switcher.as_ref().map(|s| s.kind) {
                    let held = self
                        .app
                        .keyboard
                        .as_ref()
                        .is_some_and(|kb| kind.modifier_held(&kb.modifier_state()));
                    if !held {
                        crate::switcher::commit(&mut self.app);
                    }
                }
                // Vista espacial abierta por Win+Tab: al soltar Super se SALTA al
                // escritorio resaltado y se cierra (zoom-in hacia él), como un
                // switcher.
                if self.app.overview_open && self.app.overview_via_wintab && !self.app.overview_closing
                {
                    let super_held = self
                        .app
                        .keyboard
                        .as_ref()
                        .is_some_and(|kb| kb.modifier_state().logo);
                    if !super_held {
                        self.app.overview_commit();
                    }
                }
                // Refresca el indicador de distribución: un `grp:*toggle` pudo
                // cambiar el grupo XKB con esta tecla.
                self.app.refresh_kbd_layout();
            }

            // --- Puntero: movimiento relativo (ratón, touchpad) ----------
            InputEvent::PointerMotion { event } => {
                let (x0, y0) = self.app.pointer_loc;
                let delta = (event.delta_x(), event.delta_y());
                let delta_unaccel = (event.delta_x_unaccel(), event.delta_y_unaccel());
                // Pre-acotado al bounding box: descarta los outliers extremos
                // sin hacer rondas innecesarias en `clamp_to_outputs`.
                let x = (x0 + delta.0).clamp(0.0, self.output_size.0);
                let y = (y0 + delta.1).clamp(0.0, self.output_size.1);
                // Proyectado al output más cercano si cayó en zona muerta.
                let prop = self.clamp_to_outputs(x, y);
                // Movimiento relativo (delta crudo a la superficie con foco) +
                // restricciones de puntero (lock/confine). Si la superficie con
                // foco tiene un lock activo, el cursor queda clavado donde estaba.
                let (x, y) = self.relative_y_restriccion(prop, delta, delta_unaccel, time);
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
                self.update_hot_corners(x, y); // esquinas calientes
                self.app.follow_pointer_output(); // el escritorio activo sigue al monitor del mouse
                if !self.drag_update() {
                    self.pointer_motion(time);
                    self.update_divider_cursor(); // cursor «redimensionar» sobre el divisor
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
                self.update_hot_corners(x, y); // esquinas calientes
                self.app.follow_pointer_output(); // el escritorio activo sigue al monitor del mouse
                if !self.drag_update() {
                    self.pointer_motion(time);
                    self.update_divider_cursor(); // cursor «redimensionar» sobre el divisor
                }
            }

            // --- Puntero: botones ----------------------------------------
            InputEvent::PointerButton { event } => {
                let pressed = event.state() == ButtonState::Pressed;
                let button = event.button_code();
                // Un botón del puntero mientras Super está sostenida (p.ej.
                // Super+arrastre para mover/redimensionar) tampoco es un tap solo.
                if pressed {
                    self.app.super_tap_armed = false;
                }

                // Popups (menús de apps GTK/Qt) abiertos: al APRETAR, un click
                // sobre el menú se le reenvía (el motion ya lo enfocó), sin pasar
                // por la lógica de ventanas/drag; un click AFUERA cierra el menú y
                // consume el click. El release cae al reenvío normal de abajo, así
                // el ítem se activa al soltar.
                if pressed && self.app.has_popups() {
                    let (x, y) = self.app.pointer_loc;
                    if self.app.popup_under(x, y).is_some() {
                        if let Some(pointer) = self.app.pointer.clone() {
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
                        return;
                    }
                    self.app.dismiss_popups();
                    return;
                }

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
                            let target = self.menu_window.take();
                            self.root_menu = None;
                            self.menu_output_idx = None;
                            // Comandos `@win:*` = acción sobre la ventana del menú
                            // contextual (no son shell). El resto va al usuario.
                            if let Some(action) = cmd.strip_prefix("@win:") {
                                if let Some(id) = target {
                                    self.app.accion_ventana_menu(id, action);
                                }
                            } else {
                                self.app.spawn_user(&cmd);
                            }
                        }
                        ClickResult::Stay => {}
                        ClickResult::Close => {
                            self.menu_window = None;
                            self.root_menu = None;
                            self.menu_output_idx = None;
                        }
                    }
                    // El click cambió el menú (abrió submenú o lo cerró):
                    // daño para screencopy. Grueso pero raro.
                    crate::screencopy::danar_todo(&mut self.app);
                    return; // el menú captura el botón
                }

                // Vista espacial (Prezi) abierta: un click izquierdo sobre un
                // tile salta a ese escritorio; cualquier click la cierra. Los
                // tiles están en coords locales de la salida primaria.
                if pressed && self.app.overview_open && !self.app.overview_closing {
                    // Click izquierdo sobre un tile → salta a ese escritorio; luego
                    // anima el cierre (zoom-in). Cualquier otro click sólo cierra.
                    if button == BTN_LEFT {
                        let (gx, gy) = self.app.pointer_loc;
                        let origin = self
                            .outputs
                            .get(Self::PRIMARY)
                            .map(|o| o.rect)
                            .unwrap_or(Rect::new(0, 0, 0, 0));
                        let lx = gx.round() as i32 - origin.x;
                        let ly = gy.round() as i32 - origin.y;
                        if let Some(&(ws, _)) =
                            self.overview_tiles.iter().find(|(_, r)| r.contains(lx, ly))
                        {
                            self.app.overview_selected = ws;
                            self.app.cambiar_workspace(ws);
                        }
                    }
                    // Cierre ANIMADO (zoom-in hacia el elegido).
                    self.app.overview_closing = true;
                    crate::screencopy::danar_todo(&mut self.app);
                    return;
                }

                // Click DERECHO sobre la BARRA DE TÍTULO de una ventana: abre el
                // menú **contextual de ventana** (minimizar/maximizar/flotar/
                // ¿Hay una layer surface INTERACTIVA (con input-region) bajo el
                // puntero? —el panel/drawer de pata abierto, una barra—. Si la hay,
                // es la dueña del píxel: ningún gesto de chrome de VENTANA (menú o
                // mover por titlebar, botón de titlebar, divisor de tesela,
                // Super+arrastre) debe dispararse; el click cae al forwarding normal,
                // que ya rutea por input-region a la layer. Sin esto, la barrita del
                // panel «atravesaba» al titlebar de una ventana que caía a esa altura.
                // (Con el drawer cerrado la región es vacía → `layer_under` da None →
                // el chrome de ventana funciona igual que siempre.)
                let sobre_layer = pressed && {
                    let (px, py) = self.app.pointer_loc;
                    self.app.layer_under(px, py).is_some()
                };

                // enviar-a/cerrar). Va ANTES del menú del fondo. No en greeter.
                if pressed && button == BTN_RIGHT && !self.app.shell_activo() && !sobre_layer {
                    let (x, y) = self.app.pointer_loc;
                    if let Some(i) = self.titlebar_at(x, y) {
                        let id = self.app.windows[i].id;
                        self.open_window_menu(id);
                        return;
                    }
                }

                // Click DERECHO sobre el fondo (sin ventana ni `Super`): abre el
                // menú raíz, si hay entradas configuradas. No aplica en greeter.
                if pressed
                    && button == BTN_RIGHT
                    && !self.menu_entries.is_empty()
                    && !self.app.shell_activo()
                    && !sobre_layer
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
                        self.menu_window = None; // menú del fondo, no de ventana
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
                if pressed && self.app.drag.is_none() && !self.app.shell_activo() && !sobre_layer {
                    let super_held = self
                        .app
                        .keyboard
                        .as_ref()
                        .is_some_and(|kb| kb.modifier_state().logo);
                    // `Super`+izquierdo **mueve** la ventana (estilo Hyprland):
                    // la saca del mosaico flotándola y la sigue al puntero; al
                    // soltar sobre una tesela el Cerebro la reacomoda, sobre vacío
                    // queda flotando. `Super`+derecho redimensiona. (Antes el
                    // izquierdo sobre una teselada sólo hacía swap sin moverse en
                    // vivo, y daba la sensación de que «win+drag no mueve».)
                    let (x, y) = self.app.pointer_loc;
                    let hit = self.window_at(x, y);
                    let mode = match (button, hit) {
                        (BTN_LEFT, Some(_)) if super_held => Some(DragMode::Move),
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

                // Click izquierdo sobre el **divisor entre teselas** (sin `Super`):
                // arranca un arrastre de redimensionado del mosaico — el divisor
                // sigue al puntero y el Cerebro re-reparte el teselado en vivo
                // (`master_ratio`). Va antes del titlebar/foco-al-click para que
                // agarrar el borde no enfoque la ventana ni la arrastre.
                if pressed
                    && button == BTN_LEFT
                    && self.app.drag.is_none()
                    && !self.app.shell_activo()
                    && !sobre_layer
                {
                    let super_held = self
                        .app
                        .keyboard
                        .as_ref()
                        .is_some_and(|kb| kb.modifier_state().logo);
                    let (x, y) = self.app.pointer_loc;
                    if !super_held && self.tile_divider_at(x, y) {
                        self.app.drag = Some(DragGrab {
                            id: 0,
                            mode: DragMode::TileResize,
                            start_pointer: (x, y),
                            start_rect: (0, 0, 0, 0),
                        });
                        return; // el arrastre del divisor captura el botón
                    }
                }

                // Click izquierdo sobre un BOTÓN del titlebar: ejecuta su acción y
                // no arranca arrastre. Va ANTES del drag. La acción la define el
                // layout (configurable); `Menu` abre el menú contextual.
                if pressed && button == BTN_LEFT && !self.app.shell_activo() && !sobre_layer {
                    let (x, y) = self.app.pointer_loc;
                    if let Some(target) = self.titlebar_button_at(x, y) {
                        match target {
                            TbClickTarget::Sys { id, action } => {
                                if !self.app.accion_titlebar(id, &action) {
                                    self.open_window_menu(id); // era `Menu`
                                }
                            }
                            // Botón aportado por una app: encolar el click para que
                            // la app lo drene con PollClicks (protocolo mirada-aware).
                            TbClickTarget::App { app_id, item_id, window, window_title } => {
                                self.app
                                    .aware_clicks
                                    .entry(app_id)
                                    .or_default()
                                    .push(mirada_aware::AwareClick { item_id, window, window_title });
                            }
                        }
                        return;
                    }
                }

                // Click izquierdo sobre la BARRA DE TÍTULO (sin `Super`): arranca
                // un arrastre Move — saca la ventana de su tile y la lleva
                // flotante, lista para aterrizar en una zona (drag-to-zone) o
                // quedar overflow. La barra deja de ser chrome inerte.
                if pressed
                    && button == BTN_LEFT
                    && self.app.drag.is_none()
                    && !self.app.shell_activo()
                    && !sobre_layer
                {
                    let (x, y) = self.app.pointer_loc;
                    if let Some(i) = self.titlebar_at(x, y) {
                        let (id, loc, size) = {
                            let w = &self.app.windows[i];
                            (w.id, w.loc, w.size)
                        };
                        // Doble-click sobre la barra de título: maximiza/restaura
                        // (mismo gesto que el escritorio clásico), en vez de
                        // arrastrar. Ventana de 400 ms sobre la misma ventana.
                        let now = std::time::Instant::now();
                        let doble = self
                            .last_titlebar_click
                            .is_some_and(|(prev, t)| {
                                prev == id
                                    && now.duration_since(t)
                                        < std::time::Duration::from_millis(400)
                            });
                        if doble {
                            self.last_titlebar_click = None;
                            self.app.maximizar_ventana(id);
                            return;
                        }
                        self.last_titlebar_click = Some((id, now));
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
                                match zone {
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
                        } else {
                            // Click en escritorio vacío (ni layer focusable ni
                            // ventana): el teclado cae al shell —sea toplevel o
                            // layer-shell (shuma/pata en barra)— para tipear sin
                            // tener que clickear primero la barra.
                            if let Some(kb) = self.app.keyboard.clone() {
                                let target = self.app.keyboard_fallback_target();
                                if kb.current_focus() != target {
                                    kb.set_focus(
                                        &mut self.app,
                                        target,
                                        SERIAL_COUNTER.next_serial(),
                                    );
                                }
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

            // Dispositivo nuevo (ratón/touchpad): aplica las preferencias de
            // libinput de la config — scroll natural, tap-to-click y velocidad
            // del puntero. Los dispositivos que no soportan una opción la
            // ignoran (devuelven error, que descartamos).
            InputEvent::DeviceAdded { mut device } => {
                let (natural, tap, speed) = self.app.input_prefs();
                if device.config_scroll_has_natural_scroll() {
                    let _ = device.config_scroll_set_natural_scroll_enabled(natural);
                }
                if device.config_tap_finger_count() > 0 {
                    let _ = device.config_tap_set_enabled(tap);
                }
                if device.config_accel_is_available() {
                    let _ = device.config_accel_set_speed(speed);
                }
                // Estado inicial de los LEDs (un teclado recién enchufado debe
                // reflejar el Bloq Mayús/Num/Despl ya vigente). No-op si el
                // dispositivo no tiene esos LEDs.
                device.led_update(self.app.led_state.into());
            }

            _ => {} // otros dispositivos: aún no
        }
    }

    /// Emite el movimiento **relativo** (delta crudo, sin acotar a la pantalla)
    /// a la superficie con foco del puntero y aplica las restricciones de puntero
    /// (`zwp_pointer_constraints_v1`):
    /// - **Lock**: el cursor queda clavado donde estaba (apps 3D / FPS).
    /// - **Confine**: el cursor se acota al rectángulo de la superficie.
    ///
    /// Devuelve la posición final del cursor (la propuesta `prop` salvo que una
    /// restricción la corrija). El movimiento relativo se emite SIEMPRE: es un
    /// no-op si el cliente con foco no usó el protocolo `relative_pointer`.
    fn relative_y_restriccion(
        &mut self,
        prop: (f64, f64),
        delta: (f64, f64),
        delta_unaccel: (f64, f64),
        time: u32,
    ) -> (f64, f64) {
        use smithay::wayland::pointer_constraints::{with_pointer_constraint, PointerConstraint};
        let Some(pointer) = self.app.pointer.clone() else {
            return prop;
        };
        // Superficie con foco = la ventana bajo el puntero ACTUAL (para un lock
        // el cursor no se movió, así que sigue siendo la misma).
        let (cx, cy) = self.app.pointer_loc;
        let Some(i) = self.window_at(cx, cy) else {
            // Sin ventana bajo el puntero no hay a quién mandarle movimiento
            // relativo ni restricción que aplicar: va a la posición propuesta.
            return prop;
        };
        let tbh = self.app.decorations.titlebar_height;
        let (surface, lx, ly, sw, sh) = {
            let w = &self.app.windows[i];
            let (lx, ly) = crate::render_loc(w, self.app.output_size.1, tbh);
            let tb = crate::titlebar_for(w, tbh);
            let (sw, sh) =
                crate::surface_px_size(w).unwrap_or((w.size.0, (w.size.1 - tb).max(1)));
            (w.surface.clone(), lx, ly, sw, sh)
        };
        // Movimiento relativo a la superficie con foco (delta sin acotar).
        let foco = Some((
            surface.clone(),
            Point::<f64, Logical>::from((lx as f64, ly as f64)),
        ));
        pointer.relative_motion(
            &mut self.app,
            foco,
            &RelativeMotionEvent {
                delta: Point::from(delta),
                delta_unaccel: Point::from(delta_unaccel),
                utime: time as u64 * 1000,
            },
        );
        pointer.frame(&mut self.app);
        // Restricción activa sobre esa superficie, si la hay.
        let mut resultado = prop;
        with_pointer_constraint(&surface, &pointer, |c| {
            let Some(c) = c else { return };
            // El puntero está sobre la superficie restringida → tiene su foco:
            // activamos la restricción si aún no lo estaba (la desactivación al
            // perder el foco la maneja smithay sola).
            if !c.is_active() {
                c.activate();
            }
            match &*c {
                // Lock: el cursor no se mueve.
                PointerConstraint::Locked(_) => resultado = (cx, cy),
                // Confine: acotado al rectángulo de la superficie. (Aprox.: no se
                // recorta a la región fina `c.region()` — TODO si hace falta.)
                PointerConstraint::Confined(_) => {
                    let x = prop.0.clamp(lx as f64, (lx + sw) as f64 - 1.0);
                    let y = prop.1.clamp(ly as f64, (ly + sh) as f64 - 1.0);
                    resultado = (x, y);
                }
            }
        });
        resultado
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

        // Un popup (menú de app) abierto está por encima de las ventanas: el
        // puntero va ahí primero, así sus ítems resaltan y reciben el click.
        if let Some((surface, loc)) = self.app.popup_under(x, y) {
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
        // Arrastre del divisor del teselado: no toca ninguna ventana, sólo le
        // pide al Cerebro que mueva la frontera maestro/pila al puntero. El
        // cursor se mantiene en «redimensionar» mientras dura el gesto.
        if mode == DragMode::TileResize {
            self.app.cursor_status =
                CursorImageStatus::Named(smithay::input::pointer::CursorIcon::EwResize);
            self.app
                .brain_feed(BodyEvent::ResizeMaster { x: px as i32, y: py as i32 });
            return true;
        }
        // Drag-to-zone: resalta la zona bajo el puntero (Move/Tile, no Resize).
        // Sobre una zona, la ventana aterrizará ahí al soltar.
        let nueva_zona = if mode == DragMode::Resize { None } else { self.zone_at(px, py) };
        // Diagnóstico opcional (MIRADA_ZONE_DEBUG=1): traza el arrastre y la zona
        // objetivo cada vez que cambia — para ver en vivo si el snap se dispara.
        if nueva_zona != self.drag_zone && std::env::var_os("MIRADA_ZONE_DEBUG").is_some() {
            eprintln!(
                "mirada-zone · drag mode={mode:?} ptr=({:.0},{:.0}) zona={nueva_zona:?} (zonas={})",
                px,
                py,
                self.zones.len()
            );
        }
        self.drag_zone = nueva_zona;
        // Arrastre de una teselada: el swap con la tesela destino se resuelve
        // al SOLTAR (ver la rama de release del botón), no en cada frame.
        // Durante el arrastre sólo resaltamos la zona/tesela bajo el puntero.
        // Antes acá se emitía `WindowDragged` en CADA movimiento, así que el
        // stack se reordenaba sin parar mientras arrastrabas — daba la
        // sensación de que «si muevo una ventana, se mueven todas».
        if mode == DragMode::Tile {
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
            DragMode::TileResize => unreachable!("TileResize se maneja arriba"),
        };
        self.app.brain_feed(BodyEvent::WindowFloatTo { id, rect });
        true
    }

    /// `true` si `(x, y)` (global) cae sobre el **divisor vertical** entre dos
    /// teselas adyacentes — la frontera maestro/pila que se arrastra para
    /// redimensionar el mosaico. Las teseladas usan su celda (`loc`/`size`, ya en
    /// coords globales); buscamos un par izquierda/derecha cuyos bordes se
    /// enfrenten (tolerando el `gap` entre ambas) y que se solapen en `y` con el
    /// puntero, y miramos si el puntero está sobre la línea media. Geometría pura
    /// del Cuerpo: el Cerebro decide qué hacer con el arrastre (sólo
    /// `MasterStack`/`CenteredMaster` tienen un divisor maestro útil).
    pub(super) fn tile_divider_at(&self, x: f64, y: f64) -> bool {
        // Tolerancia a cada lado de la línea media, en px — cuán «grueso» es el
        // asidero del divisor para el ratón.
        const TOL: f64 = 8.0;
        let tiled: Vec<(f64, f64, f64, f64)> = self
            .app
            .windows
            .iter()
            .filter(|w| {
                w.visible && !w.is_shell && !w.floating && !w.fullscreen && !w.is_greeter
            })
            .map(|w| (w.loc.0 as f64, w.loc.1 as f64, w.size.0 as f64, w.size.1 as f64))
            .collect();
        for &(ax, ay, aw, ah) in &tiled {
            let aright = ax + aw;
            for &(bx, by, _bw, bh) in &tiled {
                // B estrictamente a la derecha de A.
                if bx <= ax + 1.0 {
                    continue;
                }
                // Adyacentes: el hueco entre el borde derecho de A y el izquierdo
                // de B es el del teselado (`2·gap`); toleramos hasta ~40px.
                let hueco = bx - aright;
                if !(-2.0..=40.0).contains(&hueco) {
                    continue;
                }
                // Solape vertical de A, B y el puntero.
                let top = ay.max(by);
                let bot = (ay + ah).min(by + bh);
                if y < top || y >= bot {
                    continue;
                }
                let mid = (aright + bx) / 2.0;
                if (x - mid).abs() <= (hueco / 2.0).max(TOL) {
                    return true;
                }
            }
        }
        false
    }

    /// Si el puntero (sin arrastre en curso) sobrevuela un divisor del teselado,
    /// muestra el cursor de redimensionado horizontal — pista visual de que el
    /// borde es arrastrable. Se llama tras el `pointer_motion` de cada evento.
    pub(super) fn update_divider_cursor(&mut self) {
        if self.app.drag.is_some() {
            return;
        }
        let (x, y) = self.app.pointer_loc;
        if self.tile_divider_at(x, y) {
            self.app.cursor_status =
                CursorImageStatus::Named(smithay::input::pointer::CursorIcon::EwResize);
        }
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

    /// El **objetivo de click** del botón de titlebar bajo `(x, y)` global: un
    /// botón de sistema (con su acción) o uno aportado por una app mirada-aware.
    /// Mismas celdas que el render (`titlebar_cells_for`), así click y dibujo
    /// nunca divergen. `None` si ahí no hay botón actuable (título, hueco → el
    /// llamador lo trata como arrastre).
    pub(super) fn titlebar_button_at(&self, x: f64, y: f64) -> Option<TbClickTarget> {
        let tbh = self.app.decorations.titlebar_height;
        if tbh <= 0 {
            return None;
        }
        let primary = self.outputs.get(Self::PRIMARY)?;
        let output_h = primary.rect.h;
        let (px, py) = (x.round() as i32, y.round() as i32);
        // Orden front-to-back: la primera que matchea gana (la de encima).
        let mut idx: Vec<usize> = (0..self.app.windows.len())
            .filter(|&i| self.app.windows[i].visible && !self.app.windows[i].is_shell)
            .collect();
        idx.sort_by_key(|&i| {
            let w = &self.app.windows[i];
            (!w.floating, !w.focused)
        });
        let layout = &self.app.titlebar_layout;
        for i in idx {
            let w = &self.app.windows[i];
            let tb = crate::titlebar_for(w, tbh);
            if tb == 0 {
                continue;
            }
            let (lx, ly) = crate::render_loc(w, output_h, tbh);
            let (sw, _) = crate::surface_px_size(w).unwrap_or((w.size.0, 1));
            let top = ly - tb;
            if py < top || py >= top + tb {
                continue;
            }
            let contribs: &[mirada_aware::AwareItem] =
                self.app.aware_items.get(&w.app_id).map(|v| v.as_slice()).unwrap_or(&[]);
            for (cell_x, cell) in crate::titlebar_cells_for(layout, contribs, lx, sw) {
                if px < cell_x || px >= cell_x + crate::TB_BTN_W {
                    continue;
                }
                match &cell {
                    crate::TbCell::Sys(_) => {
                        if let Some(action) = cell.action() {
                            return Some(TbClickTarget::Sys { id: w.id, action: action.clone() });
                        }
                    }
                    crate::TbCell::App { item_id, .. } => {
                        return Some(TbClickTarget::App {
                            app_id: w.app_id.clone(),
                            item_id: (*item_id).to_string(),
                            window: w.id,
                            window_title: w.title.clone(),
                        });
                    }
                }
                break; // celda hallada pero no actuable → arrastre
            }
        }
        None
    }

    /// Abre el **menú contextual de ventana** (minimizar/maximizar/flotar/
    /// enviar-a/cerrar) anclado en el puntero, sobre la ventana `id`. Lo usan el
    /// click derecho en el titlebar y el botón `Menu` del titlebar.
    pub(super) fn open_window_menu(&mut self, id: u64) {
        let (x, y) = self.app.pointer_loc;
        let idx = self.output_at_point(x.round() as i32, y.round() as i32);
        if let Some(r) = self.outputs.get(idx).map(|o| o.rect) {
            let ev = self.app.body.clicked(id); // enfoca la ventana
            self.app.brain_feed(ev);
            self.menu_window = Some(id);
            self.menu_output_idx = Some(idx);
            self.root_menu = Some(crate::menu::RootMenu::open(
                x.round() as i32 - r.x,
                y.round() as i32 - r.y,
                crate::menu::window_menu_entries(mirada_brain::action::WORKSPACE_COUNT),
                r.w,
                r.h,
            ));
            crate::screencopy::danar_todo(&mut self.app);
        }
    }
}

/// El objetivo de un click sobre un botón del titlebar: un botón de **sistema**
/// (con su acción) o uno **aportado por una app** mirada-aware (a rutear de
/// vuelta al cliente por `app_id`). Lo produce `titlebar_button_at`.
pub(super) enum TbClickTarget {
    Sys { id: u64, action: mirada_brain::TitlebarAction },
    App { app_id: String, item_id: String, window: u64, window_title: String },
}
