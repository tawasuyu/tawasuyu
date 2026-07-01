//! Implementaciones de los traits de smithay-client-toolkit para `LayerApp`:
//! handlers de compositor, layer shell, output, seat, teclado y puntero,
//! más los `Dispatch` de los protocolos wlr-foreign-toplevel.

use smithay_client_toolkit::{
    compositor::CompositorHandler,
    output::OutputHandler,
    registry::ProvidesRegistryState,
    registry_handlers,
    seat::{
        keyboard::{KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT},
        Capability, SeatHandler,
    },
    shell::wlr_layer::{KeyboardInteractivity, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
};
use wayland_client::{
    event_created_child,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1, EVT_TOPLEVEL_OPCODE},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::{self, ExtIdleNotifierV1},
};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::{self, ZwpIdleInhibitManagerV1},
    zwp_idle_inhibitor_v1::{self, ZwpIdleInhibitorV1},
};

use llimphi_ui::llimphi_compositor::{hit_test_click, hit_test_hover, hit_test_scroll, DragPhase};

use crate::toplevel::Toplevel;

use super::{app_impl::*, diag, LayerApp, LayerDrag, MenuKind, MENU_LEAVE_GRACE};

/// Si el puntero se aleja más que esto (px) del origen del press, el `on_click`
/// armado deja de contar como click (fue un arrastre/barrido). Espeja el umbral
/// del runtime de Llimphi para una sensación uniforme en todo el escritorio.
const CLICK_MOVE_CANCEL: f32 = 6.0;

