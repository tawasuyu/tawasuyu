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

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::input::{InputEvent, KeyState, KeyboardKeyEvent};
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::{
    draw_render_elements, on_commit_buffer_handler, with_renderer_surface_state,
};
use smithay::backend::renderer::{Color32F, Frame, ImportDma, Renderer};
use smithay::backend::winit::{self, WinitEvent};
use smithay::input::keyboard::{xkb, FilterResult, KeyboardHandle, Keysym, ModifiersState};
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData, PointerHandle};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer;
use smithay::reexports::wayland_server::protocol::wl_output;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display, DisplayHandle, ListeningSocket};
use smithay::reexports::winit::platform::pump_events::PumpStatus;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER};
use smithay::utils::{Serial, Transform};
use smithay::backend::egl::EGLDevice;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::dmabuf::{
    DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier,
};
use smithay::wayland::compositor::{
    with_states, with_surface_tree_downward, CompositorClientState, CompositorHandler,
    CompositorState, SurfaceAttributes, TraversalAction,
};
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::selection::wlr_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData,
    WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::desktop::{layer_map_for_output, LayerSurface as DesktopLayerSurface, WindowSurfaceType};
use smithay::output::Output;
use smithay::{
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_dmabuf,
    delegate_layer_shell, delegate_output, delegate_seat, delegate_shm, delegate_xdg_decoration,
    delegate_xdg_shell,
};

use auth_core::{SessionTicket, UserInfo};
use mirada_body::{BodyOp, BodyState};
use mirada_brain::{
    BodyEvent, BrainCommand, CtlReply, CtlRequest, CtlServer, Desktop, Keymap, Rules,
};
use mirada_link::BodyLink;

mod drm_backend;
mod menu;
mod text;

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

/// La fase del ciclo de vida del Cuerpo. Es un eje **ortogonal** a
/// [`Brain`]: `Brain` dice de dónde sale la geometría; `BodyMode` dice
/// si el compositor está pidiendo credenciales o sirviendo una sesión.
/// Un arranque normal nace ya en [`BodyMode::Session`]; un arranque de
/// DM (`--greeter`) nace en [`BodyMode::Greeter`] y muta una sola vez,
/// al recibir el tiquet de un login válido — la «mutación atómica».
#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyMode {
    /// Pantalla de login: el único cliente es el greeter, no se
    /// registran atajos, se rechaza `Spawn` y no hay autoarranque.
    Greeter,
    /// Sesión de usuario: el compositor funciona con normalidad.
    Session,
}

/// Grosor por defecto de la franja del shell (px), si el entorno no lo fija.
const SHELL_DOCK_DEFAULT: i32 = 40;

/// El borde de la salida al que se acopla la franja del shell (el marco `pata`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ShellAnchor {
    Top,
    Bottom,
    Left,
    Right,
}

