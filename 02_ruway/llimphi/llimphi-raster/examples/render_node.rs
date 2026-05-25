//! Fase 2 de Llimphi: un nodo (círculo + halo) renderizado por vello con AA
//! perfecto sobre el swapchain de llimphi-hal.
//!
//! Corre con: `cargo run -p llimphi-raster --example render_node --release`.

use std::sync::Arc;
use std::time::Instant;

use llimphi_hal::winit::application::ApplicationHandler;
use llimphi_hal::winit::dpi::LogicalSize;
use llimphi_hal::winit::event::WindowEvent;
use llimphi_hal::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use llimphi_hal::winit::window::{Window, WindowAttributes, WindowId};
use llimphi_hal::{Hal, Surface, WinitSurface};
use llimphi_raster::kurbo::{Affine, Circle, Stroke};
use llimphi_raster::peniko::{color::palette, Color, Fill};
use llimphi_raster::{vello, Renderer};

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
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("llimphi · render_node")
                    .with_inner_size(LogicalSize::new(960u32, 540u32)),
            )
            .expect("create window");
        let window = Arc::new(window);
        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        let surface = WinitSurface::new(&hal, window.clone()).expect("surface");
        let renderer = Renderer::new(&hal).expect("renderer");
        window.request_redraw();
        self.state = Some(State {
            window,
            hal,
            surface,
            renderer,
            scene: vello::Scene::new(),
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
                build_node(&mut state.scene, w as f64, h as f64, self.started.elapsed().as_secs_f64());
                if let Err(e) = state.renderer.render(
                    &state.hal,
                    &state.scene,
                    &frame,
                    palette::css::BLACK,
                ) {
                    eprintln!("render error: {e}");
                }
                frame.present();
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

/// Pinta un nodo centrado (círculo lleno + halo) que respira con `t`.
fn build_node(scene: &mut vello::Scene, w: f64, h: f64, t: f64) {
    let cx = w * 0.5;
    let cy = h * 0.5;
    let pulse = 1.0 + 0.06 * (t * 1.6).sin();
    let r = (h.min(w) * 0.18) * pulse;

    // Halo
    scene.stroke(
        &Stroke::new(2.0),
        Affine::IDENTITY,
        Color::from_rgba8(60, 120, 200, 180),
        None,
        &Circle::new((cx, cy), r * 1.35),
    );
    // Cuerpo
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgba8(90, 160, 230, 255),
        None,
        &Circle::new((cx, cy), r),
    );
    // Borde
    scene.stroke(
        &Stroke::new(3.0),
        Affine::IDENTITY,
        Color::from_rgba8(20, 50, 100, 255),
        None,
        &Circle::new((cx, cy), r),
    );
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        state: None,
        started: Instant::now(),
    };
    event_loop.run_app(&mut app).expect("run app");
}
