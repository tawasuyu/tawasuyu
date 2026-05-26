//! Demo Tier 1 Android: pinta la pantalla con LEAD_GRAY usando llimphi-hal.
//!
//! Logging exhaustivo en cada paso del bootstrap para diagnosticar
//! cuelgues en device real desde `adb logcat -s llimphi-android:V`.
//! Panic hook captura backtraces a logcat — sin esto el crash es
//! invisible (Android cierra el proceso silenciosamente).
//!
//! Orden de inicialización en `resumed`:
//!   1. crear Window via winit
//!   2. crear wgpu::Instance
//!   3. crear Surface con la NativeWindow
//!   4. request_adapter pasándole compatible_surface=Some(&surface)
//!   5. request_device
//!   6. configurar surface (formato, tamaño)
//!   7. crear textura intermedia + blitter (llimphi-hal::WinitSurface)
//!
//! El orden 3 antes que 4 es lo que **garantiza** que el adapter
//! elegido sabe presentar a esa NativeWindow concreta. Llamar
//! `Hal::new(None)` (como hacía la primera versión) elige un adapter
//! "cualquiera" y después la creación de surface puede fallar — o
//! peor, parecer OK y crashear en el primer `present`.

use std::sync::Arc;
use std::time::Instant;

use llimphi_hal::winit::application::ApplicationHandler;
use llimphi_hal::winit::event::WindowEvent;
use llimphi_hal::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use llimphi_hal::winit::window::{Window, WindowAttributes, WindowId};
use llimphi_hal::{wgpu, Hal, Surface, WinitSurface};

const LEAD_GRAY: wgpu::Color = wgpu::Color {
    r: 0.235,
    g: 0.239,
    b: 0.247,
    a: 1.0,
};

const TAG: &str = "llimphi-android";

struct State {
    window: Arc<Window>,
    hal: Hal,
    surface: WinitSurface,
}

struct App {
    state: Option<State>,
    frames: u64,
    last_report: Instant,
}

impl App {
    fn new() -> Self {
        Self {
            state: None,
            frames: 0,
            last_report: Instant::now(),
        }
    }

    /// Bootstrap: crea el estado completo o devuelve un mensaje
    /// explicando dónde falló. **No panic-ea** — los panics en
    /// `android_main` arrancan la cierre del proceso antes que el
    /// logcat flushee.
    fn boot(&self, event_loop: &ActiveEventLoop) -> Result<State, String> {
        log::info!("[boot] 1/7 creando Window");
        let window = event_loop
            .create_window(WindowAttributes::default().with_title("llimphi · clear_screen"))
            .map_err(|e| format!("create_window: {e}"))?;
        let window = Arc::new(window);
        let size = window.inner_size();
        log::info!(
            "[boot] window ok · inner_size = {}x{}",
            size.width,
            size.height
        );

        log::info!("[boot] 2/7 creando wgpu::Instance");
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        log::info!("[boot] instance ok · backends activos = {:?}", instance);

        log::info!("[boot] 3/7 creando Surface contra la NativeWindow");
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("create_surface: {e}"))?;
        log::info!("[boot] surface creada");

        log::info!("[boot] 4/7 request_adapter (compatible_surface=Some)");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .ok_or_else(|| "request_adapter devolvió None — sin GPU compatible".to_string())?;
        let info = adapter.get_info();
        log::info!(
            "[boot] adapter ok · backend={:?} name={:?} driver={:?}",
            info.backend,
            info.name,
            info.driver_info
        );

        log::info!("[boot] 5/7 request_device");
        // En Android (Mali/Adreno entry-level) Limits::default suele exceder
        // el hardware. using_resolution recorta lo recortable preservando
        // los counts mínimos (5 storage buffers/stage que vello necesita).
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("clear-screen-android-device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e| format!("request_device: {e}"))?;
        log::info!("[boot] device + queue ok");

        log::info!("[boot] 6/7 ensamblando Hal");
        let hal = Hal {
            instance,
            adapter,
            device,
            queue,
        };

        log::info!("[boot] 7/7 envolviendo en WinitSurface (intermediate + blitter)");
        // Crítico: usar `from_surface` (no `new`), pasando la surface que
        // ya creamos en el paso 3. `WinitSurface::new` haría un segundo
        // create_surface contra la misma NativeWindow y Android responde
        // ERROR_NATIVE_WINDOW_IN_USE_KHR → panic.
        let llimphi_surface = WinitSurface::from_surface(&hal, window.clone(), surface)
            .map_err(|e| format!("WinitSurface::from_surface: {e}"))?;
        log::info!("[boot] ✓ bootstrap completo, pidiendo redraw");
        window.request_redraw();

        Ok(State {
            window,
            hal,
            surface: llimphi_surface,
        })
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        log::info!("Resumed event");
        match self.boot(event_loop) {
            Ok(state) => self.state = Some(state),
            Err(e) => {
                log::error!("BOOT FAILED: {e}");
                // No exit-amos para que el process siga vivo y se vea el
                // log; el usuario cerrará la app manualmente.
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("Suspended event — liberando surface");
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
            WindowEvent::CloseRequested => {
                log::info!("CloseRequested");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                log::info!("Resized → {}x{}", size.width, size.height);
                state.surface.resize(size.width, size.height);
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let frame = match state.surface.acquire() {
                    Ok(f) => f,
                    Err(e) => {
                        log::warn!("acquire falló ({e}); reconfigurando");
                        let (w, h) = state.surface.size();
                        state.surface.resize(w, h);
                        state.window.request_redraw();
                        return;
                    }
                };
                let mut encoder =
                    state
                        .hal
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("clear_screen-encoder"),
                        });
                {
                    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("clear_screen-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: frame.view(),
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(LEAD_GRAY),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                }
                state.hal.queue.submit(std::iter::once(encoder.finish()));
                state.surface.present(frame, &state.hal);

                self.frames += 1;
                let elapsed = self.last_report.elapsed();
                if elapsed.as_secs() >= 1 {
                    let fps = self.frames as f64 / elapsed.as_secs_f64();
                    log::info!("{fps:.1} fps");
                    self.frames = 0;
                    self.last_report = Instant::now();
                }
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

#[cfg(target_os = "android")]
fn install_panic_logger() {
    // Sin esto los panic son invisibles: Android mata el proceso antes
    // que la línea de stderr llegue a logcat. set_hook redirige el panic
    // info a log::error que sí sale en logcat (vía android_logger).
    std::panic::set_hook(Box::new(|info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("<unknown panic payload>");
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown location>".into());
        log::error!("PANIC at {location} — {payload}");
        // Forzar flush stdio del android_logger (mejor que nada).
    }));
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Debug)
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
            log::error!("EventLoop::build failed: {e}");
            return;
        }
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    log::info!("event_loop construido, entrando a run_app");

    let mut app_handler = App::new();
    if let Err(e) = event_loop.run_app(&mut app_handler) {
        log::error!("run_app: {e}");
    }
    log::info!("android_main END");
}
