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
//! **Estado**: pinta todas las barras de la config (varios bordes a la vez),
//! con input (teclado/clicks), drawer Quake, window_list, clipboard y tray.
//! Verificado en Hyprland. No se verifica headless: se itera en un compositor
//! real.
//!
//! **Gotcha Vulkan WSI + smithay (mirada):** NO reconfigurar el swapchain por
//! cuadro. [`Self::draw`] llama a `surface.resize(w, h)` cada frame; por eso
//! `RawSurface::resize` es no-op cuando el tamaño no cambia. Reconfigurar el
//! swapchain reconstruye el `wl_buffer` y destruye el recién presentado antes de
//! que el compositor lo componga — wlroots lo tolera, smithay no (la barra queda
//! negra, el compositor ve `buffer=None`). Ver commit `b8747b90`.

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
/// Para depurar el camino layer-shell en hardware sin recompilar A/B:
/// `PATA_DIAG=1 pata-llimphi 2>&1 | tee /tmp/pata.log`.
macro_rules! diag {
    ($($a:tt)*) => {
        if std::env::var_os("PATA_DIAG").is_some() {
            eprintln!($($a)*);
        }
    };
}

/// El estado wgpu de **una** layer surface (una barra). El `Hal` (instancia +
/// device de wgpu) se comparte entre todas las barras, en [`LayerApp::hal`].
struct PanelGpu {
    surface: RawSurface,
    renderer: Renderer,
    typesetter: Typesetter,
    scene: vello::Scene,
    layout: LayoutTree,
}

/// El árbol pintado en el último frame de un panel, para hacer hit-test de los
/// clicks (qué nodo está bajo el puntero y qué `on_click` dispara).
struct RenderCache {
    mounted: Mounted<Msg>,
    computed: ComputedLayout,
}

/// Un arrastre en curso sobre un nodo arrastrable (p. ej. un nodo del grafo del
/// navegador, que selecciona al soltar). El backend layer-shell rastrea el press→
/// move→release mínimo para invocar el handler `draggable` del nodo —el bucle
/// winit lo hace nativo; acá lo replicamos a mano para que el modo grafo
/// seleccione también bajo Wayland.
struct LayerDrag {
    /// El handler del nodo: `Fn(DragPhase, dx, dy) -> Option<Msg>`.
    handler: DragFn<Msg>,
    /// Última posición del puntero, para el delta de cada `Move`.
    last: (f32, f32),
}

/// El estado de una tarjeta flotante (estilo conky) montada como su propia layer
/// surface: la spec (título/tamaño) + sus widgets vivos. La diferencia con una
/// barra es que vive en `Layer::Bottom` sobre el escritorio, no reserva franja y
/// no toma teclado.
struct CardState {
    spec: FloatingCard,
    widgets: Vec<Box<dyn Widget>>,
}

/// Una layer surface de pata: o una **barra** anclada a un borde (con sus tres
/// slots), o una **tarjeta flotante** (`card`). En ambos casos lleva su propio
/// estado wgpu y su cache de hit-test.
struct Panel {
    /// Índice de su superficie en `cfg.surfaces` (la barra, o el `Panel` dueño de
    /// la tarjeta).
    idx: usize,
    /// `Some` si esta surface es una tarjeta flotante; `None` si es una barra.
    card: Option<CardState>,
    layer: LayerSurface,
    /// El árbol del último frame (para hit-test de clicks).
    cache: Option<RenderCache>,
    width: u32,
    height: u32,
    /// `true` cuando hay algo nuevo que pintar (cambió el muestreo o el tamaño).
    dirty: bool,
    /// Nodo bajo el puntero en este panel (para `hover_fill` y, a futuro,
    /// tooltips). `None` si el puntero no está sobre ningún nodo hovereable.
    hover_idx: Option<usize>,
    gpu: Option<PanelGpu>,
}

/// Alto del drawer Quake cuando se despliega (px). El compositor lo clampa a la
/// salida; la barra crece hacia arriba hasta este alto.
const DRAWER_H: u32 = 420;

/// Alto de la barra superior cuando despliega el menú de inicio (px): crece hacia
/// abajo hasta este alto, manteniendo su exclusive zone en el grosor de la barra.
const MENU_H: u32 = 480;

/// Qué cuerpo muestra el drawer que crece de la barra del `start_button`: el
/// menú de apps, o un popup de widget que reusa el mismo crecimiento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum MenuKind {
    /// El menú de inicio (lista de apps con buscador, toma el teclado).
    #[default]
    Apps,
    /// El historial de portapapeles (lista de copias, sólo clicks).
    Clipboard,
    /// El panel del reloj (spinners de fecha/hora, sólo clicks).
    Clock,
}

/// El cliente Wayland del backend layer-shell.
struct LayerApp {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    conn: Connection,
    /// `Hal` compartido (una instancia/device de wgpu para todas las barras).
    hal: Option<Hal>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    /// El seat (para activar ventanas: `activate(seat)` lo exige).
    seat: Option<wl_seat::WlSeat>,
    /// El manager de wlr-foreign-toplevel, si el compositor lo expone. `None` en
    /// compositores sin el protocolo: el `window_list` queda vacío, sin romper.
    /// Se guarda para mantener vivo el binding (de él cuelgan los eventos de cada
    /// toplevel), aunque no se vuelva a leer.
    #[allow(dead_code)]
    toplevel_mgr: Option<ZwlrForeignToplevelManagerV1>,
    /// Las ventanas abiertas que reporta el compositor.
    toplevels: Vec<Toplevel>,
    /// Contador para asignar [`Toplevel::id`] estables.
    next_toplevel_id: u32,
    /// Texto del portapapeles (una línea), para el widget `clipboard`. Se
    /// re-muestrea con el resto del sistema (~1Hz) vía `wl-paste`.
    clipboard: Option<String>,
    /// La bandeja del sistema (StatusNotifierItem), corriendo en su propio hilo.
    /// `None` si la config no tiene ningún widget `tray`.
    tray: Option<TrayHandle>,
    /// Feed de clima en su propio hilo. `None` si la config no declara `weather`.
    weather: Option<crate::weather::WeatherHandle>,
    /// Última lectura del clima.
    weather_now: Option<crate::weather::Weather>,
    /// Visualizador de audio (cava) en su propio hilo. `None` si no hay `cava`.
    cava: Option<crate::cava::CavaHandle>,
    /// Último cuadro del visualizador (una fracción `0..1` por banda).
    cava_frame: Vec<f32>,
    theme: Theme,
    cfg: Config,
    surfaces: Vec<crate::SurfaceWidgets>,
    shuma: crate::shuma::ShumaState,
    /// Índice (en `panels`) de la barra que hospeda el `shuma_input`, si hay.
    shuma_panel: Option<usize>,
    /// Grosor original (px) de esa barra — al que vuelve al replegar el drawer.
    shuma_bar_px: u32,
    /// Registro de apps para el menú de inicio (descubierto del dir canónico).
    registry: app_bus::AppRegistry,
    /// `true` cuando el drawer de la barra del menú está desplegado (apps, o un
    /// popup de widget como el portapapeles — ver [`menu_kind`]).
    menu_open: bool,
    /// Qué cuerpo muestra el drawer desplegado: el menú de apps o un popup de
    /// widget (historial de portapapeles). Reusa el mismo crecimiento de la
    /// barra del `start_button`.
    menu_kind: MenuKind,
    /// Historial de copias (más reciente al frente, sin repetidos, tope 16).
    clip_history: Vec<String>,
    /// Borrador de fecha/hora que el panel del reloj edita.
    clock_draft: crate::ClockDraft,
    /// Texto del buscador del menú de inicio (filtra apps por label). Se limpia
    /// al cerrar el menú.
    menu_query: String,
    /// Desplazamiento de la lista del menú (px), para recorrer muchas apps.
    menu_scroll: f32,
    /// Índice (en `panels`) de la barra que hospeda el `start_button`, si hay.
    menu_panel: Option<usize>,
    /// Grosor original (px) de esa barra — al que vuelve al replegar el menú.
    menu_bar_px: u32,
    /// Muestreador del sistema en su propio hilo (subprocesos wpctl/wl-paste sin
    /// tocar el bucle de UI). Publica un snapshot ~1Hz; `maybe_sample` lo recoge.
    sampler: SamplerHandle,
    /// Último snapshot del sistema recogido del hilo de muestreo.
    ctx: WidgetCtx,
    /// Estado del sidebar navegador (Mónadas de nouser). Vacío si la config no
    /// declara un navegador.
    nav: NavState,
    /// Canal por donde el hilo de poll de `list_monads` entrega resultados (~2s).
    /// `None` si la config no tiene navegador (no se arranca el hilo).
    nav_rx: Option<Receiver<PollOutcome>>,
    /// Canal para que los hilos one-shot de `resolve_monad` entreguen miembros.
    members_tx: Sender<MembersOutcome>,
    members_rx: Receiver<MembersOutcome>,
    /// Arrastre en curso (selección de nodo del grafo). `None` si no se arrastra.
    drag: Option<LayerDrag>,
    /// Servidor del rail hospedado: apps que prestan sus dientes a pata mientras
    /// tienen foco. `None` si la config no tiene ningún sidebar donde alojarlos.
    host: Option<HostServer>,
    /// Última revisión vista del `host` (para detectar altas/bajas/updates y
    /// re-pintar los sidebars).
    last_host_rev: u64,
    /// Una layer surface por cada barra de la config.
    panels: Vec<Panel>,
    /// Índice (en `panels`) de la surface del **tooltip flotante**: una layer
    /// surface en `Overlay`, reubicada al hover. `None` si no se creó.
    tooltip_pi: Option<usize>,
    /// Texto del tooltip actualmente visible (`None` = oculto).
    tooltip_text: Option<String>,
    /// Modificadores activos del teclado (ctrl/alt/shift/logo). SCTK los entrega
    /// en `update_modifiers`, aparte de la tecla; los necesitamos para armar el
    /// `llimphi_ui::KeyEvent` que recibe el shell hospedado.
    mods: Modifiers,
    exit: bool,
}

