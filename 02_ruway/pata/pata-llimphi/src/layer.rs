//! Backend `wlr-layer-shell`: hace que `pata` se siente **al nivel de eww/
//! waybar** en cualquier compositor wlroots (Hyprland, Sway, river…), no como
//! una ventana cliente.
//!
//! Una *layer surface* se ancla a un borde y declara una *exclusive zone* —el
//! compositor le reserva esa franja y tesela el resto alrededor—, exactamente
//! lo que hace eww. Aquí: nos conectamos a Wayland con `smithay-client-toolkit`,
//! creamos la layer surface anclada según la config de `pata`, sacamos una
//! `wgpu::Surface` de los punteros raw del `wl_surface`/`wl_display` (envuelta en
//! [`RawSurface`]), y pintamos la barra reusando el pipeline de Llimphi
//! (`mount → compute → paint → render`).
//!
//! **Estado**: pinta la primera barra de la config (anclaje + exclusive zone).
//! El input (teclado para el Quake, clicks) y el resto de superficies llegan en
//! el siguiente incremento. No se puede verificar headless: se itera en un
//! compositor real.

use std::error::Error;
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

/// El estado wgpu de la layer surface, creado cuando llega el primer `configure`
/// (ahí conocemos el tamaño real que asignó el compositor).
struct Gpu {
    hal: Hal,
    surface: RawSurface,
    renderer: Renderer,
    typesetter: Typesetter,
    scene: vello::Scene,
    layout: LayoutTree,
}

/// El cliente Wayland del backend layer-shell.
struct LayerApp {
    registry_state: RegistryState,
    output_state: OutputState,
    conn: Connection,
    layer: LayerSurface,
    theme: Theme,
    cfg: Config,
    surfaces: Vec<crate::SurfaceWidgets>,
    shuma: crate::shuma::ShumaState,
    sampler: Sampler,
    /// Último snapshot del sistema y cuándo se tomó: el frame-callback corre a
    /// ~60fps, pero re-muestrear (y cambiar el CPU%) sólo tiene sentido ~1Hz —
    /// si no, el porcentaje baila varias veces por segundo.
    ctx: WidgetCtx,
    ultimo_sample: Option<Instant>,
    /// `true` cuando hay algo nuevo que pintar (cambió el muestreo o el tamaño).
    /// Entre cambios sólo mantenemos vivo el latido del frame-callback sin
    /// re-rasterizar — pintar 60 veces por segundo un contenido idéntico es puro
    /// gasto de GPU/CPU.
    dirty: bool,
    /// Índice (en `cfg.surfaces`) de la barra que esta layer surface pinta.
    bar_index: usize,
    gpu: Option<Gpu>,
    width: u32,
    height: u32,
    exit: bool,
}

/// Levanta el backend layer-shell. Devuelve error si no hay sesión Wayland o el
/// compositor no expone `wlr-layer-shell` (en ese caso el caller cae a winit).
pub fn run() -> Result<(), Box<dyn Error>> {
    let cfg = pata_config::load();
    // La barra a anclar: la primera superficie `Bar` de la config.
    let bar_index = cfg
        .surfaces
        .iter()
        .position(|s| s.kind == SurfaceKind::Bar)
        .ok_or("pata · la config no tiene ninguna superficie 'bar' para anclar")?;
    let anchor = cfg.surfaces[bar_index].anchor;
    let thickness = cfg.surfaces[bar_index].thickness.max(1.0) as u32;

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh: QueueHandle<LayerApp> = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let wl_surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        wl_surface,
        Layer::Top,
        Some("pata".to_string()),
        None,
    );

    // Anclaje: el borde + estirar a lo largo del eje perpendicular. El tamaño
    // en el eje libre va en 0 → el compositor lo estira al ancho/alto de la
    // salida; el otro eje es el grosor de la barra.
    let (sctk_anchor, size) = match anchor {
        Anchor::Top => (LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT, (0, thickness)),
        Anchor::Bottom => (
            LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT,
            (0, thickness),
        ),
        Anchor::Left => (LayerAnchor::LEFT | LayerAnchor::TOP | LayerAnchor::BOTTOM, (thickness, 0)),
        Anchor::Right => (
            LayerAnchor::RIGHT | LayerAnchor::TOP | LayerAnchor::BOTTOM,
            (thickness, 0),
        ),
    };
    layer.set_anchor(sctk_anchor);
    layer.set_size(size.0, size.1);
    layer.set_exclusive_zone(thickness as i32);
    layer.commit();

    let (surfaces, shuma) = Model::construir(&cfg);
    let mut app = LayerApp {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        conn: conn.clone(),
        layer,
        theme: Theme::dark(),
        cfg,
        surfaces,
        shuma,
        sampler: Sampler::new(),
        ctx: WidgetCtx::default(),
        ultimo_sample: None,
        dirty: true,
        bar_index,
        gpu: None,
        width: size.0.max(1),
        height: thickness,
        exit: false,
    };

    while !app.exit {
        event_queue.blocking_dispatch(&mut app)?;
    }
    Ok(())
}

