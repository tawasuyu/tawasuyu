//! Tier 1.75 Android: texto multi-script con parley + vello + llimphi-text.
//!
//! Verifica que en Android funciona:
//!  - parley::FontContext::new() resolviendo fuentes via fontique sobre
//!    /system/fonts (Roboto + Noto fallback CJK/Arabic vienen en todas
//!    las builds AOSP).
//!  - shaping con kerning, ligaduras, bidi, fallback inter-script en
//!    una misma línea.
//!  - rasterización de glifos por vello::Scene::draw_glyphs (compute
//!    pipeline sobre la intermediate Rgba8).
//!
//! Si esta corre estable y se ven los tres scripts (latino, arábigo,
//! CJK) sin tofu (cuadrados vacíos), llimphi-ui está habilitado en
//! Android — el resto de las apps (text-viewer, file-explorer,
//! pluma-md-reader) usan exactamente esta misma pipa.
//!
//! El factor de scale por DPI se calcula desde el `inner_size` real
//! del Window que Android nos pasa (ya incluye la densidad del
//! display). En desktop el window es 960x540 lógico; en mobile típico
//! es ~1080x2400 físico → fuentes 2-3× más grandes para legibilidad.

use std::sync::Arc;
use std::time::Instant;

use llimphi_hal::winit::application::ApplicationHandler;
use llimphi_hal::winit::event::WindowEvent;
use llimphi_hal::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use llimphi_hal::winit::window::{Window, WindowAttributes, WindowId};
use llimphi_hal::{wgpu, Hal, Surface, WinitSurface};
use llimphi_raster::peniko::Color;
use llimphi_raster::vello;
use llimphi_text::{draw_block, Alignment, TextBlock, Typesetter};

const TAG: &str = "llimphi-text";

const COSMOS_NIGHT: Color = Color::from_rgba8(0x0E, 0x10, 0x16, 255);
const FG_TEXT: Color = Color::from_rgba8(0xD6, 0xDE, 0xE8, 255);
const FG_MUTED: Color = Color::from_rgba8(0x8C, 0x98, 0xAA, 255);
const ACCENT: Color = Color::from_rgba8(0x6E, 0x8C, 0xDC, 255);
const AMBER: Color = Color::from_rgba8(0xE8, 0xC9, 0x7A, 255);

const PARRAFO: &str = "Llimphi pinta vector preciso sobre el silicio: \
geometrías exactas, sin cajas negras. شكراً 你好 こんにちは — el shaping \
de parley maneja kerning, ligaduras y fallback CJK/Árabe en la misma \
línea, resuelto por fontique sobre las fuentes Noto de Android.";

const TECNICO: &str = "stack: wgpu(Vulkan) → llimphi-hal → vello compute → \
parley shaping → fontique fallback. APK firmado v2, ~7 MB stripped.";

struct State {
    window: Arc<Window>,
    hal: Hal,
    surface: WinitSurface,
    renderer: llimphi_raster::Renderer,
    scene: vello::Scene,
    typesetter: Typesetter,
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

    fn boot(&self, event_loop: &ActiveEventLoop) -> Result<State, String> {
        log::info!("[boot] 1/9 Window");
        let window = event_loop
            .create_window(WindowAttributes::default().with_title("llimphi · vello-text"))
            .map_err(|e| format!("create_window: {e}"))?;
        let window = Arc::new(window);
        let size = window.inner_size();
        log::info!("[boot] window {}x{}", size.width, size.height);

        log::info!("[boot] 2/9 Instance");
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        log::info!("[boot] 3/9 Surface");
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("create_surface: {e}"))?;

        log::info!("[boot] 4/9 Adapter compatible");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .ok_or_else(|| "request_adapter → None".to_string())?;
        let info = adapter.get_info();
        log::info!(
            "[boot] adapter {:?} · {} · {:?}",
            info.backend,
            info.name,
            info.driver_info
        );

