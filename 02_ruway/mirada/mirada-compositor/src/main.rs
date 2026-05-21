//! `mirada-compositor` — el Cuerpo del compositor carmen.
//!
//! Un compositor Wayland teselante real, sobre `smithay`, con backend
//! `winit`: corre **anidado** como una ventana dentro de tu sesión
//! gráfica actual (X11 o Wayland). Habla el protocolo Wayland con los
//! clientes, compone sus superficies y aplica la geometría que decide el
//! Cerebro.
//!
//! Dos modos:
//!
//! - **Autónomo** (por defecto): lleva un [`Desktop`] embebido — es un
//!   compositor teselante completo en un solo proceso. Lánzalo y abre
//!   clientes; el teclado (`Super+…`) maneja el escritorio.
//! - **Enlazado** (`MIRADA_SOCKET=/ruta`): el Cuerpo escucha ahí y la
//!   app `mirada` (el Cerebro GPUI) se conecta; la geometría viaja por
//!   [`mirada_link`].
//!
//! Cómo probarlo en un Linux real: ver `crates/apps/mirada-compositor/README.md`.

use std::sync::Arc;
use std::time::Instant;

use smithay::backend::input::{InputEvent, KeyState, KeyboardKeyEvent};
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::{draw_render_elements, on_commit_buffer_handler};
use smithay::backend::renderer::{Color32F, Frame, Renderer};
use smithay::backend::winit::{self, WinitEvent};
use smithay::input::keyboard::{xkb, FilterResult, KeyboardHandle, Keysym, ModifiersState};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer;
use smithay::reexports::wayland_server::protocol::wl_output;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display, ListeningSocket};
use smithay::reexports::winit::platform::pump_events::PumpStatus;
use smithay::utils::{Rectangle, SERIAL_COUNTER};
use smithay::utils::{Serial, Transform};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    with_states, with_surface_tree_downward, CompositorClientState, CompositorHandler,
    CompositorState, SurfaceAttributes, TraversalAction,
};
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_seat, delegate_shm, delegate_xdg_shell,
};

use mirada_body::{BodyOp, BodyState};
use mirada_brain::{
    BodyEvent, BrainCommand, CtlReply, CtlRequest, CtlServer, Desktop, Keymap, Rules,
};
use mirada_link::BodyLink;

mod drm_backend;

// ---------------------------------------------------------------------
// Estado
// ---------------------------------------------------------------------

/// De dónde salen las decisiones de geometría.
enum Brain {
    /// El compositor lleva su propio `Desktop` — proceso único.
    Embedded(Desktop),
    /// Un Cerebro externo (la app `mirada`) por socket.
    Linked(BodyLink),
}

/// Una ventana de cliente que el compositor gestiona.
struct ManagedWindow {
    id: u64,
    toplevel: ToplevelSurface,
    surface: WlSurface,
    /// Esquina superior-izquierda en píxeles, según el Cerebro.
    loc: (i32, i32),
    visible: bool,
    /// `true` si flota: se compone por encima de las teseladas.
    floating: bool,
}

/// El estado global del compositor.
struct App {
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    seat_state: SeatState<Self>,
    data_device_state: DataDeviceState,
    seat: Seat<Self>,
    keyboard: Option<KeyboardHandle<Self>>,

    /// Ventanas gestionadas, en orden de aparición.
    windows: Vec<ManagedWindow>,
    /// La contabilidad del Cuerpo (mirada-body).
    body: BodyState,
    /// El Cerebro: embebido o enlazado.
    brain: Brain,
    /// Atajos globales a interceptar (los registra el Cerebro).
    grabs: Vec<String>,
    /// Atajo capturado en el último evento de teclado, pendiente de enviar.
    pending_keybind: Option<String>,
    next_id: u64,
    running: bool,
}

impl App {
    /// Inyecta un evento del Cuerpo en el Cerebro y aplica su respuesta.
    fn brain_feed(&mut self, event: BodyEvent) {
        let cmds = match &mut self.brain {
            Brain::Embedded(desktop) => desktop.on_event(event),
            Brain::Linked(link) => {
                let _ = link.send(&event);
                Vec::new()
            }
        };
        self.apply_commands(cmds);
    }

    /// Drena los comandos de un Cerebro enlazado (no hace nada si es embebido).
    fn brain_poll(&mut self) {
        let cmds = match &self.brain {
            Brain::Linked(link) => link.drain(),
            Brain::Embedded(_) => Vec::new(),
        };
        if !cmds.is_empty() {
            self.apply_commands(cmds);
        }
    }

