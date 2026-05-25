//! llimphi-ui — Árbol de Estado Monádico (DAG UI).
//!
//! Bucle Elm sobre llimphi-hal + llimphi-layout + llimphi-raster:
//!
//! ```text
//!   input → update(model, msg) → view(model) → layout → raster → present
//! ```
//!
//! El estado del [`App`] es inmutable: cada evento produce un `Model`
//! nuevo. La vista (`view`) es una función pura `&Model -> View<Msg>`.

use std::sync::Arc;

use llimphi_hal::winit::application::ApplicationHandler;
use llimphi_hal::winit::dpi::{LogicalSize, PhysicalPosition};
use llimphi_hal::winit::event::{ElementState, MouseButton, WindowEvent};
use llimphi_hal::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use llimphi_hal::winit::window::{Window, WindowAttributes, WindowId};
use llimphi_hal::{Hal, Surface, WinitSurface};
use llimphi_layout::taffy::NodeId;
use llimphi_layout::{ComputedLayout, LayoutTree, Style};
use llimphi_raster::kurbo::{Affine, RoundedRect};
use llimphi_raster::peniko::{color::palette, Color, Fill};
use llimphi_raster::{vello, Renderer};

pub use llimphi_hal;
pub use llimphi_layout;
pub use llimphi_raster;
pub use llimphi_text;

/// Cadena de fallback de fuentes para Linux que intenta el runtime al
/// arrancar. La primera disponible gana; si ninguna existe, el texto no
/// se pinta y se emite un warning por stderr.
pub const DEFAULT_FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
    "/usr/share/fonts/inter/Inter-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/droid/DroidSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
];

/// Aplicación Elm: estado inmutable, transición pura, vista pura.
pub trait App: 'static {
    type Model: 'static;
    type Msg: Clone + 'static;

    fn init() -> Self::Model;
    fn update(model: Self::Model, msg: Self::Msg) -> Self::Model;
    fn view(model: &Self::Model) -> View<Self::Msg>;

    /// Título de la ventana (sólo se lee al arrancar).
    fn title() -> &'static str {
        "llimphi"
    }
}

/// Texto a pintar centrado dentro de un nodo.
pub struct TextSpec {
    pub content: String,
    pub size_px: f32,
    pub color: Color,
}

/// Nodo de la vista declarativa. Estilo de layout (taffy) + relleno opcional
/// (vello) + texto opcional (skrifa+vello) + Msg al click opcional + hijos.
pub struct View<Msg> {
    pub style: Style,
    pub fill: Option<Color>,
    pub radius: f64,
    pub text: Option<TextSpec>,
    pub on_click: Option<Msg>,
    pub children: Vec<View<Msg>>,
}

impl<Msg> View<Msg> {
    pub fn new(style: Style) -> Self {
        Self {
            style,
            fill: None,
            radius: 0.0,
            text: None,
            on_click: None,
            children: Vec::new(),
        }
    }

    pub fn fill(mut self, color: Color) -> Self {
        self.fill = Some(color);
        self
    }

    pub fn radius(mut self, r: f64) -> Self {
        self.radius = r;
        self
    }

    pub fn text(mut self, content: impl Into<String>, size_px: f32, color: Color) -> Self {
        self.text = Some(TextSpec {
            content: content.into(),
            size_px,
            color,
        });
        self
    }

    pub fn on_click(mut self, msg: Msg) -> Self {
        self.on_click = Some(msg);
        self
    }

    pub fn children(mut self, children: Vec<View<Msg>>) -> Self {
        self.children = children;
        self
    }
}

/// Versión "instalada" del árbol: cada nodo tiene su NodeId de taffy, color
/// y handler. Se mantiene en orden de inserción (recorrido pre-orden), así
/// el hit-test puede iterar al revés para honrar el orden de pintado.
struct Mounted<Msg> {
    root: NodeId,
    nodes: Vec<MountedNode<Msg>>,
}

struct MountedNode<Msg> {
    id: NodeId,
    fill: Option<Color>,
    radius: f64,
    text: Option<TextSpec>,
    on_click: Option<Msg>,
}

fn mount<Msg: Clone>(layout: &mut LayoutTree, v: View<Msg>) -> Mounted<Msg> {
    let mut nodes = Vec::new();
    let root = mount_recursive(layout, v, &mut nodes);
    Mounted { root, nodes }
}