        log::info!("[boot] 5/9 Device");
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("vello-text-device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e| format!("request_device: {e}"))?;

        log::info!("[boot] 6/9 Hal");
        let hal = Hal {
            instance,
            adapter,
            device,
            queue,
        };

        log::info!("[boot] 7/9 WinitSurface::from_surface");
        let surface = WinitSurface::from_surface(&hal, window.clone(), surface)
            .map_err(|e| format!("WinitSurface: {e}"))?;

        log::info!("[boot] 8/9 Renderer (vello)");
        let renderer =
            llimphi_raster::Renderer::new(&hal).map_err(|e| format!("Renderer: {e}"))?;

        log::info!("[boot] 9/9 Typesetter (parley + fontique scan /system/fonts)");
        let typesetter = Typesetter::new();
        log::info!("[boot] ✓ stack texto listo");

        window.request_redraw();
        Ok(State {
            window,
            hal,
            surface,
            renderer,
            scene: vello::Scene::new(),
            typesetter,
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
        log::info!("Suspended");
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
                state.surface.resize(size.width, size.height);
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let frame = match state.surface.acquire() {
                    Ok(f) => f,
                    Err(e) => {
                        log::warn!("acquire {e}");
                        let (w, h) = state.surface.size();
                        state.surface.resize(w, h);
                        state.window.request_redraw();
                        return;
                    }
                };
                let (w, h) = frame.size();
                state.scene.reset();
                paint_page(&mut state.scene, &mut state.typesetter, w, h);
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
                if self.last_report.elapsed().as_secs() >= 2 {
                    let fps = self.frames as f64 / self.last_report.elapsed().as_secs_f64();
                    log::info!("{fps:.1} fps · {w}x{h}");
                    self.frames = 0;
                    self.last_report = Instant::now();
                }
                // No request_redraw: el texto es estático, evita drenar batería.
            }
            _ => {}
        }
    }
}

/// Pinta la página completa de texto. Escala las fuentes proporcionales al
/// ancho del viewport: en mobile (1080+ px) el texto queda ~1.4× más
/// grande que en desktop (960 px) — lectura cómoda con device a 30 cm.
fn paint_page(scene: &mut vello::Scene, ts: &mut Typesetter, w: u32, h: u32) {
    // Escala lineal sobre el ancho del viewport. base = 1080 px → factor 1.0.
    let scale = (w as f32 / 1080.0).clamp(0.6, 2.4);
    let margin_x = (w as f64 * 0.06).max(24.0);
    let margin_y = (h as f64 * 0.08).max(32.0);
    let inner_w = (w as f32 - 2.0 * margin_x as f32).max(160.0);

    // Título grande
    draw_block(
        scene,
        ts,
        &TextBlock {
            text: "Llimphi",
            size_px: 96.0 * scale,
            color: FG_TEXT,
            origin: (margin_x, margin_y),
            max_width: Some(inner_w),
            alignment: Alignment::Center,
            line_height: 1.0,
        },
    );

    // Subtítulo en accent
    draw_block(
        scene,
        ts,
        &TextBlock {
            text: "texto multi-script sobre Android",
            size_px: 22.0 * scale,
            color: ACCENT,
            origin: (margin_x, margin_y + (110.0 * scale as f64)),
            max_width: Some(inner_w),
            alignment: Alignment::Center,
            line_height: 1.0,
        },
    );

    // Línea separadora dorada (un guion largo en amber)
    draw_block(
        scene,
        ts,
        &TextBlock {
            text: "—",
            size_px: 32.0 * scale,
            color: AMBER,
            origin: (margin_x, margin_y + (155.0 * scale as f64)),
            max_width: Some(inner_w),
            alignment: Alignment::Center,
            line_height: 1.0,
        },
    );

    // Párrafo justificado con scripts mixtos
    draw_block(
        scene,
        ts,
        &TextBlock {
            text: PARRAFO,
            size_px: 22.0 * scale,
            color: FG_TEXT,
            origin: (margin_x, margin_y + (220.0 * scale as f64)),
            max_width: Some(inner_w),
            alignment: Alignment::Justify,
            line_height: 1.5,
        },
    );

    // Pie técnico mute
    draw_block(
        scene,
        ts,
        &TextBlock {
            text: TECNICO,
            size_px: 16.0 * scale,
            color: FG_MUTED,
            origin: (margin_x, h as f64 - margin_y - (50.0 * scale as f64)),
            max_width: Some(inner_w),
            alignment: Alignment::Start,
            line_height: 1.3,
        },
    );
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
    // Wait (no Poll): el texto es estático, el redraw lo dispara
    // Resized/Resumed. Ahorra batería vs vello-hello que anima.
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut handler = App::new();
    if let Err(e) = event_loop.run_app(&mut handler) {
        log::error!("run_app: {e}");
    }
}
