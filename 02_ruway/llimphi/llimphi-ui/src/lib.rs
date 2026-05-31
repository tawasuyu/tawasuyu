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
use llimphi_hal::winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use llimphi_hal::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use llimphi_hal::winit::keyboard::ModifiersState;
use llimphi_hal::winit::window::{Window, WindowAttributes, WindowId};
use llimphi_hal::{Hal, Surface, WinitSurface};

pub use llimphi_hal::winit::keyboard::{Key, NamedKey};
use llimphi_layout::taffy::NodeId;
use llimphi_layout::{ComputedLayout, LayoutTree, Style};
use llimphi_raster::kurbo::{Affine, Rect as KurboRect, RoundedRect};
use llimphi_raster::peniko::{color::palette, Color, Fill, Image, Mix};
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

    /// Maneja una rueda del mouse. `delta` está normalizado a "líneas"
    /// (positivo arriba/izquierda, negativo abajo/derecha). En backends
    /// que reportan píxeles, llimphi-ui divide por 20 para aproximar.
    fn on_wheel(
        _model: &Self::Model,
        _delta: WheelDelta,
        _cursor: (f32, f32),
        _modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        None
    }

    /// Capa de overlay opcional. Si devuelve `Some(view)`, el runtime
    /// la pinta encima del árbol principal y los clicks/hover se
    /// rutean exclusivamente a ella (el árbol de fondo queda "bajo
    /// vidrio" hasta que se cierre el overlay). Pensado para menús
    /// contextuales, diálogos modales, popovers — el patrón usual es
    /// envolver los items en un scrim a pantalla completa con
    /// `on_click = DismissOverlay` para que los clicks afuera lo
    /// cierren.
    ///
    /// La transición entre "con overlay" y "sin overlay" la maneja la
    /// app vía su Model: cuando el state diga "menu abierto",
    /// `view_overlay` devuelve `Some`; cuando se cierre, `None`.
    fn view_overlay(_model: &Self::Model) -> Option<View<Self::Msg>> {
        None
    }

    /// Maneja un drop de archivo desde el sistema operativo (drag&drop
    /// desde el file manager hacia la ventana). El runtime invoca este
    /// callback una vez por archivo soltado — si el usuario suelta varios,
    /// llega un evento por path. Devolver `Some(Msg)` dispara un update;
    /// `None` (default) ignora el drop.
    ///
    /// Backend: mapea directamente `winit::WindowEvent::DroppedFile(PathBuf)`.
    /// La posición del drop no se reporta porque winit no la expone hasta
    /// que el compositor la propague — en Wayland depende del extension
    /// `data_device_manager`, en X11 viene en el ClientMessage XDND.
    fn on_file_drop(_model: &Self::Model, _path: std::path::PathBuf) -> Option<Self::Msg> {
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
///
/// Tests pueden construir un handle "muerto" con [`Handle::for_test`]: los
/// `dispatch`/`quit`/`spawn` siguen siendo seguros de llamar pero los
/// `Msg` que generan no van a ningún lado (no hay event loop detrás).
pub struct Handle<Msg: Send + 'static> {
    inner: HandleInner<Msg>,
}

enum HandleInner<Msg: Send + 'static> {
    Real(EventLoopProxy<UserEvent<Msg>>),
    /// Handle de tests: drop silencioso de todos los dispatches. Permite
    /// llamar funciones que toman `&Handle<Msg>` sin levantar un event
    /// loop real (que en CI sin display tiraría).
    Test,
}

impl<Msg: Send + 'static> Clone for Handle<Msg> {
    fn clone(&self) -> Self {
        Self {
            inner: match &self.inner {
                HandleInner::Real(p) => HandleInner::Real(p.clone()),
                HandleInner::Test => HandleInner::Test,
            },
        }
    }
}

