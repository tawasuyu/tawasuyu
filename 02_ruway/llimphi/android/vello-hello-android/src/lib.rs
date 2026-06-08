//! Tier 1.5 Android: chacana animada con vello + llimphi-raster.
//!
//! Smoke test del stack raster completo en device móvil:
//!   wgpu (Vulkan/Adreno) → llimphi-hal (intermediate Rgba8) →
//!   vello::Scene (kurbo paths + peniko brushes) →
//!   llimphi_raster::Renderer (compute pipeline AA) →
//!   blit a swapchain.
//!
//! El bootstrap es el mismo orden estricto que `clear-screen-android`:
//! create_surface antes que request_adapter (compatible_surface=Some),
//! WinitSurface::from_surface (no `new`), panic hook al logcat.
//!
//! Si esta app pinta y mantiene fps en device, todas las apps Llimphi
//! basadas en vello están listas para portar mecánicamente — solo hay
//! que envolver su `build_scene` con este shell.

use std::sync::Arc;
use std::time::Instant;

use llimphi_hal::winit::application::ApplicationHandler;
use llimphi_hal::winit::event::WindowEvent;
use llimphi_hal::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use llimphi_hal::winit::window::{Window, WindowAttributes, WindowId};
use llimphi_hal::{wgpu, Hal, Surface, WinitSurface};
use llimphi_raster::kurbo::{Affine, BezPath, Circle, Stroke};
use llimphi_raster::peniko::{Color, Fill};
use llimphi_raster::{vello, Renderer};

const TAG: &str = "llimphi-vello";

// Paleta tawasuyu (mismos hex que la web/Llimphi-theme).
const COSMOS_NIGHT: Color = Color::from_rgba8(0x0E, 0x10, 0x16, 255);
const ACCENT_CYAN: Color = Color::from_rgba8(0xA6, 0xD8, 0xFF, 255);
const ACCENT_AMBER: Color = Color::from_rgba8(0xE8, 0xC9, 0x7A, 255);
const ACCENT_BLUE: Color = Color::from_rgba8(0x6E, 0x8C, 0xDC, 255);
const ACCENT_VIOLET: Color = Color::from_rgba8(0xC3, 0x9C, 0xE8, 255);

struct State {
    window: Arc<Window>,
    hal: Hal,
    surface: WinitSurface,
    renderer: Renderer,
    scene: vello::Scene,
}

struct App {
    state: Option<State>,
    started: Instant,
    frames: u64,
    last_report: Instant,
}