    /// Atiende una petición del API de control (`mirada-ctl`).
    fn serve_ctl(&mut self, req: CtlRequest) -> CtlReply {
        match req {
            CtlRequest::Do(action) => {
                let cmds = match &mut self.brain {
                    Brain::Embedded(d) => Some(d.apply(action)),
                    Brain::Linked(_) => None,
                };
                match cmds {
                    Some(cmds) => {
                        self.apply_commands(cmds);
                        CtlReply::Ok
                    }
                    None => CtlReply::Error(
                        "el Cerebro es externo; usa mirada-ctl contra la app mirada".into(),
                    ),
                }
            }
            CtlRequest::ListWindows => match &self.brain {
                Brain::Embedded(d) => CtlReply::Windows(d.window_lines()),
                Brain::Linked(_) => CtlReply::Error("el Cerebro es externo".into()),
            },
        }
    }

    /// Traduce los comandos del Cerebro a operaciones y las ejecuta.
    fn apply_commands(&mut self, cmds: Vec<BrainCommand>) {
        for cmd in cmds {
            let ops = self.body.apply(cmd);
            for op in ops {
                self.exec_op(op);
            }
        }
    }

    /// Ejecuta una operación concreta sobre las superficies reales.
    fn exec_op(&mut self, op: BodyOp) {
        match op {
            BodyOp::Configure { id, rect, visible, floating, fullscreen } => {
                if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                    w.loc = (rect.x, rect.y);
                    w.visible = visible;
                    w.floating = floating;
                    w.toplevel.with_pending_state(|s| {
                        s.size = Some((rect.w.max(1), rect.h.max(1)).into());
                        if fullscreen {
                            s.states.set(xdg_toplevel::State::Fullscreen);
                        } else {
                            s.states.unset(xdg_toplevel::State::Fullscreen);
                        }
                    });
                    w.toplevel.send_pending_configure();
                }
            }
            BodyOp::Focus(id) => {
                let mut target = None;
                for w in &self.windows {
                    let active = w.id == id;
                    if active {
                        target = Some(w.surface.clone());
                    }
                    w.toplevel.with_pending_state(|s| {
                        if active {
                            s.states.set(xdg_toplevel::State::Activated);
                        } else {
                            s.states.unset(xdg_toplevel::State::Activated);
                        }
                    });
                    w.toplevel.send_pending_configure();
                }
                if let Some(kb) = self.keyboard.clone() {
                    kb.set_focus(self, target, SERIAL_COUNTER.next_serial());
                }
            }
            BodyOp::Unfocus => {
                if let Some(kb) = self.keyboard.clone() {
                    kb.set_focus(self, Option::<WlSurface>::None, SERIAL_COUNTER.next_serial());
                }
            }
            BodyOp::CloseClient(id) | BodyOp::KillClient(id) => {
                if let Some(w) = self.windows.iter().find(|w| w.id == id) {
                    w.toplevel.send_close();
                }
            }
            BodyOp::SetGrabs(keys) => self.grabs = keys,
            BodyOp::SetCursor(_) => {}
            BodyOp::Shutdown => self.running = false,
        }
    }

    /// Registra un toplevel recién creado y avisa al Cerebro.
    fn register_toplevel(&mut self, toplevel: ToplevelSurface) {
        let surface = toplevel.wl_surface().clone();
        let id = self.next_id;
        self.next_id += 1;

        let (app_id, title) = with_states(&surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().ok())
                .map(|d| {
                    (
                        d.app_id.clone().unwrap_or_default(),
                        d.title.clone().unwrap_or_default(),
                    )
                })
                .unwrap_or_default()
        });
        let app_id = if app_id.is_empty() { "cliente".into() } else { app_id };
        let title = if title.is_empty() { format!("ventana {id}") } else { title };

        self.windows.push(ManagedWindow {
            id,
            toplevel,
            surface,
            loc: (0, 0),
            visible: false,
            floating: false,
        });
        let ev = self.body.open_surface(id, app_id, title);
        self.brain_feed(ev);
    }
}

// ---------------------------------------------------------------------
// Handlers de protocolo
// ---------------------------------------------------------------------

impl CompositorHandler for App {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
    }
}

