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
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor as LayerAnchor, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_surface},
    Connection, Proxy, QueueHandle,
};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint, Mounted};
use llimphi_ui::llimphi_hal::{wgpu, Hal, RawSurface, Surface as _};
use llimphi_ui::llimphi_layout::{taffy, LayoutTree};
use llimphi_ui::llimphi_raster::{peniko::color::palette, vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use pata_core::widget::WidgetCtx;
use pata_core::{Anchor, Config, SurfaceKind};

use crate::sampler::Sampler;
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

/// Una barra = una layer surface anclada a un borde, con su propio estado wgpu.
struct Panel {
    /// Índice de su superficie en `cfg.surfaces`.
    idx: usize,
    layer: LayerSurface,
    width: u32,
    height: u32,
    /// `true` cuando hay algo nuevo que pintar (cambió el muestreo o el tamaño).
    dirty: bool,
    gpu: Option<PanelGpu>,
}

/// El cliente Wayland del backend layer-shell.
struct LayerApp {
    registry_state: RegistryState,
    output_state: OutputState,
    conn: Connection,
    /// `Hal` compartido (una instancia/device de wgpu para todas las barras).
    hal: Option<Hal>,
    theme: Theme,
    cfg: Config,
    surfaces: Vec<crate::SurfaceWidgets>,
    shuma: crate::shuma::ShumaState,
    sampler: Sampler,
    /// Último snapshot del sistema y cuándo se tomó: los frame-callbacks corren a
    /// ~60fps, pero re-muestrear (y cambiar el CPU%) sólo tiene sentido ~1Hz.
    ctx: WidgetCtx,
    ultimo_sample: Option<Instant>,
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
            width: size.0.max(1),
            height: thickness,
            dirty: true,
            gpu: None,
        });
    }

    let (surfaces, shuma) = Model::construir(&cfg);
    let mut app = LayerApp {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        conn,
        hal: None,
        theme: Theme::dark(),
        cfg,
        surfaces,
        shuma,
        sampler: Sampler::new(),
        ctx: WidgetCtx::default(),
        ultimo_sample: None,
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
        self.ensure_gpu(pi);

        if !self.panels[pi].dirty {
            self.latido(pi, qh);
            return;
        }
        self.panels[pi].dirty = false;

        let idx = self.panels[pi].idx;
        let (w, h) = (self.panels[pi].width, self.panels[pi].height);
        let view = render::bar_view(
            &self.cfg.surfaces[idx],
            &self.surfaces[idx],
            &self.shuma,
            &self.theme,
        );

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
                drop(gpu);
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

        self.latido(pi, qh);
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
        let Some(pi) = self.panel_de(layer.wl_surface()) else {
            return;
        };
        // El compositor nos da el tamaño definitivo (el eje libre ya resuelto).
        let (cw, ch) = configure.new_size;
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

impl ProvidesRegistryState for LayerApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_compositor!(LayerApp);
delegate_output!(LayerApp);
delegate_layer!(LayerApp);
delegate_registry!(LayerApp);