/// El anclaje sctk + el tamaño `(w, h)` pedido para un borde y grosor. El eje
/// libre va en 0 → el compositor lo estira al ancho/alto de la salida.
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
/// compositor no expone `wlr-layer-shell` (en ese caso el caller cae a winit).
pub fn run() -> Result<(), Box<dyn Error>> {
    let cfg = pata_config::load();
    let mut theme = Theme::dark();
    if let Some(c) = crate::render::parse_hex(&cfg.general.accent) {
        theme.accent = c;
    }
    let bars: Vec<usize> = cfg
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SurfaceKind::Bar)
        .map(|(i, _)| i)
        .collect();
    // Los sidebars (Fase 11) también se anclan como layer surfaces propias.
    let sidebars: Vec<usize> = cfg
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SurfaceKind::Sidebar)
        .map(|(i, _)| i)
        .collect();
    if bars.is_empty() && sidebars.is_empty() {
        return Err("pata · la config no tiene ninguna superficie anclable (bar/sidebar)".into());
    }
    diag!(
        "pata diag · backend LAYER-SHELL arranca · {} barra(s) + {} sidebar(s)",
        bars.len(),
        sidebars.len()
    );

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh: QueueHandle<LayerApp> = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;

    // El manager de ventanas (window_list): opcional. Si el compositor no lo
    // expone, el widget queda vacío en vez de fallar el arranque.
    let toplevel_mgr = globals
        .bind::<ZwlrForeignToplevelManagerV1, _, _>(&qh, 1..=3, ())
        .ok();
    if toplevel_mgr.is_none() {
        eprintln!("pata layer · el compositor no expone wlr-foreign-toplevel; window_list vacío");
    }

    // El tray sólo arranca (y toma el nombre del watcher) si la config lo pide.
    let tray = crate::config_tiene_widget(&cfg, "tray")
        .then(TrayHandle::spawn)
        .flatten();
    let weather = crate::config_tiene_widget(&cfg, "weather")
        .then(|| crate::weather::WeatherHandle::spawn(crate::weather_place(&cfg)));
    let cava = crate::config_tiene_widget(&cfg, "cava")
        .then(|| crate::cava::CavaHandle::spawn(crate::cava_bars(&cfg)));

    // Plano de datos del sidebar: un hilo que poolea `list_monads` cada ~2s y
    // entrega por canal (mismo patrón que el sampler/exec — el bucle Wayland lo
    // sondea sin bloquear). Sólo arranca si la config declara un navegador.
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
                    break; // el bucle de UI terminó
                }
                std::thread::sleep(nouser::REFRESH_INTERVAL);
            }
        });
        rx
    });
    let (members_tx, members_rx) = std::sync::mpsc::channel::<MembersOutcome>();

    let (surfaces, shuma) = Model::construir(&cfg);
    // El sampler en UTC si la config lo pide (se lee antes de mover `cfg` al app).
    let utc = crate::usa_utc(&cfg);
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
        toplevels: Vec::new(),
        next_toplevel_id: 0,
        clipboard: None,
        tray,
        weather,
        weather_now: None,
        cava,
        cava_frame: Vec::new(),
        theme,
        cfg,
        surfaces,
        shuma,
        // Se calculan después, una vez creados los panels.
        shuma_panel: None,
        shuma_bar_px: 40,
        registry: app_bus::AppRegistry::discover_merged(),
        menu_open: false,
        menu_kind: MenuKind::Apps,
        clip_history: Vec::new(),
        clock_draft: crate::ClockDraft::default(),
        menu_query: String::new(),
        menu_scroll: 0.0,
        menu_panel: None,
        menu_bar_px: 32,
        sampler: SamplerHandle::spawn(utc),
        ctx: WidgetCtx::default(),
        nav: NavState::default(),
        nav_rx,
        members_tx,
        members_rx,
        drag: None,
        // El rail hospedado sólo tiene sentido si hay un sidebar donde alojar los
        // dientes de la app enfocada.
        host: (!sidebars.is_empty()).then(HostServer::spawn).flatten(),
        last_host_rev: 0,
        panels: Vec::new(),
        tooltip_pi: None,
        tooltip_text: None,
        mods: Modifiers::default(),
        exit: false,
    };

    // Roundtrip para que `OutputState` reciba `wl_output.geometry` + el
    // `xdg_output.name` de cada monitor: lo necesitamos para resolver
    // `Surface::output` (nombre del conector) a un `wl_output` real antes
    // de pedir cada layer surface. SCTK publica el nombre en el segundo
    // roundtrip (xdg-output llega después del wl_output base).
    event_queue.roundtrip(&mut app)?;
    event_queue.roundtrip(&mut app)?;

    // Mapa `nombre del conector → wl_output` (ej. `"HDMI-A-1" → WlOutput`).
    // Sin nombre (compositor sin xdg-output) la entrada se omite — esa
    // salida sólo es alcanzable con `output: ""` (= primario, sin hint).
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

    // Resuelve `output: String` de la config a `Option<&wl_output>`. Vacío =
    // None (el compositor decide). Nombre desconocido = None + aviso (la
    // surface cae al primario en vez de fallar el arranque).
    let resolve_output =
        |name: &str| -> Option<wl_output::WlOutput> {
            if name.is_empty() {
                return None;
            }
            if let Some(o) = outputs_by_name.get(name) {
                return Some(o.clone());
            }
            eprintln!("pata layer · output «{name}» no conectado; cae al primario");
            None
        };

    // Una layer surface por barra: anclada a su borde, con su exclusive zone.
    for &idx in &bars {
        let s = &app.cfg.surfaces[idx];
        let thickness = s.thickness.max(1.0) as u32;
        let (sctk_anchor, size) = anchor_y_size(s.anchor, thickness);
        let wl_surface = compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(
            &qh,
            wl_surface,
            Layer::Top,
            Some("pata".to_string()),
            resolve_output(&s.output).as_ref(),
        );
        layer.set_anchor(sctk_anchor);
        layer.set_size(size.0, size.1);
        layer.set_exclusive_zone(thickness as i32);
        layer.commit();
        app.panels.push(Panel {
            idx,
            card: None,
            layer,
            cache: None,
            width: size.0.max(1),
            height: thickness,
            dirty: true,
            hover_idx: None,
            gpu: None,
        });
    }

    // Una layer surface por sidebar (Fase 11): rail anclado al borde vertical con
    // exclusive zone = su grosor. Colapsado mide `thickness`; al abrir un diente
    // la surface CRECE en ancho (a `thickness + panel_width`) manteniendo la
    // exclusive zone, así el panel flota sobre el área de trabajo sin recolocar el
    // teselado — el mismo truco del drawer de shuma, pero en el eje horizontal.
    for &idx in &sidebars {
        let s = &app.cfg.surfaces[idx];
        let thickness = s.thickness.max(1.0) as u32;
        let (sctk_anchor, size) = anchor_y_size(s.anchor, thickness);
        let wl_surface = compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(
            &qh,
            wl_surface,
            Layer::Top,
            Some("pata-sidebar".to_string()),
            resolve_output(&s.output).as_ref(),
        );
        layer.set_anchor(sctk_anchor);
        layer.set_size(size.0, size.1);
        layer.set_exclusive_zone(thickness as i32);
        // Sin teclado: la navegación es por clic (como las barras). Se cierra el
        // panel volviendo a clickear el diente.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        app.panels.push(Panel {
            idx,
            card: None,
            layer,
            cache: None,
            width: thickness,
            height: size.1.max(1),
            dirty: true,
            hover_idx: None,
            gpu: None,
        });
    }

    // Tarjetas flotantes (estilo conky): cada `card` de una superficie `Panel`
    // es su propia layer surface en `Layer::Bottom` (sobre el escritorio,
    // debajo de las ventanas), anclada a la esquina superior-izquierda con
    // margen (x, y) y del tamaño (w, h) de la tarjeta. No reserva franja ni
    // toma teclado. Heredan el `output` del Panel padre.
    for (idx, s) in app.cfg.surfaces.iter().enumerate() {
        if s.kind != SurfaceKind::Panel {
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
            // Margen: (top, right, bottom, left). (x, y) desde la esquina sup-izq.
            layer.set_margin(card.y as i32, 0, 0, card.x as i32);
            layer.set_exclusive_zone(0);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();
            let widgets = card.widgets.iter().map(pata_core::widget::build).collect();
            app.panels.push(Panel {
                idx,
                card: Some(CardState { spec: card.clone(), widgets }),
                layer,
                cache: None,
                width: cw,
                height: ch,
                dirty: true,
                hover_idx: None,
                gpu: None,
            });
        }
    }

    // ¿Qué barra hospeda el shuma_input? Esa recibe foco de teclado al clickearla
    // (OnDemand) para poder desplegar el Quake y escribir.
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
        // Barra cerrada: NO pide teclado. Con `OnDemand` el compositor
        // consumía el primer click para darle foco (de ahí «dos clicks para
        // desplegar»); con `None` el click va directo a togglear.
        app.panels[pi]
            .layer
            .set_keyboard_interactivity(KeyboardInteractivity::None);
        app.panels[pi].layer.commit();
    }

    // La surface del tooltip flotante: una layer surface en Overlay (sobre todo),
    // anclada arriba-izquierda, sin teclado ni zona exclusiva y con **región de
    // input vacía** (no roba clicks ni hover). Arranca 1×1 fuera de vista; al
    // hover se redimensiona y reubica con `set_margin`. Sin buffer hasta el primer
    // tooltip → no se mapea (invisible). Sin output específico: la pone el
    // compositor donde caiga el puntero.
    app.tooltip_pi = {
        let wl_surface = compositor.create_surface(&qh);
        if let Ok(region) = Region::new(&compositor) {
            // Región vacía (sin add): el tooltip nunca intercepta el puntero.
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
        layer.set_margin(100_000, 0, 0, 0); // fuera de vista
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        app.panels.push(Panel {
            idx: 0,
            card: None,
            layer,
            cache: None,
            width: 1,
            height: 1,
            dirty: false,
            hover_idx: None,
            gpu: None,
        });
        Some(app.panels.len() - 1)
    };

    // ¿Qué barra hospeda el start_button? Esa crece hacia abajo al desplegar el
    // menú de inicio (mismo truco que shuma, hacia el otro lado).
    app.menu_panel = app.panels.iter().position(|p| {
        let s = &app.cfg.surfaces[p.idx];
        s.start
            .iter()
            .chain(&s.center)
            .chain(&s.end)
            .any(|w| w.kind == "start_button")
    });
    app.menu_bar_px = app
        .menu_panel
        .map(|pi| app.cfg.surfaces[app.panels[pi].idx].thickness.max(1.0) as u32)
        .unwrap_or(32);

    while !app.exit {
        if let Err(e) = event_queue.blocking_dispatch(&mut app) {
            // El compositor cerró la conexión (se apagó / Ctrl+Alt+Backspace):
            // es una salida normal, no una falla del backend. Devolvemos Ok para
            // que el caller NO caiga a la ventana winit (que paniquearía al no
            // encontrar compositor). La caída a winit es sólo para cuando el
            // layer-shell no arranca de entrada (X11, sin wlr-layer-shell).
            eprintln!("pata layer · el compositor cerró la conexión: {e}");
            break;
        }
    }
    Ok(())
}

