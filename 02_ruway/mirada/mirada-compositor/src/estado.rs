// Tipos y estado del compositor — se re-exportan desde la raíz del crate.

use std::sync::Arc;
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::input::keyboard::{KeyboardHandle, LedState};
use smithay::input::pointer::{CursorImageStatus, PointerHandle};
use smithay::input::{Seat, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::output::Output;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::dmabuf::{DmabufState};
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListState};
use smithay::wayland::selection::wlr_data_control::DataControlState;
use smithay::wayland::pointer_constraints::PointerConstraintsState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState;
use smithay::wayland::shell::xdg::{XdgShellState};
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shm::ShmState;
use auth_core::UserInfo;
use mirada_body::BodyState;
use mirada_brain::{BrainCommand, Desktop, Decorations, Permisos, WindowEffects};
use mirada_link::{BodyLink, BodyLinkServer};
use crate::gamma_control;
use crate::idle_notify;
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
    /// registran atajos, se rechaza `Spawn` y no hay autoarranque. Aún no
    /// hay ninguna sesión hosteada.
    Greeter,
    /// Sesión de usuario: el compositor funciona con normalidad.
    Session,
    /// Sesión activa pero **bloqueada**: el shell de credenciales (greeter en
    /// modo lock) se compone encima y se traga el input hasta que el usuario
    /// desbloquee. La sesión de abajo sigue residente — el lock es un overlay,
    /// no un congelamiento; por eso es reentrante (a diferencia del flip
    /// Greeter→Session, de una sola vía). Comparte con [`Greeter`](BodyMode::Greeter)
    /// el comportamiento de «hay un shell arriba»: ver [`App::shell_activo`].
    Locked,
}

