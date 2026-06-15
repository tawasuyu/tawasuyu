// Tipos y estado del compositor — se re-exportan desde la raíz del crate.

use std::sync::Arc;
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::{CursorImageStatus, PointerHandle};
use smithay::input::{Seat, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::output::Output;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::dmabuf::{DmabufState};
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListState};
use smithay::wayland::selection::wlr_data_control::DataControlState;
use smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState;
use smithay::wayland::shell::xdg::{XdgShellState};
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shm::ShmState;
use auth_core::UserInfo;
use mirada_body::BodyState;
use mirada_brain::{BrainCommand, Desktop, Decorations, Permisos};
use mirada_link::BodyLink;
use crate::screencopy;

/// De dónde salen las decisiones de geometría.
pub(crate) enum Brain {
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
pub(crate) enum BodyMode {
    /// Pantalla de login: el único cliente es el greeter, no se
    /// registran atajos, se rechaza `Spawn` y no hay autoarranque.
    Greeter,
    /// Sesión de usuario: el compositor funciona con normalidad.
    Session,
}

/// Grosor por defecto de la franja del shell (px), si el entorno no lo fija.
pub(crate) const SHELL_DOCK_DEFAULT: i32 = 40;

/// El borde de la salida al que se acopla la franja del shell (el marco `pata`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShellAnchor {
    Top,
    Bottom,
    Left,
    Right,
}

