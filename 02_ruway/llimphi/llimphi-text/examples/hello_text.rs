//! Carga una fuente del sistema y pinta "Llimphi" centrado-ish.
//!
//! Corre con: `cargo run -p llimphi-text --example hello_text --release`.

use std::sync::Arc;

use llimphi_hal::winit::application::ApplicationHandler;
use llimphi_hal::winit::dpi::LogicalSize;
use llimphi_hal::winit::event::WindowEvent;
use llimphi_hal::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use llimphi_hal::winit::window::{Window, WindowAttributes, WindowId};
use llimphi_hal::{Hal, Surface, WinitSurface};
use llimphi_text::peniko::{color::palette, Color};
use llimphi_text::{draw_block, TextBlock, Typeface};

// Cadena de fallback típica en Linux. La primera que exista gana.
const FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
    "/usr/share/fonts/inter/Inter-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/droid/DroidSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
];

struct State {
    window: Arc<Window>,
    hal: Hal,
    surface: WinitSurface,
    renderer: llimphi_raster::Renderer,
    scene: llimphi_raster::vello::Scene,
    face: Typeface,
}

struct App {
    state: Option<State>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("llimphi · hello_text")
                    .with_inner_size(LogicalSize::new(960u32, 540u32)),
            )
            .expect("create window");
        let window = Arc::new(window);
        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        let surface = WinitSurface::new(&hal, window.clone()).expect("surface");
        let renderer = llimphi_raster::Renderer::new(&hal).expect("renderer");
        let face = Typeface::first_available(FONT_CANDIDATES).expect("no candidate font available");
        window.request_redraw();
        self.state = Some(State {
            window,
            hal,
            surface,
            renderer,
            scene: llimphi_raster::vello::Scene::new(),
            face,
        });
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
                let (w, h) = frame.size();
                state.scene.reset();
                draw_block(
                    &mut state.scene,
                    &state.face,
                    &TextBlock {
                        text: "Llimphi",
                        size_px: 96.0,
                        color: Color::from_rgba8(220, 230, 240, 255),
                        origin: (w as f64 * 0.25, h as f64 * 0.5),
                    },
                );
                draw_block(
                    &mut state.scene,
                    &state.face,
                    &TextBlock {
                        text: "motor grafico soberano",
                        size_px: 22.0,
                        color: Color::from_rgba8(140, 160, 180, 255),
                        origin: (w as f64 * 0.25, h as f64 * 0.5 + 30.0),
                    },
                );
                if let Err(e) = state.renderer.render(
                    &state.hal,
                    &state.scene,
                    &frame,
                    palette::css::BLACK,
                ) {
                    eprintln!("render error: {e}");
                }
                state.surface.present(frame, &state.hal);
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App { state: None };
    event_loop.run_app(&mut app).expect("run app");
}