/// Una sesión de usuario hosteada por el compositor.
///
/// Hoy el compositor hostea 0 o 1; el vector [`App::sessions`] le da forma de
/// N para que el *fast user switching* (varias sesiones concurrentes, saltar
/// entre ellas desde el lock) sea un incremento y no una reescritura. El
/// compositor **no** hace `setuid` de sí mismo: se queda con sus privilegios y
/// lanza los clientes de cada sesión rebajados a su [`user`](Session::user) —
/// la forma que habilita multisesión.
pub(crate) struct Session {
    /// Dueño de la sesión. `None` = los procesos heredan los privilegios del
    /// compositor (modo dev / sin root): no hay a quién rebajar.
    pub(crate) user: Option<UserInfo>,
    /// Entorno inyectado a las apps nativas de la sesión: su `XDG_RUNTIME_DIR`,
    /// el `WAYLAND_DISPLAY` absoluto, el bus D-Bus y el socket de control.
    pub(crate) env: Vec<(String, String)>,
    /// **Forma del escritorio de la sesión cuando está residente** (FUS). El
    /// `Desktop` embebido es uno solo y sirve a la sesión activa; al saltar de
    /// sesión se guarda acá la forma de la saliente (`snapshot`) y se restaura la
    /// de la entrante, y sus ventanas vivas se re-inyectan — así cada usuario
    /// tesela en su propio escritorio en vez de compartir slots. `None` mientras
    /// la sesión es la activa (su forma vive en el `Desktop`) o si nunca se
    /// guardó. Ver [`App::rebuild_desktop_for_active`].
    pub(crate) shape: Option<mirada_brain::DesktopState>,
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

/// Estado de escritorios que el Cerebro **enlazado** empuja vía
/// `BrainCommand::SetWorkspaces`, para que el switcher Win+Tab (HUD + slide)
/// funcione en modo DE —donde el compositor no tiene el `Desktop` local—.
pub(crate) struct LinkedWorkspaces {
    /// Escritorio activo (0-based).
    pub(crate) active: usize,
    /// Nº de ventanas por escritorio (las cargas; el switcher lista los ocupados).
    pub(crate) loads: Vec<usize>,
    /// Duración del slide de transición en ms (`0` = salto seco, sin animación).
    pub(crate) slide_ms: u32,
    /// Modo de transición que el Cerebro empujó (Direct/Hyprland/Prezi/Cube). Sin
    /// esto el Cuerpo lo inferría de `slide_ms` y Cube/Prezi eran inalcanzables.
    pub(crate) switch_mode: mirada_brain::WorkspaceSwitchMode,
}

/// Datos para pintar la vista espacial (Prezi) en vivo. Ver
/// [`App::overview_data`](crate::App::overview_data).
pub(crate) struct OverviewData {
    /// Escritorio activo (0-based).
    pub(crate) active: usize,
    /// Colocación rica de cada escritorio en el plano del Prezi (posición libre +
    /// tamaño + giro, en unidades de celda). El overlay en vivo honra posición y
    /// tamaño; el giro viaja en el dato (lo respeta la vista espacial Llimphi —
    /// el overlay GLES dibuja quads axis-aligned, ver `emit_overview`).
    pub(crate) places: Vec<mirada_brain::OverviewPlace>,
    /// Ventanas por escritorio (para saber cuáles están ocupados).
    pub(crate) loads: Vec<usize>,
    /// Rect de referencia en el que están los `layouts` (para normalizar).
    pub(crate) work: mirada_brain::Rect,
    /// Ventanas de cada escritorio en el espacio de `work`: `(id, rect)`. El `id`
    /// permite mapear cada rect a su `ManagedWindow` y pintar su **superficie
    /// viva** a escala en la miniatura (no un rectángulo plano).
    pub(crate) layouts: Vec<Vec<(u64, mirada_brain::Rect)>>,
}

/// Cómo resolver el fondo de una salida, según la **fuente** elegida en la
/// config (`wallpaper_source`). El compositor lo materializa en un buffer.
pub(crate) enum WallpaperSpec {
    /// Imagen por su ruta + ajuste (fuentes `local`/`directory`/`remote`/`auto`
    /// con `wallpaper_path` resuelto — el slideshow/daemon pudo overridearlo).
    Image(String, mirada_brain::WallpaperFit),
    /// Color sólido RGB.
    Solid([u8; 3]),
    /// Gradiente vertical de stops RGB (de arriba a abajo).
    Gradient(Vec<[u8; 3]>),
    /// Patrón procedural + paleta.
    Procedural(mirada_procedural::Pattern, Vec<[u8; 3]>),
    /// **Video** por su ruta (`wallpaper_source = "video"`): el frame lo entrega
    /// el worker [`crate::drm_backend`] y el render lo compone por salida. La
    /// cadencia/loop viven en el worker; acá sólo viaja la ruta.
    Video(String),
    /// **Lottie/rive** (`wallpaper_source = "lottie"|"rive"`) reproducido desde
    /// la cache de frames *bakeada* por `fondo-bake`: el render sube el frame del
    /// instante actual (tamaño nativo del bake) y la GPU lo escala. Sin cache cae
    /// a la chakana animada. El compositor no rasteriza vello en caliente.
    Fondo(mirada_fondo::FondoSpec),
    /// Gradiente sobrio por defecto (auto sin imagen).
    Default,
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
    /// Política «barra sólo en flotantes» vigente para esta ventana (espejo de
    /// [`mirada_protocol::Decorations::titlebar_floating_only`]). Se re-estampa
    /// en cada `Configure` —junto con `floating`— para que [`crate::titlebar_for`]
    /// la consulte sin cambiar de firma. Con `true`, una ventana teselada no
    /// reserva ni pinta barra de título.
    pub(crate) titlebar_floating_only: bool,
    /// `true` si tiene el foco del teclado — pinta el marco resaltado.
    pub(crate) focused: bool,
    /// `true` si es la ventana del shell — acoplada al pie, sin teselar.
    pub(crate) is_shell: bool,
    /// Sesión hosteada dueña de la ventana (FUS). Con ≥2 sesiones, sólo se
    /// pintan/animan las ventanas de la sesión activa — ver [`App::session_visible`].
    /// Las ventanas del shell/greeter no pertenecen a ninguna y se ignoran aquí.
    pub(crate) session: mirada_brain::SessionId,
    /// `app_id` del cliente (la misma cadena que se le pasó al Cerebro en
    /// `WindowOpened`). Se guarda para poder **re-inyectar** la ventana en el
    /// `Desktop` al saltar de sesión (FUS) sin re-leer la superficie.
    pub(crate) app_id: String,
    /// `true` si es la ventana del greeter (DM): sin barra de título, y el
    /// backend la muda al monitor con el ratón en multi-monitor.
    pub(crate) is_greeter: bool,
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
    /// Handles `zwlr_foreign_toplevel_handle_v1` —uno por manager wlr bindeado
    /// (la barra `pata`)—. Espejan título/`app_id`/estado y reciben
    /// activar/cerrar. Vacío para la ventana del shell. Ver [`crate::foreign_toplevel`].
    pub(crate) wlr_handles: Vec<crate::foreign_toplevel::ZwlrForeignToplevelHandleV1>,
    /// Búferes de los 4 lados del marco (arriba, abajo, izq., der.) —
    /// cada uno con su `Id` estable para el seguimiento de daño.
    pub(crate) borders: [SolidColorBuffer; 4],
    /// `true` si la decoración la pone el servidor (mirada dibuja barra de
    /// título + marco). `false` = el cliente se decora solo (CSD: Firefox/Zen,
    /// GTK como pavucontrol) y mirada se hace a un lado para no duplicar la
    /// barra ni forrar la sombra del cliente en un margen. Se resuelve por la
    /// negociación `xdg-decoration` ([`App::ssd_surfaces`]); las apps que ni
    /// hablan el protocolo quedan en CSD (no las decoramos).
    pub(crate) ssd: bool,
    /// Efectos visuales (opacidad, sombra…) que el Cerebro fija con
    /// `BrainCommand::SetEffects` (Tier-2: atenuar/sombrear según foco, etc.).
    /// El render los aplica. Por defecto: opaca y sin sombra.
    pub(crate) effects: WindowEffects,
    /// Instante (ms desde `DrmState::start`) en que la ventana pintó su PRIMER
    /// frame sano — sella el fade-in de apertura («animaciones de Wayland»).
    /// `None` hasta entonces: el render lo estampa la primera vez que la ventana
    /// es visible con buffer sano, y a partir de ahí la rampa de alfa corre por
    /// `window_open_ms`. Se sella una sola vez (re-mostrar no re-anima — el
    /// slide entre escritorios ya cubre eso).
    pub(crate) mapped_ms: Option<u32>,
    /// Instante (ms desde `start`) del último cambio de foco — origen del *glow*
    /// del marco (crossfade del color sin-foco↔con-foco). `None` hasta el primer
    /// cambio (color estático). Lo estampa el render comparando contra
    /// [`was_focused`](Self::was_focused).
    pub(crate) focus_ms: Option<u32>,
    /// Estado de foco con que se estampó `focus_ms` — para detectar el flanco.
    pub(crate) was_focused: bool,
    /// **Instantánea para el fade al cerrar** (motor de transición). Cuando el
    /// fade de cierre está activo (`window_close_ms>0`), el render captura el
    /// contenido del cliente a bytes CPU cada cierto rato; al destruirse la
    /// ventana, [`App::toplevel_destroyed`] la mueve a un [`ClosingGhost`] que se
    /// desvanece. `None` con el efecto apagado (default) → costo cero. Es CPU,
    /// no una textura GPU: no arrastra vida de recursos GL.
    pub(crate) close_snapshot: Option<CloseSnapshot>,
    /// Último instante (ms desde `start`) en que se tomó la instantánea — para
    /// estrangular la captura (no en cada frame). `0` = nunca.
    pub(crate) last_snapshot_ms: u32,
}

/// Instantánea CPU del contenido de una ventana, en coords **globales**, para
/// el fade al cerrar. `rgba` son bytes `Argb8888` listos para un
/// `MemoryRenderBuffer` (ya corregidos de orientación por el offscreen).
pub(crate) struct CloseSnapshot {
    pub(crate) rgba: Vec<u8>,
    pub(crate) w: i32,
    pub(crate) h: i32,
    /// Origen global del contenido capturado (dónde estaba en pantalla).
    pub(crate) x: i32,
    pub(crate) y: i32,
}

/// El «fantasma» de una ventana que se está cerrando: su última instantánea,
/// desvaneciéndose (y encogiéndose un poco) durante `window_close_ms`. Vive en
/// [`App::closing_ghosts`]; el render lo pinta y lo retira al expirar. El motor
/// de transición captura-a-textura del PLAN, en su forma CPU mínima.
pub(crate) struct ClosingGhost {
    pub(crate) snap: CloseSnapshot,
    /// Instante de arranque (ms desde `start`). `None` hasta que el render lo
    /// sella en el primer frame que lo ve (App no tiene el reloj del backend).
    pub(crate) t0: Option<u32>,
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// Trackea los `xdg_popup` (menús de apps GTK/Qt: el de aplicación y los
    /// contextuales). Sin él, sus popups nunca se posicionan ni se dibujan —
    /// los menús «no abren». Lo alimentan `new_popup`/`reposition_request`, el
    /// `commit`, y el render itera [`smithay::desktop::PopupManager::popups_for_surface`].
    pub(crate) popups: smithay::desktop::PopupManager,
    pub(crate) shm_state: ShmState,
    /// Estado de `zwp_linux_dmabuf` — deja que los clientes con GPU
    /// (apps GPUI, navegadores acelerados) compartan búferes de vídeo.
    pub(crate) dmabuf_state: DmabufState,
    pub(crate) seat_state: SeatState<App>,
    pub(crate) data_device_state: DataDeviceState,
    /// Estado de `zwp_primary_selection_v1` — la **selección primaria** de X11:
    /// seleccionar texto lo copia a un buffer aparte, y el clic central lo pega.
    /// Es ortogonal al portapapeles normal (`Ctrl+C`/`Ctrl+V`, `wl_data_device`).
    pub(crate) primary_selection_state: PrimarySelectionState,
    /// Estado de `zwp_pointer_constraints_v1` — lock/confine del cursor sobre una
    /// superficie. Lo usan juegos y apps 3D para capturar el ratón (mirada libre).
    /// Sólo se conserva para mantener vivo el global (el handler no lee el estado;
    /// la activación de la restricción va por `with_pointer_constraint`).
    pub(crate) _pointer_constraints_state: PointerConstraintsState,
    /// Estado de `zwp_relative_pointer_v1` — entrega del delta crudo del ratón
    /// (sin acotar a la pantalla) a la superficie con foco; compañero natural del
    /// pointer-lock para cámaras 3D / FPS. Sólo se conserva para mantener vivo el
    /// global (la entrega va por `PointerHandle::relative_motion`).
    pub(crate) _relative_pointer_state: RelativePointerManagerState,
    /// Último estado de los LEDs del teclado (Bloq Mayús / Bloq Num / Bloq Despl),
    /// que `smithay` calcula al procesar modificadores. El backend lo propaga a
    /// los teclados físicos (`libinput::Device::led_update`).
    pub(crate) led_state: LedState,
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
    /// Copia del `DisplayHandle` — para crear recursos wayland fuera del
    /// dispatch (p. ej. handles `zwlr_foreign_toplevel` al mapear ventanas).
    pub(crate) dh: smithay::reexports::wayland_server::DisplayHandle,
    /// Estado de `zwlr_foreign_toplevel_management_v1` — el servidor wlr que
    /// alimenta el `window_list` de la barra. Ver [`crate::foreign_toplevel`].
    pub(crate) foreign_toplevel_manager: crate::foreign_toplevel::ForeignToplevelManagerState,
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
    /// Global `wlr-gamma-control` (luz nocturna: wlsunset/gammastep). Vivo toda
    /// la sesión.
    pub(crate) _gamma_control_state: gamma_control::GammaControlState,
    /// Controles de gamma activos `(salida, recurso)` — uno por salida; el
    /// segundo intento recibe `failed`. Se purga al destruirse el control.
    pub(crate) gamma_active: Vec<(
        smithay::output::Output,
        smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::zwlr_gamma_control_v1::ZwlrGammaControlV1,
    )>,
    /// Rampas de gamma pendientes de aplicar `(salida, rampa | None=reset)` — las
    /// drena el backend DRM (set_gamma sobre el CRTC). El protocolo, que corre
    /// sobre `App`, no toca el hardware: sólo deja el pedido (patrón DPMS/sesión).
    pub(crate) pending_gamma: Vec<(smithay::output::Output, Option<gamma_control::GammaRamp>)>,
    /// Global `ext_idle_notify_v1` (notifica ocio a clientes externos). Vivo toda
    /// la sesión. Conducido por el tick (`App::drive_idle_notifs`), ver
    /// [`idle_notify`].
    pub(crate) _idle_notify_state: idle_notify::IdleNotifyState,
    /// Notificaciones de ocio vivas, cada una con su propio timeout.
    pub(crate) idle_notifs: Vec<idle_notify::IdleNotif>,
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
    /// Id estable del Cerebro de cada salida, **en el mismo orden** que
    /// [`Self::outputs`]. Las reservas (`reserve_output`) se direccionan por
    /// este id, no por el índice: tras un hotplug la lista se reordena por
    /// `(order, name)` pero el id sigue señalando al mismo monitor físico.
    pub(crate) output_ids: Vec<u32>,
    /// Gestor de salidas con `xdg-output` (`zxdg_output_manager_v1`): waybar
    /// y otras barras lo exigen para conocer nombre/geometría de las salidas.
    /// Se conserva sólo para mantener vivo el global (de ahí el `allow`).
    #[allow(dead_code)]
    pub(crate) output_manager_state: OutputManagerState,
    pub(crate) keyboard: Option<KeyboardHandle<App>>,
    /// Foco de teclado **diferido**: cuando el Cerebro enfoca una ventana
    /// recién abierta, su superficie todavía no presentó buffer (no está
    /// mapeada) y `set_focus` se perdería —el cliente puede no haber bindeado
    /// `wl_keyboard` aún, así que el `enter` no llega y el teclado queda mudo
    /// hasta abrir otra ventana. Guardamos acá el destino y lo aplicamos en el
    /// primer commit con buffer de esa superficie (ya mapeada, ya con teclado
    /// bindeado). `None` cuando no hay foco pendiente.
    pub(crate) pending_kb_focus: Option<WlSurface>,
    /// Mientras hay un menú (popup con grab) abierto, guarda **a quién** hay que
    /// devolverle el foco de teclado al cerrarse (la ventana que lo tenía). El
    /// foco se mueve al popup para navegar con flechas/Enter/Escape (lo maneja
    /// el cliente). `Some(prev)` = menú activo; `None` = sin menú. Ver
    /// `reconcile_popup_keyboard`.
    pub(crate) popup_saved_focus: Option<Option<WlSurface>>,
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
    /// Fantasmas de cierre en curso (fade-out de ventanas recién cerradas). Las
    /// puebla [`App::toplevel_destroyed`] con la última instantánea de la ventana;
    /// el render las pinta desvaneciéndose y las retira al expirar. Vacío con el
    /// efecto apagado (default).
    pub(crate) closing_ghosts: Vec<ClosingGhost>,
    /// La contabilidad del Cuerpo (mirada-body).
    pub(crate) body: BodyState,
    /// El Cerebro: embebido o enlazado.
    pub(crate) brain: Brain,
    /// El listener persistente del Cerebro enlazado (`MIRADA_SOCKET`): sobrevive
    /// a la muerte del Cerebro para **re-aceptar** uno nuevo (reinicio o crash)
    /// sin tirar el Cuerpo ni las conexiones Wayland de los clientes. `None` en
    /// modo embebido. Ver [`App::reconcile_brain`].
    pub(crate) brain_server: Option<BodyLinkServer>,
    /// Fase del ciclo de vida — login, sesión o sesión bloqueada (ver [`BodyMode`]).
    pub(crate) mode: BodyMode,
    /// Sesiones hosteadas (FUS) con id estable. 0 (greeter), 1 (tras el traspaso
    /// del DM) o N (varias sesiones concurrentes, saltables desde el lock). La
    /// activa es la que se pinta y recibe input; sus procesos se rebajan a su
    /// usuario y heredan su entorno — ver [`App::active_user`]/[`App::active_env`].
    /// Ver [`mirada_brain::SessionRoster`] y [`Session`].
    pub(crate) roster: mirada_brain::SessionRoster<Session>,
    /// Atajos globales a interceptar (los registra el Cerebro).
    pub(crate) grabs: Vec<String>,
    /// Diagnóstico opt-in (`MIRADA_DEBUG_KEYS=1`): loguea cada combo con
    /// modificador que se reenvía por no estar en [`grabs`](Self::grabs).
    pub(crate) debug_keys: bool,
    /// Switcher visual de ventanas (Alt-Tab) en curso, o `None`. Ver
    /// [`crate::switcher`].
    pub(crate) switcher: Option<crate::switcher::Switcher>,
    /// Señal del filtro de teclado al bucle: qué switcher (ventanas/escritorios)
    /// y si adelantar (`true`) o retroceder, tras procesar la tecla.
    pub(crate) switcher_step: Option<(crate::switcher::SwitcherKind, bool)>,
    /// Señal del filtro: cancelar el switcher (Esc) tras procesar la tecla.
    pub(crate) switcher_cancel: bool,
    /// Vista espacial (Prezi) abierta: zoom-out a todos los escritorios. Se
    /// togglea desde el filtro de teclado (Super+e) y el render la pinta.
    pub(crate) overview_open: bool,
    /// Pedido de **cierre** de la vista espacial: el render anima el zoom de
    /// salida y, al terminar, baja `overview_open`. Así el cierre no es seco.
    pub(crate) overview_closing: bool,
    /// La vista espacial se abrió por **Win+Tab** (Super sostenido): se cierra al
    /// soltar Super, como un switcher. Si se abrió por Super+e (toggle), no.
    pub(crate) overview_via_wintab: bool,
    /// Escritorio **resaltado** (cursor de navegación) en la vista espacial.
    /// Tab/Shift+Tab lo mueven mientras Super está sostenido; al soltar Super se
    /// salta a éste. El borde activo del mosaico lo marca.
    pub(crate) overview_selected: usize,
    /// Win+Tab de Prezi en curso **en modo enlazado**: la vista espacial la pinta
    /// la app (el Cuerpo no la tiene en linked), pero sólo el Cuerpo ve el release
    /// de Super. Mientras esto sea `true`, al soltar Super el Cuerpo le reenvía a
    /// la app el keybind sentinela de commit (`OVERVIEW_WINTAB_COMMIT`) para que
    /// salte al destino resaltado. Embebido NO lo usa (ahí pinta el Cuerpo).
    pub(crate) prezi_wintab_linked: bool,
    /// Estado de escritorios empujado por el Cerebro enlazado (`SetWorkspaces`),
    /// para el switcher Win+Tab + slide en modo DE. `None` con Cerebro embebido.
    pub(crate) linked_ws: Option<LinkedWorkspaces>,
    /// Parámetros de decoración de ventana (marco, …) que fija el Cerebro.
    pub(crate) decorations: mirada_brain::Decorations,
    /// Layout de la barra de título (botones, grupos, alineación, estilo) que
    /// fija el Cerebro vía `BodyOp::SetTitlebarLayout`; default = histórico.
    pub(crate) titlebar_layout: mirada_brain::TitlebarLayout,
    /// Superficies cuyo cliente aceptó decoración del servidor (SSD) vía
    /// `xdg-decoration`. Fuente de verdad de [`ManagedWindow::ssd`]; una
    /// ventana ausente de este set se decora sola (CSD) y mirada no le pinta
    /// barra ni marco. Se mantiene en el handler de `xdg-decoration` y se
    /// limpia al destruirse el toplevel.
    pub(crate) ssd_surfaces: std::collections::HashSet<WlSurface>,
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
    /// Tubería de escritura al stdin del greeter (sólo en modo DM). El
    /// compositor le empuja por acá la disposición de monitores y cuál tiene
    /// el ratón, para que la tarjeta de login viaje al monitor activo. `None`
    /// fuera de modo greeter o si la tubería se cerró.
    pub(crate) greeter_stdin: Option<std::process::ChildStdin>,
    /// Pedido pendiente de capturar las miniaturas de las sesiones para el lock:
    /// lo pone [`App::push_sessions_to_greeter`] al enganchar el candado y lo
    /// consume el bucle del backend en el próximo cuadro (necesita el renderer,
    /// que no vive en `App`). Ver [`crate::thumbs`].
    pub(crate) pending_thumbs: bool,
    /// Último índice de salida que se le informó al greeter como «activo»
    /// (la del ratón). `usize::MAX` ⇒ aún no se empujó nada — fuerza el
    /// primer envío.
    pub(crate) greeter_active_output: usize,
    /// Pedido de bloqueo pendiente: el nombre de usuario a quien pedirle la
    /// contraseña. Lo pone [`App::request_lock`] (desde `BrainCommand::Lock`) y
    /// lo consume el bucle del backend, que lanza el shell de credenciales en
    /// modo lock (necesita el emisor del canal, que no vive en `App`).
    pub(crate) pending_lock: Option<String>,