impl LayerApp {
    /// Índice del panel cuya layer surface es `surface`.
    fn panel_de(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.panels
            .iter()
            .position(|p| p.layer.wl_surface() == surface)
    }

    /// Marca la barra de shuma para re-pintar (tras teclear, etc.).
    fn marcar_shuma_dirty(&mut self) {
        if let Some(pi) = self.shuma_panel {
            self.panels[pi].dirty = true;
        }
    }

    /// Marca todas las barras para re-pintar (p. ej. cambió la lista de ventanas).
    fn marcar_todo_dirty(&mut self) {
        for p in &mut self.panels {
            p.dirty = true;
        }
    }

    /// La lista de ventanas para el render del `window_list`, desde los toplevels
    /// que reporta el compositor.
    fn window_entries(&self) -> Vec<WindowEntry> {
        self.toplevels
            .iter()
            .map(|t| WindowEntry {
                id: t.id,
                label: t.etiqueta(),
                app_id: t.app_id.clone(),
                active: t.activated,
                minimized: t.minimized,
            })
            .collect()
    }

    /// El toplevel con ese `id`, si sigue abierto.
    fn toplevel_por_id(&self, id: u32) -> Option<&Toplevel> {
        self.toplevels.iter().find(|t| t.id == id)
    }

    /// Despliega o repliega el drawer Quake: agranda/encoge la layer surface de
    /// la barra de shuma hacia arriba (su exclusive zone queda en el grosor de la
    /// barra, así no recoloca el teselado) y toma/suelta el foco de teclado.
    fn set_shuma_open(&mut self, open: bool) {
        let Some(pi) = self.shuma_panel else { return };
        if self.shuma.open == open {
            return;
        }
        self.shuma.open = open;
        let h = if open { DRAWER_H } else { self.shuma_bar_px };
        // El contenido es el shell real hospedado (`self.shuma.inner`), vivo
        // desde el arranque: no hace falta spawnear nada al abrir. El cwd, los
        // jobs y el historial persisten en el `State` aunque se repliegue.
        let layer = &self.panels[pi].layer;
        layer.set_size(0, h);
        // Abierto: foco Exclusive para escribir. Cerrado: `None` — no
        // retiene el teclado, así una app lanzada (kitty) lo recibe.
        layer.set_keyboard_interactivity(if open {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::None
        });
        layer.commit();
        // El cache de hit-test es del layout viejo; invalidarlo evita que el
        // click siguiente pegue contra el árbol previo («no se guarda»). Se
        // re-arma en el próximo frame con la geometría nueva.
        self.panels[pi].cache = None;
        self.panels[pi].dirty = true;
    }

