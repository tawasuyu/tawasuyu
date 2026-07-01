//! Backend `wlr-layer-shell`: hace que `pata` se siente **al nivel de eww/
//! waybar** en cualquier compositor wlroots (Hyprland, Sway, river…), no como
//! una ventana cliente.
//!
//! Una *layer surface* se ancla a un borde y declara una *exclusive zone* —el
//! compositor le reserva esa franja y tesela el resto alrededor—, igual que eww.
//! Aquí: nos conectamos a Wayland con `smithay-client-toolkit`, creamos **una
//! layer surface por cada superficie `Bar`** de la config (cada una anclada a su
//! borde con su exclusive zone), sacamos su `wgpu::Surface` de los punteros raw
//! del `wl_surface`/`wl_display` (envuelta en [`RawSurface`]) y la pintamos
//! reusando el pipeline de Llimphi (`mount → compute → paint → render`).
//!
//! Estructura interna:
//! - `mod.rs`          — tipos, constantes, `run()` y delegaciones de protocolo.
//! - `app_impl.rs`     — métodos de `LayerApp` (lógica de la app).
//! - `event_handlers.rs` — implementaciones de los traits de smithay-client-toolkit.

pub(super) mod app_impl;
pub(super) mod event_handlers;

use std::error::Error;
use std::ffi::c_void;
use std::ptr::NonNull;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent as KbEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT, BTN_RIGHT},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor as LayerAnchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler,
            LayerSurface, LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};
use wayland_client::{
    event_created_child,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1, EVT_TOPLEVEL_OPCODE},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::ExtIdleNotificationV1, ext_idle_notifier_v1::ExtIdleNotifierV1,
};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};

/// Segundos de inactividad **adicional** entre reintentos de suspensión cuando
/// se pospuso por trabajo en curso: el compositor nos vuelve a despertar para
/// reintentar en cuanto el trabajo termine (sin volver a pedir input).
pub(super) const REINTENTO_ENERGIA_SECS: u32 = 60;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_compositor::{
    hit_test_click, hit_test_hover, hit_test_scroll, measure_text_node, mount, paint, DragFn,
    DragPhase, Mounted,
};
use llimphi_ui::llimphi_hal::{wgpu, Hal, RawSurface, Surface as _};
use llimphi_ui::llimphi_layout::{taffy, ComputedLayout, LayoutTree};
use llimphi_ui::llimphi_raster::{peniko::color::palette, vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use pata_core::config::FloatingCard;
use pata_core::widget::{Widget, WidgetCtx};
use pata_core::{Anchor, Config, SurfaceKind};

use crate::nouser::{self, MembersOutcome, NavState, PollOutcome};
use crate::sampler::SamplerHandle;
use pata_host::HostServer;
use crate::toplevel::{Toplevel, WindowEntry};
use crate::tray::TrayHandle;
use crate::{render, Model, Msg};

use std::sync::mpsc::{Receiver, Sender};

/// Traza de diagnóstico gateada por `PATA_DIAG` (cualquier valor la enciende).
macro_rules! diag {
    ($($a:tt)*) => {
        if std::env::var_os("PATA_DIAG").is_some() {
            eprintln!($($a)*);
        }
    };
}
pub(super) use diag;

/// El estado wgpu de **una** layer surface (una barra).
pub(super) struct PanelGpu {
    pub(super) surface: RawSurface,
    pub(super) renderer: Renderer,
    pub(super) typesetter: Typesetter,
    pub(super) scene: vello::Scene,
    pub(super) layout: LayoutTree,
}

/// El árbol pintado en el último frame de un panel, para hacer hit-test.
pub(super) struct RenderCache {
    pub(super) mounted: Mounted<Msg>,
    pub(super) computed: ComputedLayout,
}

/// Un arrastre en curso sobre un nodo arrastrable.
pub(super) struct LayerDrag {
    /// El handler del nodo: `Fn(DragPhase, dx, dy) -> Option<Msg>`.
    pub(super) handler: DragFn<Msg>,
    /// Última posición del puntero, para el delta de cada `Move`.
    pub(super) last: (f32, f32),
}

/// Estado de un arrastre de **reordenamiento** de un botón del task manager.
/// Mientras dura, `LayerApp::task_order` se reescribe en vivo.
pub(super) struct TaskDrag {
    /// `id` de la ventana que se arrastra.
    pub(super) id: u32,
    /// Delta horizontal acumulado desde el inicio del arrastre (px), con signo.
    pub(super) dx_acc: f32,
    /// Movimiento absoluto total recorrido (px). Sirve para distinguir un click
    /// (apenas se movió) de un arrastre real.
    pub(super) movido: f32,
    /// Orden de `id`s visible al iniciar el arrastre (la base sobre la que se
    /// recalcula la posición destino en cada `Move`, sin acumular deriva).
    pub(super) orden_base: Vec<u32>,
    /// Índice de `id` dentro de `orden_base`.
    pub(super) idx_base: usize,
}

/// El estado de una tarjeta flotante montada como su propia layer surface.
pub(super) struct CardState {
    pub(super) spec: FloatingCard,
    pub(super) widgets: Vec<Box<dyn Widget>>,
}

/// Una layer surface de pata: o una **barra** anclada a un borde, o una
/// **tarjeta flotante** (`card`).
pub(super) struct Panel {
    /// Índice de su superficie en `cfg.surfaces`.
    pub(super) idx: usize,
    /// `Some` si esta surface es una tarjeta flotante; `None` si es una barra.
    pub(super) card: Option<CardState>,
    /// `true` si esta surface es el **panel flotante** de un sidebar (el «drawer»
    /// que se despliega al abrir un diente), creado a demanda como una layer
    /// surface APARTE del rail. Su `idx` apunta al mismo `SurfaceKind::Sidebar` que
    /// el rail. Los rails tienen `false`. Ver [`super::app_impl::LayerApp::reconcile_drawer`].
    pub(super) drawer: bool,
    /// El `wl_output` destino de esta surface (o `None` = primario). Se guarda para
    /// poder crear el drawer en el MISMO monitor que su rail.
    pub(super) output: Option<wl_output::WlOutput>,
    pub(super) layer: LayerSurface,
    /// El árbol del último frame (para hit-test de clicks).
    pub(super) cache: Option<RenderCache>,
    pub(super) width: u32,
    pub(super) height: u32,
    /// `true` cuando hay algo nuevo que pintar.
    pub(super) dirty: bool,
    /// Nodo bajo el puntero en este panel (para `hover_fill`).
    pub(super) hover_idx: Option<usize>,
    /// X local del puntero sobre el panel (o `None` si está fuera). Sólo lo usa
    /// el dock para la magnificación por cercanía; se actualiza en cada `Motion`.
    pub(super) cursor_x: Option<f32>,
    pub(super) gpu: Option<PanelGpu>,
}

/// Alto del drawer Quake cuando se despliega (px).
const DRAWER_H: u32 = 420;

/// Alto de la barra superior cuando despliega el menú de inicio (px).
pub(super) const MENU_H: u32 = 480;

/// Tras abrir el menú, ignoramos el `leave`-cierre durante este lapso: el
/// compositor reacomoda el foco al darle el teclado al menú (Exclusive) y le
/// manda un `leave` espurio que, sin guarda, lo cerraría al instante. Un `leave`
/// legítimo (el usuario clava el foco en una ventana) llega mucho más tarde.
pub(super) const MENU_LEAVE_GRACE: std::time::Duration = std::time::Duration::from_millis(400);

/// Duración del viaje del resaltado del switcher al cambiar de escritorio.
pub(super) const WS_ANIM: std::time::Duration = std::time::Duration::from_millis(420);

/// Duración de la animación de apertura del menú de inicio (fade + slide).
pub(super) const MENU_OPEN: std::time::Duration = std::time::Duration::from_millis(170);

/// Estado de la animación del switcher: el resaltado viaja de `from` a `to`
/// (1-based) desde `start`. La cometa se calcula por frame (ver `LayerApp::ws_comet`).
#[derive(Clone, Copy)]
pub(super) struct WsAnimState {
    pub(super) from: u8,
    pub(super) to: u8,
    pub(super) start: std::time::Instant,
}

/// Qué cuerpo muestra el drawer que crece de la barra del `start_button`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum MenuKind {
    /// El menú de inicio (lista de apps con buscador, toma el teclado).
    #[default]
    Apps,
    /// El historial de portapapeles (lista de copias, sólo clicks).
    Clipboard,
    /// El panel del reloj (spinners de fecha/hora, sólo clicks).
    Clock,
    /// El control panel (ajustes rápidos: volumen/brillo/batería/radios).
    Control,
    /// El applet de red (lista de redes Wi-Fi para conectarse).
    Network,
    /// El mezclador de volumen (sink por defecto + corrientes por app).
    Volume,
    /// El menú de sesión/energía (bloquear/suspender/reiniciar/apagar/logout).
    Session,
    /// El applet de Bluetooth (switch + dispositivos emparejados).
    Bluetooth,
    /// La campanita de notificaciones (no-molestar + historial reciente).
    Notifications,
    /// El diálogo de autenticación de polkit (lo abre una solicitud entrante).
    Polkit,
}

