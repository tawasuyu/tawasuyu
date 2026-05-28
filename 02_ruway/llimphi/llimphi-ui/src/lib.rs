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
        let proxy = self.proxy.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(period);
            if proxy.send_event(UserEvent::Msg(f())).is_err() {
                // Event loop cerrado — el thread puede morir.
                break;
            }
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
    pub children: Vec<View<Msg>>,
}

impl<Msg> View<Msg> {
    pub fn new(style: Style) -> Self {
        Self {
            style,
            fill: None,
            hover_fill: None,
            radius: 0.0,
            text: None,
            image: None,
            painter: None,
            on_click: None,
            on_click_at: None,
            on_right_click: None,
            on_right_click_at: None,
            drag: None,
            drag_at: None,
            drag_payload: None,
            on_drop: None,
            drop_hover_fill: None,
            clip: false,
            children: Vec::new(),
        }
    }

    pub fn fill(mut self, color: Color) -> Self {
        self.fill = Some(color);
        self
    }

    /// Color a usar cuando el cursor está sobre este nodo. Habilita
    /// el hit-test de hover sobre el nodo.
    pub fn hover_fill(mut self, color: Color) -> Self {
        self.hover_fill = Some(color);
        self
    }

    /// Marca este nodo como draggable. Mientras el usuario sostenga el
    /// botón izquierdo sobre él, el runtime llama `handler(Move, dx, dy)`
    /// por cada `CursorMoved` (dx/dy = delta desde el evento anterior) y
    /// `handler(End, 0, 0)` al soltar. Sobreescribe `on_click` para este
    /// nodo: un nodo es draggable **o** clickable.
    pub fn draggable<F>(mut self, handler: F) -> Self
    where
        F: Fn(DragPhase, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.drag = Some(Arc::new(handler));
        self
    }

    /// Como `draggable`, pero el handler también recibe la posición
    /// inicial del press relativa al rect del nodo `(initial_lx,
    /// initial_ly)`. Útil cuando el caller necesita resolver qué
    /// entidad bajo el cursor inició el drag (Conceptos, lemmings,
    /// nodos de un grafo, etc.). Gana sobre `draggable` si ambos están.
    pub fn draggable_at<F>(mut self, handler: F) -> Self
    where
        F: Fn(DragPhase, f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.drag_at = Some(Arc::new(handler));
        self
    }

    /// Declara el payload `u64` que viaja con el drag de este nodo. Los
    /// drop targets bajo cursor al soltar reciben este valor en su
    /// `on_drop`. Sin payload, los drop targets no reaccionan (útil para
    /// drags de "resize/scroll" que no representan transferencia).
    pub fn drag_payload(mut self, payload: u64) -> Self {
        self.drag_payload = Some(payload);
        self
    }

