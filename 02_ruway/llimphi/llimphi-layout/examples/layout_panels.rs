//! Fase 3 de Llimphi: 3 paneles (sidebar + header/body/footer) que se
//! reorganizan al redimensionar la ventana. Pintados por vello a través
//! de llimphi-raster.
//!
//! Corre con: `cargo run -p llimphi-layout --example layout_panels --release`.

use std::sync::Arc;

use llimphi_hal::winit::application::ApplicationHandler;
use llimphi_hal::winit::dpi::LogicalSize;
use llimphi_hal::winit::event::WindowEvent;
use llimphi_hal::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use llimphi_hal::winit::window::{Window, WindowAttributes, WindowId};
use llimphi_hal::{Hal, Surface, WinitSurface};
use llimphi_layout::{
    taffy::{prelude::*, Style},
    ComputedLayout, LayoutTree, Rect,
};
use llimphi_raster::kurbo::{Affine, RoundedRect};
use llimphi_raster::peniko::{color::palette, Color, Fill};
use llimphi_raster::{vello, Renderer};

struct Panels {
    sidebar: NodeId,
    header: NodeId,
    body: NodeId,
    footer: NodeId,
    root: NodeId,
}

struct State {
    window: Arc<Window>,
    hal: Hal,
    surface: WinitSurface,
    renderer: Renderer,
    scene: vello::Scene,
    layout: LayoutTree,
    panels: Panels,
}

struct App {
    state: Option<State>,
}

fn build_tree(layout: &mut LayoutTree) -> Panels {
    let sidebar = layout
        .leaf(Style {
            size: Size {
                width: length(220.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .unwrap();

    let header = layout
        .leaf(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(64.0_f32),
            },
            ..Default::default()
        })
        .unwrap();

    let body = layout
        .leaf(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .unwrap();

    let footer = layout
        .leaf(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(40.0_f32),
            },
            ..Default::default()
        })
        .unwrap();

    let content = layout
        .node(
            Style {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                size: Size {
                    width: Dimension::auto(),
                    height: percent(1.0_f32),
                },
                gap: Size {
                    width: length(0.0_f32),
                    height: length(8.0_f32),
                },
                padding: Rect_(length(8.0_f32)),
                ..Default::default()
            },
            &[header, body, footer],
        )
        .unwrap();

    let root = layout
        .node(
            Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                ..Default::default()
            },
            &[sidebar, content],
        )
        .unwrap();

    Panels {
        sidebar,
        header,
        body,
        footer,
        root,
    }
}

/// Helper para pasar el mismo length a todos los lados de un Rect.
#[allow(non_snake_case)]
fn Rect_(v: LengthPercentage) -> taffy::Rect<LengthPercentage> {
    taffy::Rect {
        left: v,
        right: v,
        top: v,
        bottom: v,
    }
}

fn paint(scene: &mut vello::Scene, computed: &ComputedLayout, panels: &Panels) {
    fn rect(scene: &mut vello::Scene, r: Rect, color: Color, radius: f64) {
        let rr = RoundedRect::new(
            r.x as f64,
            r.y as f64,
            (r.x + r.w) as f64,
            (r.y + r.h) as f64,
            radius,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rr);
    }

    if let Some(r) = computed.get(panels.sidebar) {
        rect(scene, r, Color::from_rgba8(36, 44, 60, 255), 0.0);
    }
    if let Some(r) = computed.get(panels.header) {
        rect(scene, r, Color::from_rgba8(60, 80, 110, 255), 8.0);
    }
    if let Some(r) = computed.get(panels.body) {
        rect(scene, r, Color::from_rgba8(80, 110, 150, 255), 8.0);
    }
    if let Some(r) = computed.get(panels.footer) {
        rect(scene, r, Color::from_rgba8(60, 80, 110, 255), 8.0);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("llimphi · layout_panels")
                    .with_inner_size(LogicalSize::new(960u32, 540u32)),
            )
            .expect("create window");
        let window = Arc::new(window);
        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        let surface = WinitSurface::new(&hal, window.clone()).expect("surface");
        let renderer = Renderer::new(&hal).expect("renderer");
        let mut layout = LayoutTree::new();
        let panels = build_tree(&mut layout);
        window.request_redraw();
        self.state = Some(State {
            window,
            hal,
            surface,
            renderer,
            scene: vello::Scene::new(),
            layout,
            panels,
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
                let computed = state
                    .layout
                    .compute(state.panels.root, (w as f32, h as f32))
                    .expect("compute layout");
                state.scene.reset();
                paint(&mut state.scene, &computed, &state.panels);
                if let Err(e) = state.renderer.render(
                    &state.hal,
                    &state.scene,
                    &frame,
                    palette::css::BLACK,
                ) {
                    eprintln!("render error: {e}");
                }
                state.surface.present(frame, &state.hal);
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App { state: None };
    event_loop.run_app(&mut app).expect("run app");
}