impl<Msg: Send + 'static> Handle<Msg> {
    /// Construye un handle desactivado para tests — todos los dispatch
    /// se descartan silenciosamente. Útil para probar funciones que toman
    /// `&Handle<Msg>` sin levantar un event loop real (que en CI sin
    /// display tiraría).
    pub fn for_test() -> Self {
        Self {
            inner: HandleInner::Test,
        }
    }

    /// Cierra la ventana y termina el bucle. La transición en curso (si la
    /// hay) se completa antes de salir.
    pub fn quit(&self) {
        match &self.inner {
            HandleInner::Real(p) => {
                let _ = p.send_event(UserEvent::Quit);
            }
            HandleInner::Test => {}
        }
    }

    /// Encola un Msg para procesarse en el próximo turno del bucle. Útil
    /// para que un callback externo reentre al update.
    pub fn dispatch(&self, msg: Msg) {
        match &self.inner {
            HandleInner::Real(p) => {
                let _ = p.send_event(UserEvent::Msg(msg));
            }
            HandleInner::Test => {}
        }
    }

    /// Lanza una closure en un hilo aparte; cuando devuelve `Msg`, el
    /// runtime la entrega al `update` en el hilo de UI. Pensado para
    /// trabajo bloqueante (PAM tarda ~2 s ante un fallo, p. ej.).
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() -> Msg + Send + 'static,
    {
        match &self.inner {
            HandleInner::Real(p) => {
                let proxy = p.clone();
                std::thread::spawn(move || {
                    let msg = f();
                    let _ = proxy.send_event(UserEvent::Msg(msg));
                });
            }
            HandleInner::Test => {
                // Corremos la closure igual (para no perder side-effects de
                // tests que dependan de su side) pero el msg se descarta.
                std::thread::spawn(move || {
                    let _ = f();
                });
            }
        }
    }

    /// Lanza un loop periódico en un hilo aparte: cada `period` invoca
    /// `f()` y dispatcha el `Msg` resultante al `update`. El thread
    /// queda corriendo hasta que el event loop se cierra (en ese
    /// punto el `send_event` falla silenciosamente y el thread spinea
    /// hasta el exit del proceso, costo despreciable).
    ///
    /// Útil para ticks de simulación (~11 Hz en dominium), polling de
    /// hardware, o cualquier feed que necesite Msgs a intervalos
    /// regulares. Si `f` necesita state, capturalo en la closure por
    /// move; la closure se ejecuta en un thread aparte así que el
    /// state capturado debe ser `Send`.
    pub fn spawn_periodic<F>(&self, period: std::time::Duration, f: F)
    where
        F: Fn() -> Msg + Send + 'static,
    {
        match &self.inner {
            HandleInner::Real(p) => {
                let proxy = p.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(period);
                    if proxy.send_event(UserEvent::Msg(f())).is_err() {
                        // Event loop cerrado — el thread puede morir.
                        break;
                    }
                });
            }
            HandleInner::Test => {
                // Un thread vivo eternamente sin sumidero ni manera de
                // pararlo sería un leak — en for_test simplemente no
                // arrancamos el loop. Los tests que necesiten verificar
                // periodic behaviour deben usar el callback directo.
                let _ = f;
            }
        }
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

/// Delta de rueda en "líneas" lógicas (normalizado a través de backends).
/// Convención CSS: positivo = scroll **hacia abajo** (contenido sube).
/// `x` similar para scroll horizontal (touchpads, ratones de 2 ejes).
#[derive(Debug, Clone, Copy, Default)]
pub struct WheelDelta {
    pub x: f32,
    pub y: f32,
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
    /// `true` = forzar variante italic en la fuente activa. Default false.
    pub italic: bool,
    /// CSS-style font-family string (acepta lista con fallbacks). `None`
    /// = la fuente default de parley.
    pub font_family: Option<String>,
}

/// Fase de un drag activo. `Move` se emite por cada `CursorMoved` con el
/// delta desde el evento anterior; `End` se emite al soltar el botón.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragPhase {
    Move,
    End,
}

/// Handler de drag. Recibe la fase + delta (`dx`, `dy`) **desde el evento
/// anterior** (no acumulado desde el press). Devolver `None` deja el drag
/// activo sin disparar Msg. `Arc<dyn Fn>` para que el runtime pueda
/// clonarlo barato al iniciar el drag y mantenerlo vivo aunque el cache
/// de la vista se regenere mientras tanto.
pub type DragFn<Msg> = Arc<dyn Fn(DragPhase, f32, f32) -> Option<Msg> + Send + Sync>;

/// Handler de drop. El runtime lo invoca cuando un drag activo se suelta
/// sobre este nodo. Recibe el `payload` `u64` que el origen del drag
/// declaró vía [`View::drag_payload`]. Devolver `None` ignora el drop.
///
/// Los IDs `u64` son opacos para el runtime: el widget elige una
/// convención (índice de tile, hash del item, etc.) y el handler decide
/// qué Msg emitir en función de ese ID.
pub type DropFn<Msg> = Arc<dyn Fn(u64) -> Option<Msg> + Send + Sync>;

/// Handler de click con posición. Recibe `(x_local, y_local, rect_w,
/// rect_h)`: las dos primeras son la posición del cursor **relativa a
/// la esquina superior-izquierda del nodo** y las dos últimas son el
/// ancho/alto actual del nodo en pixels — útil cuando el caller
/// necesita centrar o normalizar. Devolver `None` no dispara update.
pub type ClickAtFn<Msg> = Arc<dyn Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync>;