    /// Marca este nodo como drop target. El runtime invoca `handler(payload)`
    /// cuando un drag termina sobre el rect de este nodo y el origen del
    /// drag declaró un payload. Si devuelve `Some(Msg)`, se dispatchea al
    /// `update` antes del `DragPhase::End` del origen.
    pub fn on_drop<F>(mut self, handler: F) -> Self
    where
        F: Fn(u64) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_drop = Some(Arc::new(handler));
        self
    }

    /// Color de relleno cuando un drag activo está hovereando este drop
    /// target. Análogo a `hover_fill` pero solo aplica mientras dura un
    /// drag. Útil para resaltar el destino válido.
    pub fn drop_hover_fill(mut self, color: Color) -> Self {
        self.drop_hover_fill = Some(color);
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
            italic: false,
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
            italic: false,
        });
        self
    }

    /// Como `text_aligned` pero con un flag `italic`. Si la fuente activa
    /// no tiene variante italic, parley aplica synthesizing.
    pub fn text_aligned_italic(
        mut self,
        content: impl Into<String>,
        size_px: f32,
        color: Color,
        alignment: llimphi_text::Alignment,
        italic: bool,
    ) -> Self {
        self.text = Some(TextSpec {
            content: content.into(),
            size_px,
            color,
            alignment,
            italic,
        });
        self
    }

    pub fn on_click(mut self, msg: Msg) -> Self {
        self.on_click = Some(msg);
        self
    }

    /// Como `on_click`, pero el handler recibe `(local_x, local_y,
    /// rect_w, rect_h)` — la posición del cursor relativa al rect del
    /// nodo más las dimensiones actuales del nodo. Útil para canvas
    /// elements que necesitan saber dónde fue el click para convertirlo
    /// a coordenadas de mundo. Sobrescribe `on_click` para este nodo
    /// si ambos están presentes.
    pub fn on_click_at<F>(mut self, handler: F) -> Self
    where
        F: Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_click_at = Some(Arc::new(handler));
        self
    }

    /// Declara el `Msg` a emitir cuando el usuario hace click derecho
    /// sobre este nodo. Para menús contextuales, conviene pasar un
    /// `Msg::OpenMenu { ... }` y dejar que el modelo guarde la
    /// posición; el overlay se abre vía [`App::view_overlay`].
    pub fn on_right_click(mut self, msg: Msg) -> Self {
        self.on_right_click = Some(msg);
        self
    }

    /// Variante posicional de [`Self::on_right_click`]. El handler recibe
    /// `(local_x, local_y, rect_w, rect_h)` para que un nodo "grilla"
    /// pueda resolver internamente qué subcelda recibió el click. La
    /// posición está relativa al rect del nodo.
    pub fn on_right_click_at<F>(mut self, handler: F) -> Self
    where
        F: Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_right_click_at = Some(Arc::new(handler));
        self
    }

    /// Pinta `image` dentro del rect del nodo, centrada y escalada
    /// preservando aspect ratio. Re-exporta `peniko::Image` vía
    /// `llimphi_raster::peniko::Image` — el caller decodifica los
    /// bytes con el crate `image` (u otro) y construye el `Image`
    /// con `Blob<u8>` + `ImageFormat::Rgba8`.
    pub fn image(mut self, image: Image) -> Self {
        self.image = Some(image);
        self
    }

    /// Registra una closure de pintura custom. El runtime la invoca
    /// con `(&mut vello::Scene, &mut Typesetter, PaintRect)` durante
    /// el paint del nodo. La closure es responsable de pintar
    /// primitivas custom dentro del rect; no debe dejar `push_layer`
    /// sin par. Soporte para canvas elements estilo
    /// dominium/pluma/cosmos.
    pub fn paint_with<F>(mut self, painter: F) -> Self
    where
        F: Fn(&mut vello::Scene, &mut llimphi_text::Typesetter, PaintRect)
            + Send
            + Sync
            + 'static,
    {
        self.painter = Some(Arc::new(painter));
        self
    }

    /// Recorta los hijos al rect de este nodo (paint y hit-test). Útil
    /// para paneles con contenido virtualizado que no debe sangrar a
    /// vecinos (listas, scrollers, viewers).
    pub fn clip(mut self, enabled: bool) -> Self {
        self.clip = enabled;
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
    hover_fill: Option<Color>,
    radius: f64,
    text: Option<TextSpec>,
    image: Option<Image>,
    painter: Option<PaintFn>,
    on_click: Option<Msg>,
    on_click_at: Option<ClickAtFn<Msg>>,
    on_right_click: Option<Msg>,
    on_right_click_at: Option<ClickAtFn<Msg>>,
    drag: Option<DragFn<Msg>>,
    drag_at: Option<DragAtFn<Msg>>,
    drag_payload: Option<u64>,
    on_drop: Option<DropFn<Msg>>,
    drop_hover_fill: Option<Color>,
    clip: bool,
    /// Índice (exclusivo) del fin del subárbol en `Mounted::nodes`. Los
    /// descendientes ocupan `[idx + 1, subtree_end)`. Hace de "barrera" en
    /// paint/hit_test para `pop_layer` y para saltar subárboles enteros.
    subtree_end: usize,
}

fn mount<Msg: Clone>(layout: &mut LayoutTree, v: View<Msg>) -> Mounted<Msg> {
    let mut nodes = Vec::new();
    let root = mount_recursive(layout, v, &mut nodes);
    Mounted { root, nodes }
}