impl App {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            state: None,
            started: now,
            frames: 0,
            last_report: now,
        }
    }

    fn boot(&self, event_loop: &ActiveEventLoop) -> Result<State, String> {
        log::info!("[boot] 1/8 Window");
        let window = event_loop
            .create_window(WindowAttributes::default().with_title("llimphi · vello-hello"))
            .map_err(|e| format!("create_window: {e}"))?;
        let window = Arc::new(window);

        log::info!("[boot] 2/8 wgpu::Instance");
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        log::info!("[boot] 3/8 Surface (única create_surface en este boot)");
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("create_surface: {e}"))?;

        log::info!("[boot] 4/8 Adapter compatible con surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .map_err(|e| format!("request_adapter: {e}"))?;
        let info = adapter.get_info();
        log::info!(
            "[boot] adapter ok · {:?} · {} · {:?}",
            info.backend,
            info.name,
            info.driver_info
        );

        log::info!("[boot] 5/8 Device + Queue");
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("vello-hello-device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::Off,
            },
        ))
        .map_err(|e| format!("request_device: {e}"))?;

        log::info!("[boot] 6/8 Hal");
        let hal = Hal {
            instance,
            adapter,
            device,
            queue,
        };

        log::info!("[boot] 7/8 WinitSurface::from_surface");
        let surface = WinitSurface::from_surface(&hal, window.clone(), surface)
            .map_err(|e| format!("WinitSurface: {e}"))?;

        log::info!("[boot] 8/8 vello Renderer");
        let renderer =
            Renderer::new(&hal).map_err(|e| format!("Renderer::new: {e}"))?;

        log::info!("[boot] ✓ stack raster listo, primer redraw");
        window.request_redraw();

        Ok(State {
            window,
            hal,
            surface,
            renderer,
            scene: vello::Scene::new(),
        })
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        log::info!("Resumed");
        match self.boot(event_loop) {
            Ok(s) => self.state = Some(s),
            Err(e) => log::error!("BOOT FAILED: {e}"),
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("Suspended — liberando state");
        self.state = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                log::info!("Resized → {}x{}", size.width, size.height);
                state.surface.resize(size.width, size.height);
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let frame = match state.surface.acquire() {
                    Ok(f) => f,
                    Err(e) => {
                        log::warn!("acquire {e}, reconfig");
                        let (w, h) = state.surface.size();
                        state.surface.resize(w, h);
                        state.window.request_redraw();
                        return;
                    }
                };
                let (w, h) = frame.size();
                let t = self.started.elapsed().as_secs_f64();
                state.scene.reset();
                build_chacana(&mut state.scene, w as f64, h as f64, t);
                if let Err(e) = state.renderer.render(
                    &state.hal,
                    &state.scene,
                    &frame,
                    COSMOS_NIGHT,
                ) {
                    log::error!("render: {e}");
                }
                state.surface.present(frame, &state.hal);

                self.frames += 1;
                let elapsed = self.last_report.elapsed();
                if elapsed.as_secs() >= 1 {
                    let fps = self.frames as f64 / elapsed.as_secs_f64();
                    log::info!("{fps:.1} fps · {w}x{h}");
                    self.frames = 0;
                    self.last_report = Instant::now();
                }
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

/// Construye la chacana (cruz andina escalonada) animada, centrada en el
/// viewport. El sol central late con sin(t); cuatro rayos cardinales
/// rotan en una vuelta cada 12 s; halo cyan constante.
fn build_chacana(scene: &mut vello::Scene, w: f64, h: f64, t: f64) {
    let cx = w * 0.5;
    let cy = h * 0.5;
    let unit = (w.min(h)) * 0.06; // tamaño de la escala de la cruz

    // Halo radial (anillo cyan suave)
    scene.stroke(
        &Stroke::new(2.0),
        Affine::IDENTITY,
        Color::from_rgba8(0xA6, 0xD8, 0xFF, 80),
        None,
        &Circle::new((cx, cy), unit * 4.6),
    );
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        Color::from_rgba8(0xA6, 0xD8, 0xFF, 140),
        None,
        &Circle::new((cx, cy), unit * 4.0),
    );

    // Rayos cardinales rotantes (4 trazos a 90°)
    let theta = t * (std::f64::consts::TAU / 12.0); // 1 vuelta cada 12 s
    let rotate = Affine::translate((cx, cy)) * Affine::rotate(theta);
    for i in 0..4 {
        let angle = i as f64 * std::f64::consts::FRAC_PI_2;
        let dir = (angle.cos(), angle.sin());
        let mut p = BezPath::new();
        p.move_to((dir.0 * unit * 3.2, dir.1 * unit * 3.2));
        p.line_to((dir.0 * unit * 4.4, dir.1 * unit * 4.4));
        scene.stroke(
            &Stroke::new(1.5),
            rotate,
            ACCENT_BLUE,
            None,
            &p,
        );
    }

    // Chacana: cruz escalonada de 12 puntas. Construida como BezPath.
    // La forma clásica: cuadrado central + escalones en 4 direcciones.
    let chacana = chacana_path(unit);
    let center = Affine::translate((cx, cy));

    // Glow ambar exterior
    scene.stroke(
        &Stroke::new(6.0),
        center,
        Color::from_rgba8(0xE8, 0xC9, 0x7A, 110),
        None,
        &chacana,
    );
    // Outline cyan
    scene.stroke(
        &Stroke::new(2.0),
        center,
        ACCENT_CYAN,
        None,
        &chacana,
    );
    // Relleno violeta tenue
    scene.fill(
        Fill::NonZero,
        center,
        Color::from_rgba8(0xC3, 0x9C, 0xE8, 40),
        None,
        &chacana,
    );

    // Sol central que late
    let pulse = 1.0 + 0.18 * (t * 1.8).sin();
    let r_sun = unit * 0.7 * pulse;
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        ACCENT_AMBER,
        None,
        &Circle::new((cx, cy), r_sun),
    );
    // Corona
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        Color::from_rgba8(0xE8, 0xC9, 0x7A, 120),
        None,
        &Circle::new((cx, cy), r_sun * 1.7),
    );
    // Punto interior violeta para contraste
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        ACCENT_VIOLET,
        None,
        &Circle::new((cx, cy), r_sun * 0.35),
    );
}

/// Path de la chacana centrada en el origen, con `u` como ancho de cada
/// escalón. Reconstruye la forma clásica de 12 esquinas escalonadas
/// (3 escalones por cada brazo cardinal).
fn chacana_path(u: f64) -> BezPath {
    let mut p = BezPath::new();
    // Empezamos en la esquina superior-derecha del brazo norte y vamos
    // en sentido horario alrededor de toda la cruz.
    p.move_to((u, 3.0 * u));
    p.line_to((u, u));
    p.line_to((3.0 * u, u));
    p.line_to((3.0 * u, -u));
    p.line_to((u, -u));
    p.line_to((u, -3.0 * u));
    p.line_to((-u, -3.0 * u));
    p.line_to((-u, -u));
    p.line_to((-3.0 * u, -u));
    p.line_to((-3.0 * u, u));
    p.line_to((-u, u));
    p.line_to((-u, 3.0 * u));
    p.close_path();
    p
}

#[cfg(target_os = "android")]
fn install_panic_logger() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("<unknown>");
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<?>".into());
        log::error!("PANIC at {loc} — {payload}");
    }));
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag(TAG),
    );
    install_panic_logger();
    log::info!("android_main START");

    use llimphi_hal::winit::event_loop::EventLoopBuilder;
    use llimphi_hal::winit::platform::android::EventLoopBuilderExtAndroid;

    let event_loop: EventLoop<()> = match EventLoopBuilder::default().with_android_app(app).build()
    {
        Ok(el) => el,
        Err(e) => {
            log::error!("EventLoop: {e}");
            return;
        }
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut handler = App::new();
    if let Err(e) = event_loop.run_app(&mut handler) {
        log::error!("run_app: {e}");
    }
}