impl BufferHandler for App {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
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
            let id = self.windows.remove(pos).id;
            if let Some(ev) = self.body.close_surface(id) {
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
        if let Some(ev) = self.body.retitle_surface(id, title) {
            self.brain_feed(ev);
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

impl SelectionHandler for App {
    type SelectionUserData = ();
}

impl DataDeviceHandler for App {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
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
    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }
}

delegate_compositor!(App);
delegate_xdg_shell!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_data_device!(App);

// ---------------------------------------------------------------------
// Datos por cliente
// ---------------------------------------------------------------------

#[derive(Default)]
struct ClientState {
    compositor_state: CompositorClientState,
}
impl ClientData for ClientState {
    fn initialized(&self, _id: ClientId) {}
    fn disconnected(&self, _id: ClientId, _reason: DisconnectReason) {}
}

// ---------------------------------------------------------------------
// Utilidades
// ---------------------------------------------------------------------

/// Construye la cadena de un atajo (`"Super+Shift+j"`) desde el estado de
/// modificadores y el keysym, con el mismo formato que el mapa de teclas
/// de [`mirada_brain`]. `None` si no es una tecla mapeable.
fn combo_string(mods: &ModifiersState, sym: Keysym) -> Option<String> {
    let utf = xkb::keysym_to_utf8(sym);
    let key = utf.trim_end_matches('\0');
    let name = if key == " " {
        "space".to_string()
    } else {
        // ¿Es un único carácter imprimible? Entonces la tecla es ese carácter.
        let mut chars = key.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_graphic() => c.to_ascii_lowercase().to_string(),
            // Si no, una tecla con nombre: Return, Tab, Up, F5…
            _ => named_key(sym)?,
        }
    };
    let mut combo = String::new();
    if mods.logo {
        combo.push_str("Super+");
    }
    if mods.ctrl {
        combo.push_str("Ctrl+");
    }
    if mods.shift {
        combo.push_str("Shift+");
    }
    if mods.alt {
        combo.push_str("Alt+");
    }
    combo.push_str(&name);
    Some(combo)
}

/// El nombre canónico de una tecla especial — `Return`, `Tab`, `Up`,
/// `F5`… `None` si xkb no le da un nombre razonable.
fn named_key(sym: Keysym) -> Option<String> {
    let name = xkb::keysym_get_name(sym);
    if name.is_empty() || name == "NoSymbol" || name.starts_with("0x") {
        None
    } else {
        Some(name)
    }
}

/// Despacha los callbacks de frame de un árbol de superficies: avisa a
/// cada cliente de que puede dibujar el siguiente cuadro.
fn send_frames_surface_tree(surface: &WlSurface, time: u32) {
    with_surface_tree_downward(
        surface,
        (),
        |_, _, &()| TraversalAction::DoChildren(()),
        |_surf, states, &()| {
            for callback in states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .frame_callbacks
                .drain(..)
            {
                callback.done(time);
            }
        },
        |_, _, &()| true,
    );
}

// ---------------------------------------------------------------------
// Bucle principal
// ---------------------------------------------------------------------

/// Carga las reglas de ventana del usuario, o ninguna si no hay archivo.
fn load_user_rules() -> Rules {
    match Rules::default_path() {
        Some(p) => Rules::load_or_default(&p),
        None => Rules::default(),
    }
}

/// El backend `winit`: corre anidado dentro de una sesión gráfica.
fn run_winit() -> Result<(), Box<dyn std::error::Error>> {
    let mut display: Display<App> = Display::new()?;
    let dh = display.handle();

    let mut seat_state = SeatState::new();
    let seat = seat_state.new_wl_seat(&dh, "mirada");

    // El keymap del usuario (`~/.config/mirada/keymap.ron`). Sólo lo usa
    // el Cerebro embebido; con un Cerebro enlazado, el keymap es asunto suyo.
    let keymap_path = Keymap::default_path();

    // Elige el Cerebro: enlazado si `MIRADA_SOCKET` está puesto.
    let brain = match std::env::var("MIRADA_SOCKET") {
        Ok(path) => {
            println!("mirada-compositor · esperando al Cerebro en {path} …");
            let link = BodyLink::listen(&path)?;
            println!("mirada-compositor · Cerebro conectado.");
            Brain::Linked(link)
        }
        Err(_) => {
            println!("mirada-compositor · modo autónomo (Cerebro embebido).");
            let keymap = match &keymap_path {
                Some(p) => Keymap::load_or_init(p),
                None => Keymap::default(),
            };
            let mut desktop = Desktop::with_keymap(keymap);
            desktop.set_rules(load_user_rules());
            Brain::Embedded(desktop)
        }
    };

    let mut state = App {
        compositor_state: CompositorState::new::<App>(&dh),
        xdg_shell_state: XdgShellState::new::<App>(&dh),
        shm_state: ShmState::new::<App>(&dh, Vec::new()),
        seat_state,
        data_device_state: DataDeviceState::new::<App>(&dh),
        seat,
        keyboard: None,
        windows: Vec::new(),
        body: BodyState::new(),
        brain,
        grabs: Vec::new(),
        pending_keybind: None,
        next_id: 1,
        running: true,
    };

    let keyboard = state.seat.add_keyboard(Default::default(), 200, 25)?;
    state.keyboard = Some(keyboard.clone());

    // En modo embebido, el propio Desktop dicta los atajos a interceptar.
    if let Brain::Embedded(desktop) = &state.brain {
        let grab = desktop.grab_keys();
        state.apply_commands(vec![grab]);
    }

    // Vigilancia del keymap para recargarlo en caliente — sólo tiene
    // sentido con el Cerebro embebido.
    let keymap_watch = match (&state.brain, &keymap_path) {
        (Brain::Embedded(_), Some(p)) => Keymap::watch(p).ok(),
        _ => None,
    };
    if keymap_watch.is_some() {
        println!("mirada-compositor · vigilando el keymap (recarga en caliente).");
    }

    // API de control (mirada-ctl) — sólo con el Cerebro embebido; si es
    // externo, el socket de control lo abre él.
    let ctl = match &state.brain {
        Brain::Embedded(_) => {
            let path = mirada_brain::ctl::default_socket_path();
            match CtlServer::bind(&path) {
                Ok(s) => {
                    println!("mirada-compositor · API de control en {}", path.display());
                    Some(s)
                }
                Err(e) => {
                    eprintln!("mirada-compositor · sin API de control: {e}");
                    None
                }
            }
        }
        Brain::Linked(_) => None,
    };

    // El backend gráfico va primero. winit abre la ventana del compositor
    // dentro de tu sesión gráfica anfitriona, y para encontrarla lee
    // `WAYLAND_DISPLAY` / `DISPLAY` del entorno. Si publicáramos antes
    // nuestro propio socket en `WAYLAND_DISPLAY`, winit intentaría
    // anidarse en nosotros mismos —un socket que aún no atiende a nadie—
    // y se quedaría colgado para siempre.
    let (mut backend, mut winit) = match winit::init::<GlesRenderer>() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("mirada-compositor · no pude abrir la ventana: {e}");
            eprintln!(
                "   El backend `winit` necesita una sesión gráfica anfitriona\n   \
                 (X11 o Wayland) donde dibujar la ventana del compositor.\n   \
                 Aquí no hay ninguna: DISPLAY='{}', WAYLAND_DISPLAY='{}',\n   \
                 XDG_SESSION_TYPE='{}'.\n   \
                 Lánzalo desde un escritorio gráfico, o desde un servidor X\n   \
                 virtual (Xvfb) al que te conectes por VNC.",
                std::env::var("DISPLAY").unwrap_or_default(),
                std::env::var("WAYLAND_DISPLAY").unwrap_or_default(),
                std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "tty".into()),
            );
            return Err(e.into());
        }
    };

    // Ahora sí, nuestro propio socket Wayland — y `WAYLAND_DISPLAY` se
    // publica *después* de winit, sólo para los clientes que lancemos
    // como procesos hijos.
    let listener = ListeningSocket::bind_auto("wayland", 1..32)?;
    let socket_name = listener
        .socket_name()
        .and_then(|s| s.to_str())
        .unwrap_or("wayland-?")
        .to_string();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    println!("mirada-compositor · escuchando en WAYLAND_DISPLAY={socket_name}");
    println!("   lanza un cliente:  WAYLAND_DISPLAY={socket_name} foot");

    let start = Instant::now();
    let mut clients = Vec::new();

    // Salida inicial = el tamaño de la ventana winit.
    {
        let size = backend.window_size();
        let ev = state.body.add_output(0, size.w, size.h);
        state.brain_feed(ev);
    }

    while state.running {
        // 1 · Eventos del backend (teclado, redimensión, cierre).
        let status = winit.dispatch_new_events(|event| match event {
            WinitEvent::CloseRequested => state.running = false,
            WinitEvent::Resized { size, .. } => {
                let ev = state.body.remove_output(0);
                state.brain_feed(ev);
                let ev = state.body.add_output(0, size.w, size.h);
                state.brain_feed(ev);
            }
            WinitEvent::Input(InputEvent::Keyboard { event }) => {
                let code = event.key_code();
                let key_state = event.state();
                let pressed = key_state == KeyState::Pressed;
                let time = start.elapsed().as_millis() as u32;
                keyboard.clone().input::<(), _>(
                    &mut state,
                    code,
                    key_state,
                    SERIAL_COUNTER.next_serial(),
                    time,
                    |st, mods, handle| {
                        if !pressed {
                            return FilterResult::Forward;
                        }
                        if let Some(combo) = combo_string(mods, handle.modified_sym()) {
                            if st.grabs.contains(&combo) {
                                st.pending_keybind = Some(combo);
                                return FilterResult::Intercept(());
                            }
                        }
                        FilterResult::Forward
                    },
                );
                if let Some(combo) = state.pending_keybind.take() {
                    let ev = state.body.keybind(combo);
                    state.brain_feed(ev);
                }
            }
            _ => {}
        });
        if let PumpStatus::Exit(_) = status {
            break;
        }

        // 2 · Comandos de un Cerebro enlazado.
        state.brain_poll();

        // 2 bis · Recarga del keymap si el archivo cambió en disco.
        if keymap_watch.as_ref().is_some_and(|w| w.changed()) {
            if let Some(path) = &keymap_path {
                match Keymap::load(path) {
                    Ok(km) => {
                        let cmd = if let Brain::Embedded(d) = &mut state.brain {
                            Some(d.set_keymap(km))
                        } else {
                            None
                        };
                        if let Some(cmd) = cmd {
                            state.apply_commands(vec![cmd]);
                        }
                        println!("mirada-compositor · keymap recargado.");
                    }
                    Err(e) => eprintln!(
                        "mirada-compositor · keymap inválido, conservo el anterior: {e}"
                    ),
                }
            }
        }

        // 2 ter · Peticiones del API de control (mirada-ctl).
        if let Some(ctl) = &ctl {
            while let Some(mut conn) = ctl.poll() {
                let reply = match conn.read_request() {
                    Ok(Some(req)) => state.serve_ctl(req),
                    Ok(None) => continue,
                    Err(e) => CtlReply::Error(format!("{e}")),
                };
                let _ = conn.reply(&reply);
            }
        }

        // 3 · Composición de las superficies en sus rectángulos.
        let size = backend.window_size();
        let damage: Rectangle<i32, smithay::utils::Physical> = Rectangle::from_size(size);
        {
            let (renderer, mut framebuffer) = backend.bind().unwrap();
            // Orden de pintado: la lista de elementos va front-to-back
            // (índice 0 = encima), así que las flotantes —que deben
            // quedar sobre las teseladas— se ordenan primero. `sort_by_key`
            // es estable: dentro de cada grupo se respeta el orden de apertura.
            let mut shown: Vec<&ManagedWindow> =
                state.windows.iter().filter(|w| w.visible).collect();
            shown.sort_by_key(|w| !w.floating);
            let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = shown
                .iter()
                .flat_map(|w| {
                    render_elements_from_surface_tree(
                        renderer,
                        &w.surface,
                        w.loc,
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )
                })
                .collect();
            let mut frame = renderer
                .render(&mut framebuffer, size, Transform::Flipped180)
                .unwrap();
            frame
                .clear(Color32F::new(0.05, 0.05, 0.08, 1.0), &[damage])
                .unwrap();
            draw_render_elements(&mut frame, 1.0, &elements, &[damage]).unwrap();
            let _ = frame.finish().unwrap();
        }

        // 4 · Callbacks de frame + clientes nuevos + flush.
        let time = start.elapsed().as_millis() as u32;
        for w in &state.windows {
            send_frames_surface_tree(&w.surface, time);
        }
        if let Some(stream) = listener.accept()? {
            let client = display
                .handle()
                .insert_client(stream, Arc::new(ClientState::default()))
                .unwrap();
            clients.push(client);
        }
        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;

        backend.submit(Some(&[damage])).unwrap();
    }

    println!("mirada-compositor · adiós.");
    Ok(())
}

fn main() {
    let arg = std::env::args().nth(1);
    let result = match arg.as_deref() {
        Some("--drm") => drm_backend::run(),
        Some("--winit") => run_winit(),
        Some(other) => {
            eprintln!("mirada-compositor: opción desconocida «{other}» — usa --drm o --winit");
            std::process::exit(2);
        }
        None => {
            // Auto: con sesión gráfica anfitriona → winit (anidado);
            // sin ella (una TTY pelada) → backend DRM.
            let nested = std::env::var_os("WAYLAND_DISPLAY").is_some()
                || std::env::var_os("DISPLAY").is_some();
            if nested {
                println!("mirada-compositor · sesión gráfica detectada → backend winit.");
                run_winit()
            } else {
                println!("mirada-compositor · sin sesión gráfica → backend DRM.");
                drm_backend::run()
            }
        }
    };
    if let Err(e) = result {
        eprintln!("mirada-compositor · error: {e}");
        std::process::exit(1);
    }
}
