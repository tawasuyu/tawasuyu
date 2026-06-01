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
//! **Estado**: pinta todas las barras de la config (varios bordes a la vez). El
//! input (teclado para el Quake, clicks) y el drawer Quake llegan después. No se
//! verifica headless: se itera en un compositor real.

use std::error::Error;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent as KbEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT},
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
use llimphi_ui::llimphi_compositor::{hit_test_click, measure_text_node, mount, paint, Mounted};
use llimphi_ui::llimphi_hal::{wgpu, Hal, RawSurface, Surface as _};
use llimphi_ui::llimphi_layout::{taffy, ComputedLayout, LayoutTree};
use llimphi_ui::llimphi_raster::{peniko::color::palette, vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use pata_core::widget::WidgetCtx;
use pata_core::{Anchor, Config, SurfaceKind};

use crate::sampler::Sampler;
use crate::toplevel::{Toplevel, WindowEntry};
use crate::tray::TrayHandle;
use crate::{render, Model, Msg};

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

/// Una barra = una layer surface anclada a un borde, con su propio estado wgpu.
struct Panel {
    /// Índice de su superficie en `cfg.surfaces`.
    idx: usize,
    layer: LayerSurface,
    /// El árbol del último frame (para hit-test de clicks).
    cache: Option<RenderCache>,
    width: u32,
    height: u32,
    /// `true` cuando hay algo nuevo que pintar (cambió el muestreo o el tamaño).
    dirty: bool,
    gpu: Option<PanelGpu>,
}

/// Alto del drawer Quake cuando se despliega (px). El compositor lo clampa a la
/// salida; la barra crece hacia arriba hasta este alto.
const DRAWER_H: u32 = 420;

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
    theme: Theme,
    cfg: Config,
    surfaces: Vec<crate::SurfaceWidgets>,
    shuma: crate::shuma::ShumaState,
    /// Índice (en `panels`) de la barra que hospeda el `shuma_input`, si hay.
    shuma_panel: Option<usize>,
    /// Grosor original (px) de esa barra — al que vuelve al replegar el drawer.
    shuma_bar_px: u32,
    sampler: Sampler,
    /// Último snapshot del sistema y cuándo se tomó: los frame-callbacks corren a
    /// ~60fps, pero re-muestrear (y cambiar el CPU%) sólo tiene sentido ~1Hz.
    ctx: WidgetCtx,
    ultimo_sample: Option<Instant>,
    /// Comando del Quake corriendo en un hilo: su resultado llega por aquí. El
    /// latido del frame-callback lo sondea (`try_recv`) sin bloquear el loop.
    exec_rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// Una layer surface por cada barra de la config.
    panels: Vec<Panel>,
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
    let bars: Vec<usize> = cfg
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SurfaceKind::Bar)
        .map(|(i, _)| i)
        .collect();
    if bars.is_empty() {
        return Err("pata · la config no tiene ninguna superficie 'bar' para anclar".into());
    }

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

    // Una layer surface por barra: anclada a su borde, con su exclusive zone.
    let mut panels = Vec::new();
    for &idx in &bars {
        let s = &cfg.surfaces[idx];
        let thickness = s.thickness.max(1.0) as u32;
        let (sctk_anchor, size) = anchor_y_size(s.anchor, thickness);
        let wl_surface = compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(
            &qh,
            wl_surface,
            Layer::Top,
            Some("pata".to_string()),
            None,
        );
        layer.set_anchor(sctk_anchor);
        layer.set_size(size.0, size.1);
        layer.set_exclusive_zone(thickness as i32);
        layer.commit();
        panels.push(Panel {
            idx,
            layer,
            cache: None,
            width: size.0.max(1),
            height: thickness,
            dirty: true,
            gpu: None,
        });
    }

    // ¿Qué barra hospeda el shuma_input? Esa recibe foco de teclado al clickearla
    // (OnDemand) para poder desplegar el Quake y escribir.
    let shuma_panel = panels.iter().position(|p| {
        let s = &cfg.surfaces[p.idx];
        s.start
            .iter()
            .chain(&s.center)
            .chain(&s.end)
            .any(|w| w.kind == "shuma_input")
    });
    let shuma_bar_px = shuma_panel
        .map(|pi| cfg.surfaces[panels[pi].idx].thickness.max(1.0) as u32)
        .unwrap_or(40);
    if let Some(pi) = shuma_panel {
        panels[pi]
            .layer
            .set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        panels[pi].layer.commit();
    }

    // El tray sólo arranca (y toma el nombre del watcher) si la config lo pide.
    let tray = crate::config_tiene_widget(&cfg, "tray")
        .then(TrayHandle::spawn)
        .flatten();

    let (surfaces, shuma) = Model::construir(&cfg);
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
        theme: Theme::dark(),
        cfg,
        surfaces,
        shuma,
        shuma_panel,
        shuma_bar_px,
        sampler: Sampler::new(),
        ctx: WidgetCtx::default(),
        ultimo_sample: None,
        exec_rx: None,
        panels,
        exit: false,
    };

    while !app.exit {
        event_queue.blocking_dispatch(&mut app)?;
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
                active: t.activated,
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
        let layer = &self.panels[pi].layer;
        layer.set_size(0, h);
        layer.set_keyboard_interactivity(if open {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::OnDemand
        });
        layer.commit();
        self.panels[pi].dirty = true;
    }

    /// Enter en el drawer: corre el comando en un **hilo** (no bloquea el loop) y
    /// deja el resultado en `exec_rx`; el latido lo recoge con [`Self::poll_exec`].
    /// El puente real a `shuma` reemplaza a esto.
    fn shuma_submit(&mut self) {
        let cmd = std::mem::take(&mut self.shuma.buffer);
        if cmd.is_empty() {
            self.marcar_shuma_dirty();
            return;
        }
        self.shuma.pending = true;
        self.shuma.output = None;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::shuma::ejecutar_stand_in(&cmd));
        });
        self.exec_rx = Some(rx);
        self.marcar_shuma_dirty();
    }

    /// Sondea (sin bloquear) si el comando del Quake terminó; si sí, guarda su
    /// salida y re-pinta. Se llama en cada frame (el latido del shuma corre a
    /// ~60fps, así que el resultado aparece a los ~16ms de terminar).
    fn poll_exec(&mut self) {
        let got = self.exec_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(res) = got {
            self.shuma.output = Some(res);
            self.shuma.pending = false;
            self.exec_rx = None;
            self.marcar_shuma_dirty();
        }
    }

    /// Re-muestrea el sistema si pasó ~1s; si lo hace, marca todas las barras
    /// para re-pintar. Muestrear a 60fps haría bailar el CPU% (delta ruidoso).
    fn maybe_sample(&mut self) {
        let toca = self
            .ultimo_sample
            .map(|t| t.elapsed() >= Duration::from_secs(1))
            .unwrap_or(true);
        if !toca {
            return;
        }
        self.ctx = self.sampler.sample();
        self.clipboard = crate::sampler::leer_clipboard();
        self.ultimo_sample = Some(Instant::now());
        let ctx = self.ctx;
        for sw in &mut self.surfaces {
            for w in sw.core_mut() {
                w.tick(&ctx);
            }
        }
        for p in &mut self.panels {
            p.dirty = true;
        }
    }

    /// Crea el estado wgpu de un panel sobre los punteros raw de Wayland
    /// (`wl_display` + `wl_surface`). El `Hal` se comparte; lo crea el primero.
    fn ensure_gpu(&mut self, pi: usize) {
        if self.panels[pi].gpu.is_some() {
            return;
        }
        if self.hal.is_none() {
            self.hal = Some(pollster::block_on(Hal::new(None)).expect("hal"));
        }
        let hal = self.hal.as_ref().expect("hal");
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
        let wgpu_surface = unsafe {
            hal.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: display_handle,
                    raw_window_handle: window_handle,
                })
                .expect("create_surface")
        };
        let surface = RawSurface::from_surface(hal, wgpu_surface, w, h).expect("surface");
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
        self.poll_exec();
        self.ensure_gpu(pi);
        eprintln!(
            "pata · draw pi={pi} {}x{} dirty={} gpu={}",
            self.panels[pi].width,
            self.panels[pi].height,
            self.panels[pi].dirty,
            self.panels[pi].gpu.is_some()
        );

        if !self.panels[pi].dirty {
            self.latido(pi, qh);
            return;
        }
        self.panels[pi].dirty = false;

        let idx = self.panels[pi].idx;
        let (w, h) = (self.panels[pi].width, self.panels[pi].height);
        let windows = self.window_entries();
        let tray_items = self.tray.as_ref().map(|t| t.items()).unwrap_or_default();
        let data = render::BarData {
            windows: &windows,
            clipboard: self.clipboard.as_deref(),
            tray: &tray_items,
        };
        // La barra de shuma desplegada pinta el drawer (cuerpo + cabezal); el
        // resto pinta su barra normal.
        let view = if self.shuma_panel == Some(pi) && self.shuma.open {
            render::shuma_open_view(
                &self.cfg.surfaces[idx],
                &self.surfaces[idx],
                &self.shuma,
                &data,
                &self.theme,
                self.shuma_bar_px as f32,
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
        paint(&mut gpu.scene, &mounted, &computed, &mut gpu.typesetter, None, None);
        if let Err(e) = gpu.renderer.render(hal, &gpu.scene, &frame, palette::css::BLACK) {
            eprintln!("pata layer · render: {e}");
        }
        gpu.surface.present(frame, hal);
        eprintln!("pata · present pi={pi} ({w}x{h}) hecho");

        // Guarda el árbol pintado para el hit-test de los clicks.
        self.panels[pi].cache = Some(RenderCache { mounted, computed });
        self.latido(pi, qh);
    }

    /// Aplica el `Msg` que produjo un click: togglear shuma (su cabezal) o lanzar
    /// el comando de un widget con `exec`. El resto no sale de un click.
    fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::ShumaToggle => self.set_shuma_open(!self.shuma.open),
            Msg::Spawn(cmd) => crate::spawn_cmd(&cmd),
            Msg::ActivateWindow(id) => self.activar_ventana(id),
            Msg::TrayActivate(key) => {
                if let Some(t) = &self.tray {
                    t.activate(key);
                }
            }
            Msg::Quit => self.exit = true,
            _ => {}
        }
    }

    /// Trae al frente la ventana `id` vía `activate(seat)`. Sin seat (raro) no
    /// hace nada. El compositor manda luego un `state`/`done` que actualiza el
    /// resaltado de la activa.
    fn activar_ventana(&mut self, id: u32) {
        let Some(seat) = self.seat.clone() else { return };
        if let Some(t) = self.toplevel_por_id(id) {
            t.handle.activate(&seat);
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
        let pi_dbg = self.panel_de(layer.wl_surface());
        eprintln!("pata · configure recibido: panel={pi_dbg:?} new_size={cw}x{ch}");
        let Some(pi) = pi_dbg else {
            return;
        };
        // El compositor nos da el tamaño definitivo (el eje libre ya resuelto).
        if cw > 0 {
            self.panels[pi].width = cw;
        }
        if ch > 0 {
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
        // El teclado sólo nos importa con el drawer abierto (foco Exclusive).
        if !self.shuma.open {
            return;
        }
        match event.keysym {
            Keysym::Escape => self.set_shuma_open(false),
            Keysym::BackSpace => {
                self.shuma.buffer.pop();
                self.marcar_shuma_dirty();
            }
            Keysym::Return | Keysym::KP_Enter => self.shuma_submit(),
            _ => {
                if let Some(txt) = event.utf8 {
                    if !txt.is_empty() && !txt.chars().any(|c| c.is_control()) {
                        self.shuma.buffer.push_str(&txt);
                        self.marcar_shuma_dirty();
                    }
                }
            }
        }
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
        _: Modifiers,
        _: u32,
    ) {
    }
}

impl PointerHandler for LayerApp {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for e in events {
            if let PointerEventKind::Press { button, .. } = e.kind {
                if button != BTN_LEFT {
                    continue;
                }
                // Hit-test: qué nodo está bajo el puntero y qué `on_click` dispara.
                // Así el cabezal `› shuma` togglea con precisión (clickear el reloj
                // o un medidor no hace nada). El click ya dio foco de teclado.
                let Some(pi) = self.panel_de(&e.surface) else {
                    continue;
                };
                let (px, py) = (e.position.0 as f32, e.position.1 as f32);
                let msg = self.panels[pi].cache.as_ref().and_then(|c| {
                    hit_test_click(&c.mounted, &c.computed, px, py)
                        .and_then(|i| c.mounted.nodes.get(i))
                        .and_then(|n| n.on_click.clone())
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