    /// Traduce un evento de teclado de SCTK al `llimphi_ui::KeyEvent` que consume
    /// el shell hospedado (`shuma_module_shell::Msg::Key`). El backend layer-shell
    /// trabaja con `Keysym` + `Modifiers` crudos; el shell —y su PTY/TUI interno—
    /// esperan el evento normalizado de llimphi (igual que la app standalone bajo
    /// winit). Las teclas con nombre van a `Key::Named`; las de texto a
    /// `Key::Character` con su `utf8`. `None` si la tecla no produce nada útil.
    fn keysym_to_keyevent(&self, event: &KbEvent) -> Option<llimphi_ui::KeyEvent> {
        use llimphi_ui::{Key, NamedKey};
        use Keysym as K;
        let named = match event.keysym {
            K::Return | K::KP_Enter => Some(NamedKey::Enter),
            K::BackSpace => Some(NamedKey::Backspace),
            K::Tab | K::ISO_Left_Tab => Some(NamedKey::Tab),
            K::Escape => Some(NamedKey::Escape),
            K::Up => Some(NamedKey::ArrowUp),
            K::Down => Some(NamedKey::ArrowDown),
            K::Right => Some(NamedKey::ArrowRight),
            K::Left => Some(NamedKey::ArrowLeft),
            K::Home => Some(NamedKey::Home),
            K::End => Some(NamedKey::End),
            K::Page_Up => Some(NamedKey::PageUp),
            K::Page_Down => Some(NamedKey::PageDown),
            K::Delete => Some(NamedKey::Delete),
            K::Insert => Some(NamedKey::Insert),
            K::F1 => Some(NamedKey::F1),
            K::F2 => Some(NamedKey::F2),
            K::F3 => Some(NamedKey::F3),
            K::F4 => Some(NamedKey::F4),
            K::F5 => Some(NamedKey::F5),
            K::F6 => Some(NamedKey::F6),
            K::F7 => Some(NamedKey::F7),
            K::F8 => Some(NamedKey::F8),
            K::F9 => Some(NamedKey::F9),
            K::F10 => Some(NamedKey::F10),
            K::F11 => Some(NamedKey::F11),
            K::F12 => Some(NamedKey::F12),
            _ => None,
        };
        let modifiers = llimphi_ui::Modifiers {
            shift: self.mods.shift,
            ctrl: self.mods.ctrl,
            alt: self.mods.alt,
            meta: self.mods.logo,
        };
        let (key, text) = if let Some(n) = named {
            (Key::Named(n), None)
        } else {
            // Tecla de texto: preferimos el utf8 del compositor, salvo que sea un
            // carácter de control (combos con Ctrl) — ahí usamos el char limpio
            // del keysym para que el shell vea 'c' y no 0x03 (y decida con ctrl).
            let txt = match event.utf8.as_deref() {
                Some(s) if !s.is_empty() && !s.chars().all(char::is_control) => s.to_string(),
                _ => event.keysym.key_char()?.to_string(),
            };
            (Key::Character(txt.as_str().into()), Some(txt))
        };
        Some(llimphi_ui::KeyEvent {
            key,
            state: llimphi_ui::KeyState::Pressed,
            text,
            modifiers,
            repeat: false,
        })
    }

    /// Despliega/repliega el menú de inicio: agranda/encoge hacia abajo la layer
    /// surface de la barra del `start_button` (su exclusive zone queda en el
    /// grosor de la barra, así no recoloca el teselado). No toma teclado (clics).
    fn set_menu_open(&mut self, open: bool) {
        let Some(pi) = self.menu_panel else { return };
        if self.menu_open == open {
            return;
        }
        self.menu_open = open;
        // Al cerrar, reseteamos el buscador y el scroll (la próxima apertura
        // arranca limpia).
        if !open {
            self.menu_query.clear();
            self.menu_scroll = 0.0;
        }
        let h = if open { MENU_H } else { self.menu_bar_px };
        let layer = &self.panels[pi].layer;
        layer.set_size(0, h);
        // El menú de apps toma el teclado (buscador); los popups de widget
        // (portapapeles) son sólo clicks, así que no roban teclado.
        let toma_teclado = open && self.menu_kind == MenuKind::Apps;
        layer.set_keyboard_interactivity(if toma_teclado {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::None
        });
        layer.commit();
        // Invalida el cache de hit-test (geometría vieja) — igual que shuma.
        self.panels[pi].cache = None;
        self.panels[pi].dirty = true;
    }

    /// Abre/cierra el drawer de la barra del menú mostrando el cuerpo `kind`. Si
    /// ya está abierto con otro `kind`, cambia el cuerpo (y el modo de teclado)
    /// sin recrear; si es el mismo, lo cierra (toggle).
    fn toggle_menu(&mut self, kind: MenuKind) {
        if self.menu_open && self.menu_kind == kind {
            self.set_menu_open(false);
        } else if self.menu_open {
            self.menu_kind = kind;
            if let Some(pi) = self.menu_panel {
                let toma = kind == MenuKind::Apps;
                let layer = &self.panels[pi].layer;
                layer.set_keyboard_interactivity(if toma {
                    KeyboardInteractivity::Exclusive
                } else {
                    KeyboardInteractivity::None
                });
                layer.commit();
                self.panels[pi].cache = None;
                self.panels[pi].dirty = true;
            }
        } else {
            self.menu_kind = kind;
            self.set_menu_open(true);
        }
    }

