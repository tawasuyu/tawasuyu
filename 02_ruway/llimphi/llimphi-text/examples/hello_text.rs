//! Texto via parley sobre vello: párrafo wrappeable + shaping (kerning,
//! ligatures, bidi, fallback CJK/emoji).
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
use llimphi_text::{draw_block, Alignment, TextBlock, Typesetter};

const PARRAFO: &str = "Llimphi pinta vector preciso sobre el silicio: \
geometrías exactas, sin cajas negras. شكراً 你好 — el shaping de parley \
maneja kerning, ligaduras y fallback CJK/Arabic en la misma línea.";

struct State {
    window: Arc<Window>,
    hal: Hal,
    surface: WinitSurface,
    renderer: llimphi_raster::Renderer,
    scene: llimphi_raster::vello::Scene,
    typesetter: Typesetter,
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
        let typesetter = Typesetter::new();
        window.request_redraw();
        self.state = Some(State {
            window,
            hal,
            surface,
            renderer,
            scene: llimphi_raster::vello::Scene::new(),
            typesetter,
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
                let (w, _h) = frame.size();
                let margin_x = 64.0_f64;
                let margin_y = 64.0_f64;
                let inner_w = (w as f32 - 2.0 * margin_x as f32).max(100.0);
                state.scene.reset();

                // Título centrado
                draw_block(
                    &mut state.scene,
                    &mut state.typesetter,
                    &TextBlock {
                        text: "Llimphi",
                        size_px: 96.0,
                        color: Color::from_rgba8(220, 230, 240, 255),
                        origin: (margin_x, margin_y),
                        max_width: Some(inner_w),
                        alignment: Alignment::Center,
                        line_height: 1.0,
                    },
                );

                // Subtítulo centrado
                draw_block(
                    &mut state.scene,
                    &mut state.typesetter,
                    &TextBlock {
                        text: "motor gráfico soberano · parley + vello",
                        size_px: 20.0,
                        color: Color::from_rgba8(140, 160, 180, 255),
                        origin: (margin_x, margin_y + 110.0),
                        max_width: Some(inner_w),
                        alignment: Alignment::Center,
                        line_height: 1.0,
                    },
                );

                // Párrafo justificado con wrap
                draw_block(
                    &mut state.scene,
                    &mut state.typesetter,
                    &TextBlock {
                        text: PARRAFO,
                        size_px: 22.0,
                        color: Color::from_rgba8(200, 210, 220, 255),
                        origin: (margin_x, margin_y + 170.0),
                        max_width: Some(inner_w),
                        alignment: Alignment::Justify,
                        line_height: 1.4,
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