/// El cliente Wayland del backend layer-shell.
pub(super) struct LayerApp {
    pub(super) registry_state: RegistryState,
    pub(super) output_state: OutputState,
    pub(super) seat_state: SeatState,
    pub(super) conn: Connection,
    /// `Hal` compartido (una instancia/device de wgpu para todas las barras).
    pub(super) hal: Option<Hal>,
    pub(super) keyboard: Option<wl_keyboard::WlKeyboard>,
    pub(super) pointer: Option<wl_pointer::WlPointer>,
    /// El seat (para activar ventanas: `activate(seat)` lo exige).
    pub(super) seat: Option<wl_seat::WlSeat>,
    /// El manager de wlr-foreign-toplevel, si el compositor lo expone.
    #[allow(dead_code)]
    pub(super) toplevel_mgr: Option<ZwlrForeignToplevelManagerV1>,
    /// Notificador de inactividad del compositor (ext-idle-notify-v1), si lo
    /// expone. Fuente del idle inteligente de energía.
    pub(super) idle_notifier: Option<ExtIdleNotifierV1>,
    /// Notificación de inactividad viva (timeout para suspender). Se re-arma al
    /// posponer; `None` hasta que haya seat + notifier.
    pub(super) idle_notif: Option<ExtIdleNotificationV1>,
    /// Política del idle de energía (suspender/apagar por inactividad).
    pub(super) energia_cfg: crate::energia::ConfigEnergia,
    /// Ya se emitió la suspensión/apagado en este ciclo de inactividad.
    pub(super) energia_disparado: bool,
    /// Ya se avisó «pospuesto» en este ciclo (no repetir en cada reintento).
    pub(super) energia_pospuesto: bool,
    /// Manager de idle-inhibit del compositor (zwp_idle_inhibit_manager_v1), si
    /// lo expone. Sostiene el «mantener despierto» (café): pausa el apagado de
    /// pantalla y el bloqueo del compositor.
    pub(super) idle_inhibit_mgr: Option<ZwpIdleInhibitManagerV1>,
    /// Inhibidor vivo mientras el café está encendido; `None` si apagado.
    pub(super) idle_inhibitor: Option<ZwpIdleInhibitorV1>,
    /// Las ventanas abiertas que reporta el compositor.
    pub(super) toplevels: Vec<Toplevel>,
    /// Orden propio de los botones del task manager (`id`s de toplevel). Vacío =
    /// orden natural de `toplevels`. Lo edita el drag-to-reorder; las ventanas
    /// nuevas (no presentes) quedan al final en su orden natural.
    pub(super) task_order: Vec<u32>,
    /// Arrastre de reordenamiento del task manager en curso, si hay uno.
    pub(super) task_drag: Option<TaskDrag>,
    /// Contador para asignar [`Toplevel::id`] estables.
    pub(super) next_toplevel_id: u32,
    /// Texto del portapapeles (una línea).
    pub(super) clipboard: Option<String>,
    /// La bandeja del sistema (StatusNotifierItem).
    pub(super) tray: Option<TrayHandle>,
    /// Feed de clima en su propio hilo.
    pub(super) weather: Option<crate::weather::WeatherHandle>,
    /// Última lectura del clima.
    pub(super) weather_now: Option<crate::weather::Weather>,
    /// Feed de red (Wi-Fi/Ethernet) en su propio hilo.
    pub(super) network: Option<crate::network::NetworkHandle>,
    /// Última lectura de la red.
    pub(super) network_now: Option<crate::network::NetState>,
    /// Corrientes de audio por app (sink-inputs) para el mezclador de volumen.
    pub(super) sink_inputs: Vec<crate::sampler::SinkInput>,
    /// Dispositivos de salida (sinks) para el selector de salida del volumen.
    pub(super) sinks: Vec<crate::sampler::Sink>,
    /// Entrada de contraseña Wi-Fi en curso: `(ssid, tecleado)`. `None` = lista.
    pub(super) net_password: Option<(String, String)>,
    /// Acción de sesión pendiente de confirmación en el menú de energía.
    pub(super) session_confirm: Option<crate::SessionAction>,
    /// Feed MPRIS (reproductor) en su propio hilo.
    pub(super) mpris: Option<crate::mpris::MprisHandle>,
    /// Último estado del reproductor.
    pub(super) media_now: Option<crate::mpris::MediaState>,
    /// Feed de Bluetooth en su propio hilo.
    pub(super) bluetooth: Option<crate::bluetooth::BluetoothHandle>,
    /// Última lectura de Bluetooth.
    pub(super) bluetooth_now: Option<crate::bluetooth::BtState>,
    /// Cliente del daemon de notificaciones (la campanita), en su propio hilo.
    pub(super) notifications: Option<crate::notifications::NotificationsHandle>,
    /// Peor nivel de batería ya avisado (0/1/2). Ver [`crate::bateria`].
    pub(super) bat_avisado: u8,
    /// Agente de autenticación polkit en su propio hilo.
    pub(super) polkit: Option<crate::polkit::PolkitHandle>,
    /// Solicitud de autenticación polkit en curso (con el canal de respuesta).
    pub(super) polkit_prompt: Option<crate::polkit::PolkitRequest>,
    /// Contraseña tecleada en el diálogo de polkit.
    pub(super) polkit_input: String,
    /// Índice del panel de la surface dedicada del OSD (volumen/brillo), o `None`.
    pub(super) osd_pi: Option<usize>,
    /// Cartel OSD vigente, o `None`. Se dispara desde la rueda/slider y se oculta
    /// al cumplir su tiempo.
    pub(super) osd: Option<crate::render::Osd>,
    /// Visualizador de audio (cava) en su propio hilo.
    pub(super) cava: Option<crate::cava::CavaHandle>,
    /// Último cuadro del visualizador.
    pub(super) cava_frame: Vec<f32>,
    /// Árbitro del **diente vivo** (música/volumen/CPU/batería/reposo).
    pub(super) atencion: pata_core::atencion::Atencion,
    /// Reloj monotónico del diente vivo (origen para `elapsed()`).
    pub(super) diente_t0: std::time::Instant,
    /// Última lectura de batería `(fracción 0..1, cargando)`.
    pub(super) bat_now: Option<(f32, bool)>,
    /// Última temperatura de CPU (°C), o `None` si no hay sensor.
    pub(super) cpu_temp: Option<f32>,
    /// Manifestación actual del diente vivo.
    pub(super) diente_manifest: pata_core::atencion::Manifestacion,
    /// Inventario de flota (matilda), read-only, para el diente «Flota».
    pub(super) flota: Option<matilda_core::Inventory>,
    /// Discover remoto de la flota (SSH read-only) en su hilo.
    pub(super) flota_discover: Option<crate::flota_discover::FlotaDiscoverHandle>,
    /// Último estado real observado por host.
    pub(super) flota_remoto: Option<Vec<crate::flota_discover::HostObs>>,
    /// Feed de unidades del plano de control (sandokan).
    pub(super) unidades: Option<crate::unidades::UnidadesHandle>,
    /// Último snapshot de unidades.
    pub(super) unidades_now: Option<sandokan_monitor_core::MonitorSnapshot>,
    pub(super) theme: Theme,
    pub(super) cfg: Config,
    pub(super) surfaces: Vec<crate::SurfaceWidgets>,
    pub(super) shuma: crate::shuma::ShumaState,
    /// Live-wire (`PATA_SHUMA_FULL`): la shuma COMPLETA hospedada (dientes/
    /// sesiones). `None` = path bare por defecto.
    pub(super) shuma_full: Option<crate::shuma_app::Model>,
    /// Handle channel-backed para los efectos/`update` de la shuma completa: sus
    /// `Msg` (ticks, async) caen en `shuma_full_rx`, drenados cada frame.
    pub(super) shuma_full_handle: Option<llimphi_ui::Handle<crate::shuma_app::Msg>>,
    /// Cola de `Msg` de la shuma completa, alimentada por su handle desde hilos
    /// de fondo (ticks, contenedores, explorer…). Se drena en `draw`.
    pub(super) shuma_full_rx: Option<Receiver<crate::shuma_app::Msg>>,
    /// Vigía del `launcher.toml` para recargar el contenido del dock.
    pub(super) cfg_watch: crate::config_watch::ConfigWatch,
    /// Índice (en `panels`) de la barra que hospeda el `shuma_input`.
    pub(super) shuma_panel: Option<usize>,
    /// Grosor original (px) de esa barra.
    pub(super) shuma_bar_px: u32,
    /// Último valor visto de `WawaConfig.dientes_outside` (posición del rail); si
    /// cambia, re-exec.
    pub(super) dientes_outside: bool,
    /// Último valor visto de `WawaConfig.sidebar_docked` (reserva de franja); si
    /// cambia, re-exec (cambia el `exclusive_zone` de los sidebars).
    pub(super) sidebar_docked: bool,
    /// Registro de apps para el menú de inicio.
    pub(super) registry: app_bus::AppRegistry,
    /// `true` cuando el drawer de la barra del menú está desplegado.
    pub(super) menu_open: bool,
    /// Cuándo se abrió el menú. El menú toma el teclado (Exclusive) al abrir, y
    /// el compositor reacomoda el foco en ese instante (p.ej. el fallback «teclado
    /// al shell en escritorio vacío»): eso le manda un `leave` espurio al panel del
    /// menú que, sin guarda, lo cerraría de inmediato. Ignoramos el `leave`-cierre
    /// durante [`MENU_LEAVE_GRACE`] tras abrir; un `leave` legítimo (clic en una
    /// ventana) llega mucho después.
    pub(super) menu_opened_at: Option<std::time::Instant>,
    /// Cuándo se abrió el drawer de shuma — misma guarda anti-churn que
    /// `menu_opened_at`: al abrir, el drawer toma el teclado (Exclusive) y el
    /// compositor reacomoda el foco/puntero; ignoramos el `leave`-cierre por
    /// hover durante [`MENU_LEAVE_GRACE`] para no togglear apenas se abre.
    pub(super) shuma_opened_at: Option<std::time::Instant>,
    /// Categoría activa del menú de inicio (índice en la lista de categorías):
    /// sus apps se muestran en el panel derecho. `None` = la primera. La fija el
    /// hover sobre la columna de categorías (`Msg::MenuHoverCategory`).
    pub(super) menu_cat: Option<usize>,
    /// Qué cuerpo muestra el drawer desplegado.
    pub(super) menu_kind: MenuKind,
    /// Historial de copias (más reciente al frente, sin repetidos, tope 16).
    pub(super) clip_history: Vec<String>,
    /// Borrador de fecha/hora que el panel del reloj edita.
    pub(super) clock_draft: crate::ClockDraft,
    /// Texto del buscador del menú de inicio.
    pub(super) menu_query: String,
    /// Desplazamiento de la lista del menú (px).
    pub(super) menu_scroll: f32,
    /// Índice (en `panels`) de la barra que hospeda el `start_button`.
    pub(super) menu_panel: Option<usize>,
    /// Grosor original (px) de esa barra.
    pub(super) menu_bar_px: u32,
    /// Muestreador del sistema en su propio hilo.
    pub(super) sampler: SamplerHandle,
    /// Último snapshot del sistema recogido del hilo de muestreo.
    pub(super) ctx: WidgetCtx,
    /// Lecturas extra del control panel (batería/wifi/bt), refrescadas al abrirlo.
    pub(super) control_extras: crate::render::ControlExtras,
    /// Estado del sidebar navegador.
    pub(super) nav: NavState,
    /// Estado del sidebar RAG (preguntale a tu correo).
    pub(super) rag: crate::rag::RagState,
    /// Sender que las consultas RAG (y el armado del motor) usan para devolver su
    /// `Msg` al loop; se drena por `rag_rx` cada frame.
    pub(super) rag_tx: Sender<Msg>,
    /// Canal por donde llegan los resultados del motor RAG (respuesta/error/listo).
    pub(super) rag_rx: Receiver<Msg>,
    /// Canal por donde el hilo de poll de `list_monads` entrega resultados.
    pub(super) nav_rx: Option<Receiver<PollOutcome>>,
    /// Canal para que los hilos one-shot de `resolve_monad` entreguen miembros.
    pub(super) members_tx: Sender<MembersOutcome>,
    pub(super) members_rx: Receiver<MembersOutcome>,
    /// Animación del switcher en curso (resaltado viajando entre escritorios).
    pub(super) ws_anim: Option<WsAnimState>,
    /// Último escritorio activo visto (para detectar el cambio que dispara la
    /// animación). `0` = aún sin dato.
    pub(super) ws_last_active: u8,
    /// Realce **optimista** del switcher: `(target_1based, ticks)`. Al clickear
    /// una celda el activo salta ya; se sostiene unos samples por si uno viejo
    /// (tomado antes de que el WM aplicara el salto) reportara el escritorio
    /// anterior y parpadeara. Ver [`crate::sampler::reconcile_optimistic`].
    pub(super) pending_ws: Option<(u8, u8)>,
    /// Arrastre en curso.
    pub(super) drag: Option<LayerDrag>,
    /// `on_click` plano armado en el press, pendiente de soltar (semántica de
    /// escritorio: el click se dispara al RELEASE sobre el mismo punto, no en el
    /// mousedown). Se cancela si el puntero se aleja más de [`CLICK_MOVE_CANCEL`]
    /// del origen. `(panel, msg, origen)`.
    pub(super) pending_click: Option<(usize, Msg, (f32, f32))>,
    /// Servidor del rail hospedado.
    pub(super) host: Option<HostServer>,
    /// Última revisión vista del `host`.
    pub(super) last_host_rev: u64,
    /// Una layer surface por cada barra de la config.
    pub(super) panels: Vec<Panel>,
    /// Estado del compositor Wayland, retenido para poder crear surfaces NUEVAS en
    /// runtime (el drawer del sidebar) — no sólo en el arranque.
    pub(super) compositor: Option<CompositorState>,
    /// El `wlr-layer-shell`, retenido por el mismo motivo que `compositor`.
    pub(super) layer_shell: Option<LayerShell>,
    /// Índice (en `panels`) del **drawer** del sidebar vivo, si hay uno desplegado.
    /// El drawer es una layer surface aparte del rail, creada al abrir un diente y
    /// destruida al cerrarlo — así el panel NUNCA redimensiona una surface (lo que
    /// falla en Iris Xe), es de tamaño fijo. Siempre es el ÚLTIMO panel de `panels`.
    pub(super) drawer_pi: Option<usize>,
    /// Índice de superficie (`si`) del sidebar cuyo drawer está vivo. Sirve para
    /// reconciliar: si `nav.open` cambió de sidebar, se recrea el drawer.
    pub(super) drawer_si: Option<usize>,
    /// Índice (en `panels`) de la surface del **tooltip flotante**.
    pub(super) tooltip_pi: Option<usize>,
    /// Texto del tooltip actualmente visible.
    pub(super) tooltip_text: Option<String>,
    /// Modificadores activos del teclado.
    pub(super) mods: Modifiers,
    pub(super) exit: bool,
}

