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
use llimphi_hal::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use llimphi_hal::winit::keyboard::ModifiersState;
use llimphi_hal::winit::window::{Window, WindowAttributes, WindowId};
use llimphi_hal::{Hal, Surface, WinitSurface};

pub use llimphi_hal::winit::keyboard::{Key, NamedKey};
use llimphi_layout::taffy::NodeId;
use llimphi_layout::{ComputedLayout, LayoutTree, Style};
use llimphi_raster::kurbo::{Affine, RoundedRect};
use llimphi_raster::peniko::{color::palette, Color, Fill};
use llimphi_raster::{vello, Renderer};

pub use llimphi_hal;
pub use llimphi_layout;
pub use llimphi_raster;
pub use llimphi_text;

/// Aplicación Elm: estado inmutable, transición pura, vista pura.
///
/// `init` y `update` reciben un [`Handle`] que permite hablar con el runtime
/// desde dentro de la transición (cerrar la ventana, lanzar trabajo en otro
/// hilo y reentrar con un Msg al terminar). Mantener la transición pura del
/// modelo sigue siendo el contrato — `Handle` sólo escala efectos.
pub trait App: 'static {
    type Model: 'static;
    type Msg: Clone + Send + 'static;

    fn init(handle: &Handle<Self::Msg>) -> Self::Model;
    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model;
    fn view(model: &Self::Model) -> View<Self::Msg>;

    /// Maneja una pulsación de tecla. Devuelve `Some(Msg)` para disparar
    /// una transición; `None` (default) ignora la tecla.
    fn on_key(_model: &Self::Model, _event: &KeyEvent) -> Option<Self::Msg> {
        None
    }

    /// Título de la ventana (sólo se lee al arrancar).
    fn title() -> &'static str {
        "llimphi"
    }

    /// Identificador de aplicación. En Wayland se mapea al `app_id` del
    /// xdg-toplevel (lo que el compositor usa para reconocer la ventana,
    /// p. ej. `carmen.greeter`). `None` deja que el sistema asigne uno.
    fn app_id() -> Option<&'static str> {
        None
    }

    /// Tamaño lógico inicial de la ventana, en píxeles. El usuario puede
    /// redimensionar después; sólo se lee al arrancar.
    fn initial_size() -> (u32, u32) {
        (960, 540)
    }
}

/// Mensaje interno del event loop. `Msg` lo dispara la app desde un hilo de
/// fondo vía [`Handle::dispatch`] o [`Handle::spawn`]; `Quit` cierra la
/// ventana y termina el proceso.
pub enum UserEvent<Msg> {
    Msg(Msg),
    Quit,
}

/// Asa al runtime de Llimphi. Clonable y enviable entre hilos: la usás para
/// pedir cerrar la ventana o para lanzar trabajo (PAM, IO, etc.) que al
/// terminar reentra con un Msg al `update`.
pub struct Handle<Msg: Send + 'static> {
    proxy: EventLoopProxy<UserEvent<Msg>>,
}

impl<Msg: Send + 'static> Clone for Handle<Msg> {
    fn clone(&self) -> Self {
        Self {
            proxy: self.proxy.clone(),
        }
    }
}

impl<Msg: Send + 'static> Handle<Msg> {
    /// Cierra la ventana y termina el bucle. La transición en curso (si la
    /// hay) se completa antes de salir.
    pub fn quit(&self) {
        let _ = self.proxy.send_event(UserEvent::Quit);
    }

    /// Encola un Msg para procesarse en el próximo turno del bucle. Útil
    /// para que un callback externo reentre al update.
    pub fn dispatch(&self, msg: Msg) {
        let _ = self.proxy.send_event(UserEvent::Msg(msg));
    }

    /// Lanza una closure en un hilo aparte; cuando devuelve `Msg`, el
    /// runtime la entrega al `update` en el hilo de UI. Pensado para
    /// trabajo bloqueante (PAM tarda ~2 s ante un fallo, p. ej.).
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() -> Msg + Send + 'static,
    {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let msg = f();
            let _ = proxy.send_event(UserEvent::Msg(msg));
        });
    }
}