impl LayerApp {
    /// Crea el estado wgpu sobre los punteros raw de Wayland (`wl_display` +
    /// `wl_surface`). Se llama en el primer `configure`, cuando ya hay tamaño.
    fn ensure_gpu(&mut self) {
        if self.gpu.is_some() {
            return;
        }
        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        // Handles raw: punteros que viven mientras viva la conexión / surface.
        let display_handle = {
            let ptr = self.conn.backend().display_ptr() as *mut std::ffi::c_void;
            RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                NonNull::new(ptr).expect("wl_display ptr"),
            ))
        };
        let window_handle = {
            let ptr = self.layer.wl_surface().id().as_ptr() as *mut std::ffi::c_void;
            RawWindowHandle::Wayland(WaylandWindowHandle::new(
                NonNull::new(ptr).expect("wl_surface ptr"),
            ))
        };
        // SAFETY: los handles apuntan a objetos Wayland que `self` mantiene
        // vivos (la conexión y la layer surface) durante toda la vida de la
        // `wgpu::Surface`.
        let wgpu_surface = unsafe {
            hal.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: display_handle,
                    raw_window_handle: window_handle,
                })
                .expect("create_surface")
        };
        let surface =
            RawSurface::from_surface(&hal, wgpu_surface, self.width, self.height).expect("surface");
        let renderer = Renderer::new(&hal).expect("renderer");
        self.gpu = Some(Gpu {
            hal,
            surface,
            renderer,
            typesetter: Typesetter::new(),
            scene: vello::Scene::new(),
            layout: LayoutTree::new(),
        });
    }

    /// Mantiene vivo el latido: pide el siguiente frame-callback. Un `commit`
    /// sin buffer nuevo sólo re-agenda la llamada; el contenido actual se queda.
    fn latido(&self, qh: &QueueHandle<Self>) {
        let surface = self.layer.wl_surface();
        surface.frame(qh, surface.clone());
        surface.commit();
    }

    /// Avanza el frame: re-muestrea ~1Hz, y pinta sólo si hay algo nuevo. El
    /// frame-callback late a ~60fps, pero re-rasterizar contenido idéntico sería
    /// puro gasto, así que entre cambios sólo mantenemos el latido.
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        self.ensure_gpu();

        // Re-muestrear sólo ~1Hz: muestrear a 60fps hace bailar el CPU% (delta
        // sobre ~16ms, ruidoso) y, antes del ancho fijo, reacomodaba la barra.
        let toca_muestrear = self
            .ultimo_sample
            .map(|t| t.elapsed() >= Duration::from_secs(1))
            .unwrap_or(true);
        if toca_muestrear {
            self.ctx = self.sampler.sample();
            self.ultimo_sample = Some(Instant::now());
            let ctx = self.ctx;
            for sw in &mut self.surfaces {
                for w in sw.core_mut() {
                    w.tick(&ctx);
                }
            }
            self.dirty = true;
        }

        // Sin cambios: sólo latir y salir, sin re-rasterizar.
        if !self.dirty {
            self.latido(qh);
            return;
        }
        self.dirty = false;

        let (w, h) = (self.width, self.height);
        let view = render::bar_view(
            &self.cfg.surfaces[self.bar_index],
            &self.surfaces[self.bar_index],
            &self.shuma,
            &self.theme,
        );

        let Some(gpu) = self.gpu.as_mut() else {
            self.latido(qh);
            return;
        };
        gpu.surface.resize(w, h);
        let frame = match gpu.surface.acquire() {
            Ok(f) => f,
            Err(_) => {
                self.latido(qh);
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
        if let Err(e) = gpu
            .renderer
            .render(&gpu.hal, &gpu.scene, &frame, palette::css::BLACK)
        {
            eprintln!("pata layer · render: {e}");
        }
        gpu.surface.present(frame, &gpu.hal);

        self.latido(qh);
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
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
        self.draw(qh);
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
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        // El compositor nos da el tamaño definitivo (el eje libre ya resuelto).
        let (cw, ch) = configure.new_size;
        if cw > 0 {
            self.width = cw;
        }
        if ch > 0 {
            self.height = ch;
        }
        self.dirty = true; // tamaño nuevo → hay que re-pintar
        self.draw(qh);
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