/// Variante de [`DragFn`] que **conoce la posición inicial del press**
/// relativa al rect del nodo. Útil cuando el caller necesita identificar
/// qué entidad (Concepto, lemming, etc.) bajo el cursor agarró el drag.
/// Recibe `(phase, dx, dy, initial_lx, initial_ly)`.
pub type DragAtFn<Msg> = Arc<dyn Fn(DragPhase, f32, f32, f32, f32) -> Option<Msg> + Send + Sync>;

/// Rect absoluto del nodo (en coordenadas físicas del frame). Lo
/// recibe el callback de [`View::paint_with`] para que pueda
/// posicionar sus primitivas custom dentro del nodo.
#[derive(Debug, Clone, Copy, Default)]
pub struct PaintRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Callback de pintura custom. El runtime lo invoca durante el paint
/// del nodo (entre el `fill`/`image` y el `text`) con el `Scene` vivo
/// + el `Typesetter` cacheado del runtime + el rect absoluto del nodo.
/// Pensado para "canvas elements" tipo `dominium-canvas`,
/// `pluma-editor` (osciloscopio de coherencia), `cosmos` (charts).
///
/// El `Typesetter` se pasa porque crearlo por frame es caro
/// (`FontContext::new` enumera las fontes del sistema vía fontique).
/// Los callers que no necesiten texto pueden ignorar el argumento.
///
/// El callback no debe llamar a `scene.push_layer` sin un `pop_layer`
/// correspondiente, ni reset el scene — sólo agregar primitivas que
/// pertenezcan al rect del nodo.
pub type PaintFn = Arc<
    dyn Fn(&mut vello::Scene, &mut llimphi_text::Typesetter, PaintRect) + Send + Sync,
>;

/// Callback de pintura GPU directo, sin vello intermedio. Recibe el
/// `device`/`queue` ya construidos por el runtime más un
/// `CommandEncoder` y la `TextureView` del frame (la intermediate
/// `Rgba8Unorm` de `WinitSurface`), todo durante el paint del nodo.
///
/// El caller abre su propio `begin_render_pass` con `LoadOp::Load` para
/// no sobrescribir lo que ya pintó vello, dibuja sus primitivas y
/// cierra el pass. El runtime se encarga de dispatchear (`queue.submit`)
/// el encoder ya con todas las pasadas de todos los nodos acumuladas —
/// es un solo submit por frame.
///
/// **Orden de pintura en Fase 1**: todos los `gpu_painter` corren
/// DESPUÉS de la pasada completa de vello (fill, image, painter,
/// text) sobre el `mounted` tree. Entre sí mantienen el orden DFS
/// pre-orden. Si una app necesita pintar texto **encima** del render
/// GPU directo, la forma idiomática es ponerlo en `App::view_overlay`,
/// que se renderiza como una segunda Scene de vello encima de todo.
///
/// Pensado para apps con volumen masivo de primitivos (cosmos
/// starfield Gaia, tinkuy particle viewer, nakui viewport, pineal
/// denso) — el hook que paga el costo de mantener pipelines WGSL
/// propias en `llimphi-raster` (ver `02_ruway/llimphi/SDD.md`
/// §"Roadmap — GPU directo wgpu").
pub type GpuPaintFn = Arc<
    dyn Fn(
            &llimphi_hal::wgpu::Device,
            &llimphi_hal::wgpu::Queue,
            &mut llimphi_hal::wgpu::CommandEncoder,
            &llimphi_hal::wgpu::TextureView,
            PaintRect,
            (u32, u32),
        ) + Send
        + Sync,
>;

