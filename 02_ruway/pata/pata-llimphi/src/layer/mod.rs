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
    /// Las ventanas abiertas que reporta el compositor.
    pub(super) toplevels: Vec<Toplevel>,
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
    /// Visualizador de audio (cava) en su propio hilo.
    pub(super) cava: Option<crate::cava::CavaHandle>,
    /// Último cuadro del visualizador.
    pub(super) cava_frame: Vec<f32>,
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
    /// Registro de apps para el menú de inicio.
    pub(super) registry: app_bus::AppRegistry,
    /// `true` cuando el drawer de la barra del menú está desplegado.
    pub(super) menu_open: bool,
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
    /// Estado del sidebar navegador.
    pub(super) nav: NavState,
    /// Canal por donde el hilo de poll de `list_monads` entrega resultados.
    pub(super) nav_rx: Option<Receiver<PollOutcome>>,
    /// Canal para que los hilos one-shot de `resolve_monad` entreguen miembros.
    pub(super) members_tx: Sender<MembersOutcome>,
    pub(super) members_rx: Receiver<MembersOutcome>,
    /// Arrastre en curso.
    pub(super) drag: Option<LayerDrag>,
    /// Servidor del rail hospedado.
    pub(super) host: Option<HostServer>,
    /// Última revisión vista del `host`.
    pub(super) last_host_rev: u64,
    /// Una layer surface por cada barra de la config.
    pub(super) panels: Vec<Panel>,
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

    let toplevel_mgr = globals
        .bind::<ZwlrForeignToplevelManagerV1, _, _>(&qh, 1..=3, ())
        .ok();
    if toplevel_mgr.is_none() {
        eprintln!("pata layer · el compositor no expone wlr-foreign-toplevel; window_list vacío");
    }

    let tray = crate::config_tiene_widget(&cfg, "tray")
        .then(TrayHandle::spawn)
        .flatten();
    let weather = crate::config_tiene_widget(&cfg, "weather")
        .then(|| crate::weather::WeatherHandle::spawn(crate::weather_place(&cfg)));
    let cava = crate::config_tiene_widget(&cfg, "cava")
        .then(|| crate::cava::CavaHandle::spawn(crate::cava_bars(&cfg)));

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
        shuma_full,
        shuma_full_handle,
        shuma_full_rx,
        cfg_watch: crate::config_watch::ConfigWatch::new(pata_config::loaded_path()),
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
        host: (!sidebars.is_empty()).then(HostServer::spawn).flatten(),
        last_host_rev: 0,
        panels: Vec::new(),
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
            if name.is_empty() {
                return None;
            }
            if let Some(o) = outputs_by_name.get(name) {
                return Some(o.clone());
            }
            eprintln!("pata layer · output «{name}» no conectado; cae al primario");
            None
        };

    // Los monitores destino de una superficie: `output = "*"`/`"all"` la
    // replica en CADA monitor conectado; si no, su monitor (o el primario).
    let targets_de = |out: &str| -> Vec<Option<wl_output::WlOutput>> {
        if (out == "*" || out.eq_ignore_ascii_case("all")) && !outputs_by_name.is_empty() {
            outputs_by_name.values().cloned().map(Some).collect()
        } else {
            vec![resolve_output(out)]
        }
    };

    // Una layer surface por barra (× monitor si `output = "*"`).
    for &idx in &bars {
        let s = &app.cfg.surfaces[idx];
        let thickness = s.thickness.max(1.0) as u32;
        let (sctk_anchor, size) = anchor_y_size(s.anchor, thickness);
        for target in targets_de(&s.output) {
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
        for target in targets_de(&s.output) {
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
            layer.set_exclusive_zone(thickness as i32);
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
                cursor_x: None,
                gpu: None,
            });
        }
    }

    // Tarjetas flotantes (estilo conky).
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
        app.panels[pi]
            .layer
            .set_keyboard_interactivity(KeyboardInteractivity::None);
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

    // ¿Qué barra hospeda el start_button?
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
