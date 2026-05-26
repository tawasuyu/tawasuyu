//! Demo Tier 1 Android: pinta la pantalla con LEAD_GRAY usando llimphi-hal.
//!
//! Es la contraparte Android de
//! `llimphi-hal/examples/clear_screen.rs`. Misma lógica de render; la
//! diferencia es:
//!
//! 1. Entry-point `android_main` en vez de `main`. La macro
//!    `#[no_mangle] fn android_main(app: AndroidApp)` la inyecta
//!    `android-activity`. Recibe el `AndroidApp` con el handle al
//!    `ANativeActivity` ya inicializado por la JVM.
//! 2. Construimos el `EventLoop` con `with_android_app(app)` para que
//!    winit reciba eventos del ciclo de vida Android
//!    (Resumed/Suspended/InputAvailable).
//! 3. La surface wgpu **se destruye en Suspended y se recrea en
//!    Resumed**. Android invalida la NativeWindow cada vez que la app
//!    pasa a background — usar la misma `Surface` crashearía. Por eso
//!    `State` es `Option<>` y se reconstruye en cada `resumed()`.
//!
//! El resto (vsync, fps reporting, blit a swapchain) lo hace
//! `llimphi-hal::WinitSurface` igual que en desktop.

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
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Android: cada Resumed nos llega con una NativeWindow nueva.
        // Si ya teníamos State viejo, queda obsoleto (la surface apunta
        // a una NativeWindow inválida): lo descartamos y reconstruimos.
        let window = event_loop
            .create_window(WindowAttributes::default().with_title("llimphi · clear_screen"))
            .expect("create window");
        let window = Arc::new(window);
        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        let surface = WinitSurface::new(&hal, window.clone()).expect("surface");
        window.request_redraw();
        self.state = Some(State {
            window,
            hal,
            surface,
        });
        log::info!("llimphi/android · resumed → state reconstruido");
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // Soltamos surface + ventana antes de que Android destruya la
        // NativeWindow. Si no lo hacemos, el siguiente acquire() crashea.
        self.state = None;
        log::info!("llimphi/android · suspended → surface liberada");
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
                state.surface.resize(size.width, size.height);
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let frame = match state.surface.acquire() {
                    Ok(f) => f,
                    Err(_) => {
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
                            label: Some("clear_screen-android-encoder"),
                        });
                {
                    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("clear_screen-android-pass"),
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
                    log::info!("llimphi/android · {fps:.1} fps");
                    self.frames = 0;
                    self.last_report = Instant::now();
                }
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

/// Punto de entrada Android. `android-activity` lo invoca desde el
/// `ANativeActivity_onCreate` después de inicializar la JVM y el
/// looper de eventos. Es el equivalente moral a `fn main()` en desktop.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    // `tag` aparece en `adb logcat -s clear_screen-android`.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("clear_screen-android"),
    );
    log::info!("llimphi/android · android_main start");

    use llimphi_hal::winit::event_loop::EventLoopBuilder;
    use llimphi_hal::winit::platform::android::EventLoopBuilderExtAndroid;

    let event_loop: EventLoop<()> = EventLoopBuilder::default()
        .with_android_app(app)
        .build()
        .expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app_handler = App::new();
    event_loop.run_app(&mut app_handler).expect("run app");
}

// En desktop el crate sigue compilando como cdylib pero sin entry-point:
// queda como ejercicio de compatibilidad de tipos, no produce binario
// usable. Para correr el equivalente en desktop usar
// `cargo run -p llimphi-hal --example clear_screen`.
