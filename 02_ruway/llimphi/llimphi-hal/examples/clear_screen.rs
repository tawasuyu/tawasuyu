//! Fase 1 de Llimphi: ventana gris plomo a la frecuencia máxima del display.
//!
//! Corre con: `cargo run -p llimphi-hal --example clear_screen --release`.
//!
//! Imprime fps por stderr cada segundo. En un panel de 144 Hz con AutoVsync
//! debe estabilizarse cerca de 144; en uno de 60 Hz, cerca de 60.

use std::sync::Arc;
use std::time::Instant;

use llimphi_hal::winit::application::ApplicationHandler;
use llimphi_hal::winit::dpi::LogicalSize;
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

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("llimphi · clear_screen")
                    .with_inner_size(LogicalSize::new(960u32, 540u32)),
            )
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
                frame.present();

                self.frames += 1;
                let elapsed = self.last_report.elapsed();
                if elapsed.as_secs() >= 1 {
                    let fps = self.frames as f64 / elapsed.as_secs_f64();
                    eprintln!("llimphi · clear_screen — {fps:.1} fps");
                    self.frames = 0;
                    self.last_report = Instant::now();
                }
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        state: None,
        frames: 0,
        last_report: Instant::now(),
    };
    event_loop.run_app(&mut app).expect("run app");
}
