// Handlers de protocolo Wayland.
use crate::*;
use smithay::reexports::wayland_server::protocol::wl_output;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::reexports::wayland_server::protocol::wl_buffer;
use smithay::reexports::wayland_server::Client;
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData};
use smithay::desktop::{layer_map_for_output, LayerSurface as DesktopLayerSurface, WindowSurfaceType};
use smithay::wayland::compositor::{get_parent, with_states, CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes, TraversalAction};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData};
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::wayland::selection::data_device::{ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler};
use smithay::wayland::selection::wlr_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::input::pointer::CursorImageStatus;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::foreign_toplevel_list::{ForeignToplevelListHandler, ForeignToplevelListState};
use smithay::wayland::output::OutputHandler;
use smithay::utils::SERIAL_COUNTER;
use smithay::desktop::WindowSurfaceType as WST2;
use smithay::{
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_dmabuf,
    delegate_foreign_toplevel_list, delegate_layer_shell, delegate_output, delegate_seat,
    delegate_shm, delegate_virtual_keyboard_manager, delegate_xdg_decoration, delegate_xdg_shell,
};

impl CompositorHandler for App {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        // Daño para screencopy `copy_with_damage`: el commit de un toplevel
        // gestionado (o de una de sus subsuperficies) daña su celda; el de
        // un layer surface (waybar y cía.) daña todo — granularidad gruesa
        // para un caso raro. Los demás (cursor, íconos de drag) no dañan:
        // tampoco entran en la captura. Guardado por la cola para no pagar
        // el lookup en el camino caliente.
        if self.pending_screencopy.iter().any(|p| p.con_damage()) {
            let mut raiz = surface.clone();
            while let Some(p) = get_parent(&raiz) {
                raiz = p;
            }
            if let Some(rect) = self
                .windows
                .iter()
                .find(|w| w.surface == raiz)
                .map(|w| Rectangle::new(w.loc.into(), w.size.into()))
            {
                screencopy::danar(self, rect);
            } else if self.outputs.iter().any(|o| {
                layer_map_for_output(o)
                    .layer_for_surface(&raiz, WindowSurfaceType::ALL)
                    .is_some()
            }) {
                screencopy::danar_todo(self);
            }
        }
        // Layer surface: cada commit re-arregla el mapa (zona exclusiva) y,
        // en el PRIMER commit, le mandamos el configure inicial.
        if let Some(output) = self.output.clone() {
            let mut map = layer_map_for_output(&output);
            let layer = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL | WindowSurfaceType::POPUP)
                .cloned();
            if let Some(layer) = layer {
                // ¿Ya salió el configure inicial? `arrange()` calcula y guarda
                // el tamaño anclado, pero —por el spec— NO manda el configure
                // inicial: ese hay que mandarlo en respuesta al primer commit.
                // Sin él el cliente nunca conoce su tamaño y no pinta.
                let initial_sent = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<LayerSurfaceData>()
                        .map(|d| lock_tolerante(d).initial_configure_sent)
                        .unwrap_or(false)
                });
                map.arrange();
                if !initial_sent {
                    layer.layer_surface().send_configure();
                }
                drop(map);
                self.recompute_reservations();
                // Si el commit cambió la interactividad de teclado (el drawer
                // Quake abrió/cerró), reasignamos el foco a quien corresponda.
                self.reconcile_layer_keyboard();
            }
        }
    }
}

