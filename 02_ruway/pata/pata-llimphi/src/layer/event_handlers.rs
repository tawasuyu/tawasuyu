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
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT, BTN_RIGHT},
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

use llimphi_ui::llimphi_compositor::{hit_test_click, hit_test_hover, hit_test_scroll, DragPhase};

use crate::toplevel::Toplevel;

use super::{app_impl::*, diag, LayerApp, LayerDrag, MenuKind};

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
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
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
                self.set_menu_open(false);
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
        if !self.shuma.open {
            return;
        }
        // Ctrl+Shift+W repliega el drawer.
        if self.mods.ctrl
            && self.mods.shift
            && matches!(event.keysym, Keysym::w | Keysym::W)
        {
            self.set_shuma_open(false);
            return;
        }
        if let Some(ke) = self.keysym_to_keyevent(&event) {
            if self.shuma_full.is_some() {
                // Live-wire: la tecla la traduce la shuma completa según su foco
                // interno (input de la sesión activa / PTY-TUI / rails).
                let m = self
                    .shuma_full
                    .as_ref()
                    .and_then(|f| crate::shuma_app::on_key(f, &ke));
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
                            self.panels[pi].hover_idx = nuevo;
                            self.panels[pi].dirty = true;
                            self.update_tooltip(pi, nuevo, qh);
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
                    }
                }
                continue;
            }
            if let PointerEventKind::Press { button, .. } = e.kind {
                if button != BTN_LEFT && button != BTN_RIGHT {
                    continue;
                }
                let Some(pi) = self.panel_de(&e.surface) else {
                    continue;
                };
                let (px, py) = (e.position.0 as f32, e.position.1 as f32);
                let derecho = button == BTN_RIGHT;
                // Nodo arrastrable bajo el press (izquierdo): arranca un drag.
                if !derecho {
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
                    } else {
                        if let Some(m) = n.on_click.clone() {
                            return Some(m);
                        }
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