/// Nodo de la vista declarativa. Estilo de layout (taffy) + relleno opcional
/// (vello) + texto opcional (skrifa+vello) + Msg al click opcional + hijos.
pub struct View<Msg> {
    pub style: Style,
    pub fill: Option<Color>,
    /// Relleno cuando el cursor está sobre este nodo. Sin valor (`None`)
    /// = no se reacciona al hover.
    pub hover_fill: Option<Color>,
    pub radius: f64,
    pub text: Option<TextSpec>,
    /// Imagen a pintar dentro del rect del nodo. Se centra y escala
    /// preservando aspect ratio (`min(rect.w/img.w, rect.h/img.h)`).
    /// El alfa por píxel de la imagen y el `Image::alpha` global se
    /// respetan; el `fill` (si lo hay) se pinta debajo como background.
    pub image: Option<Image>,
    /// Callback de pintura custom. Si está presente, el runtime lo
    /// invoca durante el paint del nodo con el `Scene` vivo + el rect
    /// absoluto. Pensado para "canvas elements" (dominium, pluma,
    /// cosmos) que pintan primitivas custom no expresables como una
    /// composición de Views.
    pub painter: Option<PaintFn>,
    /// Pintor GPU directo. Se invoca DESPUÉS de la pasada vello del
    /// frame; comparte tree y orden DFS con los demás. Ver
    /// [`GpuPaintFn`].
    pub gpu_painter: Option<GpuPaintFn>,
    pub on_click: Option<Msg>,
    /// Handler de click que recibe la posición **relativa al rect del
    /// nodo** (esquina superior-izquierda del nodo = `(0, 0)`). Útil
    /// para canvas elements que quieren mapear el click a coordenadas
    /// de mundo. Si está presente, gana sobre `on_click`. Devolver
    /// `None` no dispara update.
    pub on_click_at: Option<ClickAtFn<Msg>>,
    /// Equivalente a `on_click` pero para el botón derecho del ratón.
    /// Pensado para menús contextuales: el nodo declara qué `Msg`
    /// emitir cuando se le hace right-click, y la app abre el overlay
    /// con el menú.
    pub on_right_click: Option<Msg>,
    /// Variante posicional de [`Self::on_right_click`]. Útil para
    /// grillas que necesitan saber *qué celda* del rect recibió el
    /// click derecho (la celda no es un nodo aparte, sino una región
    /// dentro del nodo). Si está presente, gana sobre `on_right_click`.
    pub on_right_click_at: Option<ClickAtFn<Msg>>,
    /// Equivalente a `on_click` pero para el botón del medio del ratón
    /// (rueda presionada). Pensado para abrir en pestaña nueva — los
    /// browsers usan middle-click como atajo equivalente a Ctrl+Click.
    pub on_middle_click: Option<Msg>,
    /// Handler de drag. Si está presente, este nodo arrastra (y NO emite
    /// `on_click` al presionar — un nodo es uno u otro).
    pub drag: Option<DragFn<Msg>>,
    /// Variante de drag que recibe la posición inicial del press relativa
    /// al rect del nodo. Gana sobre `drag` si ambos están presentes.
    pub drag_at: Option<DragAtFn<Msg>>,
    /// Payload `u64` que viaja con el drag iniciado sobre este nodo. Lo
    /// recibe el handler [`Self::on_drop`] del drop target. Sin payload,
    /// el drag funciona igual pero ningún drop target reacciona.
    pub drag_payload: Option<u64>,
    /// Handler invocado al soltar un drag sobre este nodo (drop target).
    pub on_drop: Option<DropFn<Msg>>,
    /// Color a pintar mientras un drag activo está hovereando este drop
    /// target. Sobrepone a `fill`/`hover_fill` cuando aplica.
    pub drop_hover_fill: Option<Color>,
    /// Si `true`, los descendientes se recortan al rect del nodo (vía
    /// `scene.push_layer` con `Mix::Clip`). El hit-test también respeta
    /// el recorte: clicks fuera del rect ignoran a los hijos.
    pub clip: bool,
    /// Msg a emitir cuando el cursor entra al rect del nodo (transición
    /// no-hover → hover). Útil para previews tipo "URL del link al
    /// pasar el mouse".
    pub on_pointer_enter: Option<Msg>,
    /// Msg a emitir cuando el cursor sale del rect del nodo.
    pub on_pointer_leave: Option<Msg>,
    /// Opacidad multiplicada sobre TODO el subtree (este nodo + hijos),
    /// en `[0.0, 1.0]`. Se realiza con `scene.push_layer(Mix::Normal, a, …)`
    /// alrededor del rect del nodo: el subárbol se rasteriza en una capa
    /// intermedia y se compone al alfa indicado contra lo que ya hay
    /// detrás. `None` = sin capa (caso de la abrumadora mayoría de
    /// nodos). Útil para fade-in/out de overlays, ghosts mientras se
    /// arrastra, modales que aparecen, panels "vidrio". Note que la
    /// composición tiene costo (allocate + blit), por lo que sólo
    /// poblar este slot cuando hace falta — no es un atributo gratis.
    pub alpha: Option<f32>,
    pub children: Vec<View<Msg>>,
}