/// Mount en pre-orden directo sobre `out`: pusheamos el padre como
/// placeholder (id real desconocido hasta crear el taffy node), recursamos
/// hijos sobre el mismo `out`, y al volver completamos `id` + `subtree_end`.
fn mount_recursive<Msg: Clone>(
    layout: &mut LayoutTree,
    v: View<Msg>,
    out: &mut Vec<MountedNode<Msg>>,
) -> NodeId {
    let View {
        style,
        fill,
        hover_fill,
        radius,
        text,
        image,
        painter,
        on_click,
        on_click_at,
        on_right_click,
        on_right_click_at,
        drag,
        drag_at,
        drag_payload,
        on_drop,
        drop_hover_fill,
        clip,
        children,
    } = v;
    let parent_idx = out.len();
    out.push(MountedNode {
        id: NodeId::new(0), // placeholder, lo sobreescribimos abajo
        fill,
        hover_fill,
        radius,
        text,
        image,
        painter,
        on_click,
        on_click_at,
        on_right_click,
        on_right_click_at,
        drag,
        drag_at,
        drag_payload,
        on_drop,
        drop_hover_fill,
        clip,
        subtree_end: 0,
    });
    let mut child_ids = Vec::with_capacity(children.len());
    for child in children {
        child_ids.push(mount_recursive(layout, child, out));
    }
    let id = if child_ids.is_empty() {
        layout.leaf(style).expect("layout leaf")
    } else {
        layout.node(style, &child_ids).expect("layout node")
    };
    out[parent_idx].id = id;
    out[parent_idx].subtree_end = out.len();
    id
}

fn paint<Msg>(
    scene: &mut vello::Scene,
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    typesetter: &mut llimphi_text::Typesetter,
    hover_idx: Option<usize>,
    drop_hover_idx: Option<usize>,
) {
    // Stack de subtree_end de los nodos con `clip = true` que están
    // activos. Cuando el índice del próximo nodo cruza el top, pop_layer.
    let mut clip_stack: Vec<usize> = Vec::new();
    for (idx, node) in mounted.nodes.iter().enumerate() {
        // Cierre de clips que ya quedaron atrás (idx ≥ subtree_end).
        while let Some(&end) = clip_stack.last() {
            if idx >= end {
                scene.pop_layer();
                clip_stack.pop();
            } else {
                break;
            }
        }
        let Some(r) = computed.get(node.id) else {
            continue;
        };
        // Prioridad de pintura: drop-hover (drag activo) > hover normal >
        // fill base. Solo aplica el override si el slot correspondiente
        // está poblado; el siguiente cae como fallback.
        let effective_fill = if Some(idx) == drop_hover_idx {
            node.drop_hover_fill.or(node.hover_fill).or(node.fill)
        } else if Some(idx) == hover_idx {
            node.hover_fill.or(node.fill)
        } else {
            node.fill
        };
        if let Some(color) = effective_fill {
            let rr = RoundedRect::new(
                r.x as f64,
                r.y as f64,
                (r.x + r.w) as f64,
                (r.y + r.h) as f64,
                node.radius,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rr);
        }
        if let Some(image) = node.image.as_ref() {
            // Aspect-fit centrado: el min de las dos escalas ocupa
            // todo el rect en el eje más restrictivo y deja banda en
            // el otro. Defensivo: envolvemos en push_layer/pop_layer
            // con el rect del nodo para que, aunque el caller pida
            // un layout mal-dimensionado, la imagen nunca pinte fuera
            // del nodo (visualmente preferible a un overflow opaco).
            if image.width > 0 && image.height > 0 && r.w > 0.0 && r.h > 0.0 {
                let sx = r.w as f64 / image.width as f64;
                let sy = r.h as f64 / image.height as f64;
                let s = sx.min(sy);
                let disp_w = image.width as f64 * s;
                let disp_h = image.height as f64 * s;
                let tx = r.x as f64 + (r.w as f64 - disp_w) * 0.5;
                let ty = r.y as f64 + (r.h as f64 - disp_h) * 0.5;
                let transform = Affine::translate((tx, ty)) * Affine::scale(s);
                let node_rect = KurboRect::new(
                    r.x as f64,
                    r.y as f64,
                    (r.x + r.w) as f64,
                    (r.y + r.h) as f64,
                );
                scene.push_layer(Mix::Clip, 1.0, Affine::IDENTITY, &node_rect);
                scene.draw_image(image, transform);
                scene.pop_layer();
            }
        }
        if let Some(painter) = node.painter.as_ref() {
            (painter)(
                scene,
                typesetter,
                PaintRect {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                },
            );
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
                italic: text.italic,
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
        if node.clip {
            let clip_rect = KurboRect::new(
                r.x as f64,
                r.y as f64,
                (r.x + r.w) as f64,
                (r.y + r.h) as f64,
            );
            scene.push_layer(Mix::Clip, 1.0, Affine::IDENTITY, &clip_rect);
            clip_stack.push(node.subtree_end);
        }
    }
    // Cerrá clips que llegaron al final de la lista sin pop intermedio.
    while clip_stack.pop().is_some() {
        scene.pop_layer();
    }
}

/// Hit-test parametrizado por elegibilidad. Devuelve el índice del nodo
/// más al frente (último en pre-orden) cuyo rect contiene `(x, y)` y para
/// el cual `pred` devuelve `true`, respetando `clip`: si el punto cae
/// afuera de un nodo con clip, el subárbol entero es invisible.
fn hit_test_pred<Msg, F>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
    pred: F,
) -> Option<usize>
where
    F: Fn(&MountedNode<Msg>) -> bool,
{
    let mut hit: Option<usize> = None;
    let mut clip_stack: Vec<usize> = Vec::new();
    let mut idx = 0;
    while idx < mounted.nodes.len() {
        while let Some(&end) = clip_stack.last() {
            if idx >= end {
                clip_stack.pop();
            } else {
                break;
            }
        }
        let node = &mounted.nodes[idx];
        let Some(r) = computed.get(node.id) else {
            idx += 1;
            continue;
        };
        let inside = x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;
        if node.clip {
            if !inside {
                idx = node.subtree_end;
                continue;
            }
            clip_stack.push(node.subtree_end);
        }
        if inside && pred(node) {
            hit = Some(idx);
        }
        idx += 1;
    }
    hit
}