    /// **Política de inactividad** (apagar pantalla + bloquear, multimedia-aware).
    /// La alimenta el tick de cada backend ([`App::idle_tick`]) y la resetea el
    /// input ([`App::idle_activity`]). Ver [`mirada_brain::idle`].
    pub(crate) idle: mirada_brain::IdleManager,
    /// Superficies con un *idle inhibitor* activo (`zwp_idle_inhibit`): vídeo en
    /// reproducción, llamadas… Si el set no está vacío, la inactividad se pausa.
    pub(crate) idle_inhibitors: std::collections::HashSet<WlSurface>,
    /// Instante del último tick de inactividad, para medir el `dt` del próximo.
    /// `None` hasta el primer tick.
    pub(crate) last_idle_tick: Option<std::time::Instant>,
    /// Pedido de **DPMS** pendiente que el backend DRM consume: `Some(true)` =
    /// apagar pantalla, `Some(false)` = encender. Lo pone la política de
    /// inactividad; el backend `winit` (anidado) no tiene DPMS real y sólo lo
    /// limpia. `None` = sin pedido.
    pub(crate) pending_dpms: Option<bool>,
    /// Pedido de **nueva sesión** pendiente (FUS «cambiar usuario»): lo pone
    /// [`App::request_new_session`] y lo consume el bucle del backend, que
    /// relanza el greeter en modo **login** (no lock) para hostear otra sesión
    /// junto a la actual. El siguiente [`start_session`](App::start_session) que
    /// llegue da de alta una sesión más en vez de ignorarse.
    pub(crate) pending_new_session: bool,

    /// **Clipboard por zona** (`MIRADA_CLIPBOARD_POR_ZONA=1`): cada escritorio
    /// tiene su propio portapapeles de texto. `false` = comportamiento normal
    /// (un solo clipboard global). Ver [`crate::zone_clipboard`].
    pub(crate) clipboard_por_zona: bool,
    /// Almacén del portapapeles por zona (compartido con el hilo lector que
    /// captura la selección de un cliente al copiar). Inerte si
    /// [`clipboard_por_zona`](Self::clipboard_por_zona) es `false`.
    pub(crate) zone_clipboard:
        std::sync::Arc<std::sync::Mutex<crate::zone_clipboard::ZoneClipboard>>,
}