    /// Actualiza el tooltip flotante para el nodo `node_idx` bajo el cursor en el
    /// panel `pi`: si ese nodo tiene texto de tooltip, redimensiona y reubica la
    /// surface del tooltip bajo el widget y la marca para pintar; si no, la
    /// oculta. Reposiciona sólo al cambiar de nodo (no sigue al cursor). Pinta de
    /// inmediato (`draw`) porque los eventos llegan por OTRA surface.
    fn update_tooltip(&mut self, pi: usize, node_idx: Option<usize>, qh: &QueueHandle<Self>) {
        let Some(tpi) = self.tooltip_pi else { return };
        if pi == tpi {
            return;
        }
        // Texto + rect del nodo hovereado, desde el cache de hit-test del panel.
        let info = node_idx.and_then(|i| {
            let c = self.panels[pi].cache.as_ref()?;
            let node = c.mounted.nodes.get(i)?;
            let text = node.tooltip.clone()?;
            let rect = c.computed.get(node.id)?;
            Some((text, rect))
        });
        match info {
            Some((text, rect)) => {
                // Posición: bajo el widget. La barra superior (la única con
                // widgets hovereables) está en y=0, así que su alto da el offset.
                let x = rect.x.max(0.0) as i32;
                let y = self.panels[pi].height as i32 + 4;
                // Tamaño estimado (no medimos texto acá): ~8px/glifo + padding.
                let w = (text.chars().count() as u32 * 8 + 16).clamp(24, 600);
                let h = 24u32;
                self.tooltip_text = Some(text);
                {
                    let layer = &self.panels[tpi].layer;
                    layer.set_size(w, h);
                    layer.set_margin(y, 0, 0, x);
                    layer.commit();
                }
                self.panels[tpi].width = w;
                self.panels[tpi].height = h;
                self.panels[tpi].dirty = true;
                self.draw(tpi, qh);
            }
            None => self.hide_tooltip(qh),
        }
    }

    /// Oculta el tooltip: lo empuja fuera de vista (1×1 con margen enorme) y lo
    /// re-pinta una vez ahí. No hace nada si ya está oculto.
    fn hide_tooltip(&mut self, qh: &QueueHandle<Self>) {
        let Some(tpi) = self.tooltip_pi else { return };
        if self.tooltip_text.is_none() {
            return;
        }
        self.tooltip_text = None;
        {
            let layer = &self.panels[tpi].layer;
            layer.set_size(1, 1);
            layer.set_margin(100_000, 0, 0, 0);
            layer.commit();
        }
        self.panels[tpi].width = 1;
        self.panels[tpi].height = 1;
        self.panels[tpi].dirty = true;
        self.draw(tpi, qh);
    }

    /// Lanza una app del menú por su `id` y cierra el menú. Sólo `Exec` spawnea;
    /// `Action`/`Wasm` los despacharía un host con chasis (acá no aplica).
    fn lanzar_app(&mut self, id: String) {
        if let Some(app) = self.registry.get(&id) {
            let _ = app.spawn();
        }
        self.set_menu_open(false);
    }

    /// Marca para re-pintar la barra que hospeda el menú de inicio (tras teclear
    /// en el buscador). Invalida su cache de hit-test (el árbol cambió).
    fn marcar_menu_dirty(&mut self) {
        if let Some(pi) = self.menu_panel {
            self.panels[pi].cache = None;
            self.panels[pi].dirty = true;
        }
    }

    /// Enter en el menú de inicio: lanza el primer resultado del filtro actual
    /// (la lista ya viene ordenada por label). No-op si no hay coincidencias.
    fn lanzar_primero_menu(&mut self) {
        let id = render::menu_filtered(self.registry.all(), &self.menu_query)
            .first()
            .map(|a| a.id.clone());
        if let Some(id) = id {
            self.lanzar_app(id);
        }
    }

    /// Sondea (sin bloquear) el plano de datos del sidebar: aplica el último poll
    /// de `list_monads` y cualquier `resolve_monad` que haya terminado. Si cambió
    /// algo, marca las superficies sidebar para re-pintar. Se llama en cada frame.
    fn poll_nav(&mut self) {
        let mut cambios = false;
        // Drena el poll de Mónadas (nos quedamos con el último si hay varios).
        if let Some(rx) = self.nav_rx.as_ref() {
            let mut ultimo = None;
            while let Ok(o) = rx.try_recv() {
                ultimo = Some(o);
            }
            if let Some(outcome) = ultimo {
                match outcome {
                    PollOutcome::Ok { socket, resp } => {
                        self.nav.socket = Some(socket);
                        self.nav.apply_monads(*resp);
                    }
                    PollOutcome::Failed(e) => {
                        self.nav.socket = None;
                        self.nav.error = Some(e);
                    }
                }
                cambios = true;
            }
        }
        // Drena los miembros resueltos.
        while let Ok(outcome) = self.members_rx.try_recv() {
            match outcome {
                MembersOutcome::Ok { monad, members } => self.nav.apply_members(monad, members),
                MembersOutcome::Failed(e) => self.nav.error = Some(e),
            }
            cambios = true;
        }
        if cambios {
            self.marcar_sidebars_dirty();
        }
    }

    /// El `app_id` del toplevel que tiene foco ahora, si hay alguno.
    fn focused_app_id(&self) -> Option<&str> {
        self.toplevels
            .iter()
            .find(|t| t.activated)
            .map(|t| t.app_id.as_str())
    }

    /// Sondea el rail hospedado: si cambió su revisión (un alta/baja/update de
    /// dientes de alguna app), re-pinta los sidebars. El cambio de foco ya re-pinta
    /// vía `marcar_todo_dirty` (eventos de toplevel).
    fn poll_host(&mut self) {
        let Some(h) = &self.host else { return };
        let rev = h.revision();
        if rev != self.last_host_rev {
            self.last_host_rev = rev;
            self.marcar_sidebars_dirty();
        }
    }

    /// Marca todas las superficies sidebar para re-pintar.
    fn marcar_sidebars_dirty(&mut self) {
        for p in &mut self.panels {
            if p.card.is_none() && self.cfg.surfaces[p.idx].kind == SurfaceKind::Sidebar {
                p.dirty = true;
            }
        }
    }

    /// Índice (en `panels`) de la layer surface del sidebar `si`.
    fn sidebar_panel_de(&self, si: usize) -> Option<usize> {
        self.panels.iter().position(|p| p.idx == si && p.card.is_none())
    }

    /// Activa/repliega el diente `(si, ti)`: actualiza el estado y **redimensiona**
    /// la layer surface del sidebar (crece a `thickness + panel_width` al abrir,
    /// vuelve a `thickness` al cerrar). La exclusive zone no cambia, así el panel
    /// flota sobre el área de trabajo (drawer horizontal).
    fn set_sidebar_open(&mut self, si: usize, ti: usize) {
        self.nav.toggle_tab(si, ti);
        let Some(pi) = self.sidebar_panel_de(si) else {
            return;
        };
        let s = &self.cfg.surfaces[si];
        let thickness = s.thickness.max(1.0) as u32;
        let abierto = matches!(self.nav.open, Some((s2, _)) if s2 == si);
        let w = if abierto {
            thickness + s.panel_width.max(1.0) as u32
        } else {
            thickness
        };
        {
            let layer = &self.panels[pi].layer;
            layer.set_size(w, 0);
            layer.commit();
        }
        // El cache de hit-test es del layout viejo; invalidarlo (igual que shuma).
        self.panels[pi].cache = None;
        self.panels[pi].dirty = true;
    }

    /// Cierra el panel del sidebar (si alguno está abierto) y encoge su surface.
    fn cerrar_sidebar(&mut self) {
        if let Some((si, ti)) = self.nav.open {
            self.set_sidebar_open(si, ti); // toggle del abierto = cerrar
        }
    }

