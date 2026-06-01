//! llimphi-ui — Runtime Elm sobre winit.
//!
//! Maneja el bucle `input → update(model, msg) → view(model) → layout →
//! raster → present` sobre una ventana winit + GPU (`llimphi-hal` +
//! `llimphi-raster`). La parte declarativa y winit-agnóstica (el árbol
//! `View<Msg>`, `mount`, `paint`, hit-test) vive en `llimphi-compositor` y
//! se re-exporta tal cual, así los consumidores siguen escribiendo
//! `llimphi_ui::View` sin enterarse del split.
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
use llimphi_layout::{ComputedLayout, LayoutTree};
use llimphi_raster::peniko::color::palette;
use llimphi_raster::{vello, Renderer};

pub use llimphi_hal;
pub use llimphi_layout;
pub use llimphi_raster;
pub use llimphi_text;

// El compositor declarativo (View, mount, paint, hit-test, tipos de
// handler) se re-exporta entero: `llimphi_ui::View`, `llimphi_ui::DragFn`,
// etc. siguen resolviendo igual que antes del split.
pub use llimphi_compositor;
pub use llimphi_compositor::*;

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

    /// Maneja un redimensionado de la ventana. `width`/`height` son el
    /// nuevo tamaño en **píxeles físicos** (lo que reporta
    /// `winit::WindowEvent::Resized` y lo que recibe la surface). El
    /// runtime ya reconfiguró la surface y pedirá redraw; este callback
    /// es para que la app reaccione al nuevo viewport (recalcular layout
    /// dependiente del tamaño, emitir un evento `resize`, etc.).
    /// Devolver `Some(Msg)` dispara un update; `None` (default) lo ignora.
    fn on_resize(_model: &Self::Model, _width: u32, _height: u32) -> Option<Self::Msg> {
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

// --- Runtime winit. El event loop (impl ApplicationHandler) vive en
// `eventloop` y accede los campos privados de estos structs vía
// `use super::*`. La composición declarativa (View, mount, paint,
// hit-test) la trae el re-export de `llimphi_compositor`. ---
mod eventloop;

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