impl BufferHandler for App {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl WlrLayerShellHandler for App {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output_hint: Option<wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        // Si el cliente pasó `output_hint`, mapeamos al monitor que pidió.
        // Si no, cae al primario (status quo: dock/barras sin elección).
        let target = output_hint
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.output.clone());
        let Some(output) = target else {
            return; // sin outputs todavía; el cliente reintentará
        };
        // Tope de layer surfaces por cliente (mismo agotamiento que los
        // toplevels): cada layer mapeado se arregla y se pinta en cada frame.
        // Una barra/fondo legítimos usan 1-2 por salida; pasado el tope no lo
        // mapeamos (queda sin rol activo, sin costo de arrange/render). 32 cubre
        // de sobra un cliente multi-monitor (barra+fondo por salida).
        const MAX_LAYERS_POR_CLIENTE: usize = 32;
        if let Some(cid) = surface.wl_surface().client().map(|c| c.id()) {
            let n: usize = self
                .outputs
                .iter()
                .map(|o| {
                    layer_map_for_output(o)
                        .layers()
                        .filter(|l| {
                            l.wl_surface().client().map(|c| c.id()).as_ref() == Some(&cid)
                        })
                        .count()
                })
                .sum();
            if n >= MAX_LAYERS_POR_CLIENTE {
                return; // no lo mapeamos: el cliente abusó del recurso
            }
        }
        let desktop = DesktopLayerSurface::new(surface, namespace.clone());
        let mut map = layer_map_for_output(&output);
        if let Err(e) = map.map_layer(&desktop) {
            eprintln!("mirada-compositor · no pude mapear el layer surface «{namespace}»: {e:?}");
        }
        drop(map);
        self.recompute_reservations();
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        // Una layer puede haber sido mapeada a cualquier output (per-output
        // layer-shell): la buscamos en todos hasta dar con su mapa.
        let mut found = false;
        for output in self.outputs.clone() {
            let mut map = layer_map_for_output(&output);
            if let Some(layer) = map
                .layer_for_surface(surface.wl_surface(), WindowSurfaceType::ALL)
                .cloned()
            {
                map.unmap_layer(&layer);
                found = true;
                break;
            }
        }
        if !found {
            return;
        }
        self.recompute_reservations();
        // Una layer destruida pudo ser la Exclusive: devolver el teclado.
        self.reconcile_layer_keyboard();
    }
}

impl DmabufHandler for App {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    /// Un cliente importó un DMA-BUF. El `GlesRenderer` lo importará de
    /// verdad al componer; aquí basta con aceptarlo — un búfer inválido
    /// sólo dejará en blanco ese cuadro de esa ventana.
    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        _dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        let _ = notifier.successful::<App>();
    }
}