    /// Expande/colapsa un nodo del navegador; al abrir una Mónada sin miembros
    /// resueltos lanza su `resolve_monad` en un hilo one-shot (entrega por canal).
    fn nav_toggle(&mut self, id: u64) {
        if self.nav.expanded.contains(&id) {
            self.nav.expanded.remove(&id);
        } else {
            self.nav.expanded.insert(id);
            if let (Some(mid), Some(sock)) =
                (self.nav.needs_resolve(id), self.nav.socket.clone())
            {
                let tx = self.members_tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(nouser::resolve(sock, mid));
                });
            }
        }
        self.marcar_sidebars_dirty();
    }

    /// Recoge el último snapshot del hilo de muestreo (no bloquea). Si llegó uno
    /// nuevo, `tick`ea los widgets y marca todas las barras para re-pintar. El
    /// muestreo en sí (subprocesos que pueden colgarse) vive en `SamplerHandle`,
    /// nunca acá: el bucle de UI no se bloquea aunque wpctl/wl-paste se cuelguen.
    fn maybe_sample(&mut self) {
        let Some((ctx, clipboard)) = self.sampler.latest() else {
            return;
        };
        self.ctx = ctx;
        crate::push_clip_history(&mut self.clip_history, &clipboard);
        self.clipboard = clipboard;
        if let Some(h) = &self.weather {
            if let Some(w) = h.latest() {
                self.weather_now = Some(w);
            }
        }
        for sw in &mut self.surfaces {
            for w in sw.core_mut() {
                w.tick(&ctx);
            }
        }
        // Las tarjetas flotantes tienen sus widgets en su propio Panel.
        for p in &mut self.panels {
            if let Some(c) = p.card.as_mut() {
                for w in &mut c.widgets {
                    w.tick(&ctx);
                }
            }
            p.dirty = true;
        }
    }

    /// Drena el último cuadro del visualizador (cava) y, si llegó uno nuevo,
    /// marca las barras para re-pintar. El frame-callback corre continuo, así que
    /// esto da el refresco rápido del visualizador sin un timer aparte.
    fn maybe_cava(&mut self) {
        let Some(h) = &self.cava else {
            return;
        };
        let Some(frame) = h.latest() else {
            return;
        };
        self.cava_frame = frame;
        for p in &mut self.panels {
            if p.card.is_none() {
                p.dirty = true;
            }
        }
    }

    /// Crea el estado wgpu de un panel sobre los punteros raw de Wayland
    /// (`wl_display` + `wl_surface`). El `Hal` se comparte; lo crea el primer panel
    /// **eligiendo el adaptador compatible con su surface** (el dispositivo que
    /// mirada compone) — clave en multi-GPU/Optimus. Los paneles siguientes reusan
    /// ese `Hal` (mismo compositor → mismo dispositivo, el adaptador ya sirve).
    fn ensure_gpu(&mut self, pi: usize) {
        if self.panels[pi].gpu.is_some() {
            return;
        }
        let display_ptr = self.conn.backend().display_ptr() as *mut c_void;
        let surface_ptr = self.panels[pi].layer.wl_surface().id().as_ptr() as *mut c_void;
        let (w, h) = (self.panels[pi].width, self.panels[pi].height);
        let display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(display_ptr).expect("wl_display ptr"),
        ));
        let window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(surface_ptr).expect("wl_surface ptr"),
        ));
        // SAFETY: los handles apuntan a objetos Wayland que `self` mantiene vivos
        // (la conexión y la layer surface) durante toda la vida de la surface.
        let make_target = || wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: display_handle,
            raw_window_handle: window_handle,
        };

        let surface = if self.hal.is_none() {
            // Primer panel: crea el Hal pidiendo el adaptador compatible con ESTA
            // surface (no `HighPerformance` a ciegas, que en Optimus agarraría la
            // GPU equivocada → 0 formatos).
            match pollster::block_on(unsafe { Hal::new_for_raw_surface(make_target, w, h) }) {
                Ok((hal, surface)) => {
                    self.hal = Some(hal);
                    surface
                }
                Err(e) => {
                    eprintln!("pata layer · panel {pi} sin gpu: {e}");
                    return;
                }
            }
        } else {
            // Paneles siguientes: reusan el Hal ya creado.
            let hal = self.hal.as_ref().expect("hal");
            let wgpu_surface = match unsafe { hal.instance.create_surface_unsafe(make_target()) } {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("pata layer · panel {pi} sin gpu: {e}");
                    return;
                }
            };
            // Sin formatos la WSI no soporta esta surface: en vez de paniquear,
            // dejamos el panel sin gpu (no pinta) y seguimos — un panel roto no
            // tira todo el marco. Reintenta en el próximo `draw`.
            match RawSurface::from_surface(hal, wgpu_surface, w, h) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("pata layer · panel {pi} sin gpu: {e}");
                    return;
                }
            }
        };
        let hal = self.hal.as_ref().expect("hal");
        diag!(
            "pata diag · panel {pi} surface creada {w}x{h} · backend={:?} format={:?}",
            hal.adapter.get_info().backend,
            surface.format(),
        );
        let renderer = Renderer::new(hal).expect("renderer");
        self.panels[pi].gpu = Some(PanelGpu {
            surface,
            renderer,
            typesetter: Typesetter::new(),
            scene: vello::Scene::new(),
            layout: LayoutTree::new(),
        });
    }

    /// Mantiene vivo el latido de un panel: pide su siguiente frame-callback.
    fn latido(&self, pi: usize, qh: &QueueHandle<Self>) {
        let surface = self.panels[pi].layer.wl_surface();
        surface.frame(qh, surface.clone());
        surface.commit();
    }

    /// Avanza el frame de un panel: re-muestrea ~1Hz (compartido) y pinta sólo si
    /// hay algo nuevo; entre cambios sólo mantiene el latido.
    fn draw(&mut self, pi: usize, qh: &QueueHandle<Self>) {
        self.maybe_sample();
        self.maybe_cava();
        self.poll_nav();
        self.poll_host();
        self.ensure_gpu(pi);

        // Drawer abierto: el shell hospedado avanza solo (el proceso/PTY escribe
        // sin que toquemos teclas). Lo latimos cada frame (`Tick` del módulo drena
        // la salida — `update` puro) y forzamos repintado para verlo en vivo.
        if self.shuma_panel == Some(pi) && self.shuma.open {
            self.shuma.inner =
                shuma_module_shell::update(self.shuma.inner.clone(), shuma_module_shell::Msg::Tick);
            self.panels[pi].dirty = true;
        }

        if !self.panels[pi].dirty {
            self.latido(pi, qh);
            return;
        }

        let idx = self.panels[pi].idx;
        let (w, h) = (self.panels[pi].width, self.panels[pi].height);
        let windows = self.window_entries();
        let tray_items = self.tray.as_ref().map(|t| t.items()).unwrap_or_default();
        let data = render::BarData {
            windows: &windows,
            clipboard: self.clipboard.as_deref(),
            tray: &tray_items,
            weather: self.weather_now.as_ref(),
            cava: &self.cava_frame,
        };
        // Una tarjeta flotante pinta su contenido (relleno de su surface); la
        // barra de shuma desplegada pinta el drawer (cuerpo + cabezal); el resto
        // pinta su barra normal.
        let view = if self.tooltip_pi == Some(pi) {
            // La surface del tooltip: pinta la cajita con el texto actual (o vacía
            // cuando está oculta fuera de vista).
            render::tooltip_view(self.tooltip_text.as_deref().unwrap_or(""), &self.theme)
        } else if let Some(c) = self.panels[pi].card.as_ref() {
            render::card_view(&c.spec, &c.widgets, &self.theme)
        } else if self.menu_panel == Some(pi) && self.menu_open {
            match self.menu_kind {
                MenuKind::Apps => render::start_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    self.registry.all(),
                    &self.menu_query,
                    self.menu_scroll,
                    h as f32,
                ),
                MenuKind::Clipboard => render::clipboard_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    &self.clip_history,
                ),
                MenuKind::Clock => render::clock_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    &self.clock_draft,
                ),
            }
        } else if self.shuma_panel == Some(pi) && self.shuma.open {
            // El cuerpo del drawer es el **shell real** hospedado (cards/PTY/TUI);
            // abajo queda la barra (cabezal con el chip de shuma).
            render::shuma_open_view(
                &self.cfg.surfaces[idx],
                &self.surfaces[idx],
                &self.shuma,
                &data,
                &self.theme,
                self.shuma_bar_px as f32,
            )
        } else if self.cfg.surfaces[idx].kind == SurfaceKind::Sidebar {
            // Dientes hospedados de la app enfocada (si registró alguno en el host).
            let hosted = {
                let app = self.focused_app_id().map(|s| s.to_string());
                match (app, self.host.as_ref()) {
                    (Some(id), Some(h)) => h.snapshot(&id).map(|(_, teeth)| (id, teeth)),
                    _ => None,
                }
            };
            let (hosted_app, hosted_teeth): (&str, &[pata_host::HostedTooth]) = match &hosted {
                Some((id, teeth)) => (id.as_str(), teeth.as_slice()),
                None => ("", &[]),
            };
            render::sidebar_surface_view(
                &self.cfg.surfaces[idx],
                idx,
                w as f32,
                h as f32,
                &self.nav,
                hosted_teeth,
                hosted_app,
                &self.shuma,
                &self.theme,
            )
        } else {
            render::bar_view(
                &self.cfg.surfaces[idx],
                &self.surfaces[idx],
                &self.shuma,
                &data,
                &self.theme,
            )
        };

        let hover_idx = self.panels[pi].hover_idx;
        let hal = self.hal.as_ref().expect("hal");
        let gpu = match self.panels[pi].gpu.as_mut() {
            Some(g) => g,
            None => {
                self.latido(pi, qh);
                return;
            }
        };
        gpu.surface.resize(w, h);
        let frame = match gpu.surface.acquire() {
            Ok(f) => f,
            Err(_) => {
                // Soltamos el préstamo mutable de `gpu` antes de tocar `self`.
                let _ = gpu;
                self.latido(pi, qh);
                return;
            }
        };
        gpu.layout.clear();
        let mounted: Mounted<Msg> = mount(&mut gpu.layout, view);
        let computed = {
            let ts = &mut gpu.typesetter;
            let tmap = &mounted.text_measures;
            gpu.layout
                .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(ts, tm, known, avail),
                        None => taffy::Size::ZERO,
                    }
                })
                .expect("layout")
        };
        gpu.scene.reset();
        paint(&mut gpu.scene, &mounted, &computed, &mut gpu.typesetter, hover_idx, None);
        if let Err(e) = gpu.renderer.render(hal, &gpu.scene, &frame, palette::css::BLACK) {
            eprintln!("pata layer · render: {e}");
        }
        gpu.surface.present(frame, hal);
        diag!("pata diag · present panel {pi} {w}x{h}");

        // Recién con el cuadro presentado damos el panel por limpio: si la
        // adquisición hubiera fallado, `dirty` sigue puesto y el próximo
        // frame-callback reintenta (no esperamos al re-muestreo de 1 Hz).
        self.panels[pi].dirty = false;
        // Guarda el árbol pintado para el hit-test de los clicks.
        self.panels[pi].cache = Some(RenderCache { mounted, computed });
        self.latido(pi, qh);
    }

    /// Aplica el `Msg` que produjo un click: togglear shuma (su cabezal) o lanzar
    /// el comando de un widget con `exec`. El resto no sale de un click.
    fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::ShumaToggle => self.set_shuma_open(!self.shuma.open),
            // Cualquier interacción del shell hospedado (clic en cards/etapas,
            // scroll, selección del cuerpo IDE-text…) llega envuelta por el `lift`
            // del `view`: la reenviamos a `shuma_module_shell::update`.
            Msg::ShumaShell(m) => {
                self.shuma.inner = shuma_module_shell::update(self.shuma.inner.clone(), m);
                self.marcar_shuma_dirty();
            }
            Msg::Spawn(cmd) => crate::spawn_cmd(&cmd),
            Msg::VolumeWheel(dy) => {
                if dy != 0.0 {
                    crate::sampler::nudge_volume(dy > 0.0);
                }
            }
            Msg::VolumeMute => crate::sampler::toggle_mute(),
            Msg::VolumeSet(f) => crate::sampler::set_volume(f),
            Msg::BrightnessWheel(dy) => {
                if dy != 0.0 {
                    crate::sampler::nudge_brightness(dy > 0.0);
                }
            }
            Msg::BrightnessSet(f) => crate::sampler::set_brightness(f),
            Msg::ClipboardMenu => self.toggle_menu(MenuKind::Clipboard),
            Msg::ClipboardPick(text) => {
                crate::sampler::copiar_clipboard(&text);
                self.set_menu_open(false);
            }
            Msg::ClockPanel => {
                if !(self.menu_open && self.menu_kind == MenuKind::Clock) {
                    self.clock_draft = crate::ClockDraft::from_now(crate::usa_utc(&self.cfg));
                }
                self.toggle_menu(MenuKind::Clock);
            }
            Msg::ClockAdjust(f, delta) => {
                self.clock_draft.adjust(f, delta);
                self.marcar_menu_dirty();
            }
            Msg::ClockApply => {
                crate::sampler::set_system_time(&self.clock_draft.stamp());
                self.set_menu_open(false);
            }
            Msg::ClockSyncNtp => {
                crate::sampler::sync_ntp();
                self.set_menu_open(false);
            }
            Msg::StartToggle => self.toggle_menu(MenuKind::Apps),
            Msg::StartScroll(delta) => {
                // Recorre la lista del menú. content/viewport aproximados (el
                // render reclampa para pintar); evita la deriva del offset.
                let count =
                    render::menu_filtered(self.registry.all(), &self.menu_query).len();
                let content = count as f32 * 30.0;
                let viewport =
                    (MENU_H as f32 - self.menu_bar_px as f32 - 62.0).max(28.0);
                self.menu_scroll = llimphi_widget_scroll::clamp_offset(
                    self.menu_scroll + delta,
                    content,
                    viewport,
                );
                self.marcar_menu_dirty();
            }
            Msg::LaunchApp(id) => self.lanzar_app(id),
            Msg::ActivateWindow(id) => self.activar_ventana(id),
            Msg::CloseWindow(id) => self.cerrar_ventana(id),
            Msg::TrayActivate(key) => {
                if let Some(t) = &self.tray {
                    t.activate(key);
                }
            }
            // --- Sidebar navegador (Fase 11c-layer) ---
            Msg::NavTabActivate(si, ti) => self.set_sidebar_open(si, ti),
            Msg::NavClosePanel => self.cerrar_sidebar(),
            Msg::NavSetMode(m) => {
                self.nav.mode = m;
                self.marcar_sidebars_dirty();
            }
            Msg::NavSelect(id) => {
                self.nav.selected = Some(id);
                self.marcar_sidebars_dirty();
            }
            Msg::NavToggle(id) => self.nav_toggle(id),
            Msg::NavContextMenu(id) => {
                // Fase 11d-extra: right-click sobre archivo abre el menú "Abrir con…".
                if let Some(path) = self.nav.file_path(id).map(str::to_owned) {
                    let opts = crate::open::handlers_for_path(&self.registry, &path);
                    self.nav.open_menu(id, opts);
                    self.marcar_sidebars_dirty();
                }
            }
            Msg::NavOpenWith(id, app_id) => {
                if let Some(path) = self.nav.file_path(id).map(str::to_owned) {
                    match app_id {
                        Some(aid) => {
                            let _ = crate::open::open_with_id(&self.registry, &aid, &path);
                        }
                        None => {
                            let _ = crate::open::open_system(&path);
                        }
                    }
                }
                self.nav.close_menu();
                self.marcar_sidebars_dirty();
            }
            Msg::NavMenuCancel => {
                self.nav.close_menu();
                self.marcar_sidebars_dirty();
            }
            Msg::HostToothActivate(app_id, tooth) => {
                // Reenvía el clic del diente hospedado a la app enfocada; ella
                // muestra ese panel sobre su propio canvas.
                if let Some(h) = &self.host {
                    h.activate(&app_id, tooth);
                }
            }
            Msg::NavScroll(delta) => {
                self.nav.scroll = (self.nav.scroll + delta).max(0.0);
                self.marcar_sidebars_dirty();
            }
            Msg::Quit => self.exit = true,
            _ => {}
        }
    }

    /// Click en una ventana del task manager (estilo KDE): si ya está activa, la
    /// **minimiza**; si no, la trae al frente (y la desminimiza). Sin seat (raro)
    /// no hace nada. El compositor responde con `state`/`done` que actualiza el
    /// resaltado y el atenuado.
    fn activar_ventana(&mut self, id: u32) {
        let Some(seat) = self.seat.clone() else { return };
        if let Some(t) = self.toplevel_por_id(id) {
            if t.activated {
                t.handle.set_minimized();
            } else {
                t.handle.unset_minimized();
                t.handle.activate(&seat);
            }
        }
    }

    /// Cierra la ventana `id` (clic derecho en su chip del task manager). El
    /// compositor manda `closed` que la retira de la lista.
    fn cerrar_ventana(&mut self, id: u32) {
        if let Some(t) = self.toplevel_por_id(id) {
            t.handle.close();
        }
    }
}

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
        // Cerrar cualquier barra cierra el marco entero.
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
        let (cw, ch) = configure.new_size;
        let Some(pi) = self.panel_de(layer.wl_surface()) else {
            return;
        };
        diag!("pata diag · configure panel {pi} new_size={cw}x{ch}");
        // El compositor nos da el tamaño definitivo (el eje libre ya resuelto).
        // Filtramos valores degenerados: cuando escondemos el tooltip empujándolo
        // fuera de vista con un margen enorme (`set_margin(100_000, …)`), el
        // `arrange` de smithay resta ese margen del alto disponible —978 − 100000
        // = −99022— y, a diferencia de KWin, NO lo clampa a cero: nos llega
        // envuelto a un u32 gigante. Pasárselo crudo a `Surface::configure`
        // revienta la validación de wgpu (límite de textura 16384). Un cliente
        // jamás debe alimentar a la GPU con un tamaño del compositor sin validarlo.
        const MAX_DIM: u32 = 16384; // máximo de textura de wgpu/Vulkan
        if (1..=MAX_DIM).contains(&cw) {
            self.panels[pi].width = cw;
        }
        if (1..=MAX_DIM).contains(&ch) {
            self.panels[pi].height = ch;
        }
        self.panels[pi].dirty = true; // tamaño nuevo → re-pintar
        self.draw(pi, qh);
    }
}