impl ShellAnchor {
    /// Parsea el valor de `MIRADA_SHELL_ANCHOR`; cae a `Bottom` si no calza.
    pub(crate) fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "top" => Self::Top,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => Self::Bottom,
        }
    }

    /// `true` para los bordes horizontales (top/bottom): su grosor es alto.
    pub(crate) fn es_horizontal(&self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// Config del acople del shell, resuelta una vez desde el entorno:
/// - `MIRADA_SHELL_APP_ID` — identidad de la ventana-marco (default `tawasuyu.pata`).
/// - `MIRADA_SHELL_ANCHOR` — borde (`top`/`bottom`/`left`/`right`, default `bottom`).
/// - `MIRADA_SHELL_THICKNESS` — grosor en px (default `40`).
/// - `MIRADA_SHELL_AUTOHIDE` — `1`/`true` para autoesconder el dock: nunca
///   reserva su franja (las ventanas usan toda la pantalla) y sólo se muestra,
///   superpuesto, al acercar el puntero al borde anclado.
pub(crate) struct ShellDock {
    pub(crate) app_id: String,
    pub(crate) anchor: ShellAnchor,
    pub(crate) thickness: i32,
    pub(crate) autohide: bool,
}

/// Banda fina (px) del borde anclado que revela el dock autoescondido, y
/// grosor de la sutil franja-pista que se pinta mientras está oculto.
pub(crate) const SHELL_REVEAL_BAND: i32 = 3;

/// La config del shell, leída del entorno la primera vez que se consulta.
pub(crate) fn shell_dock() -> &'static ShellDock {
    static DOCK: std::sync::OnceLock<ShellDock> = std::sync::OnceLock::new();
    DOCK.get_or_init(|| {
        let app_id =
            std::env::var("MIRADA_SHELL_APP_ID").unwrap_or_else(|_| "tawasuyu.pata".to_string());
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
/// override de `MIRADA_SHELL_APP_ID`), o el alias legacy `mirada.shell`.
pub(crate) fn is_shell_app_id(app_id: &str) -> bool {
    app_id == shell_dock().app_id || app_id == "mirada.shell"
}

/// El rect `(x, y, w, h)` de la franja del shell sobre una salida `ow×oh` con
/// grosor `t`, según el borde. Pura — fácil de testear.
pub(crate) fn shell_strip(anchor: ShellAnchor, ow: i32, oh: i32, t: i32) -> (i32, i32, i32, i32) {
    match anchor {
        ShellAnchor::Top => (0, 0, ow, t),
        ShellAnchor::Bottom => (0, oh - t, ow, t),
        ShellAnchor::Left => (0, 0, t, oh),
        ShellAnchor::Right => (ow - t, 0, t, oh),
    }
}

/// Las zonas exclusivas `(top, bottom, left, right)` que reserva una franja de
/// grosor `t` en el borde `anchor` — lo que el teselado debe esquivar. Pura.
pub(crate) fn shell_insets(anchor: ShellAnchor, t: i32) -> (i32, i32, i32, i32) {
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
pub(crate) fn shell_reveal_band(anchor: ShellAnchor, ow: i32, oh: i32, t: i32, band: i32) -> (i32, i32, i32, i32) {
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
pub(crate) fn autohide_next_hidden(
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
pub(crate) struct ManagedWindow {
    pub(crate) id: u64,
    pub(crate) toplevel: smithay::wayland::shell::xdg::ToplevelSurface,
    pub(crate) surface: WlSurface,
    /// Esquina superior-izquierda de la celda asignada, según el Cerebro.
    pub(crate) loc: (i32, i32),
    /// Tamaño de la celda asignada — para centrar la ventana si el
    /// cliente presenta una superficie más pequeña.
    pub(crate) size: (i32, i32),
    pub(crate) visible: bool,
    /// `true` si flota: se compone por encima de las teseladas.
    pub(crate) floating: bool,
    /// `true` si tiene el foco del teclado — pinta el marco resaltado.
    pub(crate) focused: bool,
    /// `true` si es la ventana del shell — acoplada al pie, sin teselar.
    pub(crate) is_shell: bool,
    /// `true` si está a pantalla completa — no lleva barra de título ni marco.
    pub(crate) fullscreen: bool,
    /// `true` si duerme tras una capa de zoom: no se le envían frame
    /// callbacks (el cliente queda inerte) además de quedar oculta.
    pub(crate) suspended: bool,
    /// Divisor de frames: se le envía 1 de cada `frame_divisor` frame callbacks
    /// (1 = pleno ritmo). El throttle de fondo del Cerebro lo sube para las
    /// ventanas visibles sin foco.
    pub(crate) frame_divisor: u32,
    /// Contador de vblanks para el throttle: avanza cada frame; el callback se
    /// envía sólo cuando `frame_tick % frame_divisor == 0`.
    pub(crate) frame_tick: u32,
    /// Título del cliente — para pintar la etiqueta (barra de título).
    /// Se actualiza en `title_changed`.
    pub(crate) title: String,
    /// Handle en el censo `ext_foreign_toplevel_list` — espeja título y
    /// `app_id` hacia los clientes autorizados. `None` para la ventana del
    /// shell (el marco no es una ventana del usuario).
    pub(crate) foreign_handle: Option<ForeignToplevelHandle>,
    /// Búferes de los 4 lados del marco (arriba, abajo, izq., der.) —
    /// cada uno con su `Id` estable para el seguimiento de daño.
    pub(crate) borders: [SolidColorBuffer; 4],
}

/// Un arrastre de ratón en curso: mueve o redimensiona una ventana.
pub(crate) struct DragGrab {
    /// La ventana que se arrastra.
    pub(crate) id: u64,
    /// Mover (`Super`+botón izquierdo) o redimensionar (`Super`+derecho).
    pub(crate) mode: DragMode,
    /// Posición del puntero al empezar el arrastre.
    pub(crate) start_pointer: (f64, f64),
    /// Rectángulo `(x, y, w, h)` de la ventana al empezar.
    pub(crate) start_rect: (i32, i32, i32, i32),
}

/// Qué le hace un arrastre a la ventana.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragMode {
    /// Reubicar una ventana **flotante** — la esquina la sigue al puntero.
    Move,
    /// Redimensionarla — la esquina inferior-derecha sigue al puntero.
    Resize,
    /// Reordenar una ventana **teselada**: la intercambia con la tesela
    /// bajo el puntero (el Cerebro decide el swap), sin sacarla del teselado.
    Tile,
}

/// El estado global del compositor.
pub(crate) struct App {
    pub(crate) compositor_state: CompositorState,
    pub(crate) xdg_shell_state: XdgShellState,
    pub(crate) shm_state: ShmState,
    /// Estado de `zwp_linux_dmabuf` — deja que los clientes con GPU
    /// (apps GPUI, navegadores acelerados) compartan búferes de vídeo.
    pub(crate) dmabuf_state: DmabufState,
    pub(crate) seat_state: SeatState<App>,
    pub(crate) data_device_state: DataDeviceState,
    /// Estado de `zwlr_data_control_manager_v1` — lectura/escritura del
    /// portapapeles SIN robar foco. Sin esto, `wl-paste` (el widget `clipboard`
    /// de pata lo corre ~1Hz) caía a su fallback: crear una surface de tamaño 0,
    /// robar el foco de teclado para leer la selección, y destruirla — titilando
    /// el foco cada segundo. También lo usan cliphist y los gestores de
    /// portapapeles.
    pub(crate) data_control_state: DataControlState,
    /// Estado de `zwp_virtual_keyboard_manager_v1` — inyección de pulsaciones
    /// sintéticas (teclados en pantalla, `wtype`, automatización, el
    /// asistente NL→input). El global se crea con un filtro por ejecutable
    /// (espejo del de `data_control`): los clientes en `virtual_input_denylist`
    /// no lo ven. Se guarda para mantenerlo vivo durante toda la sesión.
    pub(crate) _virtual_keyboard_state: VirtualKeyboardManagerState,
    /// Estado de `ext_foreign_toplevel_list_v1` — el censo de ventanas
    /// (título + `app_id` de todo lo abierto) para taskbars, docks y switchers.
    /// El global se crea con un filtro por ejecutable (espejo de los otros
    /// dos): los clientes en `window_list_denylist` no lo ven.
    pub(crate) foreign_toplevel_state: ForeignToplevelListState,
    /// Estado de `zwlr_screencopy_v1` — captura de pantalla (implementado a
    /// mano en [`screencopy`]; smithay 0.7 no lo trae). El global se crea con
    /// un filtro por ejecutable: los clientes en `screencopy_denylist` no lo
    /// ven. Se guarda para mantenerlo vivo durante toda la sesión.
    pub(crate) _screencopy_state: screencopy::ScreencopyState,
    /// Capturas screencopy aceptadas, esperando la próxima composición de su
    /// salida — el backend las drena con [`screencopy::tomar_capturas`].
    pub(crate) pending_screencopy: Vec<screencopy::PendingScreencopy>,
    pub(crate) seat: Seat<App>,
    /// Estado del protocolo `wlr-layer-shell` (barras/fondos/overlays como
    /// waybar, swaybg, wofi, mako).
    pub(crate) layer_shell_state: WlrLayerShellState,
    /// La salida **primaria** — la necesita `layer_map_for_output` para
    /// arreglar anclajes y zonas exclusivas de los layer surfaces que el
    /// cliente no ate a un output específico (cae al primario).
    pub(crate) output: Option<Output>,
    /// Todas las salidas activas (la primaria es `outputs[0]`). El compositor
    /// las publica acá tras armarlas, así un layer surface con `output_hint`
    /// puede mapearse al monitor que el cliente pidió, no siempre al primario.
    pub(crate) outputs: Vec<Output>,
    /// Gestor de salidas con `xdg-output` (`zxdg_output_manager_v1`): waybar
    /// y otras barras lo exigen para conocer nombre/geometría de las salidas.
    /// Se conserva sólo para mantener vivo el global (de ahí el `allow`).
    #[allow(dead_code)]
    pub(crate) output_manager_state: OutputManagerState,
    pub(crate) keyboard: Option<KeyboardHandle<App>>,
    pub(crate) pointer: Option<PointerHandle<App>>,
    /// Posición del puntero en coordenadas globales.
    pub(crate) pointer_loc: (f64, f64),
    /// Qué cursor pide el cliente enfocado — una superficie suya, un
    /// cursor con nombre, u oculto. El backend lo pinta en consecuencia.
    pub(crate) cursor_status: CursorImageStatus,
    /// Arrastre de ventana en curso (mover o redimensionar con el ratón).
    pub(crate) drag: Option<DragGrab>,
    /// Rutas del drag-and-drop **de archivos** en curso, leídas del origen al
    /// iniciar el drag (`text/uri-list`). Suple el DnD que winit NO recibe en
    /// Wayland: al soltar sobre una app tawasuyu, mirada reenvía estas rutas
    /// por `drop-bridge`. El `Option` interno es `None` hasta que el hilo
    /// lector termina; `None` externo = no hay drag de archivos.
    pub(crate) dnd_paths:
        Option<std::sync::Arc<std::sync::Mutex<Option<Vec<std::path::PathBuf>>>>>,
    /// Tamaño real de la salida (con la franja del shell incluida) — lo
    /// fija el backend; sirve para acoplar la ventana del shell.
    pub(crate) output_size: (i32, i32),
    /// Con el dock autoescondido (`MIRADA_SHELL_AUTOHIDE`), si está oculto
    /// ahora. Sin autohide se ignora. El puntero cerca del borde lo alterna.
    pub(crate) shell_hidden: bool,
    /// Última reserva publicada `(top, bottom, left, right)` en px — define el
    /// área de trabajo (salida menos dock/layers). Las zonas se escalan a ella.
    pub(crate) reserved: (i32, i32, i32, i32),

    /// Ventanas gestionadas, en orden de aparición.
    pub(crate) windows: Vec<ManagedWindow>,
    /// La contabilidad del Cuerpo (mirada-body).
    pub(crate) body: BodyState,
    /// El Cerebro: embebido o enlazado.
    pub(crate) brain: Brain,
    /// Fase del ciclo de vida — login o sesión (ver [`BodyMode`]).
    pub(crate) mode: BodyMode,
    /// Entorno de sesión (XDG_RUNTIME_DIR del usuario, WAYLAND_DISPLAY
    /// absoluto, bus D-Bus) inyectado a las apps nativas tras el traspaso.
    /// Vacío en modo greeter.
    pub(crate) session_env: Vec<(String, String)>,
    /// Identidad a la que rebajar privilegios al lanzar procesos de
    /// sesión. `None` salvo tras el traspaso del DM — entonces cada
    /// `spawn` hace `setuid`/`setgid` a este usuario (si somos root).
    pub(crate) session_user: Option<UserInfo>,
    /// Atajos globales a interceptar (los registra el Cerebro).
    pub(crate) grabs: Vec<String>,
    /// Parámetros de decoración de ventana (marco, …) que fija el Cerebro.
    pub(crate) decorations: mirada_brain::Decorations,
    /// Permisos de capacidad por ejecutable que fija el Cerebro. El filtro del
    /// global `zwlr_data_control` (creado al arrancar) los consulta para decidir
    /// qué clientes ven el snoop de portapapeles — de ahí el [`Arc`]/[`RwLock`]:
    /// el filtro vive `'static` dentro de smithay y `exec_op` los reemplaza
    /// cuando el Cerebro recarga la política.
    pub(crate) caps: Arc<std::sync::RwLock<mirada_brain::Permisos>>,
    /// Atajo capturado en el último evento de teclado, pendiente de enviar.
    pub(crate) pending_keybind: Option<String>,
    /// VT a la que conmutar, capturada por `Ctrl+Alt+Fn`. El backend DRM
    /// la consume tras el evento de teclado (sólo él puede `change_vt`).
    pub(crate) pending_vt: Option<i32>,
    /// Sesión ajena a ejecutar tras cerrar el compositor: el handoff a un
    /// compositor foráneo suelta el DRM (saliendo del bucle) y recién
    /// entonces hace `exec`. `(comando, usuario)`.
    pub(crate) pending_session: Option<(String, Option<UserInfo>)>,
    pub(crate) next_id: u64,
    pub(crate) running: bool,
}