/// El anclaje sctk + el tamaño `(w, h)` pedido para un borde y grosor.
fn anchor_y_size(anchor: Anchor, thickness: u32) -> (LayerAnchor, (u32, u32)) {
    match anchor {
        Anchor::Top => (
            LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT,
            (0, thickness),
        ),
        Anchor::Bottom => (
            LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT,
            (0, thickness),
        ),
        Anchor::Left => (
            LayerAnchor::LEFT | LayerAnchor::TOP | LayerAnchor::BOTTOM,
            (thickness, 0),
        ),
        Anchor::Right => (
            LayerAnchor::RIGHT | LayerAnchor::TOP | LayerAnchor::BOTTOM,
            (thickness, 0),
        ),
    }
}

/// Levanta el backend layer-shell. Devuelve error si no hay sesión Wayland o el
/// compositor no expone `wlr-layer-shell`.
pub fn run() -> Result<(), Box<dyn Error>> {
    rimay_localize::init();
    let _ = rimay_localize::set_locale(&wawa_config::WawaConfig::load().lang);
    let cfg = pata_config::load();
    let mut theme = Theme::dark();
    if let Some(c) = crate::render::parse_hex(&cfg.general.accent) {
        theme.accent = c;
    }
    let bars: Vec<usize> = cfg
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, s)| s.enabled && s.kind == SurfaceKind::Bar)
        .map(|(i, _)| i)
        .collect();
    let sidebars: Vec<usize> = cfg
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, s)| s.enabled && s.kind == SurfaceKind::Sidebar)
        .map(|(i, _)| i)
        .collect();
    let docks: Vec<usize> = cfg
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, s)| s.enabled && s.kind == SurfaceKind::Dock)
        .map(|(i, _)| i)
        .collect();
    let backgrounds: Vec<usize> = cfg
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, s)| s.enabled && s.kind == SurfaceKind::Background)
        .map(|(i, _)| i)
        .collect();
    if bars.is_empty() && sidebars.is_empty() && docks.is_empty() && backgrounds.is_empty() {
        return Err("pata · la config no tiene ninguna superficie anclable (bar/sidebar/dock/fondo)".into());
    }
    diag!(
        "pata diag · backend LAYER-SHELL arranca · {} barra(s) + {} sidebar(s) + {} dock(s)",
        bars.len(),
        sidebars.len(),
        docks.len()
    );

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh: QueueHandle<LayerApp> = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;

    let toplevel_mgr = globals
        .bind::<ZwlrForeignToplevelManagerV1, _, _>(&qh, 1..=3, ())
        .ok();
    if toplevel_mgr.is_none() {
        eprintln!("pata layer · el compositor no expone wlr-foreign-toplevel; window_list vacío");
    }

    // Inactividad del sistema (idle inteligente de energía). Si el compositor no
    // lo expone, el idle queda inactivo (no es fatal: pata sigue como barra).
    let idle_notifier = globals
        .bind::<ExtIdleNotifierV1, _, _>(&qh, 1..=2, ())
        .ok();
    if idle_notifier.is_none() {
        eprintln!("pata layer · el compositor no expone ext-idle-notify; idle de energía inactivo");
    }
    // idle-inhibit (para el «mantener despierto»). Opcional: si no está, el café
    // igual inhibe la suspensión de pata, pero no el apagado de pantalla.
    let idle_inhibit_mgr = globals
        .bind::<ZwpIdleInhibitManagerV1, _, _>(&qh, 1..=1, ())
        .ok();

    let tray = crate::config_tiene_widget(&cfg, "tray")
        .then(TrayHandle::spawn)
        .flatten();
    let weather = crate::config_tiene_widget(&cfg, "weather")
        .then(|| crate::weather::WeatherHandle::spawn(crate::weather_place(&cfg)));
    let network = (crate::config_tiene_widget(&cfg, "network")
        || crate::config_tiene_widget(&cfg, "wifi"))
    .then(crate::network::NetworkHandle::spawn);
    let mpris = (crate::config_tiene_widget(&cfg, "mpris")
        || crate::config_tiene_widget(&cfg, "media_player"))
    .then(crate::mpris::MprisHandle::spawn);
    let bluetooth = (crate::config_tiene_widget(&cfg, "bluetooth")
        || crate::config_tiene_widget(&cfg, "bt"))
    .then(crate::bluetooth::BluetoothHandle::spawn);
    let notifications = (crate::config_tiene_widget(&cfg, "notifications")
        || crate::config_tiene_widget(&cfg, "notify"))
    .then(crate::notifications::NotificationsHandle::spawn)
    .flatten();
    // El agente polkit no es un widget: pata es el shell de la sesión, así que
    // registra el agente siempre (si ya hay otro, el registro falla y se loguea).
    let polkit = crate::polkit::PolkitHandle::spawn();
    let cava = crate::config_tiene_widget(&cfg, "cava")
        .then(|| crate::cava::CavaHandle::spawn(crate::cava_bars(&cfg)));
    let flota = crate::config_tiene_flota(&cfg).then(crate::load_flota).flatten();
    let flota_discover = flota.as_ref().and_then(|inv| {
        let hosts: Vec<crate::flota_discover::HostConn> = inv
            .hosts()
            .map(|h| crate::flota_discover::HostConn {
                name: h.name.clone(),
                address: h.address.clone(),
                user: h.ssh_user().to_string(),
                port: h.ssh_port(),
            })
            .collect();
        let units: Vec<String> = inv.services().map(|s| s.unit.clone()).collect();
        (!hosts.is_empty())
            .then(|| crate::flota_discover::FlotaDiscoverHandle::spawn(hosts, units))
    });
    let unidades = crate::config_tiene_unidades(&cfg).then(crate::unidades::UnidadesHandle::spawn);

    let nav_rx = crate::config_tiene_navigator(&cfg).then(|| {
        let (tx, rx) = std::sync::mpsc::channel::<PollOutcome>();
        std::thread::spawn(move || {
            let mut socket = None;
            loop {
                let outcome = nouser::poll(socket.clone());
                socket = match &outcome {
                    PollOutcome::Ok { socket: s, .. } => Some(s.clone()),
                    PollOutcome::Failed(_) => None,
                };
                if tx.send(outcome).is_err() {
                    break;
                }
                std::thread::sleep(nouser::REFRESH_INTERVAL);
            }
        });
        rx
    });
    let (members_tx, members_rx) = std::sync::mpsc::channel::<MembersOutcome>();

    // Sidebar RAG: igual modelo channel-backed que el resto del path layer. El
    // motor (pesado: daemon + caché de paloma + LLM) se arma en un hilo aparte y
    // sus resultados —y el aviso de «listo»— caen en `rag_rx`, drenado cada frame.
    let rag_present = crate::config_tiene_rag(&cfg);
    let (rag_tx, rag_rx) = std::sync::mpsc::channel::<Msg>();
    let rag = if rag_present {
        crate::rag::RagState::presente()
    } else {
        crate::rag::RagState::default()
    };
    if rag_present {
        let slot = rag.engine.clone();
        let tx = rag_tx.clone();
        let source = crate::rag_source(&cfg);
        std::thread::spawn(move || {
            // willay (eventos) o paloma (correo, default), ambos `dyn RagMotor`.
            let engine: Option<Box<dyn rag_motor::RagMotor>> = match source.as_str() {
                "willay" | "eventos" => willay_rag::Engine::try_build()
                    .map(|e| Box::new(e) as Box<dyn rag_motor::RagMotor>),
                _ => paloma_rag::RagEngine::try_build()
                    .map(|e| Box::new(e) as Box<dyn rag_motor::RagMotor>),
            };
            let (ok, corpus) = match &engine {
                Some(e) => (true, e.corpus_len()),
                None => (false, 0),
            };
            if let Ok(mut g) = slot.lock() {
                *g = engine;
            }
            let _ = tx.send(Msg::RagEngineReady { ok, corpus });
        });
    }

    let (surfaces, shuma) = Model::construir(&cfg);

    // Live-wire de la shuma COMPLETA (opt-in). El loop smithay no tiene un
    // `Handle<Msg>` de llimphi; fabricamos uno **channel-backed**: un handle
    // lifteado sobre un `for_test` cuyo `lift` empuja cada `Msg` a un canal. Los
    // efectos de la shuma (ticks/async en hilos de fondo) y los follow-ups de su
    // `update` caen en `shuma_full_rx`, que `draw` drena cada frame (el loop de
    // frames de pata se auto-sostiene, así que la shuma avanza ~vsync).
    let (shuma_full, shuma_full_handle, shuma_full_rx) =
        if crate::shuma_full_enabled() && shuma.present {
            let (tx, rx) = std::sync::mpsc::channel::<crate::shuma_app::Msg>();
            let tx = std::sync::Mutex::new(tx);
            let handle: llimphi_ui::Handle<crate::shuma_app::Msg> =
                llimphi_ui::Handle::<()>::for_test().lift(move |m: crate::shuma_app::Msg| {
                    let _ = tx.lock().unwrap().send(m);
                });
            let mut full = crate::shuma_app::new();
            // lift identidad: el handle ya es `Handle<shuma_app::Msg>`.
            crate::shuma_app::wire_effects(&mut full, &handle, |m| m);
            (Some(full), Some(handle), Some(rx))
        } else {
            (None, None, None)
        };

    let utc = crate::usa_utc(&cfg);
    // Decisiones GLOBALes (WawaConfig), dos ejes independientes: `dientes_outside`
    // = POSICIÓN del rail (visual, adentro/afuera); `sidebar_docked` = si el
    // sidebar RESERVA franja (`exclusive_zone`). Las leemos una vez; si cambian en
    // runtime, `maybe_sample` re-ejecuta pata para reanclar.
    let wcfg = wawa_config::WawaConfig::load();
    let dientes_outside = wcfg.dientes_outside;
    let sidebar_docked = wcfg.sidebar_docked;
    let mut app = LayerApp {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        conn,
        hal: None,
        keyboard: None,
        pointer: None,
        seat: None,
        toplevel_mgr,
        idle_notifier,
        idle_notif: None,
        energia_cfg: crate::energia::ConfigEnergia::from_core(&cfg.general.energia),
        energia_disparado: false,
        energia_pospuesto: false,
        idle_inhibit_mgr,
        idle_inhibitor: None,
        toplevels: Vec::new(),
        task_order: Vec::new(),
        task_drag: None,
        next_toplevel_id: 0,
        clipboard: None,
        tray,
        weather,
        weather_now: None,
        network,
        network_now: None,
        sink_inputs: Vec::new(),
        sinks: Vec::new(),
        net_password: None,
        session_confirm: None,
        mpris,
        media_now: None,
        bluetooth,
        bluetooth_now: None,
        notifications,
        bat_avisado: 0,
        polkit,
        polkit_prompt: None,
        polkit_input: String::new(),
        osd_pi: None,
        osd: None,
        cava,
        cava_frame: Vec::new(),
        atencion: pata_core::atencion::Atencion::new(),
        diente_t0: std::time::Instant::now(),
        bat_now: None,
        cpu_temp: None,
        diente_manifest: pata_core::atencion::Manifestacion::Reposo,
        flota,
        flota_discover,
        flota_remoto: None,
        unidades,
        unidades_now: None,
        theme,
        cfg,
        surfaces,
        shuma,
        shuma_full,
        shuma_full_handle,
        shuma_full_rx,
        cfg_watch: crate::config_watch::ConfigWatch::new(pata_config::loaded_path()),
        shuma_panel: None,
        shuma_bar_px: 40,
        dientes_outside,
        sidebar_docked,
        registry: app_bus::AppRegistry::discover_merged(),
        menu_open: false,
        menu_opened_at: None,
        shuma_opened_at: None,
        menu_cat: None,
        menu_kind: MenuKind::Apps,
        clip_history: Vec::new(),
        clock_draft: crate::ClockDraft::default(),
        menu_query: String::new(),
        menu_scroll: 0.0,
        menu_panel: None,
        menu_bar_px: 32,
        sampler: SamplerHandle::spawn(utc),
        ctx: WidgetCtx::default(),
        control_extras: crate::render::ControlExtras::default(),
        nav: NavState::default(),
        nav_rx,
        members_tx,
        members_rx,
        rag,
        rag_tx,
        rag_rx,
        ws_anim: None,
        ws_last_active: 0,
        pending_ws: None,
        drag: None,
        pending_click: None,
        host: (!sidebars.is_empty()).then(HostServer::spawn).flatten(),
        last_host_rev: 0,
        panels: Vec::new(),
        compositor: None,
        layer_shell: None,
        drawer_pi: None,
        drawer_si: None,
        tooltip_pi: None,
        tooltip_text: None,
        mods: Modifiers::default(),
        exit: false,
    };

    // Roundtrip para que `OutputState` reciba `wl_output.geometry`.
    event_queue.roundtrip(&mut app)?;
    event_queue.roundtrip(&mut app)?;

    // Mapa `nombre del conector → wl_output`.
    let mut outputs_by_name: std::collections::HashMap<String, wl_output::WlOutput> =
        std::collections::HashMap::new();
    for out in app.output_state.outputs() {
        if let Some(info) = app.output_state.info(&out) {
            if let Some(name) = info.name {
                outputs_by_name.insert(name, out);
            }
        }
    }
    diag!("pata diag · outputs descubiertos: {:?}", outputs_by_name.keys().collect::<Vec<_>>());

    let resolve_output =
        |name: &str| -> Option<wl_output::WlOutput> {
            // Vacío o el comodín `"*"`/`"all"` → primario (None). El comodín cae
            // acá sólo en los paths de una sola surface (tarjetas flotantes), no
            // es un nombre de conector — no se loguea como «no conectado».
            if name.is_empty() || name == "*" || name.eq_ignore_ascii_case("all") {
                return None;
            }
            if let Some(o) = outputs_by_name.get(name) {
                return Some(o.clone());
            }
            eprintln!("pata layer · output «{name}» no conectado; cae al primario");
            None
        };

    // Los monitores destino de una superficie: `output = "*"`/`"all"` la replica
    // en CADA monitor conectado MENOS los de `exclude`; si no, su monitor (o el
    // primario). El default de `output` es `"*"`, así que sin config una barra va
    // a todas las pantallas.
    let targets_de = |out: &str, exclude: &[String]| -> Vec<Option<wl_output::WlOutput>> {
        if (out == "*" || out.eq_ignore_ascii_case("all")) && !outputs_by_name.is_empty() {
            // Si la exclusión vacía la lista (excluyeron todos), no se crea
            // ninguna surface: la barra simplemente no aparece, que es lo pedido.
            outputs_by_name
                .iter()
                .filter(|(name, _)| !exclude.iter().any(|ex| ex.eq_ignore_ascii_case(name)))
                .map(|(_, o)| Some(o.clone()))
                .collect()
        } else {
            vec![resolve_output(out)]
        }
    };

    // Una layer surface por barra (× monitor si `output = "*"`).
    for &idx in &bars {
        let s = &app.cfg.surfaces[idx];
        let thickness = s.thickness.max(1.0) as u32;
        let (sctk_anchor, size) = anchor_y_size(s.anchor, thickness);
        for target in targets_de(&s.output, &s.exclude_outputs) {
            let wl_surface = compositor.create_surface(&qh);
            let layer = layer_shell.create_layer_surface(
                &qh,
                wl_surface,
                Layer::Top,
                Some("pata".to_string()),
                target.as_ref(),
            );
            layer.set_anchor(sctk_anchor);
            layer.set_size(size.0, size.1);
            layer.set_exclusive_zone(thickness as i32);
            layer.commit();
            app.panels.push(Panel {
                idx,
                card: None,
                drawer: false,
                output: target.clone(),
                layer,
                cache: None,
                width: size.0.max(1),
                height: thickness,
                dirty: true,
                hover_idx: None,
                cursor_x: None,
                gpu: None,
            });
        }
    }

    // Una layer surface por sidebar (× monitor si `output = "*"`).
    for &idx in &sidebars {
        let s = &app.cfg.surfaces[idx];
        let thickness = s.thickness.max(1.0) as u32;
        let (sctk_anchor, size) = anchor_y_size(s.anchor, thickness);
        for target in targets_de(&s.output, &s.exclude_outputs) {
            let wl_surface = compositor.create_surface(&qh);
            let layer = layer_shell.create_layer_surface(
                &qh,
                wl_surface,
                Layer::Top,
                Some("pata-sidebar".to_string()),
                target.as_ref(),
            );
            layer.set_anchor(sctk_anchor);
            layer.set_size(size.0, size.1);
            // Reserva franja si el sidebar está DOCKED (`reserve` por-superficie
            // pisa el global `sidebar_docked`) y no es autohide; si no, flota como
            // overlay. Es el eje independiente de la posición del rail — mismo
            // criterio que `pata_core::layout::resolve`.
            let docked = s.reserve.unwrap_or(sidebar_docked);
            let excl = if docked && !s.autohide { thickness as i32 } else { 0 };
            layer.set_exclusive_zone(excl);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();
            app.panels.push(Panel {
                idx,
                card: None,
                drawer: false,
                output: target.clone(),
                layer,
                cache: None,
                width: thickness,
                height: size.1.max(1),
                dirty: true,
                hover_idx: None,
                cursor_x: None,
                gpu: None,
            });
        }
    }

    // Una layer surface por **dock** (estilo macOS): como una barra (anclada a
    // su borde, ancho completo) pero SIN zona exclusiva — flota sobre las
    // ventanas en vez de reservar su franja, y el `dock_view` centra sus íconos.
    for &idx in &docks {
        let s = &app.cfg.surfaces[idx];
        let thickness = s.thickness.max(1.0) as u32;
        let (sctk_anchor, size) = anchor_y_size(s.anchor, thickness);
        for target in targets_de(&s.output, &s.exclude_outputs) {
            let wl_surface = compositor.create_surface(&qh);
            let layer = layer_shell.create_layer_surface(
                &qh,
                wl_surface,
                Layer::Top,
                Some("pata-dock".to_string()),
                target.as_ref(),
            );
            layer.set_anchor(sctk_anchor);
            layer.set_size(size.0, size.1);
            layer.set_exclusive_zone(0); // un dock no reserva espacio: flota.
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();
            app.panels.push(Panel {
                idx,
                card: None,
                drawer: false,
                output: target.clone(),
                layer,
                cache: None,
                width: size.0.max(1),
                height: thickness,
                dirty: true,
                hover_idx: None,
                cursor_x: None,
                gpu: None,
            });
        }
    }

    // Una layer surface por **fondo** de escritorio (capa Background):
    // detrás de las ventanas, anclada a los 4 bordes y de
    // tamaño 0 → el compositor la estira a la salida completa; `configure`
    // reporta el tamaño real. Sin zona exclusiva ni teclado.
    for &idx in &backgrounds {
        let s = &app.cfg.surfaces[idx];
        for target in targets_de(&s.output, &s.exclude_outputs) {
            let wl_surface = compositor.create_surface(&qh);
            let layer = layer_shell.create_layer_surface(
                &qh,
                wl_surface,
                Layer::Background,
                Some("pata-fondo".to_string()),
                target.as_ref(),
            );
            layer.set_anchor(
                LayerAnchor::TOP | LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT,
            );
            layer.set_size(0, 0); // anclado a los 4 bordes → llena la salida.
            layer.set_exclusive_zone(0);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();
            app.panels.push(Panel {
                idx,
                card: None,
                drawer: false,
                output: target.clone(),
                layer,
                cache: None,
                width: 1,
                height: 1,
                dirty: true,
                hover_idx: None,
                cursor_x: None,
                gpu: None,
            });
        }
    }

    // Tarjetas flotantes (estilo conky).
    for (idx, s) in app.cfg.surfaces.iter().enumerate() {
        if !s.enabled || s.kind != SurfaceKind::Panel {
            continue;
        }
        let panel_output = resolve_output(&s.output);
        for card in &s.cards {
            let (cw, ch) = (card.w.max(1.0) as u32, card.h.max(1.0) as u32);
            let wl_surface = compositor.create_surface(&qh);
            let layer = layer_shell.create_layer_surface(
                &qh,
                wl_surface,
                Layer::Bottom,
                Some("pata-card".to_string()),
                panel_output.as_ref(),
            );
            layer.set_anchor(LayerAnchor::TOP | LayerAnchor::LEFT);
            layer.set_size(cw, ch);
            layer.set_margin(card.y as i32, 0, 0, card.x as i32);
            layer.set_exclusive_zone(0);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();
            let widgets = card.widgets.iter().map(pata_core::widget::build).collect();
            app.panels.push(Panel {
                idx,
                card: Some(CardState { spec: card.clone(), widgets }),
                drawer: false,
                output: panel_output.clone(),
                layer,
                cache: None,
                width: cw,
                height: ch,
                dirty: true,
                hover_idx: None,
            cursor_x: None,
                gpu: None,
            });
        }
    }

    // ¿Qué barra hospeda el shuma_input?
    app.shuma_panel = app.panels.iter().position(|p| {
        let s = &app.cfg.surfaces[p.idx];
        s.start
            .iter()
            .chain(&s.center)
            .chain(&s.end)
            .any(|w| w.kind == "shuma_input")
    });
    app.shuma_bar_px = app
        .shuma_panel
        .map(|pi| app.cfg.surfaces[app.panels[pi].idx].thickness.max(1.0) as u32)
        .unwrap_or(40);
    if let Some(pi) = app.shuma_panel {
        // `OnDemand` (no `None`): con el drawer plegado la barra igual puede
        // reclamar el teclado. mirada lo enruta al shell-layer cuando el
        // escritorio está vacío (keyboard_fallback_target), así shuma agarra el
        // teclado en workspaces sin ventanas y podés tipear sin clickear.
        app.panels[pi]
            .layer
            .set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        app.panels[pi].layer.commit();
    }

    // La surface del tooltip flotante.
    app.tooltip_pi = {
        let wl_surface = compositor.create_surface(&qh);
        if let Ok(region) = Region::new(&compositor) {
            wl_surface.set_input_region(Some(region.wl_region()));
        }
        let layer = layer_shell.create_layer_surface(
            &qh,
            wl_surface,
            Layer::Overlay,
            Some("pata-tooltip".to_string()),
            None,
        );
        layer.set_anchor(LayerAnchor::TOP | LayerAnchor::LEFT);
        layer.set_size(1, 1);
        layer.set_margin(0, 0, 0, 0);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        app.panels.push(Panel {
            idx: 0,
            card: None,
            drawer: false,
            output: None,
            layer,
            cache: None,
            width: 1,
            height: 1,
            dirty: false,
            hover_idx: None,
            cursor_x: None,
            gpu: None,
        });
        Some(app.panels.len() - 1)
    };

    // La surface del OSD (cartel de volumen/brillo): Overlay anclado abajo,
    // centrada horizontalmente. Arranca 1×1 y crece al dispararse.
    app.osd_pi = {
        let wl_surface = compositor.create_surface(&qh);
        if let Ok(region) = Region::new(&compositor) {
            wl_surface.set_input_region(Some(region.wl_region()));
        }
        let layer = layer_shell.create_layer_surface(
            &qh,
            wl_surface,
            Layer::Overlay,
            Some("pata-osd".to_string()),
            None,
        );
        layer.set_anchor(LayerAnchor::BOTTOM);
        layer.set_size(1, 1);
        layer.set_margin(0, 0, 80, 0);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        app.panels.push(Panel {
            idx: 0,
            card: None,
            drawer: false,
            output: None,
            layer,
            cache: None,
            width: 1,
            height: 1,
            dirty: false,
            hover_idx: None,
            cursor_x: None,
            gpu: None,
        });
        Some(app.panels.len() - 1)
    };

    // ¿Qué barra hospeda el menú de inicio? La del `start_button` o, en CDE, la
    // del `front_panel` (su botón ☰ «Gestor de aplicaciones» abre el mismo menú).
    app.menu_panel = app.panels.iter().position(|p| {
        let s = &app.cfg.surfaces[p.idx];
        s.start
            .iter()
            .chain(&s.center)
            .chain(&s.end)
            .any(|w| w.kind == "start_button" || w.kind == "front_panel")
    });
    app.menu_bar_px = app
        .menu_panel
        .map(|pi| app.cfg.surfaces[app.panels[pi].idx].thickness.max(1.0) as u32)
        .unwrap_or(32);

    // Retenemos `compositor` y `layer_shell` en `app` para crear el drawer del
    // sidebar en runtime (ya no se usan más en el arranque).
    app.compositor = Some(compositor);
    app.layer_shell = Some(layer_shell);

    while !app.exit {
        if let Err(e) = event_queue.blocking_dispatch(&mut app) {
            eprintln!("pata layer · el compositor cerró la conexión: {e}");
            break;
        }
    }
    Ok(())
}

delegate_compositor!(LayerApp);
delegate_output!(LayerApp);
delegate_layer!(LayerApp);
delegate_seat!(LayerApp);
delegate_keyboard!(LayerApp);
delegate_pointer!(LayerApp);
delegate_registry!(LayerApp);