impl OutputHandler for LayerApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for LayerApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        // Guardamos el seat para poder activar ventanas (`activate(seat)`).
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
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KbEvent,
    ) {
        // Menú de inicio abierto: el teclado va al buscador (filtra apps).
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
        // El teclado sólo nos importa con el drawer abierto (foco Exclusive).
        if !self.shuma.open {
            return;
        }
        // Ctrl+Shift+W repliega el drawer (el shell sigue vivo). Es el único
        // atajo que el shell NO ve — Escape, Ctrl+C, etc. van al shell hospedado.
        if self.mods.ctrl
            && self.mods.shift
            && matches!(event.keysym, Keysym::w | Keysym::W)
        {
            self.set_shuma_open(false);
            return;
        }
        // Todo lo demás se normaliza a un `llimphi_ui::KeyEvent` y se reenvía al
        // **shell real** hospedado (`Msg::Key`), que decide entre su input de
        // línea y el PTY/TUI. La vista se repinta cada frame mientras esté abierto.
        if let Some(ke) = self.keysym_to_keyevent(&event) {
            self.shuma.inner = shuma_module_shell::update(
                self.shuma.inner.clone(),
                shuma_module_shell::Msg::Key(ke),
            );
        }
        self.marcar_shuma_dirty();
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KbEvent,
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
        // Los guarda para que el terminal traduzca Ctrl+C/Alt+x a bytes.
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
            // Hover: el nodo bajo el puntero da feedback (`hover_fill`) y, si
            // tiene texto de tooltip, lo muestra en la surface flotante. El
            // layer-shell no trackeaba hover (pasaba `None` a `paint`), así que el
            // realce estaba muerto en todas las barras.
            match e.kind {
                PointerEventKind::Motion { .. } => {
                    // Drag en curso: el delta va al handler del nodo (Move). El
                    // nodegraph del navegador no reposiciona (devuelve None en
                    // Move); selecciona al soltar (End).
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
                    }
                    continue;
                }
                PointerEventKind::Leave { .. } => {
                    if let Some(pi) = self.panel_de(&e.surface) {
                        if self.panels[pi].hover_idx.is_some() {
                            self.panels[pi].hover_idx = None;
                            self.panels[pi].dirty = true;
                        }
                    }
                    self.hide_tooltip(qh);
                    continue;
                }
                _ => {}
            }
            // Rueda sobre el historial del drawer: el nodo de scroll bajo el
            // cursor consume el delta y emite `ShumaScroll`. Convención de
            // signo igual que llimphi-ui (wayland y winit traen el eje y con
            // signos opuestos, así que acá NO se niega).
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
            // Soltar el botón izquierdo termina un drag en curso: el handler del
            // nodo recibe `End` y emite su Msg (p. ej. seleccionar el nodo del
            // grafo). Los deltas en End los ignoran los consumidores.
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
                // Hit-test: qué nodo está bajo el puntero y qué handler dispara.
                // El izquierdo usa `on_click` (cabezal de shuma, activar ventana,
                // lanzar exec); el derecho `on_right_click` (cerrar ventana del
                // task manager). El click ya dio foco de teclado.
                let Some(pi) = self.panel_de(&e.surface) else {
                    continue;
                };
                let (px, py) = (e.position.0 as f32, e.position.1 as f32);
                let derecho = button == BTN_RIGHT;
                // Nodo arrastrable bajo el press (izquierdo): arranca un drag y NO
                // lo tratamos como click (el nodegraph selecciona al soltar).
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
                let msg = self.panels[pi].cache.as_ref().and_then(|c| {
                    let i = hit_test_click(&c.mounted, &c.computed, px, py)?;
                    let n = c.mounted.nodes.get(i)?;
                    // Primero el handler simple (`on_click`/`on_right_click`); si no
                    // hay, el `*_at` (coords locales al nodo) — lo usan widgets que
                    // coexisten con drag, como los dientes del rail. Paridad con el
                    // bucle winit, que también prioriza así.
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
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

/// El manager de ventanas: anuncia un toplevel nuevo (creando su handle hijo) y
/// el fin del servicio. `event_created_child!` declara cómo enrutar el handle que
/// nace en el evento `toplevel` (sin esto, wayland-client paniquea al recibirlo).
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

/// Un handle de toplevel: el compositor le manda título / app_id / estado en
/// eventos sueltos y los confirma con `done`; `closed` lo retira. Acumulamos en
/// el [`Toplevel`] y aplicamos en `done` para no pintar estados a medias.
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

delegate_compositor!(LayerApp);
delegate_output!(LayerApp);
delegate_layer!(LayerApp);
delegate_seat!(LayerApp);
delegate_keyboard!(LayerApp);
delegate_pointer!(LayerApp);
delegate_registry!(LayerApp);