// --- Submódulos internos del runtime. Los tipos (View, Mounted, Runtime,
// caches) viven aquí en el root; los módulos acceden sus campos privados
// por la regla descendiente vía `use super::*`. Las free-fns de render se
// re-exportan pub(crate) para que el event-loop las llame bare; el impl
// ApplicationHandler de `eventloop` se registra solo (no necesita re-export). ---
mod eventloop;
mod render;
mod view;
pub(crate) use render::*;


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
    hover_fill: Option<Color>,
    radius: f64,
    text: Option<TextSpec>,
    image: Option<Image>,
    painter: Option<PaintFn>,
    gpu_painter: Option<GpuPaintFn>,
    on_click: Option<Msg>,
    on_click_at: Option<ClickAtFn<Msg>>,
    on_right_click: Option<Msg>,
    on_right_click_at: Option<ClickAtFn<Msg>>,
    on_middle_click: Option<Msg>,
    drag: Option<DragFn<Msg>>,
    drag_at: Option<DragAtFn<Msg>>,
    drag_payload: Option<u64>,
    on_drop: Option<DropFn<Msg>>,
    drop_hover_fill: Option<Color>,
    clip: bool,
    on_pointer_enter: Option<Msg>,
    on_pointer_leave: Option<Msg>,
    alpha: Option<f32>,
    /// Índice (exclusivo) del fin del subárbol en `Mounted::nodes`. Los
    /// descendientes ocupan `[idx + 1, subtree_end)`. Hace de "barrera" en
    /// paint/hit_test para `pop_layer` y para saltar subárboles enteros.
    subtree_end: usize,
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
    /// Árboles de layout reusados entre frames: `clear()` + `mount` en
    /// vez de re-allocar el slotmap de taffy en cada redraw. Uno para el
    /// árbol principal, otro para el overlay (sus `NodeId` no deben
    /// colisionar dentro del mismo frame).
    layout: LayoutTree,
    overlay_layout: LayoutTree,
    /// Último frame renderizado: árbol montado + rects absolutos +
    /// nodo con hover. Lo consume el handler de click para hit-testear
    /// sin reconstruir `view` + layout, y CursorMoved para detectar si
    /// el hover cambió y disparar redraw.
    last_render: Option<RenderCache<A::Msg>>,
    /// Drag activo. Mantiene su propio handler clonado del MountedNode
    /// — así el drag sobrevive aunque el cache se invalide entre
    /// eventos.
    drag: Option<DragState<A::Msg>>,
}

struct RenderCache<Msg> {
    mounted: Mounted<Msg>,
    computed: ComputedLayout,
    /// Índice del nodo en hover en el frame ya pintado. `None` si el
    /// cursor no toca ningún `hover_fill`.
    hover_idx: Option<usize>,
    /// Índice del drop target hovereado en el frame ya pintado. Solo
    /// se setea durante un drag activo con `payload` declarado.
    drop_hover_idx: Option<usize>,
    /// Capa de overlay (menú contextual, modal). Cuando está presente,
    /// hover/click/right-click se rutean a ella exclusivamente — el
    /// árbol principal queda "bajo vidrio" hasta que la app cierre el
    /// overlay devolviendo `None` desde [`App::view_overlay`].
    overlay: Option<OverlayCache<Msg>>,
}

struct OverlayCache<Msg> {
    mounted: Mounted<Msg>,
    computed: ComputedLayout,
    hover_idx: Option<usize>,
}

/// Dos sabores de handler de drag activo: el simple `(phase, dx, dy)`
/// o la variante que conserva la posición local del press original
/// `(phase, dx, dy, lx0, ly0)`. El runtime elige uno al iniciar el drag.
enum DragHandlerKind<Msg> {
    Delta(DragFn<Msg>),
    DeltaAt(DragAtFn<Msg>, f32, f32),
}

struct DragState<Msg> {
    handler: DragHandlerKind<Msg>,
    /// Cursor en el último evento (Press o CursorMoved). El delta del
    /// próximo Move se calcula contra este, no contra el inicio del
    /// drag — el caller acumula los deltas en su modelo si los necesita.
    last_cursor: PhysicalPosition<f64>,
    /// Payload `u64` que viaja con el drag. `None` si el draggable
    /// origen no declaró ninguno (drag de resize/scroll/etc.). Los drop
    /// targets sólo reaccionan cuando hay payload.
    payload: Option<u64>,
}


/// Punto de entrada: corre el bucle Elm hasta que el usuario cierre la
/// ventana (o la app llame [`Handle::quit`]).
pub fn run<A: App>() {
    let event_loop = EventLoop::<UserEvent<A::Msg>>::with_user_event()
        .build()
        .expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let handle = Handle {
        inner: HandleInner::Real(event_loop.create_proxy()),
    };
    let mut runtime: Runtime<A> = Runtime {
        handle,
        state: None,
    };
    event_loop.run_app(&mut runtime).expect("run app");
}