impl ShmHandler for App {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl XdgShellHandler for App {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Tope de toplevels por cliente: un cliente con fuga (o malicioso) podía
        // crear ventanas sin freno y agotar la memoria del compositor —que
        // además recorre `windows` en cada frame (sort, hit-test, render)—. Más
        // allá del tope cerramos el toplevel nuevo (el cliente recibe `close`)
        // en vez de registrarlo. 64 es holgado para cualquier app real.
        const MAX_TOPLEVELS_POR_CLIENTE: usize = 64;
        if let Some(cid) = surface.wl_surface().client().map(|c| c.id()) {
            let n = self
                .windows
                .iter()
                .filter(|w| w.surface.client().map(|c| c.id()).as_ref() == Some(&cid))
                .count();
            if n >= MAX_TOPLEVELS_POR_CLIENTE {
                surface.send_close();
                return;
            }
        }
        surface.with_pending_state(|s| {
            s.states.set(xdg_toplevel::State::Activated);
        });
        surface.send_configure();
        self.register_toplevel(surface);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let pos = self
            .windows
            .iter()
            .position(|w| w.surface == *surface.wl_surface());
        if let Some(pos) = pos {
            let w = self.windows.remove(pos);
            // La celda que ocupaba queda dañada (screencopy): se repinta
            // lo que hubiera debajo.
            screencopy::danar(self, Rectangle::new(w.loc.into(), w.size.into()));
            // Baja en el censo: los clientes autorizados reciben `closed`.
            if let Some(handle) = &w.foreign_handle {
                self.foreign_toplevel_state.remove_toplevel(handle);
            }
            if w.is_shell {
                // El shell se cerró: libera su reserva (insets en cero), el
                // Cerebro vuelve a teselar en la salida entera.
                let ev = self.body.reserve_output(0, 0, 0, 0, 0);
                self.brain_feed(ev);
            } else if let Some(ev) = self.body.close_surface(w.id) {
                self.brain_feed(ev);
            }
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        let id = self
            .windows
            .iter()
            .find(|w| w.surface == *surface.wl_surface())
            .map(|w| w.id);
        let Some(id) = id else { return };
        let title = with_states(surface.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().ok())
                .and_then(|d| d.title.clone())
                .unwrap_or_default()
        });
        // Espeja el título en la ventana gestionada (para pintar la etiqueta)
        // y en el censo `ext_foreign_toplevel_list`.
        let mut danio = None;
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            if w.title != title {
                // La barra de título se repinta (screencopy).
                danio = Some(Rectangle::new(w.loc.into(), w.size.into()));
            }
            w.title = title.clone();
            if let Some(handle) = &w.foreign_handle {
                handle.send_title(&title);
                handle.send_done();
            }
        }
        if let Some(d) = danio {
            screencopy::danar(self, d);
        }
        if let Some(ev) = self.body.retitle_surface(id, title) {
            self.brain_feed(ev);
        }
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        // Espeja el `app_id` en el censo `ext_foreign_toplevel_list` (los
        // clientes suelen fijarlo después de crear el toplevel).
        let app_id = with_states(surface.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().ok())
                .and_then(|d| d.app_id.clone())
                .unwrap_or_default()
        });
        let w = self
            .windows
            .iter()
            .find(|w| w.surface == *surface.wl_surface());
        if let Some(handle) = w.and_then(|w| w.foreign_handle.as_ref()) {
            handle.send_app_id(&app_id);
            handle.send_done();
        }
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<wl_output::WlOutput>,
    ) {
        let id = self
            .windows
            .iter()
            .find(|w| w.surface == *surface.wl_surface())
            .map(|w| w.id);
        if let Some(id) = id {
            self.brain_feed(BodyEvent::FullscreenRequest { id, fullscreen: true });
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        let id = self
            .windows
            .iter()
            .find(|w| w.surface == *surface.wl_surface())
            .map(|w| w.id);
        if let Some(id) = id {
            self.brain_feed(BodyEvent::FullscreenRequest { id, fullscreen: false });
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let _ = surface.send_configure();
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }
}

/// Decoración de ventana: carmen tesela, así que las ventanas no llevan
/// barra de título. Le decimos a todo cliente que la decoración la pone
/// el servidor (`ServerSide`) — y como el servidor no dibuja ninguna, la
/// ventana queda sin marco. Sin esto, clientes como `foot` se dibujan su
/// propia barra (CSD), que estorba en un escritorio teselante.
impl XdgDecorationHandler for App {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        toplevel.send_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        toplevel.send_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        toplevel.send_configure();
    }
}

impl SelectionHandler for App {
    type SelectionUserData = ();
}

impl DataDeviceHandler for App {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl DataControlHandler for App {
    fn data_control_state(&self) -> &DataControlState {
        &self.data_control_state
    }
}
impl ClientDndGrabHandler for App {}
impl ServerDndGrabHandler for App {
    fn send(&mut self, _mime_type: String, _fd: std::os::unix::io::OwnedFd, _seat: Seat<Self>) {}
}

impl SeatHandler for App {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}

    /// El cliente enfocado pidió un cursor — guardamos su petición; el
    /// backend la pinta (su superficie, o el cuadrado si es con nombre).
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
    }
}

/// El protocolo `wl_output` no necesita estado propio — basta con
/// anunciar el global para que los clientes vean que hay un monitor.
impl OutputHandler for App {}

delegate_compositor!(App);
delegate_layer_shell!(App);
delegate_xdg_shell!(App);
delegate_xdg_decoration!(App);
delegate_dmabuf!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_data_device!(App);
delegate_data_control!(App);
delegate_virtual_keyboard_manager!(App);
delegate_foreign_toplevel_list!(App);

impl ForeignToplevelListHandler for App {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_state
    }
}
delegate_output!(App);