/// Hit-test específico para clicks (incluye nodos draggables).
fn hit_test_click<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| {
        n.on_click.is_some()
            || n.on_click_at.is_some()
            || n.drag.is_some()
            || n.drag_at.is_some()
    })
}

/// Hit-test específico para right-click. Sólo considera nodos que
/// declararon `on_right_click` o `on_right_click_at` — un right-click
/// sobre un nodo sin handler no hace nada (no se "filtra" al click
/// izquierdo).
fn hit_test_right_click<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| {
        n.on_right_click.is_some() || n.on_right_click_at.is_some()
    })
}

/// Hit-test específico para hover (nodos con `hover_fill`).
fn hit_test_hover<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.hover_fill.is_some())
}

/// Hit-test específico para drop targets (nodos con `on_drop`). Usado
/// durante un drag activo para resaltar el destino y para invocar el
/// handler al soltar.
fn hit_test_drop<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.on_drop.is_some())
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
            drag: None,
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
                let prev_cursor = state.cursor;
                state.cursor = position;
                // Drag activo: dispatchear delta al handler + actualizar
                // tracking del drop target hovereado (solo si hay payload).
                if let Some(drag) = state.drag.as_mut() {
                    let dx = (position.x - drag.last_cursor.x) as f32;
                    let dy = (position.y - drag.last_cursor.y) as f32;
                    drag.last_cursor = position;
                    let payload_active = drag.payload.is_some();
                    let mut need_redraw = false;
                    if dx != 0.0 || dy != 0.0 {
                        let msg_opt = match &drag.handler {
                            DragHandlerKind::Delta(h) => h(DragPhase::Move, dx, dy),
                            DragHandlerKind::DeltaAt(h, lx0, ly0) => {
                                h(DragPhase::Move, dx, dy, *lx0, *ly0)
                            }
                        };
                        if let Some(msg) = msg_opt {
                            let model = state.model.take().expect("model");
                            state.model = Some(A::update(model, msg, &self.handle));
                            // Durante drag NO invalidamos el cache —
                            // queda válido para el próximo Move.
                            need_redraw = true;
                        }
                    }
                    if payload_active {
                        if let Some(cache) = state.last_render.as_mut() {
                            let new_drop = hit_test_drop(
                                &cache.mounted,
                                &cache.computed,
                                position.x as f32,
                                position.y as f32,
                            );
                            if new_drop != cache.drop_hover_idx {
                                cache.drop_hover_idx = new_drop;
                                need_redraw = true;
                            }
                        }
                    }
                    if need_redraw {
                        state.window.request_redraw();
                    }
                } else if let Some(cache) = state.last_render.as_ref() {
                    // Sin drag: chequear hover. Si hay overlay, el
                    // hover-test va contra él; el árbol principal queda
                    // congelado mientras el overlay esté arriba.
                    if let Some(ov) = cache.overlay.as_ref() {
                        let new_hover = hit_test_hover(
                            &ov.mounted,
                            &ov.computed,
                            position.x as f32,
                            position.y as f32,
                        );
                        if new_hover != ov.hover_idx {
                            state.window.request_redraw();
                        }
                    } else {
                        let new_hover = hit_test_hover(
                            &cache.mounted,
                            &cache.computed,
                            position.x as f32,
                            position.y as f32,
                        );
                        if new_hover != cache.hover_idx {
                            state.window.request_redraw();
                        }
                    }
                    let _ = prev_cursor;
                }
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
            WindowEvent::MouseWheel { delta, .. } => {
                // Convención winit: LineDelta es líneas; PixelDelta es
                // píxeles físicos (touchpads). En CSS y aquí, positivo
                // (rueda hacia adelante / dos dedos arriba) = scroll
                // hacia arriba, así que invertimos `y` para que el
                // contenido "siga al dedo" en y positivo. `x` queda
                // como llega.
                let wd = match delta {
                    MouseScrollDelta::LineDelta(x, y) => WheelDelta { x, y: -y },
                    MouseScrollDelta::PixelDelta(p) => WheelDelta {
                        x: (p.x as f32) / 20.0,
                        y: -(p.y as f32) / 20.0,
                    },
                };
                let cursor = (state.cursor.x as f32, state.cursor.y as f32);
                if let Some(msg) = A::on_wheel(
                    state.model.as_ref().expect("model"),
                    wd,
                    cursor,
                    state.modifiers,
                ) {
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
                // Hit-test contra el cache del último redraw (siempre
                // representa lo visible). Fallback raro: cache vacío.
                let cursor = state.cursor;
                // Tupla: (drag_fn, drag_at_fn, payload, on_click_msg,
                //         on_click_at_handler, rect: (x, y, w, h))
                type HitInfo<M> = (
                    Option<DragFn<M>>,
                    Option<DragAtFn<M>>,
                    Option<u64>,
                    Option<M>,
                    Option<ClickAtFn<M>>,
                    Option<(f32, f32, f32, f32)>,
                );
                let lookup_hit = |m: &Mounted<A::Msg>, c: &ComputedLayout| -> Option<HitInfo<A::Msg>> {
                    hit_test_click(m, c, cursor.x as f32, cursor.y as f32).map(|i| {
                        let node = &m.nodes[i];
                        let rect = c.get(node.id).map(|r| (r.x, r.y, r.w, r.h));
                        (
                            node.drag.clone(),
                            node.drag_at.clone(),
                            node.drag_payload,
                            node.on_click.clone(),
                            node.on_click_at.clone(),
                            rect,
                        )
                    })
                };
                // Con overlay activo, los clicks van EXCLUSIVAMENTE a él.
                // Si el cursor cae sobre un nodo del overlay sin handler,
                // el click se descarta — la convención de "scrim que
                // dismissa" pide que la app meta su propio fondo
                // clicable con `on_click = DismissOverlay`.
                let idx_and_action: Option<HitInfo<A::Msg>> = if let Some(cache) =
                    state.last_render.as_ref()
                {
                    if let Some(ov) = cache.overlay.as_ref() {
                        lookup_hit(&ov.mounted, &ov.computed)
                    } else {
                        lookup_hit(&cache.mounted, &cache.computed)
                    }
                } else {
                    let model_ref = state.model.as_ref().expect("model");
                    let view = A::view(model_ref);
                    let overlay_view = A::view_overlay(model_ref);
                    let mut layout = LayoutTree::new();
                    let mounted: Mounted<A::Msg> = mount(&mut layout, view);
                    let (w, h) = state.surface.size();
                    let computed = layout
                        .compute(mounted.root, (w as f32, h as f32))
                        .expect("layout");
                    if let Some(ov) = overlay_view {
                        let mut olay = LayoutTree::new();
                        let omounted: Mounted<A::Msg> = mount(&mut olay, ov);
                        let ocomp = olay
                            .compute(omounted.root, (w as f32, h as f32))
                            .expect("layout overlay");
                        lookup_hit(&omounted, &ocomp)
                    } else {
                        lookup_hit(&mounted, &computed)
                    }
                };
                // drag_at + on_click_at COEXISTEN: el press dispara
                // on_click_at (si está) y arranca un drag rastreado con la
                // posición inicial. Diseño pensado para canvas elements
                // que necesitan select-on-press + move-on-drag.
                //
                // En cambio, `drag` simple (sin _at) mantiene la semántica
                // antigua: gana exclusivo sobre on_click.
                if let Some((_, Some(handler_at), payload, _, click_at, Some((ox, oy, rw, rh)))) =
                    &idx_and_action
                {
                    let lx0 = cursor.x as f32 - ox;
                    let ly0 = cursor.y as f32 - oy;
                    // Disparar on_click_at en el press (si también está).
                    if let Some(click_at_h) = click_at {
                        if let Some(msg) = click_at_h(lx0, ly0, *rw, *rh) {
                            let model = state.model.take().expect("model");
                            state.model = Some(A::update(model, msg, &self.handle));
                            state.last_render = None;
                        }
                    }
                    state.drag = Some(DragState {
                        handler: DragHandlerKind::DeltaAt(handler_at.clone(), lx0, ly0),
                        last_cursor: cursor,
                        payload: *payload,
                    });
                    state.window.request_redraw();
                } else if let Some((Some(handler), _, payload, _, _, _)) = &idx_and_action {
                    state.drag = Some(DragState {
                        handler: DragHandlerKind::Delta(handler.clone()),
                        last_cursor: cursor,
                        payload: *payload,
                    });
                    // Si hay payload, repintar para que el drop target
                    // bajo cursor (si lo hay) se ilumine de entrada.
                    if payload.is_some() {
                        if let Some(cache) = state.last_render.as_mut() {
                            let new_drop = hit_test_drop(
                                &cache.mounted,
                                &cache.computed,
                                cursor.x as f32,
                                cursor.y as f32,
                            );
                            if new_drop != cache.drop_hover_idx {
                                cache.drop_hover_idx = new_drop;
                                state.window.request_redraw();
                            }
                        }
                    }
                } else if let Some((_, _, _, _, Some(handler), Some((ox, oy, rw, rh)))) =
                    &idx_and_action
                {
                    // on_click_at gana sobre on_click si ambos existen.
                    let lx = cursor.x as f32 - ox;
                    let ly = cursor.y as f32 - oy;
                    if let Some(msg) = handler(lx, ly, *rw, *rh) {
                        let model = state.model.take().expect("model");
                        state.model = Some(A::update(model, msg, &self.handle));
                        state.last_render = None;
                        state.window.request_redraw();
                    }
                } else if let Some((_, _, _, Some(msg), _, _)) = idx_and_action {
                    let model = state.model.take().expect("model");
                    state.model = Some(A::update(model, msg, &self.handle));
                    state.last_render = None;
                    state.window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                // Right-click: dispatcheamos `on_right_click` o
                // `on_right_click_at` del nodo bajo cursor. La capa
                // overlay tiene prioridad (mismo razonamiento que el
                // left-click). Nodos sin handler de right-click no
                // reaccionan — no "filtramos" al left.
                let cursor = state.cursor;
                let lookup =
                    |m: &Mounted<A::Msg>, c: &ComputedLayout| -> Option<(Option<A::Msg>, Option<ClickAtFn<A::Msg>>, (f32, f32, f32, f32))> {
                        hit_test_right_click(m, c, cursor.x as f32, cursor.y as f32).map(|i| {
                            let node = &m.nodes[i];
                            let rect = c
                                .get(node.id)
                                .map(|r| (r.x, r.y, r.w, r.h))
                                .unwrap_or((0.0, 0.0, 0.0, 0.0));
                            (
                                node.on_right_click.clone(),
                                node.on_right_click_at.clone(),
                                rect,
                            )
                        })
                    };
                let hit = if let Some(cache) = state.last_render.as_ref() {
                    if let Some(ov) = cache.overlay.as_ref() {
                        lookup(&ov.mounted, &ov.computed)
                    } else {
                        lookup(&cache.mounted, &cache.computed)
                    }
                } else {
                    None
                };
                if let Some((msg_opt, at_opt, (ox, oy, rw, rh))) = hit {
                    let msg = if let Some(handler) = at_opt {
                        handler(
                            cursor.x as f32 - ox,
                            cursor.y as f32 - oy,
                            rw,
                            rh,
                        )
                    } else {
                        msg_opt
                    };
                    if let Some(msg) = msg {
                        let model = state.model.take().expect("model");
                        state.model = Some(A::update(model, msg, &self.handle));
                        state.last_render = None;
                        state.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(drag) = state.drag.take() {
                    let cursor = state.cursor;
                    // 1. Drop: si hay payload + drop target bajo cursor,
                    //    invocamos su handler. El Msg resultante se aplica
                    //    ANTES del End del drag — la convención es "drop
                    //    primero, cleanup del drag después".
                    if let Some(payload) = drag.payload {
                        if let Some(cache) = state.last_render.as_ref() {
                            if let Some(idx) = hit_test_drop(
                                &cache.mounted,
                                &cache.computed,
                                cursor.x as f32,
                                cursor.y as f32,
                            ) {
                                if let Some(drop_h) =
                                    cache.mounted.nodes[idx].on_drop.clone()
                                {
                                    if let Some(msg) = (drop_h)(payload) {
                                        let model = state.model.take().expect("model");
                                        state.model = Some(A::update(model, msg, &self.handle));
                                    }
                                }
                            }
                        }
                    }
                    // 2. Cierre del drag.
                    let end_msg = match &drag.handler {
                        DragHandlerKind::Delta(h) => h(DragPhase::End, 0.0, 0.0),
                        DragHandlerKind::DeltaAt(h, lx0, ly0) => {
                            h(DragPhase::End, 0.0, 0.0, *lx0, *ly0)
                        }
                    };
                    if let Some(msg) = end_msg {
                        let model = state.model.take().expect("model");
                        state.model = Some(A::update(model, msg, &self.handle));
                    }
                    // Cache invalidado siempre — hover/drop pueden cambiar
                    // y el modelo posiblemente mutó.
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
                let model_ref = state.model.as_ref().expect("model");
                let view = A::view(model_ref);
                let overlay_view = A::view_overlay(model_ref);
                let mut layout = LayoutTree::new();
                let mounted: Mounted<A::Msg> = mount(&mut layout, view);
                let computed = layout
                    .compute(mounted.root, (w as f32, h as f32))
                    .expect("layout");
                // Mount + layout del overlay en un árbol aparte. Lo
                // computamos con el mismo tamaño de viewport para que
                // un scrim a percent(1.0) cubra toda la pantalla.
                let overlay_built = overlay_view.map(|v| {
                    let mut olayout = LayoutTree::new();
                    let omounted: Mounted<A::Msg> = mount(&mut olayout, v);
                    let ocomputed = olayout
                        .compute(omounted.root, (w as f32, h as f32))
                        .expect("layout overlay");
                    let ohover = hit_test_hover(
                        &omounted,
                        &ocomputed,
                        state.cursor.x as f32,
                        state.cursor.y as f32,
                    );
                    OverlayCache {
                        mounted: omounted,
                        computed: ocomputed,
                        hover_idx: ohover,
                    }
                });
                // Hover en el main solo si NO hay overlay — durante un
                // menú abierto, el fondo no debe reaccionar al ratón.
                let hover_idx = if overlay_built.is_some() {
                    None
                } else {
                    hit_test_hover(
                        &mounted,
                        &computed,
                        state.cursor.x as f32,
                        state.cursor.y as f32,
                    )
                };
                // Drop hover sólo si hay drag activo con payload (un
                // drag bloquea el overlay; rara combinación pero la
                // resolvemos a favor del drag).
                let drop_hover_idx = state
                    .drag
                    .as_ref()
                    .and_then(|d| d.payload.map(|_| ()))
                    .and_then(|_| {
                        hit_test_drop(
                            &mounted,
                            &computed,
                            state.cursor.x as f32,
                            state.cursor.y as f32,
                        )
                    });
                state.scene.reset();
                paint(
                    &mut state.scene,
                    &mounted,
                    &computed,
                    &mut state.typesetter,
                    hover_idx,
                    drop_hover_idx,
                );
                if let Some(ov) = overlay_built.as_ref() {
                    paint(
                        &mut state.scene,
                        &ov.mounted,
                        &ov.computed,
                        &mut state.typesetter,
                        ov.hover_idx,
                        None,
                    );
                }
                if let Err(e) = state.renderer.render(
                    &state.hal,
                    &state.scene,
                    &frame,
                    palette::css::BLACK,
                ) {
                    eprintln!("render error: {e}");
                }
                state.surface.present(frame, &state.hal);
                state.last_render = Some(RenderCache {
                    mounted,
                    computed,
                    hover_idx,
                    drop_hover_idx,
                    overlay: overlay_built,
                });
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