/// Evento de teclado normalizado.
#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub key: Key,
    pub state: KeyState,
    /// Texto resultante (con modifiers e IME aplicados). Útil para inserción
    /// directa; `None` para teclas que no producen texto (flechas, etc.).
    pub text: Option<String>,
    pub modifiers: Modifiers,
    pub repeat: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl From<ModifiersState> for Modifiers {
    fn from(m: ModifiersState) -> Self {
        Self {
            shift: m.shift_key(),
            ctrl: m.control_key(),
            alt: m.alt_key(),
            meta: m.super_key(),
        }
    }
}

/// Texto a pintar dentro de un nodo. Alineación por defecto `Center`
/// (horizontal y vertical), apta para labels de botón. Para layouts tipo
/// editor o párrafo, usar `.text_aligned(...)` con `Alignment::Start`.
pub struct TextSpec {
    pub content: String,
    pub size_px: f32,
    pub color: Color,
    pub alignment: llimphi_text::Alignment,
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
            alignment: llimphi_text::Alignment::Center,
        });
        self
    }

    pub fn text_aligned(
        mut self,
        content: impl Into<String>,
        size_px: f32,
        color: Color,
        alignment: llimphi_text::Alignment,
    ) -> Self {
        self.text = Some(TextSpec {
            content: content.into(),
            size_px,
            color,
            alignment,
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
    let (root, nodes) = mount_recursive(layout, v);
    Mounted { root, nodes }
}

/// Devuelve `(NodeId del subárbol, lista de MountedNode en pre-orden)`.
/// Pre-orden = padre antes que hijos, así `paint` recorre en orden e imita
/// painter's algorithm (padre = background, hijos encima).
fn mount_recursive<Msg: Clone>(
    layout: &mut LayoutTree,
    v: View<Msg>,
) -> (NodeId, Vec<MountedNode<Msg>>) {
    let View {
        style,
        fill,
        radius,
        text,
        on_click,
        children,
    } = v;
    let children_results: Vec<(NodeId, Vec<MountedNode<Msg>>)> = children
        .into_iter()
        .map(|c| mount_recursive(layout, c))
        .collect();
    let child_ids: Vec<NodeId> = children_results.iter().map(|(id, _)| *id).collect();
    let id = if child_ids.is_empty() {
        layout.leaf(style).expect("layout leaf")
    } else {
        layout.node(style, &child_ids).expect("layout node")
    };
    let mut nodes = Vec::with_capacity(1 + children_results.iter().map(|(_, n)| n.len()).sum::<usize>());
    nodes.push(MountedNode {
        id,
        fill,
        radius,
        text,
        on_click,
    });
    for (_, child_nodes) in children_results {
        nodes.extend(child_nodes);
    }
    (id, nodes)
}

fn paint<Msg>(
    scene: &mut vello::Scene,
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    typesetter: &mut llimphi_text::Typesetter,
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
        if let Some(text) = node.text.as_ref() {
            // Parley resuelve la alineación horizontal vía max_width + alignment.
            // Para Center también centramos verticalmente; para Start/End/Justify
            // anclamos arriba (comportamiento esperado de párrafo/editor).
            let block = llimphi_text::TextBlock {
                text: &text.content,
                size_px: text.size_px,
                color: text.color,
                origin: (r.x as f64, r.y as f64),
                max_width: Some(r.w),
                alignment: text.alignment,
                line_height: 1.2,
            };
            // Shaping una sola vez: el `Layout` retornado se reusa para
            // medir (cuando hay centrado vertical) y para pintar.
            let layout = llimphi_text::layout_block(typesetter, &block);
            let origin = if matches!(text.alignment, llimphi_text::Alignment::Center) {
                let m = llimphi_text::measurement(&layout);
                (
                    r.x as f64,
                    r.y as f64 + ((r.h - m.height) as f64 * 0.5).max(0.0),
                )
            } else {
                block.origin
            };
            llimphi_text::draw_layout(scene, &layout, text.color, origin);
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
    handle: Handle<A::Msg>,
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
    modifiers: Modifiers,
    typesetter: llimphi_text::Typesetter,
    /// Último frame renderizado: árbol montado + rects absolutos. Lo
    /// consume el handler de click para hit-testear sin reconstruir
    /// `view` + layout (que ya hizo el redraw anterior).
    last_render: Option<RenderCache<A::Msg>>,
}

struct RenderCache<Msg> {
    mounted: Mounted<Msg>,
    computed: ComputedLayout,
}

fn build_window_attributes<A: App>() -> WindowAttributes {
    let (w, h) = A::initial_size();
    let attrs = WindowAttributes::default()
        .with_title(A::title())
        .with_inner_size(LogicalSize::new(w, h));
    // En Linux, `with_name` del trait de Wayland mapea al `app_id` del
    // xdg-toplevel — lo que el compositor (`mirada-compositor`) usa para
    // reconocer ventanas especiales (greeter, launcher…).
    #[cfg(all(target_os = "linux", not(target_os = "android")))]
    {
        if let Some(id) = A::app_id() {
            use llimphi_hal::winit::platform::wayland::WindowAttributesExtWayland;
            return attrs.with_name(id, "");
        }
    }
    attrs
}

impl<A: App> ApplicationHandler<UserEvent<A::Msg>> for Runtime<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = event_loop
            .create_window(build_window_attributes::<A>())
            .expect("create window");
        let window = Arc::new(window);
        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        let surface = WinitSurface::new(&hal, window.clone()).expect("surface");
        let renderer = Renderer::new(&hal).expect("renderer");
        let typesetter = llimphi_text::Typesetter::new();
        window.request_redraw();
        self.state = Some(RuntimeState {
            window,
            hal,
            surface,
            renderer,
            scene: vello::Scene::new(),
            model: Some(A::init(&self.handle)),
            cursor: PhysicalPosition::new(0.0, 0.0),
            modifiers: Modifiers::default(),
            typesetter,
            last_render: None,
        });
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent<A::Msg>) {
        match event {
            UserEvent::Quit => event_loop.exit(),
            UserEvent::Msg(msg) => {
                let Some(state) = self.state.as_mut() else {
                    return;
                };
                let model = state.model.take().expect("model");
                state.model = Some(A::update(model, msg, &self.handle));
                state.last_render = None; // model cambió → cache obsoleto
                state.window.request_redraw();
            }
        }
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
            WindowEvent::ModifiersChanged(mods) => {
                state.modifiers = mods.state().into();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let ev = KeyEvent {
                    key: event.logical_key.clone(),
                    state: match event.state {
                        ElementState::Pressed => KeyState::Pressed,
                        ElementState::Released => KeyState::Released,
                    },
                    text: event.text.as_ref().map(|t| t.to_string()),
                    modifiers: state.modifiers,
                    repeat: event.repeat,
                };
                if let Some(msg) = A::on_key(state.model.as_ref().expect("model"), &ev) {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                // Camino normal: reusa el `mounted` + `computed` del último
                // redraw — siempre representa lo que el usuario está viendo.
                // Fallback (raro): no hubo redraw aún o el cache se invalidó
                // por un Msg sin redraw intermedio; rehacé view + layout.
                let msg = if let Some(cache) = state.last_render.as_ref() {
                    hit_test(
                        &cache.mounted,
                        &cache.computed,
                        state.cursor.x as f32,
                        state.cursor.y as f32,
                    )
                } else {
                    let view = A::view(state.model.as_ref().expect("model"));
                    let mut layout = LayoutTree::new();
                    let mounted: Mounted<A::Msg> = mount(&mut layout, view);
                    let (w, h) = state.surface.size();
                    let computed = layout
                        .compute(mounted.root, (w as f32, h as f32))
                        .expect("layout");
                    hit_test(
                        &mounted,
                        &computed,
                        state.cursor.x as f32,
                        state.cursor.y as f32,
                    )
                };
                if let Some(msg) = msg {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
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
                paint(&mut state.scene, &mounted, &computed, &mut state.typesetter);
                if let Err(e) = state.renderer.render(
                    &state.hal,
                    &state.scene,
                    &frame,
                    palette::css::BLACK,
                ) {
                    eprintln!("render error: {e}");
                }
                state.surface.present(frame, &state.hal);
                // Guardá el árbol pintado para que el próximo click haga
                // hit-test sin repetir `view` + layout.
                state.last_render = Some(RenderCache { mounted, computed });
            }
            _ => {}
        }
    }
}

/// Punto de entrada: corre el bucle Elm hasta que el usuario cierre la
/// ventana (o la app llame [`Handle::quit`]).
pub fn run<A: App>() {
    let event_loop = EventLoop::<UserEvent<A::Msg>>::with_user_event()
        .build()
        .expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let handle = Handle {
        proxy: event_loop.create_proxy(),
    };
    let mut runtime: Runtime<A> = Runtime {
        handle,
        state: None,
    };
    event_loop.run_app(&mut runtime).expect("run app");
}