impl CompositorHandler for LayerApp {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if let Some(pi) = self.panel_de(surface) {
            self.draw(pi, qh);
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for LayerApp {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        use smithay_client_toolkit::shell::WaylandSurface;
        let pi = self.panel_de(layer.wl_surface());
        diag!("pata diag · CLOSED panel={pi:?} drawer_pi={:?}", self.drawer_pi);
        // Si el compositor cierra EL DRAWER (surface de runtime), NO matamos la app:
        // sólo soltamos el drawer para que se pueda recrear al reabrir un diente.
        // (En arje-DRM mirada manda `closed` al resetear el output; matar pata ahí
        // hacía que el diente "no hiciera nada" y el mouse atravesara.)
        if pi.is_some() && pi == self.drawer_pi {
            self.destroy_drawer();
            return;
        }
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        use smithay_client_toolkit::shell::WaylandSurface;
        let (cw, ch) = configure.new_size;
        let Some(pi) = self.panel_de(layer.wl_surface()) else {
            return;
        };
        diag!("pata diag · configure panel {pi} new_size={cw}x{ch}");
        const MAX_DIM: u32 = 16384;
        if (1..=MAX_DIM).contains(&cw) {
            self.panels[pi].width = cw;
        }
        if (1..=MAX_DIM).contains(&ch) {
            self.panels[pi].height = ch;
        }
        self.panels[pi].dirty = true;
        self.draw(pi, qh);
    }
}

impl OutputHandler for LayerApp {
    fn output_state(&mut self) -> &mut smithay_client_toolkit::output::OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for LayerApp {
    fn seat_state(&mut self) -> &mut smithay_client_toolkit::seat::SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.seat.is_none() {
            self.seat = Some(seat);
        }
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => {
                if let Ok(kbd) = self.seat_state.get_keyboard(qh, &seat, None) {
                    self.keyboard = Some(kbd);
                }
            }
            Capability::Pointer if self.pointer.is_none() => {
                if let Ok(ptr) = self.seat_state.get_pointer(qh, &seat) {
                    self.pointer = Some(ptr);
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard => {
                if let Some(k) = self.keyboard.take() {
                    k.release();
                }
            }
            Capability::Pointer => {
                if let Some(p) = self.pointer.take() {
                    p.release();
                }
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for LayerApp {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        let pi = self.panel_de(surface);
        if let Some(pi) = pi {
            if self.menu_panel == Some(pi)
                && self.menu_open
                && self.menu_kind == MenuKind::Apps
            {
                // Ignorá el `leave` que llega justo al abrir: es el reacomodo de
                // foco del compositor al darle el teclado al menú (Exclusive), no
                // que el usuario se haya ido. Sin esta guarda el menú se cerraba
                // al instante en escritorio vacío (regresión del foco-al-shell).
                let churn = self
                    .menu_opened_at
                    .is_some_and(|t| t.elapsed() < MENU_LEAVE_GRACE);
                if !churn {
                    self.set_menu_open(false);
                }
            }
            if self.shuma_panel == Some(pi) && self.shuma.open {
                self.set_shuma_open(false);
            }
        }
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) {
        // Diálogo de polkit (modal): el teclado va al campo de contraseña.
        if self.polkit_prompt.is_some() {
            match event.keysym {
                Keysym::Escape => self.handle_msg(crate::Msg::PolkitCancel),
                Keysym::BackSpace => self.handle_msg(crate::Msg::PolkitBackspace),
                Keysym::Return | Keysym::KP_Enter => self.handle_msg(crate::Msg::PolkitSubmit),
                _ => {
                    if let Some(txt) = event.utf8 {
                        for c in txt.chars().filter(|c| !c.is_control()) {
                            self.handle_msg(crate::Msg::PolkitChar(c));
                        }
                    }
                }
            }
            return;
        }
        // Entrada de contraseña Wi-Fi: el teclado va al campo (Enter conecta, Esc
        // cancela, Backspace borra). Se evalúa antes que el buscador del menú.
        if self.net_password.is_some() {
            match event.keysym {
                Keysym::Escape => self.handle_msg(crate::Msg::NetworkPasswordCancel),
                Keysym::BackSpace => self.handle_msg(crate::Msg::NetworkPasswordBackspace),
                Keysym::Return | Keysym::KP_Enter => {
                    self.handle_msg(crate::Msg::NetworkPasswordSubmit)
                }
                _ => {
                    if let Some(txt) = event.utf8 {
                        for c in txt.chars().filter(|c| !c.is_control()) {
                            self.handle_msg(crate::Msg::NetworkPasswordChar(c));
                        }
                    }
                }
            }
            return;
        }
        // Menú de inicio abierto: el teclado va al buscador.
        if self.menu_open {
            match event.keysym {
                Keysym::Escape => self.set_menu_open(false),
                Keysym::BackSpace => {
                    self.menu_query.pop();
                    self.menu_scroll = 0.0;
                    self.marcar_menu_dirty();
                }
                Keysym::Return | Keysym::KP_Enter => self.lanzar_primero_menu(),
                _ => {
                    if let Some(txt) = event.utf8 {
                        if !txt.is_empty() && !txt.chars().any(|c| c.is_control()) {
                            self.menu_query.push_str(&txt);
                            self.menu_scroll = 0.0;
                            self.marcar_menu_dirty();
                        }
                    }
                }
            }
            return;
        }
        // Panel RAG abierto: el teclado va a su buscador (texto → consulta, Enter
        // pregunta, Esc cierra el panel, Backspace borra).
        if self.rag_panel_open() {
            match event.keysym {
                Keysym::Escape => self.cerrar_sidebar(),
                Keysym::BackSpace => self.handle_msg(crate::Msg::RagBackspace),
                Keysym::Return | Keysym::KP_Enter => self.handle_msg(crate::Msg::RagSubmit),
                _ => {
                    if let Some(txt) = event.utf8 {
                        for c in txt.chars().filter(|c| !c.is_control()) {
                            self.handle_msg(crate::Msg::RagChar(c));
                        }
                    }
                }
            }
            return;
        }
        // Ctrl+Shift+W repliega el drawer (sólo tiene sentido abierto).
        if self.shuma.open
            && self.mods.ctrl
            && self.mods.shift
            && matches!(event.keysym, Keysym::w | Keysym::W)
        {
            self.set_shuma_open(false);
            return;
        }
        // OJO: NO descartamos las teclas con el drawer plegado. `press_key` sólo
        // llega cuando la barra TIENE el foco de teclado (clic, o el fallback de
        // mirada en escritorio vacío); en ese caso ruteamos las teclas a shuma
        // para poder tipear en la barra sin abrir el drawer. Enter despliega el
        // drawer para ver la salida del comando.
        let abrir_por_enter =
            !self.shuma.open && matches!(event.keysym, Keysym::Return | Keysym::KP_Enter);
        if let Some(ke) = self.keysym_to_keyevent(&event) {
            if self.shuma_full.is_some() {
                // Live-wire: la tecla la traduce la shuma completa según su foco
                // interno (input de la sesión activa / PTY-TUI / rails).
                let m = self
                    .shuma_full
                    .as_ref()
                    .and_then(|f| crate::shuma_app::on_key(f, &ke));
                if std::env::var_os("PATA_DIAG").is_some() {
                    eprintln!(
                        "pata·shuma key={:?} ctrl={} shift={} alt={} → on_key={}",
                        ke.key,
                        self.mods.ctrl,
                        self.mods.shift,
                        self.mods.alt,
                        if m.is_some() { "Some(msg)" } else { "None" }
                    );
                }
                if let Some(m) = m {
                    self.apply_shuma_full(vec![m]);
                }
            } else {
                self.shuma.inner = shuma_module_shell::update(
                    self.shuma.inner.clone(),
                    shuma_module_shell::Msg::Key(ke),
                );
            }
        }
        if abrir_por_enter {
            self.set_shuma_open(true);
        }
        self.marcar_shuma_dirty();
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: u32,
    ) {
        self.mods = modifiers;
    }
}

impl PointerHandler for LayerApp {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for e in events {
            match e.kind {
                PointerEventKind::Motion { .. } => {
                    // `on_click` armado: si el puntero se alejó del origen del
                    // press más que el umbral, fue un arrastre/barrido → cancelar
                    // el click (no se disparará al soltar).
                    if let Some((_, _, (ox, oy))) = self.pending_click.as_ref() {
                        let (dx, dy) = (e.position.0 as f32 - ox, e.position.1 as f32 - oy);
                        if (dx * dx + dy * dy).sqrt() > CLICK_MOVE_CANCEL {
                            self.pending_click = None;
                        }
                    }
                    // Drag en curso: el delta va al handler del nodo.
                    if self.drag.is_some() {
                        let (px, py) = (e.position.0 as f32, e.position.1 as f32);
                        let (handler, last) = {
                            let d = self.drag.as_ref().unwrap();
                            (d.handler.clone(), d.last)
                        };
                        if let Some(d) = self.drag.as_mut() {
                            d.last = (px, py);
                        }
                        if let Some(msg) = (handler)(DragPhase::Move, px - last.0, py - last.1) {
                            self.handle_msg(msg);
                        }
                        continue;
                    }
                    if let Some(pi) = self.panel_de(&e.surface) {
                        let (px, py) = (e.position.0 as f32, e.position.1 as f32);
                        let nuevo = self.panels[pi]
                            .cache
                            .as_ref()
                            .and_then(|c| hit_test_hover(&c.mounted, &c.computed, px, py));
                        if self.panels[pi].hover_idx != nuevo {
                            let viejo = self.panels[pi].hover_idx;
                            self.panels[pi].hover_idx = nuevo;
                            self.panels[pi].dirty = true;
                            self.update_tooltip(pi, nuevo, qh);
                            // Despachar on_pointer_leave del nodo que abandonamos y
                            // on_pointer_enter del recién hovereado — habilita los
                            // submenús al hover (categorías → apps del menú inicio).
                            let leave = viejo.and_then(|i| {
                                self.panels[pi].cache.as_ref().and_then(|c| {
                                    c.mounted.nodes.get(i).and_then(|n| n.on_pointer_leave.clone())
                                })
                            });
                            let enter = nuevo.and_then(|i| {
                                self.panels[pi].cache.as_ref().and_then(|c| {
                                    c.mounted.nodes.get(i).and_then(|n| n.on_pointer_enter.clone())
                                })
                            });
                            if let Some(m) = leave {
                                self.handle_msg(m);
                            }
                            if let Some(m) = enter {
                                self.handle_msg(m);
                            }
                        }
                        // Dock: la lupa sigue al puntero — re-render por cada
                        // motion guardando la X local del panel.
                        let idx = self.panels[pi].idx;
                        if self.cfg.surfaces[idx].kind == pata_core::config::SurfaceKind::Dock
                            && self.panels[pi].cursor_x != Some(px)
                        {
                            self.panels[pi].cursor_x = Some(px);
                            self.panels[pi].dirty = true;
                        }
                    }
                    continue;
                }
                PointerEventKind::Leave { .. } => {
                    // El puntero salió de la superficie: cancelá cualquier click armado.
                    self.pending_click = None;
                    if let Some(pi) = self.panel_de(&e.surface) {
                        if self.panels[pi].hover_idx.is_some() {
                            self.panels[pi].hover_idx = None;
                            self.panels[pi].dirty = true;
                        }
                        // El dock vuelve a reposo cuando el puntero se va.
                        if self.panels[pi].cursor_x.is_some() {
                            self.panels[pi].cursor_x = None;
                            self.panels[pi].dirty = true;
                        }
                        // Hover-drawer: el puntero abandonó la superficie de shuma →
                        // replegá el drawer ("cierre con deshover"). Con guarda
                        // anti-churn: ignorá el `leave` espurio del reacomodo de foco
                        // justo al abrir (mismo patrón que el menú de inicio).
                        if self.shuma_panel == Some(pi) && self.shuma.open {
                            let churn = self
                                .shuma_opened_at
                                .is_some_and(|t| t.elapsed() < MENU_LEAVE_GRACE);
                            if !churn {
                                self.set_shuma_open(false);
                            }
                        }
                    }
                    self.hide_tooltip(qh);
                    continue;
                }
                _ => {}
            }
            // Rueda sobre el historial del drawer.
            if let PointerEventKind::Axis { vertical, .. } = e.kind {
                let dy = if vertical.discrete != 0 {
                    vertical.discrete as f32
                } else {
                    vertical.absolute as f32 / 20.0
                };
                if dy != 0.0 {
                    let (px, py) = (e.position.0 as f32, e.position.1 as f32);
                    if let Some(pi) = self.panel_de(&e.surface) {
                        let msg = self.panels[pi].cache.as_ref().and_then(|c| {
                            hit_test_scroll(&c.mounted, &c.computed, px, py)
                                .and_then(|i| c.mounted.nodes.get(i))
                                .and_then(|n| n.on_scroll.as_ref().and_then(|h| h(0.0, dy)))
                        });
                        if let Some(msg) = msg {
                            self.handle_msg(msg);
                        }
                    }
                }
                continue;
            }
            // Soltar el botón izquierdo termina un drag en curso.
            if let PointerEventKind::Release { button, .. } = e.kind {
                if button == BTN_LEFT {
                    if let Some(d) = self.drag.take() {
                        if let Some(msg) = (d.handler)(DragPhase::End, 0.0, 0.0) {
                            self.handle_msg(msg);
                        }
                    } else if let Some((_, msg, _)) = self.pending_click.take() {
                        // `on_click` armado en el press y no cancelado por
                        // movimiento → este es el click real, al soltar.
                        self.handle_msg(msg);
                    }
                }
                continue;
            }
            if let PointerEventKind::Press { button, .. } = e.kind {
                if button != BTN_LEFT && button != BTN_RIGHT && button != BTN_MIDDLE {
                    continue;
                }
                let Some(pi) = self.panel_de(&e.surface) else {
                    continue;
                };
                let (px, py) = (e.position.0 as f32, e.position.1 as f32);
                let derecho = button == BTN_RIGHT;
                let medio = button == BTN_MIDDLE;
                let izquierdo = button == BTN_LEFT;
                // Nodo arrastrable bajo el press (sólo botón IZQUIERDO): arranca un
                // drag. El clic medio/derecho nunca inician arrastre.
                if izquierdo {
                    let handler = self.panels[pi].cache.as_ref().and_then(|c| {
                        let i = hit_test_click(&c.mounted, &c.computed, px, py)?;
                        c.mounted.nodes.get(i)?.drag.clone()
                    });
                    if let Some(handler) = handler {
                        self.drag = Some(LayerDrag { handler, last: (px, py) });
                        continue;
                    }
                }
                // Diagnóstico del click (PATA_DIAG=1): qué nodo cae bajo el
                // press y si tiene handler. Para depurar "no hace nada".
                if std::env::var_os("PATA_DIAG").is_some() {
                    let info = self.panels[pi].cache.as_ref().map(|c| {
                        match hit_test_click(&c.mounted, &c.computed, px, py) {
                            Some(i) => {
                                let n = &c.mounted.nodes[i];
                                format!(
                                    "nodo {i} on_click={} on_click_at={} drag={}",
                                    n.on_click.is_some(),
                                    n.on_click_at.is_some(),
                                    n.drag.is_some()
                                )
                            }
                            None => "ningún nodo clickeable".into(),
                        }
                    });
                    eprintln!(
                        "pata diag · PRESS panel={pi} pos=({px:.0},{py:.0}) der={derecho} → {}",
                        info.unwrap_or_else(|| "sin cache".into())
                    );
                }
                // Para el clic IZQUIERDO con `on_click` plano NO disparamos en el
                // press: armamos el click y lo disparamos al RELEASE (semántica
                // de escritorio), salvo que el puntero se aleje del origen. Lo
                // posicional (`on_click_at`) y el clic derecho/medio sí van en el
                // press (gestos de press por diseño).
                if izquierdo {
                    let armado = self.panels[pi].cache.as_ref().and_then(|c| {
                        let i = hit_test_click(&c.mounted, &c.computed, px, py)?;
                        c.mounted.nodes.get(i)?.on_click.clone()
                    });
                    if let Some(msg) = armado {
                        self.pending_click = Some((pi, msg, (px, py)));
                        continue;
                    }
                }
                let msg = self.panels[pi].cache.as_ref().and_then(|c| {
                    let i = hit_test_click(&c.mounted, &c.computed, px, py)?;
                    let n = c.mounted.nodes.get(i)?;
                    if derecho {
                        if let Some(m) = n.on_right_click.clone() {
                            return Some(m);
                        }
                        let at = n.on_right_click_at.as_ref()?;
                        let r = c.computed.get(n.id)?;
                        at(px - r.x, py - r.y, r.w, r.h)
                    } else if medio {
                        // Clic medio: sólo nodos con `on_middle_click` reaccionan
                        // (mismo modelo que el derecho).
                        n.on_middle_click.clone()
                    } else {
                        // Izquierdo sin `on_click` plano: lo posicional va en el press.
                        let at = n.on_click_at.as_ref()?;
                        let r = c.computed.get(n.id)?;
                        at(px - r.x, py - r.y, r.w, r.h)
                    }
                });
                if let Some(msg) = msg {
                    self.handle_msg(msg);
                }
            }
        }
    }
}

impl ProvidesRegistryState for LayerApp {
    fn registry(&mut self) -> &mut smithay_client_toolkit::registry::RegistryState {
        &mut self.registry_state
    }
    registry_handlers![
        smithay_client_toolkit::output::OutputState,
        smithay_client_toolkit::seat::SeatState
    ];
}

/// El manager de ventanas: anuncia un toplevel nuevo.
impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for LayerApp {
    fn event(
        state: &mut Self,
        _mgr: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_foreign_toplevel_manager_v1::Event;
        match event {
            Event::Toplevel { toplevel } => {
                let id = state.next_toplevel_id;
                state.next_toplevel_id = state.next_toplevel_id.wrapping_add(1);
                state.toplevels.push(Toplevel::new(id, toplevel));
            }
            Event::Finished => {
                state.toplevels.clear();
                state.marcar_todo_dirty();
            }
            _ => {}
        }
    }

    event_created_child!(LayerApp, ZwlrForeignToplevelManagerV1, [
        EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

/// El notificador de inactividad no emite eventos propios; sólo fabrica
/// notificaciones. Impl vacía para poder bindear el global.
impl Dispatch<ExtIdleNotifierV1, ()> for LayerApp {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: ext_idle_notifier_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// La notificación de inactividad: `idled` cuando el sistema cumplió el timeout
/// sin actividad, `resumed` al volver. Es la señal que dispara el idle de
/// energía — y, vía re-armado, su reintento.
impl Dispatch<ExtIdleNotificationV1, ()> for LayerApp {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use ext_idle_notification_v1::Event;
        match event {
            Event::Idled => state.energia_al_ociar(qh),
            Event::Resumed => state.energia_al_volver(qh),
            _ => {}
        }
    }
}

/// El manager de idle-inhibit no emite eventos; sólo fabrica inhibidores.
impl Dispatch<ZwpIdleInhibitManagerV1, ()> for LayerApp {
    fn event(
        _: &mut Self,
        _: &ZwpIdleInhibitManagerV1,
        _: zwp_idle_inhibit_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// El inhibidor de inactividad tampoco emite eventos; existir es su efecto.
impl Dispatch<ZwpIdleInhibitorV1, ()> for LayerApp {
    fn event(
        _: &mut Self,
        _: &ZwpIdleInhibitorV1,
        _: zwp_idle_inhibitor_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Un handle de toplevel: el compositor le manda título / app_id / estado.
impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for LayerApp {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Event;
        let pos = state.toplevels.iter().position(|t| &t.handle == handle);
        let Some(i) = pos else { return };
        match event {
            Event::Title { title } => state.toplevels[i].set_title(title),
            Event::AppId { app_id } => state.toplevels[i].set_app_id(app_id),
            Event::State { state: estados } => state.toplevels[i].set_state(&estados),
            Event::Done => {
                if state.toplevels[i].confirmar() {
                    state.marcar_todo_dirty();
                }
            }
            Event::Closed => {
                let t = state.toplevels.remove(i);
                t.handle.destroy();
                state.marcar_todo_dirty();
            }
            _ => {}
        }
    }
}