impl ShellAnchor {
    /// Parsea el valor de `MIRADA_SHELL_ANCHOR`; cae a `Bottom` si no calza.
    fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "top" => Self::Top,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => Self::Bottom,
        }
    }

    /// `true` para los bordes horizontales (top/bottom): su grosor es alto.
    fn es_horizontal(&self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// Config del acople del shell, resuelta una vez desde el entorno:
/// - `MIRADA_SHELL_APP_ID` — identidad de la ventana-marco (default `gioser.pata`).
/// - `MIRADA_SHELL_ANCHOR` — borde (`top`/`bottom`/`left`/`right`, default `bottom`).
/// - `MIRADA_SHELL_THICKNESS` — grosor en px (default `40`).
/// - `MIRADA_SHELL_AUTOHIDE` — `1`/`true` para autoesconder el dock: nunca
///   reserva su franja (las ventanas usan toda la pantalla) y sólo se muestra,
///   superpuesto, al acercar el puntero al borde anclado.
struct ShellDock {
    app_id: String,
    anchor: ShellAnchor,
    thickness: i32,
    autohide: bool,
}

/// Banda fina (px) del borde anclado que revela el dock autoescondido, y
/// grosor de la sutil franja-pista que se pinta mientras está oculto.
const SHELL_REVEAL_BAND: i32 = 3;

/// La config del shell, leída del entorno la primera vez que se consulta.
fn shell_dock() -> &'static ShellDock {
    static DOCK: std::sync::OnceLock<ShellDock> = std::sync::OnceLock::new();
    DOCK.get_or_init(|| {
        let app_id =
            std::env::var("MIRADA_SHELL_APP_ID").unwrap_or_else(|_| "gioser.pata".to_string());
        let anchor = std::env::var("MIRADA_SHELL_ANCHOR")
            .map(|s| ShellAnchor::parse(&s))
            .unwrap_or(ShellAnchor::Bottom);
        let thickness = std::env::var("MIRADA_SHELL_THICKNESS")
            .ok()
            .and_then(|s| s.trim().parse::<i32>().ok())
            .filter(|t| *t > 0)
            .unwrap_or(SHELL_DOCK_DEFAULT);
        let autohide = std::env::var("MIRADA_SHELL_AUTOHIDE")
            .map(|s| matches!(s.trim(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);
        ShellDock {
            app_id,
            anchor,
            thickness,
            autohide,
        }
    })
}

/// `true` si el `app_id` corresponde al shell: la identidad de `pata` (o el
/// override de `MIRADA_SHELL_APP_ID`), o el alias legacy `carmen.shell`.
fn is_shell_app_id(app_id: &str) -> bool {
    app_id == shell_dock().app_id || app_id == "carmen.shell"
}

/// El rect `(x, y, w, h)` de la franja del shell sobre una salida `ow×oh` con
/// grosor `t`, según el borde. Pura — fácil de testear.
fn shell_strip(anchor: ShellAnchor, ow: i32, oh: i32, t: i32) -> (i32, i32, i32, i32) {
    match anchor {
        ShellAnchor::Top => (0, 0, ow, t),
        ShellAnchor::Bottom => (0, oh - t, ow, t),
        ShellAnchor::Left => (0, 0, t, oh),
        ShellAnchor::Right => (ow - t, 0, t, oh),
    }
}

/// Las zonas exclusivas `(top, bottom, left, right)` que reserva una franja de
/// grosor `t` en el borde `anchor` — lo que el teselado debe esquivar. Pura.
fn shell_insets(anchor: ShellAnchor, t: i32) -> (i32, i32, i32, i32) {
    match anchor {
        ShellAnchor::Top => (t, 0, 0, 0),
        ShellAnchor::Bottom => (0, t, 0, 0),
        ShellAnchor::Left => (0, 0, t, 0),
        ShellAnchor::Right => (0, 0, 0, t),
    }
}

/// La franja-pista `(x, y, w, h)` que se pinta en el borde mientras el dock
/// autoescondido está oculto: una banda fina de grosor `band` pegada al borde
/// anclado, a lo ancho/alto de la franja del dock. Pura.
fn shell_reveal_band(anchor: ShellAnchor, ow: i32, oh: i32, t: i32, band: i32) -> (i32, i32, i32, i32) {
    let (sx, sy, sw, sh) = shell_strip(anchor, ow, oh, t);
    match anchor {
        ShellAnchor::Top => (sx, 0, sw, band),
        ShellAnchor::Bottom => (sx, oh - band, sw, band),
        ShellAnchor::Left => (0, sy, band, sh),
        ShellAnchor::Right => (ow - band, sy, band, sh),
    }
}

/// Decide el próximo estado oculto/visible del dock autoescondido según el
/// puntero. Asimétrico (con histéresis): si está oculto, sólo se revela al
/// tocar la banda fina del borde (`edge_band`); si está visible, sólo se
/// oculta cuando el puntero sale de la franja completa del dock. Pura.
fn autohide_next_hidden(
    anchor: ShellAnchor,
    ow: i32,
    oh: i32,
    t: i32,
    px: i32,
    py: i32,
    hidden: bool,
    edge_band: i32,
) -> bool {
    let (sx, sy, sw, sh) = shell_strip(anchor, ow, oh, t);
    let over_strip = px >= sx && px < sx + sw && py >= sy && py < sy + sh;
    let at_edge = match anchor {
        ShellAnchor::Top => py <= edge_band,
        ShellAnchor::Bottom => py >= oh - edge_band,
        ShellAnchor::Left => px <= edge_band,
        ShellAnchor::Right => px >= ow - edge_band,
    };
    if hidden {
        !at_edge
    } else {
        !over_strip
    }
}

/// Una ventana de cliente que el compositor gestiona.
struct ManagedWindow {
    id: u64,
    toplevel: ToplevelSurface,
    surface: WlSurface,
    /// Esquina superior-izquierda de la celda asignada, según el Cerebro.
    loc: (i32, i32),
    /// Tamaño de la celda asignada — para centrar la ventana si el
    /// cliente presenta una superficie más pequeña.
    size: (i32, i32),
    visible: bool,
    /// `true` si flota: se compone por encima de las teseladas.
    floating: bool,
    /// `true` si tiene el foco del teclado — pinta el marco resaltado.
    focused: bool,
    /// `true` si es la ventana del shell — acoplada al pie, sin teselar.
    is_shell: bool,
    /// `true` si está a pantalla completa — no lleva barra de título ni marco.
    fullscreen: bool,
    /// Título del cliente — para pintar la etiqueta (barra de título).
    /// Se actualiza en `title_changed`.
    title: String,
    /// Búferes de los 4 lados del marco (arriba, abajo, izq., der.) —
    /// cada uno con su `Id` estable para el seguimiento de daño.
    borders: [SolidColorBuffer; 4],
}

/// Un arrastre de ratón en curso: mueve o redimensiona una ventana.
struct DragGrab {
    /// La ventana que se arrastra.
    id: u64,
    /// Mover (`Super`+botón izquierdo) o redimensionar (`Super`+derecho).
    mode: DragMode,
    /// Posición del puntero al empezar el arrastre.
    start_pointer: (f64, f64),
    /// Rectángulo `(x, y, w, h)` de la ventana al empezar.
    start_rect: (i32, i32, i32, i32),
}

/// Qué le hace un arrastre a la ventana.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DragMode {
    /// Reubicar una ventana **flotante** — la esquina la sigue al puntero.
    Move,
    /// Redimensionarla — la esquina inferior-derecha sigue al puntero.
    Resize,
    /// Reordenar una ventana **teselada**: la intercambia con la tesela
    /// bajo el puntero (el Cerebro decide el swap), sin sacarla del teselado.
    Tile,
}

/// El estado global del compositor.
struct App {
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    /// Estado de `zwp_linux_dmabuf` — deja que los clientes con GPU
    /// (apps GPUI, navegadores acelerados) compartan búferes de vídeo.
    dmabuf_state: DmabufState,
    seat_state: SeatState<Self>,
    data_device_state: DataDeviceState,
    /// Estado de `zwlr_data_control_manager_v1` — lectura/escritura del
    /// portapapeles SIN robar foco. Sin esto, `wl-paste` (el widget `clipboard`
    /// de pata lo corre ~1Hz) caía a su fallback: crear una surface de tamaño 0,
    /// robar el foco de teclado para leer la selección, y destruirla — titilando
    /// el foco cada segundo. También lo usan cliphist y los gestores de
    /// portapapeles.
    data_control_state: DataControlState,
    seat: Seat<Self>,
    /// Estado del protocolo `wlr-layer-shell` (barras/fondos/overlays como
    /// waybar, swaybg, wofi, mako).
    layer_shell_state: WlrLayerShellState,
    /// La salida persistente — la necesita `layer_map_for_output` para
    /// arreglar anclajes y zonas exclusivas de los layer surfaces.
    output: Option<Output>,
    /// Gestor de salidas con `xdg-output` (`zxdg_output_manager_v1`): waybar
    /// y otras barras lo exigen para conocer nombre/geometría de las salidas.
    /// Se conserva sólo para mantener vivo el global (de ahí el `allow`).
    #[allow(dead_code)]
    output_manager_state: OutputManagerState,
    keyboard: Option<KeyboardHandle<Self>>,
    pointer: Option<PointerHandle<Self>>,
    /// Posición del puntero en coordenadas globales.
    pointer_loc: (f64, f64),
    /// Qué cursor pide el cliente enfocado — una superficie suya, un
    /// cursor con nombre, u oculto. El backend lo pinta en consecuencia.
    cursor_status: CursorImageStatus,
    /// Arrastre de ventana en curso (mover o redimensionar con el ratón).
    drag: Option<DragGrab>,
    /// Tamaño real de la salida (con la franja del shell incluida) — lo
    /// fija el backend; sirve para acoplar la ventana del shell.
    output_size: (i32, i32),
    /// Con el dock autoescondido (`MIRADA_SHELL_AUTOHIDE`), si está oculto
    /// ahora. Sin autohide se ignora. El puntero cerca del borde lo alterna.
    shell_hidden: bool,
    /// Última reserva publicada `(top, bottom, left, right)` en px — define el
    /// área de trabajo (salida menos dock/layers). Las zonas se escalan a ella.
    reserved: (i32, i32, i32, i32),

    /// Ventanas gestionadas, en orden de aparición.
    windows: Vec<ManagedWindow>,
    /// La contabilidad del Cuerpo (mirada-body).
    body: BodyState,
    /// El Cerebro: embebido o enlazado.
    brain: Brain,
    /// Fase del ciclo de vida — login o sesión (ver [`BodyMode`]).
    mode: BodyMode,
    /// Entorno de sesión (XDG_RUNTIME_DIR del usuario, WAYLAND_DISPLAY
    /// absoluto, bus D-Bus) inyectado a las apps nativas tras el traspaso.
    /// Vacío en modo greeter.
    session_env: Vec<(String, String)>,
    /// Identidad a la que rebajar privilegios al lanzar procesos de
    /// sesión. `None` salvo tras el traspaso del DM — entonces cada
    /// `spawn` hace `setuid`/`setgid` a este usuario (si somos root).
    session_user: Option<UserInfo>,
    /// Atajos globales a interceptar (los registra el Cerebro).
    grabs: Vec<String>,
    /// Parámetros de decoración de ventana (marco, …) que fija el Cerebro.
    decorations: mirada_brain::Decorations,
    /// Atajo capturado en el último evento de teclado, pendiente de enviar.
    pending_keybind: Option<String>,
    /// VT a la que conmutar, capturada por `Ctrl+Alt+Fn`. El backend DRM
    /// la consume tras el evento de teclado (sólo él puede `change_vt`).
    pending_vt: Option<i32>,
    /// Sesión ajena a ejecutar tras cerrar el compositor: el handoff a un
    /// compositor foráneo suelta el DRM (saliendo del bucle) y recién
    /// entonces hace `exec`. `(comando, usuario)`.
    pending_session: Option<(String, Option<UserInfo>)>,
    next_id: u64,
    running: bool,
}

impl App {
    /// La layer surface **interactiva** (capas Overlay/Top — p. ej. las barras de
    /// `pata`) bajo el punto físico `(x, y)`, con el origen de su geometría (para
    /// las coords locales del puntero). Las capas Bottom/Background NO reciben
    /// puntero (son fondo, como swaybg). `None` si no hay ninguna ahí. Lo usa el
    /// ruteo del puntero para que los clicks lleguen a las barras, no sólo a las
    /// ventanas.
    fn layer_under(&self, x: f64, y: f64) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.output.as_ref()?;
        let map = layer_map_for_output(output);
        for kind in [Layer::Overlay, Layer::Top] {
            if let Some(layer) = map.layer_under(kind, (x, y)) {
                let geo = map.layer_geometry(layer)?;
                return Some((
                    layer.wl_surface().clone(),
                    Point::from((geo.loc.x as f64, geo.loc.y as f64)),
                ));
            }
        }
        None
    }

    /// La layer surface bajo `(x, y)` que **acepta foco de teclado** (OnDemand o
    /// Exclusive), para enfocarla al clickearla — el cabezal de shuma de `pata`
    /// pide `OnDemand` y, al desplegar el drawer, `Exclusive`. `None` si la layer
    /// de abajo no quiere teclado (o no hay ninguna).
    fn keyboard_focusable_layer_under(&self, x: f64, y: f64) -> Option<WlSurface> {
        let output = self.output.as_ref()?;
        let map = layer_map_for_output(output);
        for kind in [Layer::Overlay, Layer::Top] {
            if let Some(layer) = map.layer_under(kind, (x, y)) {
                return layer
                    .can_receive_keyboard_focus()
                    .then(|| layer.wl_surface().clone());
            }
        }
        None
    }

    /// La layer surface (Overlay/Top, top-most) que reclama teclado **Exclusive**,
    /// si hay alguna. Mientras exista, el foco-sigue-ratón NO le roba el teclado
    /// (el drawer Quake de `pata` lo necesita para que escribas sin que mover el
    /// mouse sobre una ventana le quite el foco).
    fn exclusive_layer_surface(&self) -> Option<WlSurface> {
        let output = self.output.as_ref()?;
        let map = layer_map_for_output(output);
        for kind in [Layer::Overlay, Layer::Top] {
            if let Some(layer) = map.layers_on(kind).rev().find(|l| {
                l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive
            }) {
                return Some(layer.wl_surface().clone());
            }
        }
        None
    }

    /// Reconcilia el foco del teclado con las layers Exclusive. Una layer que
    /// reclama `Exclusive` (el drawer Quake de `pata` abierto) debe **tener**
    /// el foco — antes lo conseguía sólo si la barra era `OnDemand` y la
    /// clickeabas; ahora se lo damos al volverse Exclusive, sin depender del
    /// click. Al soltar Exclusive (drawer cerrado o destruido) se lo
    /// devolvemos a la ventana que el Cerebro marcó enfocada, así una app
    /// recién lanzada recibe el teclado. Idempotente: sólo toca `set_focus`
    /// si el foco cambia, y nunca le roba el foco a una ventana (eso lo maneja
    /// el Cerebro vía `BodyOp::Focus`).
    fn reconcile_layer_keyboard(&mut self) {
        let Some(kb) = self.keyboard.clone() else {
            return;
        };
        let current = kb.current_focus();
        match self.exclusive_layer_surface() {
            Some(surf) => {
                if current.as_ref() != Some(&surf) {
                    kb.set_focus(self, Some(surf), SERIAL_COUNTER.next_serial());
                }
            }
            None => {
                // Si el foco ya está en una de nuestras ventanas, no lo tocamos
                // (manda el Cerebro). Sólo actuamos si quedó colgado en una
                // layer que ya no es Exclusive.
                let on_window = current
                    .as_ref()
                    .is_some_and(|s| self.windows.iter().any(|w| &w.surface == s));
                if !on_window {
                    let target = self.windows.iter().find(|w| w.focused).map(|w| w.surface.clone());
                    if current != target {
                        kb.set_focus(self, target, SERIAL_COUNTER.next_serial());
                    }
                }
            }
        }
    }

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

    /// Recarga el keymap del usuario en caliente. Conserva el anterior si
    /// el archivo nuevo es inválido. No-op con el Cerebro enlazado (el
    /// keymap es asunto suyo). Lo dispara [`ConfigWatches::poll`].
    fn reload_keymap_from(&mut self, path: &std::path::Path) {
        match Keymap::load(path) {
            Ok(km) => {
                let cmd = if let Brain::Embedded(d) = &mut self.brain {
                    Some(d.set_keymap(km))
                } else {
                    None
                };
                if let Some(cmd) = cmd {
                    self.apply_commands(vec![cmd]);
                    println!("mirada-compositor · keymap recargado.");
                }
            }
            Err(e) => {
                eprintln!("mirada-compositor · keymap inválido, conservo el anterior: {e}")
            }
        }
    }

    /// Recarga la config general (dropterm, teselado, foco, marco) en
    /// caliente y re-envía la decoración. Conserva la anterior si es
    /// inválida. No-op con el Cerebro enlazado.
    fn reload_config_from(&mut self, path: &std::path::Path) {
        match mirada_brain::Config::load(path) {
            Ok(cfg) => {
                let cmds = if let Brain::Embedded(d) = &mut self.brain {
                    d.reload_config(cfg)
                } else {
                    Vec::new()
                };
                if !cmds.is_empty() {
                    self.apply_commands(cmds);
                    println!("mirada-compositor · config recargada.");
                }
            }
            Err(e) => {
                eprintln!("mirada-compositor · config inválida, conservo la anterior: {e}")
            }
        }
    }

    /// Recarga las reglas de ventana en caliente. Aplican a las ventanas
    /// que se abran a partir de ahora; las ya abiertas no se tocan.
    /// Conserva las anteriores si son inválidas. No-op con Cerebro enlazado.
    fn reload_rules_from(&mut self, path: &std::path::Path) {
        match Rules::load(path) {
            Ok(rules) => {
                if let Brain::Embedded(d) = &mut self.brain {
                    d.set_rules(rules);
                    println!("mirada-compositor · reglas recargadas (aplican a ventanas nuevas).");
                }
            }
            Err(e) => {
                eprintln!("mirada-compositor · reglas inválidas, conservo las anteriores: {e}")
            }
        }
    }

    /// La ruta de fuente configurada (para las etiquetas del compositor), si
    /// el Cerebro es embebido y la config la fija. Vacía/None → se prueban
    /// las fuentes comunes del sistema.
    fn config_font_path(&self) -> Option<String> {
        match &self.brain {
            Brain::Embedded(d) => {
                let p = d.config().font_path.clone();
                (!p.is_empty()).then_some(p)
            }
            Brain::Linked(_) => None,
        }
    }

    /// La ruta del wallpaper configurado, si el Cerebro es embebido y la
    /// config la fija. Vacía/None → fondo de color sólido.
    fn config_wallpaper_path(&self) -> Option<String> {
        match &self.brain {
            Brain::Embedded(d) => {
                let p = d.config().wallpaper_path.clone();
                (!p.is_empty()).then_some(p)
            }
            Brain::Linked(_) => None,
        }
    }

    /// El árbol del menú raíz configurado (con submenús anidados). Vacío con
    /// Cerebro enlazado o sin entradas en la config.
    fn config_menu(&self) -> Vec<crate::menu::MenuNode> {
        match &self.brain {
            Brain::Embedded(d) => d.config().menu.iter().map(menu_node_from_entry).collect(),
            Brain::Linked(_) => Vec::new(),
        }
    }

    /// Las zonas de arrastre configuradas (fracciones de la salida). Vacío con
    /// Cerebro enlazado o sin zonas en la config.
    fn config_zones(&self) -> Vec<mirada_brain::ZoneFrac> {
        match &self.brain {
            Brain::Embedded(d) => d
                .config()
                .zones
                .iter()
                .map(|z| mirada_brain::ZoneFrac { x: z.x, y: z.y, w: z.w, h: z.h })
                .collect(),
            Brain::Linked(_) => Vec::new(),
        }
    }

    /// Lanza `cmd` como el usuario de la sesión (igual que [`BodyOp::Spawn`]),
    /// salvo en modo greeter, donde no se lanza nada. Lo usa el menú raíz.
    fn spawn_user(&self, cmd: &str) {
        if self.mode == BodyMode::Greeter {
            eprintln!("mirada-compositor · «{cmd}» rechazado — modo greeter.");
            return;
        }
        spawn_command(cmd, self.session_user.as_ref(), &self.session_env);
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
                // La barra de título reserva una franja arriba: la superficie
                // del cliente se configura más baja por `tb` (no-shell, no
                // fullscreen). `w.size` guarda la celda entera; `render_loc`
                // baja la superficie por `tb`.
                let tbh = self.decorations.titlebar_height.max(0);
                if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                    w.loc = (rect.x, rect.y);
                    w.size = (rect.w, rect.h);
                    w.visible = visible;
                    w.floating = floating;
                    w.fullscreen = fullscreen;
                    let tb = if w.is_shell || fullscreen { 0 } else { tbh };
                    w.toplevel.with_pending_state(|s| {
                        s.size = Some((rect.w.max(1), (rect.h - tb).max(1)).into());
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
                for w in &mut self.windows {
                    let active = w.id == id;
                    w.focused = active;
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
                for w in &mut self.windows {
                    w.focused = false;
                }
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
            BodyOp::SetDecorations(d) => self.decorations = d,
            BodyOp::Spawn(cmd) => {
                // En modo greeter no se lanza nada: la pantalla de login
                // no es un sitio desde donde abrir programas.
                if self.mode == BodyMode::Greeter {
                    eprintln!("mirada-compositor · «{cmd}» rechazado — modo greeter.");
                } else {
                    spawn_command(&cmd, self.session_user.as_ref(), &self.session_env);
                }
            }
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
        // La ventana del shell (el marco pata) no se tesela: se acopla a un borde.
        let is_shell = is_shell_app_id(&app_id);

        self.windows.push(ManagedWindow {
            id,
            toplevel,
            surface,
            loc: (0, 0),
            size: (0, 0),
            visible: false,
            floating: false,
            focused: false,
            is_shell,
            fullscreen: false,
            title: title.clone(),
            borders: std::array::from_fn(|_| SolidColorBuffer::default()),
        });

        if is_shell {
            self.dock_shell();
        } else {
            let app_id = if app_id.is_empty() { "cliente".into() } else { app_id };
            let title = if title.is_empty() { format!("ventana {id}") } else { title };
            let ev = self.body.open_surface(id, app_id, title);
            self.brain_feed(ev);
        }
    }

    /// Acopla la ventana del shell (el marco `pata`): reserva la zona exclusiva
    /// de su borde —el Cerebro tesela el resto, esquivándola— y dimensiona y
    /// coloca la franja ahí. Se llama al registrarla y al cambiar el tamaño de
    /// la salida. Funciona en cualquiera de los cuatro bordes: la reserva por
    /// insets desplaza y encoge el área útil sin tocar el tamaño físico.
    fn dock_shell(&mut self) {
        let (ow, oh) = self.output_size;
        if ow == 0 || oh == 0 {
            return; // la salida todavía no está lista
        }
        let dock = shell_dock();
        // El grosor no puede exceder el lado de la salida sobre el que recorta.
        let limite = if dock.anchor.es_horizontal() { oh } else { ow };
        let t = dock.thickness.clamp(1, limite.max(1));

        // Dimensiona la ventana del shell y la fija en la franja del borde.
        // Con autohide, su visibilidad la decide el puntero (estado actual).
        let visible = !(dock.autohide && self.shell_hidden);
        if let Some(w) = self.windows.iter_mut().find(|w| w.is_shell) {
            let (x, y, sw, sh) = shell_strip(dock.anchor, ow, oh, t);
            w.loc = (x, y);
            w.size = (sw, sh);
            w.visible = visible;
            w.toplevel.with_pending_state(|s| {
                s.size = Some((sw.max(1), sh.max(1)).into());
            });
            w.toplevel.send_pending_configure();
        }

        // La reserva del borde (franja pata + zonas exclusivas de
        // layer-shell) se computa en un solo lugar.
        self.recompute_reservations();
    }

    /// Recalcula y publica al Cerebro el área reservada del borde: suma la
    /// franja del shell (pata) y las zonas exclusivas de los layer surfaces
    /// (waybar y compañía). Fuente única de los insets del teselado.
    fn recompute_reservations(&mut self) {
        let (ow, oh) = self.output_size;
        if ow == 0 || oh == 0 {
            return;
        }
        let (mut top, mut bottom, mut left, mut right) = (0, 0, 0, 0);
        // Lo que los layer surfaces dejan libre (zona no exclusiva).
        if let Some(output) = self.output.clone() {
            let z = layer_map_for_output(&output).non_exclusive_zone();
            top += z.loc.y.max(0);
            left += z.loc.x.max(0);
            right += (ow - (z.loc.x + z.size.w)).max(0);
            bottom += (oh - (z.loc.y + z.size.h)).max(0);
        }
        // Franja del shell (pata), si está acoplado. Con autohide el dock
        // nunca reserva: se superpone al revelarse, las ventanas usan todo.
        let dock = shell_dock();
        if !dock.autohide && self.windows.iter().any(|w| w.is_shell) {
            let limite = if dock.anchor.es_horizontal() { oh } else { ow };
            let t = dock.thickness.clamp(1, limite.max(1));
            let (st, sb, sl, sr) = shell_insets(dock.anchor, t);
            top += st;
            bottom += sb;
            left += sl;
            right += sr;
        }
        self.reserved = (top, bottom, left, right);
        let ev = self.body.reserve_output(0, top, bottom, left, right);
        self.brain_feed(ev);
    }

    /// Con el dock autoescondido, ajusta su visibilidad según el puntero
    /// `(px, py)`: se revela al tocar la banda del borde anclado y se oculta al
    /// salir de su franja. Devuelve `true` si el estado cambió (el backend lo
    /// usa para recomponer). No-op sin autohide o sin dock acoplado.
    fn update_shell_autohide(&mut self, px: f64, py: f64) -> bool {
        let dock = shell_dock();
        if !dock.autohide {
            return false;
        }
        let (ow, oh) = self.output_size;
        if ow == 0 || oh == 0 || !self.windows.iter().any(|w| w.is_shell) {
            return false;
        }
        let limite = if dock.anchor.es_horizontal() { oh } else { ow };
        let t = dock.thickness.clamp(1, limite.max(1));
        let next = autohide_next_hidden(
            dock.anchor,
            ow,
            oh,
            t,
            px.round() as i32,
            py.round() as i32,
            self.shell_hidden,
            SHELL_REVEAL_BAND,
        );
        if next == self.shell_hidden {
            return false;
        }
        self.shell_hidden = next;
        if let Some(w) = self.windows.iter_mut().find(|w| w.is_shell) {
            w.visible = !next;
        }
        true
    }

    /// El backend informa de un tamaño de salida nuevo (arranque o
    /// redimensión): fija el tamaño físico y, si hay shell acoplado, recalcula
    /// su franja (la reserva por insets se mantiene relativa al borde).
    fn output_changed(&mut self, width: i32, height: i32) {
        self.output_size = (width, height);
        // Mantené el Output (y su LayerMap) al día con el tamaño nuevo.
        if let Some(output) = self.output.clone() {
            output.change_current_state(
                Some(smithay::output::Mode {
                    size: (width, height).into(),
                    refresh: 60_000,
                }),
                None,
                None,
                None,
            );
            layer_map_for_output(&output).arrange();
        }
        let ev = self.body.resize_output(0, width, height);
        self.brain_feed(ev);
        if self.windows.iter().any(|w| w.is_shell) {
            self.dock_shell();
        } else {
            self.recompute_reservations();
        }
    }

    /// El traspaso del DM — la «mutación atómica». Llega el tiquet de un
    /// login válido y el compositor pasa de la pantalla de greeter a la
    /// sesión del usuario **sin reiniciar el servidor Wayland**: el mismo
    /// proceso, la misma GPU, las mismas ventanas. Idempotente — un
    /// segundo tiquet (no debería llegar) se ignora.
    fn complete_greeter_handoff(&mut self, ticket: SessionTicket) {
        if self.mode == BodyMode::Session {
            return; // ya en sesión — un tiquet de más, se ignora
        }
        println!(
            "mirada-compositor · traspaso a la sesión de «{}» (uid {}).",
            ticket.user.name, ticket.user.uid
        );
        if !nix::unistd::geteuid().is_root() {
            eprintln!(
                "mirada-compositor · aviso: no corro como root — la sesión \
                 heredará mis privilegios, sin setuid al usuario."
            );
        }
        self.mode = BodyMode::Session;
        self.session_user = Some(ticket.user.clone());

        // Ya en sesión: registra los atajos del escritorio y la decoración
        // (en modo greeter se omitieron a propósito — ver `build_app`).
        if let Brain::Embedded(desktop) = &self.brain {
            let cmds = vec![desktop.grab_keys(), desktop.decorations()];
            self.apply_commands(cmds);
        }

        // Arranca la sesión. Tres caminos:
        //  · vacío         → autostart del usuario (cliente de este compositor).
        //  · nativo (pata) → comando como cliente, sin reiniciar el servidor.
        //  · ajeno         → soltar el DRM y `exec` (otro compositor toma la
        //                    GPU). Se difiere al cierre del bucle: marcamos la
        //                    sesión pendiente y pedimos salir.
        let user = self.session_user.clone();
        // Prepara el entorno de sesión del usuario (runtime dir propio,
        // WAYLAND_DISPLAY absoluto, bus D-Bus) para que las apps nativas
        // —waybar, GTK/Qt— funcionen como en una sesión de verdad.
        if let Some(u) = &user {
            self.setup_user_session_env(u);
        }
        let env = self.session_env.clone();
        let cmd = ticket.session.trim();
        if cmd.is_empty() {
            spawn_autostart(user.as_ref(), &env);
        } else if ticket.foreign {
            println!(
                "mirada-compositor · sesión ajena «{cmd}» — cierro y cedo el DRM."
            );
            self.pending_session = Some((cmd.to_string(), user));
            self.running = false;
        } else {
            spawn_command(cmd, user.as_ref(), &env);
        }
    }

    /// Arma el entorno de sesión del usuario para las apps NATIVAS (clientes
    /// de este compositor): un `XDG_RUNTIME_DIR` propio y escribible
    /// (`/run/user/<uid>`), el `WAYLAND_DISPLAY` en ruta absoluta (el socket
    /// vive en el runtime dir del compositor, no en el del usuario) y un bus
    /// de sesión D-Bus. Sin esto, dconf no puede escribir y waybar/GTK/Qt
    /// fallan por «cannot autolaunch D-Bus».
    fn setup_user_session_env(&mut self, user: &UserInfo) {
        use std::os::unix::fs::PermissionsExt;
        let xrd = format!("/run/user/{}", user.uid);
        let _ = std::fs::create_dir_all(&xrd);
        let _ = std::fs::set_permissions(&xrd, std::fs::Permissions::from_mode(0o700));
        let _ = nix::unistd::chown(
            xrd.as_str(),
            Some(nix::unistd::Uid::from_raw(user.uid)),
            Some(nix::unistd::Gid::from_raw(user.gid)),
        );
        // El socket Wayland está en el runtime dir del COMPOSITOR (p. ej.
        // /run/mirada); WAYLAND_DISPLAY absoluto para que el cliente lo
        // encuentre aunque su XDG_RUNTIME_DIR sea otro.
        let wl = match (
            std::env::var("XDG_RUNTIME_DIR"),
            std::env::var("WAYLAND_DISPLAY"),
        ) {
            (Ok(rd), Ok(wd)) if !wd.starts_with('/') => format!("{rd}/{wd}"),
            (_, Ok(wd)) => wd,
            _ => String::new(),
        };
        let bus_path = format!("{xrd}/bus");
        let dbus_addr = format!("unix:path={bus_path}");
        self.session_env = vec![
            ("XDG_RUNTIME_DIR".to_string(), xrd),
            ("WAYLAND_DISPLAY".to_string(), wl),
            ("DBUS_SESSION_BUS_ADDRESS".to_string(), dbus_addr.clone()),
        ];
        // Levanta el bus de sesión D-Bus como el usuario, si no hay uno, y
        // espera (acotado) a que el socket exista: si lanzáramos waybar/GTK
        // antes, fallarían con «cannot autolaunch D-Bus». Es un bloqueo de
        // una sola vez al iniciar la sesión, no en el bucle de render.
        if !std::path::Path::new(&bus_path).exists() {
            let env = self.session_env.clone();
            spawn_command(
                &format!("dbus-daemon --session --address={dbus_addr} --nofork --nopidfile"),
                Some(user),
                &env,
            );
            for _ in 0..40 {
                if std::path::Path::new(&bus_path).exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            if std::path::Path::new(&bus_path).exists() {
                println!("mirada-compositor · bus D-Bus de sesión listo en {bus_path}.");
            } else {
                eprintln!(
                    "mirada-compositor · el bus D-Bus no apareció (¿dbus-daemon instalado?); las apps que lo exijan pueden fallar."
                );
            }
        }
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
                        .map(|d| d.lock().unwrap().initial_configure_sent)
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
        _output: Option<wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        // Sin salida todavía no podemos colocarlo; el cliente reintentará
        // al haber output. Mapeamos al único Output de mirada.
        let Some(output) = self.output.clone() else {
            return;
        };
        let desktop = DesktopLayerSurface::new(surface, namespace.clone());
        let mut map = layer_map_for_output(&output);
        if let Err(e) = map.map_layer(&desktop) {
            eprintln!("mirada-compositor · no pude mapear el layer surface «{namespace}»: {e:?}");
        }
        drop(map);
        self.recompute_reservations();
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let Some(output) = self.output.clone() else {
            return;
        };
        let mut map = layer_map_for_output(&output);
        if let Some(layer) = map
            .layer_for_surface(surface.wl_surface(), WindowSurfaceType::ALL)
            .cloned()
        {
            map.unmap_layer(&layer);
        }
        drop(map);
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
        // Espeja el título en la ventana gestionada (para pintar la etiqueta).
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            w.title = title.clone();
        }
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
delegate_output!(App);

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
/// modificadores y el keysym, con el mismo format que el mapa de teclas
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

/// Combos cableados que **siempre** cortan el compositor, estén o no en el
/// keymap y en cualquier modo —greeter incluido, donde los atajos del
/// escritorio no están registrados—. La red de seguridad para no quedar
/// varado: el clásico «zap» de X. Funciona igual en winit y en DRM.
pub(crate) fn is_escape_hatch(combo: &str) -> bool {
    matches!(combo, "Ctrl+Alt+BackSpace" | "Ctrl+Alt+Delete")
}

/// La VT destino de una conmutación de consola (`1` … `12`), o `None` si la
/// tecla no es de cambio de VT. Sólo lo honra el backend DRM —en winit no hay
/// VTs—. Es el comportamiento clásico para saltar entre consolas sin matar el
/// compositor.
///
/// Cubre los **dos** caminos, porque cuál llega depende del keymap activo:
/// 1. el keysym dedicado `XF86Switch_VT_n` (lo emiten los keymaps con la
///    sección `srvr_ctrl`, donde `Ctrl+Alt+Fn` ya no produce «Fn»); y
/// 2. `Ctrl+Alt+Fn` literal (keymaps base sin ese binding).
pub(crate) fn vt_target(mods: &ModifiersState, sym: Keysym) -> Option<i32> {
    let name = xkb::keysym_get_name(sym);
    // 1) Keysym dedicado: vale por sí mismo, sin exigir modificadores.
    if let Some(n) = name.strip_prefix("XF86Switch_VT_") {
        if let Ok(v) = n.parse::<i32>() {
            if (1..=12).contains(&v) {
                return Some(v);
            }
        }
    }
    // 2) Ctrl+Alt+Fn directo. Exigimos ambos modificadores para no conmutar
    //    con un F-key pelado.
    if mods.ctrl && mods.alt {
        if let Some(f) = name.strip_prefix('F') {
            if let Ok(v) = f.parse::<i32>() {
                if (1..=12).contains(&v) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Cierra el compositor y `exec`-uta una sesión ajena en su lugar, como el
/// usuario autenticado. Se llama **después** de salir del bucle y soltar el
/// DRM, así el compositor entrante (sway, Plasma…) puede tomar la GPU.
/// Reemplaza la imagen del proceso: si `exec` falla, registra y aborta.
pub(crate) fn exec_session(cmd: &str, as_user: Option<&UserInfo>) -> ! {
    use std::os::unix::process::CommandExt;
    println!("mirada-compositor · cediendo a la sesión: {cmd}");
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg(cmd).envs(THEME_ENV.iter().copied());
    if let Some(user) = as_user {
        if nix::unistd::geteuid().is_root() {
            // El compositor entrante crea su PROPIO socket Wayland, así que
            // necesita un XDG_RUNTIME_DIR suyo (no el de root, donde no
            // puede escribir) y no debe heredar nuestro WAYLAND_DISPLAY (el
            // DM ya cerró). Sin esto, Plasma/sway fallan con «could not
            // create wayland socket».
            use std::os::unix::fs::PermissionsExt;
            let xrd = format!("/run/user/{}", user.uid);
            let _ = std::fs::create_dir_all(&xrd);
            let _ = std::fs::set_permissions(&xrd, std::fs::Permissions::from_mode(0o700));
            let _ = nix::unistd::chown(
                xrd.as_str(),
                Some(nix::unistd::Uid::from_raw(user.uid)),
                Some(nix::unistd::Gid::from_raw(user.gid)),
            );
            command.env("XDG_RUNTIME_DIR", &xrd);
            command.env_remove("WAYLAND_DISPLAY");
            apply_user(&mut command, user);
        }
    }
    let err = command.exec(); // sólo retorna si falla
    eprintln!("mirada-compositor · no pude ceder a «{cmd}»: {err}");
    std::process::exit(1);
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

/// Dónde pintar una ventana. La del shell se ancla al pie de la salida
/// y crece hacia arriba (su cajón de resultados se despliega sobre las
/// ventanas). Una ventana normal va en su celda; si el cliente presenta
/// una superficie más pequeña que la celda (p. ej. un terminal que
/// redondea su tamaño a celdas de texto), se centra en el hueco.
/// Elementos de render de los layer surfaces de la salida, separados en
/// `(encima, debajo)` de las ventanas: `encima` = capas Overlay+Top,
/// `debajo` = Bottom+Background. Cada layer se pinta en la geometría que
/// el `LayerMap` le calculó (anclaje + márgenes). Coordenadas top-left,
/// igual que las ventanas. Lo comparten los backends winit y DRM.
fn layer_render_elements(
    output: Option<&Output>,
    renderer: &mut GlesRenderer,
) -> (
    Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
    Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
) {
    let mut over = Vec::new();
    let mut under = Vec::new();
    let Some(output) = output else {
        return (over, under);
    };
    let map = layer_map_for_output(output);
    for layer in map.layers() {
        let Some(geo) = map.layer_geometry(layer) else {
            continue;
        };
        let els = render_elements_from_surface_tree(
            renderer,
            layer.wl_surface(),
            (geo.loc.x, geo.loc.y),
            1.0,
            1.0,
            Kind::Unspecified,
        );
        match layer.layer() {
            Layer::Overlay | Layer::Top => over.extend(els),
            Layer::Background | Layer::Bottom => under.extend(els),
        }
    }
    (over, under)
}

/// El alto efectivo de la barra de título de `w`: `0` para el shell y las
/// ventanas a pantalla completa (no llevan), el `titlebar_height` configurado
/// para el resto. Acotado a `>= 0`.
fn titlebar_for(w: &ManagedWindow, titlebar_height: i32) -> i32 {
    if w.is_shell || w.fullscreen {
        0
    } else {
        titlebar_height.max(0)
    }
}

/// La posición de la **superficie** del cliente. `titlebar_height` reserva esa
/// franja arriba de la celda (la superficie baja por `tb`); el resto centra la
/// superficie en el área de contenido si el cliente presenta algo más chico.
fn render_loc(w: &ManagedWindow, output_h: i32, titlebar_height: i32) -> (i32, i32) {
    if w.is_shell {
        // Sólo el anclaje inferior crece hacia arriba cuando el cliente
        // presenta una superficie más alta que la franja (cajón desplegado);
        // los demás bordes usan la posición acoplada tal cual.
        if shell_dock().anchor == ShellAnchor::Bottom {
            let h = surface_px_size(w).map(|(_, h)| h).unwrap_or(shell_dock().thickness);
            return (0, output_h - h);
        }
        return w.loc;
    }
    let tb = titlebar_for(w, titlebar_height);
    let content_top = w.loc.1 + tb;
    let content_h = (w.size.1 - tb).max(1);
    match with_renderer_surface_state(&w.surface, |s| s.surface_size()) {
        Some(Some(size)) => {
            let dx = ((w.size.0 - size.w) / 2).max(0);
            let dy = ((content_h - size.h) / 2).max(0);
            (w.loc.0 + dx, content_top + dy)
        }
        _ => (w.loc.0, content_top),
    }
}

/// El tamaño en píxeles de la superficie de una ventana, si el cliente
/// ya presentó un buffer. `None` mientras no haya dibujado nada — la usa
/// el backend DRM para acertar el rectángulo en el test de impacto del
/// puntero.
fn surface_px_size(w: &ManagedWindow) -> Option<(i32, i32)> {
    with_renderer_surface_state(&w.surface, |s| s.surface_size())
        .flatten()
        .map(|s| (s.w, s.h))
}

/// El punto caliente (hotspot) de una superficie de cursor: el píxel de
/// la imagen que debe quedar bajo la posición real del puntero. `(0, 0)`
/// si el cliente no lo declaró.
fn cursor_hotspot(surface: &WlSurface) -> (i32, i32) {
    with_states(surface, |states| {
        states
            .data_map
            .get::<CursorImageSurfaceData>()
            .map(|m| {
                let h = m.lock().unwrap().hotspot;
                (h.x, h.y)
            })
            .unwrap_or((0, 0))
    })
}

/// Variables de entorno de tema que el compositor inyecta a cada hijo,
/// para uniformizar GTK y Qt:
/// - `XDG_CURRENT_DESKTOP=mirada` hace que `xdg-desktop-portal` enrute
///   hacia `mirada-portal` (el backend de `org.freedesktop.appearance`).
/// - `QT_QPA_PLATFORMTHEME=gtk3` hace que las apps Qt sigan el tema GTK,
///   y por tanto el `gtk.css` que genera `nahual-theme`.
const THEME_ENV: &[(&str, &str)] = &[
    ("XDG_CURRENT_DESKTOP", "mirada"),
    ("QT_QPA_PLATFORMTHEME", "gtk3"),
];

/// Lanza un comando como proceso hijo, vía `sh -c`. El hijo hereda el
/// entorno —`WAYLAND_DISPLAY` incluido—, así que el cliente que abra se
/// conecta a este compositor; además se le inyecta [`THEME_ENV`] para
/// que GTK y Qt adopten el tema del escritorio. Lo usan la acción
/// `spawn:…` del keymap, la variable `MIRADA_STARTUP` y el autoarranque.
///
/// `as_user`: si viene una identidad y el compositor corre como root
/// (modo DM, tras el traspaso), el hijo baja a ese usuario — ver
/// [`apply_user`]. Con `None`, o sin ser root, lanza con la identidad
/// actual del compositor.
/// Convierte una entrada de config del menú en un nodo del árbol del menú
/// raíz: hoja si no tiene `submenu`, submenú (recursivo) si lo tiene.
fn menu_node_from_entry(e: &mirada_brain::MenuEntry) -> crate::menu::MenuNode {
    if e.submenu.is_empty() {
        crate::menu::MenuNode::leaf(e.label.clone(), e.command.clone())
    } else {
        crate::menu::MenuNode::submenu(
            e.label.clone(),
            e.submenu.iter().map(menu_node_from_entry).collect(),
        )
    }
}

fn spawn_command(cmd: &str, as_user: Option<&UserInfo>, session_env: &[(String, String)]) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return;
    }
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg(cmd).envs(THEME_ENV.iter().copied());
    // Entorno de sesión (runtime dir del usuario, WAYLAND_DISPLAY absoluto,
    // bus D-Bus) — vacío para el greeter, poblado tras el traspaso.
    for (k, v) in session_env {
        command.env(k, v);
    }
    if let Some(user) = as_user {
        if nix::unistd::geteuid().is_root() {
            apply_user(&mut command, user);
        }
    }
    match command.spawn() {
        Ok(child) => println!("mirada-compositor · lanzado (pid {}): {cmd}", child.id()),
        Err(e) => eprintln!("mirada-compositor · no pude lanzar «{cmd}»: {e}"),
    }
}

/// Prepara un `Command` para que el hijo corra como `user`: fija grupos
/// suplementarios, gid, uid y una sesión propia, hace `cd` a su home e
/// inyecta las variables de identidad. Sólo se llama tras comprobar que
/// el compositor es root.
///
/// La lista de grupos se calcula **en el padre**: `getgrouplist`
/// consulta NSS (abre `/etc/group`), y eso no es seguro entre `fork` y
/// `exec`; en `pre_exec` quedan sólo syscalls async-signal-safe.
fn apply_user(command: &mut std::process::Command, user: &UserInfo) {
    use nix::unistd::{setgid, setgroups, setuid, Gid, Uid};
    use std::os::unix::process::CommandExt;

    let uid = Uid::from_raw(user.uid);
    let gid = Gid::from_raw(user.gid);
    let groups: Vec<Gid> = std::ffi::CString::new(user.name.as_bytes())
        .ok()
        .and_then(|name| nix::unistd::getgrouplist(&name, gid).ok())
        .unwrap_or_else(|| vec![gid]);

    command
        .env("HOME", &user.home)
        .env("USER", &user.name)
        .env("LOGNAME", &user.name)
        .env("SHELL", &user.shell)
        .current_dir(&user.home);

    // SAFETY: corre en el hijo, entre `fork` y `exec`. Sólo syscalls
    // async-signal-safe. El orden es obligatorio: grupos y gid ANTES que
    // uid — al rebajar el uid se pierde el privilegio para fijarlos.
    unsafe {
        command.pre_exec(move || {
            setgroups(&groups)?;
            setgid(gid)?;
            setuid(uid)?;
            let _ = nix::unistd::setsid(); // sesión propia; no es crítico
            Ok(())
        });
    }
}

/// La ruta del archivo de autoarranque, `…/mirada/autostart` — junto al
/// keymap y las reglas. Con un usuario (tras el traspaso del DM) se
/// resuelve bajo su home; sin él, bajo la config del proceso actual.
fn autostart_path(user: Option<&UserInfo>) -> Option<std::path::PathBuf> {
    match user {
        Some(u) => Some(u.home.join(".config/mirada/autostart")),
        None => Keymap::default_path().and_then(|p| p.parent().map(|d| d.join("autostart"))),
    }
}

/// Lanza los programas del archivo de autoarranque: un comando por
/// línea, `#` comenta y las líneas en blanco se saltan. Sin archivo, no
/// hace nada. Se llama una vez al arrancar (o tras el traspaso del DM),
/// con el socket ya abierto. `as_user` se propaga a [`spawn_command`].
fn spawn_autostart(as_user: Option<&UserInfo>, session_env: &[(String, String)]) {
    let text = autostart_path(as_user)
        .and_then(|path| std::fs::read_to_string(&path).ok())
        .unwrap_or_default();
    let mut n = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        spawn_command(line, as_user, session_env);
        n += 1;
    }
    if n > 0 {
        println!("mirada-compositor · autoarranque: {n} programa(s).");
    } else {
        // Sin autostart: en vez de un escritorio negro y vacío, levanta el
        // marco pata para que haya algo usable de entrada.
        println!("mirada-compositor · sin autoarranque — levanto el marco pata.");
        spawn_command("pata-llimphi", as_user, session_env);
    }
}

/// Nombre o ruta del binario del greeter. `MIRADA_GREETER_BIN` lo
/// sobreescribe — cómodo en desarrollo para apuntar a `target/…`.
fn greeter_bin() -> String {
    std::env::var("MIRADA_GREETER_BIN").unwrap_or_else(|_| "mirada-greeter".to_string())
}

/// Lanza `mirada-greeter` como proceso hijo, en modo DM, con el stdout
/// capturado. Un hilo lee sus líneas: la que sea un [`SessionTicket`] se
/// entrega por `send` (el bucle de eventos hará el traspaso); el resto
/// del stdout se reenvía a la consola con el prefijo `greeter ·`. El
/// hilo es dueño del `Child` y lo cosecha cuando el greeter termina.
fn spawn_greeter<S>(send: S) -> std::io::Result<()>
where
    S: Fn(SessionTicket) + Send + 'static,
{
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let mut child = Command::new(greeter_bin())
        .envs(THEME_ENV.iter().copied())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("stdout pedido con Stdio::piped");
    println!("mirada-compositor · greeter lanzado (pid {}).", child.id());

    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            match SessionTicket::from_line(&line) {
                Some(ticket) => {
                    println!("mirada-compositor · tiquet de sesión recibido del greeter.");
                    send(ticket);
                }
                None => println!("greeter · {line}"),
            }
        }
        match child.wait() {
            Ok(status) => println!("mirada-compositor · el greeter terminó ({status})."),
            Err(e) => eprintln!("mirada-compositor · wait(greeter): {e}"),
        }
    });
    Ok(())
}

/// Carga las reglas de ventana del usuario, o ninguna si no hay archivo.
fn load_user_rules() -> Rules {
    match Rules::default_path() {
        Some(p) => Rules::load_or_default(&p),
        None => Rules::default(),
    }
}

/// Carga la config general del WM (`~/.config/mirada/config.ron`), o los
/// valores por defecto si no hay archivo.
fn load_user_config() -> mirada_brain::Config {
    match mirada_brain::Config::default_path() {
        Some(p) => mirada_brain::Config::load_or_default(&p),
        None => mirada_brain::Config::default(),
    }
}

/// Arma un Cerebro embebido: un `Desktop` con el keymap del usuario y
/// sus reglas de ventana. Lo usan tanto el modo autónomo como el modo
/// greeter (el DM es siempre autónomo — un Cerebro externo no tiene
/// sentido en la pantalla de login).
fn embedded_brain(keymap_path: &Option<std::path::PathBuf>) -> Brain {
    let keymap = match keymap_path {
        Some(p) => Keymap::load_or_init(p),
        None => Keymap::default(),
    };
    let mut desktop = Desktop::with_keymap(keymap);
    desktop.set_config(load_user_config());
    desktop.set_rules(load_user_rules());
    Brain::Embedded(desktop)
}

/// Crea y anuncia un `wl_output` (un monitor) en el protocolo Wayland —
/// muchos clientes (`foot` entre ellos) se niegan a arrancar sin uno.
/// Devuelve el [`Output`](smithay::output::Output); hay que mantenerlo
/// vivo mientras el compositor corra.
fn announce_output(
    dh: &DisplayHandle,
    name: &str,
    width: i32,
    height: i32,
    refresh_mhz: i32,
) -> smithay::output::Output {
    use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
    let output = Output::new(
        name.to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "mirada".into(),
            model: name.to_string(),
        },
    );
    output.create_global::<App>(dh);
    let mode = Mode { size: (width, height).into(), refresh: refresh_mhz };
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        Some(Scale::Integer(1)),
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    output
}

/// Anuncia el global `zwp_linux_dmabuf` con los formatos que el
/// `GlesRenderer` admite. Hay que llamarlo una vez creado el renderer
/// (no antes: los formatos salen de él) — así las apps que pintan por
/// GPU (GPUI, navegadores acelerados) pueden ser clientes del compositor.
fn announce_dmabuf(app: &mut App, dh: &DisplayHandle, renderer: &GlesRenderer) {
    let formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();
    // Nodo de render del adaptador del renderer — necesario para armar el
    // *default feedback* de dmabuf v4.
    let render_node = EGLDevice::device_for_display(renderer.egl_context().display())
        .ok()
        .and_then(|dev| dev.try_get_render_node().ok().flatten());
    let feedback = render_node.and_then(|node| {
        DmabufFeedbackBuilder::new(node.dev_id(), formats.clone())
            .build()
            .ok()
    });
    match feedback {
        // dmabuf **v4 con default feedback**. La WSI Vulkan de Mesa lo EXIGE para
        // determinar el dispositivo y los formatos presentables: con sólo el
        // global v3 (sin feedback) los clientes wgpu/Vulkan ven **0 formatos** y
        // no pueden crear swapchain (era el bug de `pata` por layer-shell, que
        // caía a winit y paniqueaba). EGL/GL y los búferes shm (waybar) andaban
        // con v3; el path Vulkan WSI no. Clientes que se bindean a v3 siguen
        // recibiendo los formatos de la tranche principal.
        Some(feedback) => {
            app.dmabuf_state
                .create_global_with_default_feedback::<App>(dh, &feedback);
            println!(
                "mirada-compositor · dmabuf v4 (feedback): {} format(s) anunciado(s).",
                formats.len()
            );
        }
        // Sin nodo de render no se puede armar feedback: caemos al global v3.
        None => {
            let n = formats.len();
            app.dmabuf_state.create_global::<App>(dh, formats);
            eprintln!(
                "mirada-compositor · dmabuf v3 sin feedback ({n} fmt) — sin nodo de render; \
                 los clientes Vulkan podrían ver 0 formatos."
            );
        }
    }
}

/// Vigías de los tres archivos de config recargables en caliente (keymap,
/// config, reglas). Cada uno es `(ruta, vigía)` o `None` si no aplica
/// (Cerebro enlazado, modo greeter o fallo al armar el watcher). Un solo
/// [`poll`](ConfigWatches::poll) atiende los tres — sin duplicar la lógica
/// entre el backend winit y el DRM.
#[derive(Default)]
struct ConfigWatches {
    keymap: Option<(std::path::PathBuf, mirada_brain::FileWatch)>,
    config: Option<(std::path::PathBuf, mirada_brain::FileWatch)>,
    rules: Option<(std::path::PathBuf, mirada_brain::FileWatch)>,
}

impl ConfigWatches {
    /// Recarga lo que haya cambiado en disco. Llamar una vez por iteración
    /// del bucle de eventos de cada backend. Devuelve `true` si la **config**
    /// general (`config.ron`) cambió — el backend DRM lo usa para refrescar sus
    /// cachés derivadas de config (menú, wallpaper, fuente).
    fn poll(&self, app: &mut App) -> bool {
        if let Some((p, w)) = &self.keymap {
            if w.changed() {
                app.reload_keymap_from(p);
            }
        }
        let mut config_changed = false;
        if let Some((p, w)) = &self.config {
            if w.changed() {
                app.reload_config_from(p);
                config_changed = true;
            }
        }
        if let Some((p, w)) = &self.rules {
            if w.changed() {
                app.reload_rules_from(p);
            }
        }
        config_changed
    }
}

/// Lo que comparten los dos backends gráficos: el `Display` de Wayland,
/// el `App` ya armado y la maquinaria de recarga en caliente y control.
struct Setup {
    display: Display<App>,
    app: App,
    watches: ConfigWatches,
    ctl: Option<CtlServer>,
}

/// Arma el estado del compositor — todo lo independiente del backend
/// gráfico (Wayland, Cerebro, teclado, keymap, control). Cada backend
/// (winit o DRM) registra luego su propia salida y monta su bucle.
fn build_app(greeter: bool) -> Result<Setup, Box<dyn std::error::Error>> {
    let display: Display<App> = Display::new()?;
    let dh = display.handle();

    let mut seat_state = SeatState::new();
    let seat = seat_state.new_wl_seat(&dh, "mirada");

    // Anuncia el gestor de decoración: las ventanas van sin marco (ver
    // `XdgDecorationHandler`). El `XdgDecorationState` sólo serviría para
    // retirar el global más tarde, cosa que nunca hacemos.
    let _ = XdgDecorationState::new::<App>(&dh);

    // El keymap del usuario (`~/.config/mirada/keymap.ron`). Sólo lo usa
    // el Cerebro embebido; con un Cerebro enlazado, el keymap es asunto suyo.
    let keymap_path = Keymap::default_path();

    // Elige el Cerebro. El modo greeter (DM) fuerza Cerebro embebido;
    // si no, enlazado cuando `MIRADA_SOCKET` está puesto, autónomo si no.
    let brain = if greeter {
        println!("mirada-compositor · modo greeter (DM) — Cerebro embebido.");
        embedded_brain(&keymap_path)
    } else {
        match std::env::var("MIRADA_SOCKET") {
            Ok(path) => {
                println!("mirada-compositor · esperando al Cerebro en {path} …");
                let link = BodyLink::listen(&path)?;
                println!("mirada-compositor · Cerebro conectado.");
                Brain::Linked(link)
            }
            Err(_) => {
                println!("mirada-compositor · modo autónomo (Cerebro embebido).");
                embedded_brain(&keymap_path)
            }
        }
    };

    let mut app = App {
        compositor_state: CompositorState::new::<App>(&dh),
        xdg_shell_state: XdgShellState::new::<App>(&dh),
        layer_shell_state: WlrLayerShellState::new::<App>(&dh),
        output_manager_state: OutputManagerState::new_with_xdg_output::<App>(&dh),
        output: None,
        shm_state: ShmState::new::<App>(&dh, Vec::new()),
        dmabuf_state: DmabufState::new(),
        seat_state,
        data_device_state: DataDeviceState::new::<App>(&dh),
        data_control_state: DataControlState::new::<App, _>(&dh, None, |_| true),
        seat,
        keyboard: None,
        pointer: None,
        pointer_loc: (0.0, 0.0),
        cursor_status: CursorImageStatus::default_named(),
        drag: None,
        output_size: (0, 0),
        // Con autohide, el dock arranca oculto (se revela al tocar el borde).
        shell_hidden: shell_dock().autohide,
        reserved: (0, 0, 0, 0),
        windows: Vec::new(),
        body: BodyState::new(),
        brain,
        mode: if greeter { BodyMode::Greeter } else { BodyMode::Session },
        session_user: None,
        session_env: Vec::new(),
        grabs: Vec::new(),
        decorations: mirada_brain::Decorations::default(),
        pending_keybind: None,
        pending_vt: None,
        pending_session: None,
        next_id: 1,
        running: true,
    };

    let keyboard = app.seat.add_keyboard(Default::default(), 200, 25)?;
    app.keyboard = Some(keyboard);
    app.pointer = Some(app.seat.add_pointer());

    // En modo embebido, el propio Desktop dicta los atajos a
    // interceptar — salvo en modo greeter: en la pantalla de login
    // todas las teclas van al greeter (que el usuario no pueda lanzar
    // nada ni cerrar el compositor). Los atajos se registran luego, en
    // el traspaso a la sesión (`complete_greeter_handoff`).
    if !greeter {
        if let Brain::Embedded(desktop) = &app.brain {
            let cmds = vec![desktop.grab_keys(), desktop.decorations()];
            app.apply_commands(cmds);
        }
    }

    // Vigilancia de los archivos de config (keymap, config, reglas) para
    // recargarlos en caliente — sólo con el Cerebro embebido y fuera del
    // modo greeter (donde no hay nada registrado que recargar). Cada vigía
    // empareja la ruta con su `FileWatch`; un fallo al armarlo deja `None`.
    let watches = if matches!(app.brain, Brain::Embedded(_)) && !greeter {
        let watch_pair = |p: &Option<std::path::PathBuf>| {
            p.as_ref()
                .and_then(|p| mirada_brain::FileWatch::new(p).ok().map(|w| (p.clone(), w)))
        };
        let w = ConfigWatches {
            keymap: watch_pair(&keymap_path),
            config: watch_pair(&mirada_brain::Config::default_path()),
            rules: watch_pair(&Rules::default_path()),
        };
        let n = [&w.keymap, &w.config, &w.rules].iter().filter(|x| x.is_some()).count();
        if n > 0 {
            println!("mirada-compositor · vigilando {n} archivo(s) de config (recarga en caliente).");
        }
        w
    } else {
        ConfigWatches::default()
    };

    // API de control (mirada-ctl) — sólo con el Cerebro embebido; si es
    // externo, el socket de control lo abre él.
    let ctl = match &app.brain {
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

    Ok(Setup { display, app, watches, ctl })
}

/// El backend `winit`: corre anidado dentro de una sesión gráfica.
fn run_winit(greeter: bool) -> Result<(), Box<dyn std::error::Error>> {
    let Setup {
        mut display,
        app: mut state,
        watches,
        ctl,
    } = build_app(greeter)?;
    let keyboard = state.keyboard.clone().expect("teclado inicializado");

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

    // Con el renderer ya creado, anuncia dmabuf (clientes con GPU).
    announce_dmabuf(&mut state, &display.handle(), backend.renderer());

    // Salida inicial = el tamaño de la ventana winit.
    let win_size = backend.window_size();
    state.output = Some(announce_output(
        &display.handle(),
        "winit",
        win_size.w,
        win_size.h,
        60_000,
    ));
    {
        let ev = state.body.add_output(0, win_size.w, win_size.h);
        state.brain_feed(ev);
        state.output_size = (win_size.w, win_size.h);
    }

    // Modo greeter (DM anidado — útil para iterar la UI del login):
    // lanza el greeter y recibe su tiquet por un canal que el bucle sondea.
    let greeter_rx = if state.mode == BodyMode::Greeter {
        let (tx, rx) = std::sync::mpsc::channel::<SessionTicket>();
        spawn_greeter(move |ticket| {
            let _ = tx.send(ticket);
        })?;
        Some(rx)
    } else {
        None
    };

    while state.running {
        // 1 · Eventos del backend (teclado, redimensión, cierre).
        let status = winit.dispatch_new_events(|event| match event {
            WinitEvent::CloseRequested => state.running = false,
            WinitEvent::Resized { size, .. } => {
                state.output_changed(size.w, size.h);
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
                            if is_escape_hatch(&combo) {
                                eprintln!("mirada-compositor · salida de emergencia ({combo}).");
                                st.running = false;
                                return FilterResult::Intercept(());
                            }
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

        // 2 bis · El tiquet del greeter (modo DM): dispara el traspaso.
        if let Some(rx) = &greeter_rx {
            while let Ok(ticket) = rx.try_recv() {
                state.complete_greeter_handoff(ticket);
            }
        }

        // 2 ter · Recarga en caliente de keymap/config/reglas si cambiaron.
        // (El backend winit anidado no cachea menú/wallpaper/fuente, así que
        // ignora si la config cambió — sólo importa en el backend DRM.)
        let _ = watches.poll(&mut state);

        // 2 quater · Peticiones del API de control (mirada-ctl).
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
            // (índice 0 = encima): el shell primero —va sobre todo—, luego
            // las flotantes, luego las teseladas. `sort_by_key` es estable:
            // dentro de cada grupo se respeta el orden de apertura.
            let output_h = state.output_size.1;
            // Layer surfaces (waybar, swaybg…): overlay/top van ENCIMA de
            // las ventanas, bottom/background DEBAJO. La lista es front-to-back.
            let (over_layers, under_layers) =
                layer_render_elements(state.output.as_ref(), renderer);
            let mut shown: Vec<&ManagedWindow> =
                state.windows.iter().filter(|w| w.visible).collect();
            shown.sort_by_key(|w| (!w.is_shell, !w.floating));
            // El backend winit anidado no pinta decoración; pasa el alto de
            // barra para que la superficie quede donde el DRM la pondría.
            let tbh = state.decorations.titlebar_height;
            let window_elems = shown.iter().flat_map(|w| {
                render_elements_from_surface_tree(
                    renderer,
                    &w.surface,
                    render_loc(w, output_h, tbh),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                )
            });
            let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = over_layers
                .into_iter()
                .chain(window_elems)
                .chain(under_layers)
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
        if let Some(output) = state.output.clone() {
            for layer in layer_map_for_output(&output).layers() {
                send_frames_surface_tree(layer.wl_surface(), time);
            }
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

    // Sesión ajena pendiente (handoff por `exec`): en anidado no hay DRM
    // que ceder, pero soltamos la ventana del host y cedemos igual.
    if let Some((cmd, user)) = state.pending_session.take() {
        drop(backend);
        exec_session(&cmd, user.as_ref());
    }

    println!("mirada-compositor · adiós.");
    Ok(())
}

fn main() {
    // Banderas en cualquier orden: `--greeter` (modo DM) es ortogonal
    // al backend (`--winit` anidado · `--drm` nativo · auto si falta).
    let args: Vec<String> = std::env::args().skip(1).collect();
    for a in &args {
        if !matches!(a.as_str(), "--greeter" | "--winit" | "--drm") {
            eprintln!(
                "mirada-compositor: opción desconocida «{a}» — usa --greeter, --winit o --drm"
            );
            std::process::exit(2);
        }
    }
    let greeter = args.iter().any(|a| a == "--greeter");
    let backend = args.iter().find(|a| matches!(a.as_str(), "--winit" | "--drm"));

    let result = match backend.map(String::as_str) {
        Some("--drm") => drm_backend::run(greeter),
        Some("--winit") => run_winit(greeter),
        _ => {
            // Auto: con sesión gráfica anfitriona → winit (anidado);
            // sin ella (una TTY pelada) → backend DRM.
            let nested = std::env::var_os("WAYLAND_DISPLAY").is_some()
                || std::env::var_os("DISPLAY").is_some();
            if nested {
                println!("mirada-compositor · sesión gráfica detectada → backend winit.");
                run_winit(greeter)
            } else {
                println!("mirada-compositor · sin sesión gráfica → backend DRM.");
                drm_backend::run(greeter)
            }
        }
    };
    if let Err(e) = result {
        eprintln!("mirada-compositor · error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vt_switch_cubre_fn_y_keysym_dedicado() {
        let ctrl_alt = ModifiersState {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        let none = ModifiersState::default();
        // Ctrl+Alt+F3 → VT3.
        assert_eq!(vt_target(&ctrl_alt, Keysym::new(xkb::keysyms::KEY_F3)), Some(3));
        // F3 sin modificadores no conmuta.
        assert_eq!(vt_target(&none, Keysym::new(xkb::keysyms::KEY_F3)), None);
        // El keysym dedicado conmuta por sí mismo (keymaps con srvr_ctrl).
        assert_eq!(
            vt_target(&none, Keysym::new(xkb::keysyms::KEY_XF86Switch_VT_5)),
            Some(5)
        );
        // Otras teclas y F-keys fuera de rango → None.
        assert_eq!(vt_target(&ctrl_alt, Keysym::new(xkb::keysyms::KEY_a)), None);
        assert_eq!(vt_target(&ctrl_alt, Keysym::new(xkb::keysyms::KEY_F13)), None);
    }

    #[test]
    fn anchor_parse_y_default() {
        assert_eq!(ShellAnchor::parse("top"), ShellAnchor::Top);
        assert_eq!(ShellAnchor::parse("LEFT"), ShellAnchor::Left);
        assert_eq!(ShellAnchor::parse("right"), ShellAnchor::Right);
        // desconocido o vacío → bottom.
        assert_eq!(ShellAnchor::parse("xyz"), ShellAnchor::Bottom);
        assert_eq!(ShellAnchor::parse(""), ShellAnchor::Bottom);
    }

    #[test]
    fn anchor_horizontalidad() {
        assert!(ShellAnchor::Top.es_horizontal());
        assert!(ShellAnchor::Bottom.es_horizontal());
        assert!(!ShellAnchor::Left.es_horizontal());
        assert!(!ShellAnchor::Right.es_horizontal());
    }

    #[test]
    fn franja_del_shell_por_borde() {
        // Salida 1920×1080, grosor 40.
        assert_eq!(shell_strip(ShellAnchor::Top, 1920, 1080, 40), (0, 0, 1920, 40));
        assert_eq!(
            shell_strip(ShellAnchor::Bottom, 1920, 1080, 40),
            (0, 1040, 1920, 40)
        );
        assert_eq!(shell_strip(ShellAnchor::Left, 1920, 1080, 40), (0, 0, 40, 1080));
        assert_eq!(
            shell_strip(ShellAnchor::Right, 1920, 1080, 40),
            (1880, 0, 40, 1080)
        );
    }

    #[test]
    fn insets_reservan_la_zona_del_borde_correcto() {
        // (top, bottom, left, right) — sólo el borde anclado lleva el grosor.
        assert_eq!(shell_insets(ShellAnchor::Top, 40), (40, 0, 0, 0));
        assert_eq!(shell_insets(ShellAnchor::Bottom, 40), (0, 40, 0, 0));
        assert_eq!(shell_insets(ShellAnchor::Left, 48), (0, 0, 48, 0));
        assert_eq!(shell_insets(ShellAnchor::Right, 48), (0, 0, 0, 48));
    }

    #[test]
    fn autohide_bottom_revela_en_el_borde_y_oculta_al_salir() {
        let (ow, oh, t, b) = (800, 600, 40, SHELL_REVEAL_BAND);
        // Oculto: sólo tocar la banda del borde inferior revela.
        assert!(!autohide_next_hidden(ShellAnchor::Bottom, ow, oh, t, 400, 599, true, b));
        assert!(autohide_next_hidden(ShellAnchor::Bottom, ow, oh, t, 400, 300, true, b));
        // Visible: se mantiene sobre la franja (y∈[560,600)), se oculta al salir.
        assert!(!autohide_next_hidden(ShellAnchor::Bottom, ow, oh, t, 400, 570, false, b));
        assert!(autohide_next_hidden(ShellAnchor::Bottom, ow, oh, t, 400, 500, false, b));
    }

    #[test]
    fn autohide_top_usa_el_borde_superior() {
        let (ow, oh, t, b) = (800, 600, 30, SHELL_REVEAL_BAND);
        assert!(!autohide_next_hidden(ShellAnchor::Top, ow, oh, t, 400, 1, true, b));
        assert!(autohide_next_hidden(ShellAnchor::Top, ow, oh, t, 400, 200, true, b));
        assert!(!autohide_next_hidden(ShellAnchor::Top, ow, oh, t, 400, 10, false, b));
        assert!(autohide_next_hidden(ShellAnchor::Top, ow, oh, t, 400, 100, false, b));
    }

    #[test]
    fn banda_de_revelado_pegada_al_borde() {
        // Bottom: 3px abajo, a todo el ancho.
        assert_eq!(shell_reveal_band(ShellAnchor::Bottom, 800, 600, 40, 3), (0, 597, 800, 3));
        // Right: 3px a la derecha, a todo el alto.
        assert_eq!(shell_reveal_band(ShellAnchor::Right, 800, 600, 40, 3), (797, 0, 3, 600));
    }
}