fn mount_recursive<Msg: Clone>(
    layout: &mut LayoutTree,
    v: View<Msg>,
    out: &mut Vec<MountedNode<Msg>>,
) -> NodeId {
    let View {
        style,
        fill,
        radius,
        text,
        on_click,
        children,
    } = v;
    let child_ids: Vec<NodeId> = children
        .into_iter()
        .map(|c| mount_recursive(layout, c, out))
        .collect();
    let id = if child_ids.is_empty() {
        layout.leaf(style).expect("layout leaf")
    } else {
        layout.node(style, &child_ids).expect("layout node")
    };
    out.push(MountedNode {
        id,
        fill,
        radius,
        text,
        on_click,
    });
    id
}

fn paint<Msg>(
    scene: &mut vello::Scene,
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    face: Option<&llimphi_text::Typeface>,
) {
    for node in &mounted.nodes {
        let Some(r) = computed.get(node.id) else {
            continue;
        };
        if let Some(color) = node.fill {
            let rr = RoundedRect::new(
                r.x as f64,
                r.y as f64,
                (r.x + r.w) as f64,
                (r.y + r.h) as f64,
                node.radius,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rr);
        }
        if let (Some(text), Some(face)) = (node.text.as_ref(), face) {
            let m = face.measure(&text.content, text.size_px);
            // Centrado dentro del rect. El origin de draw_block es el baseline.
            let x = r.x as f64 + (r.w as f64 - m.width as f64) * 0.5;
            let y = r.y as f64 + (r.h as f64 + m.ascent as f64 - m.descent.abs() as f64) * 0.5;
            llimphi_text::draw_block(
                scene,
                face,
                &llimphi_text::TextBlock {
                    text: &text.content,
                    size_px: text.size_px,
                    color: text.color,
                    origin: (x, y),
                },
            );
        }
    }
}

/// Hit-test: devuelve el Msg del nodo más al frente cuyo rect contiene (x, y).
fn hit_test<Msg: Clone>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<Msg> {
    for node in mounted.nodes.iter().rev() {
        let Some(msg) = node.on_click.as_ref() else {
            continue;
        };
        let Some(r) = computed.get(node.id) else {
            continue;
        };
        if x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h {
            return Some(msg.clone());
        }
    }
    None
}

struct Runtime<A: App> {
    state: Option<RuntimeState<A>>,
}

struct RuntimeState<A: App> {
    window: Arc<Window>,
    hal: Hal,
    surface: WinitSurface,
    renderer: Renderer,
    scene: vello::Scene,
    model: Option<A::Model>,
    cursor: PhysicalPosition<f64>,
    face: Option<llimphi_text::Typeface>,
}

impl<A: App> ApplicationHandler for Runtime<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title(A::title())
                    .with_inner_size(LogicalSize::new(960u32, 540u32)),
            )
            .expect("create window");
        let window = Arc::new(window);
        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        let surface = WinitSurface::new(&hal, window.clone()).expect("surface");
        let renderer = Renderer::new(&hal).expect("renderer");
        let face = match llimphi_text::Typeface::first_available(DEFAULT_FONT_CANDIDATES) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("llimphi-ui: sin fuente disponible ({e}); el texto no se pintará");
                None
            }
        };
        window.request_redraw();
        self.state = Some(RuntimeState {
            window,
            hal,
            surface,
            renderer,
            scene: vello::Scene::new(),
            model: Some(A::init()),
            cursor: PhysicalPosition::new(0.0, 0.0),
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
            WindowEvent::CursorMoved { position, .. } => {
                state.cursor = position;
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                // Re-build view del modelo actual para hit-test contra el frame visible.
                let model_ref = state.model.as_ref().expect("model");
                let view = A::view(model_ref);
                let mut layout = LayoutTree::new();
                let mounted: Mounted<A::Msg> = mount(&mut layout, view);
                let (w, h) = state.surface.size();
                let computed = layout
                    .compute(mounted.root, (w as f32, h as f32))
                    .expect("layout");
                if let Some(msg) =
                    hit_test(&mounted, &computed, state.cursor.x as f32, state.cursor.y as f32)
                {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg));
                    state.window.request_redraw();
                }
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
                let view = A::view(state.model.as_ref().expect("model"));
                let mut layout = LayoutTree::new();
                let mounted: Mounted<A::Msg> = mount(&mut layout, view);
                let computed = layout
                    .compute(mounted.root, (w as f32, h as f32))
                    .expect("layout");
                state.scene.reset();
                paint(&mut state.scene, &mounted, &computed, state.face.as_ref());
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

/// Punto de entrada: corre el bucle Elm hasta que el usuario cierre la ventana.
pub fn run<A: App>() {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut runtime: Runtime<A> = Runtime { state: None };
    event_loop.run_app(&mut runtime).expect("run app");
}
