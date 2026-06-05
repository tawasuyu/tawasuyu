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

    /// El foco cambió: el runtime movió el foco a `id` (`None` = nada
    /// enfocado). Pasa al pulsar Tab/Shift+Tab (recorre los nodos
    /// `View::focusable` en orden de árbol, envolviendo) o al clickear un
    /// nodo enfocable. La app guarda `id` en su `Model` para (a) pintar el
    /// focus-ring (`if model.focus == Some(id) { … }` en `view`) y (b)
    /// rutear el teclado al campo activo desde `on_key`. Devolver
    /// `Some(Msg)` dispara una transición; `None` (default) ignora.
    ///
    /// El foco lo administra el runtime (única fuente de verdad), así que
    /// Tab y click-to-focus quedan consistentes sin que la app los cablee.
    fn on_focus(_model: &Self::Model, _id: Option<u64>) -> Option<Self::Msg> {
        None
    }

    /// ¿Habilitar IME (input method editor) en esta ventana? Default
    /// `false`. Con IME activo, el texto compuesto (CJK, acentos muertos,
    /// emoji picker) llega por [`App::on_ime`] como `Commit`, **no** por
    /// `KeyEvent.text` — por eso es opt-in: las apps que sólo leen
    /// `on_key` siguen funcionando igual. Las que editan texto
    /// (`text-input`, `text-editor`) la activan e implementan `on_ime`.
    fn ime_allowed() -> bool {
        false
    }

    /// Maneja un evento de IME (sólo llega si [`App::ime_allowed`] es
    /// `true`). El flujo típico: `Enabled` → uno o más `Preedit` (texto en
    /// composición, a pintar subrayado en el caret) → `Commit(texto)` (el
    /// texto final, a insertar como si se hubiera tecleado) o `Disabled`.
    /// El `Preedit` no es definitivo: cada uno reemplaza al anterior, y un
    /// `Commit` o `Preedit` vacío lo cierra. Devolver `Some(Msg)` dispara
    /// una transición.
    fn on_ime(_model: &Self::Model, _event: &ImeEvent) -> Option<Self::Msg> {
        None
    }

    /// Área del caret en **píxeles físicos** `(x, y, w, h)` para posicionar
    /// la ventana de candidatos del IME (CJK) junto al cursor de texto. El
    /// runtime la consulta por frame cuando [`App::ime_allowed`] es `true`.
    /// `None` (default) deja que el sistema la ubique por defecto.
    fn ime_cursor_area(_model: &Self::Model) -> Option<(f32, f32, f32, f32)> {
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

    /// Maneja un cambio del factor de escala de la ventana (`scale_factor`
    /// de winit: 1.0 en pantallas normales, 2.0 en HiDPI/Retina, fraccional
    /// con escalado del compositor). El runtime lo invoca una vez al arrancar
    /// (con el factor inicial de la ventana, tras `init`) y luego en cada
    /// `WindowEvent::ScaleFactorChanged` (mover la ventana entre monitores,
    /// cambiar el escalado del sistema). Es lo que permite, p. ej., que
    /// `window.devicePixelRatio` refleje el DPI real. Devolver `Some(Msg)`
    /// dispara un update; `None` (default) lo ignora.
    fn on_scale_factor(_model: &Self::Model, _scale: f64) -> Option<Self::Msg> {
        None
    }

    /// Título de la ventana (sólo se lee al arrancar). Es el título inicial;
    /// para uno que cambie en runtime, ver [`App::window_title`].
    fn title() -> &'static str {
        "llimphi"
    }

    /// Título **dinámico** de la ventana, derivado del modelo. El runtime lo
    /// consulta tras cada render y, si cambió, lo aplica con `Window::set_title`
    /// — así el título de la barra del SO puede reflejar el estado (p. ej. el
    /// medio que se reproduce). `None` (default) deja el título fijo de
    /// [`App::title`]; una app que no lo implemente no paga nada.
    fn window_title(_model: &Self::Model) -> Option<String> {
        None
    }

    /// Vista de una ventana OS **secundaria** identificada por `key` (la que
    /// se pasó a [`Handle::open_window`]). El runtime la pinta en su propia
    /// ventana y rutea sus eventos al mismo [`App::update`] — comparte modelo
    /// con la primaria. `None` (default, o para una key desconocida) deja la
    /// ventana en blanco. Las secundarias NO tienen capa de overlay
    /// ([`App::view_overlay`] es sólo de la primaria); para diálogos dentro de
    /// una secundaria, componerlos en su propio `secondary_view`.
    fn secondary_view(_model: &Self::Model, _key: u64) -> Option<View<Self::Msg>> {
        None
    }

    /// Título dinámico de una ventana secundaria (análogo a
    /// [`App::window_title`] para la primaria). `None` deja el título con el
    /// que se abrió.
    fn secondary_title(_model: &Self::Model, _key: u64) -> Option<String> {
        None
    }

    /// El usuario cerró una ventana secundaria con el botón del SO. El runtime
    /// ya la destruyó; este callback es para que la app sincronice su modelo
    /// (p. ej. marcar el panel como cerrado). Devolver `Some(Msg)` dispara un
    /// `update`; `None` (default) no hace nada.
    fn on_secondary_close(_model: &Self::Model, _key: u64) -> Option<Self::Msg> {
        None
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
    /// Pide abrir una ventana OS **secundaria** con la `key` dada (la app la
    /// usa para distinguir cuál es en [`App::secondary_view`]). Idempotente:
    /// si ya existe una con esa key, se enfoca en vez de duplicar. La crea el
    /// event loop (que tiene el `ActiveEventLoop`); por eso va por mensaje.
    OpenWindow {
        key: u64,
        title: String,
        width: u32,
        height: u32,
    },
    /// Pide cerrar la ventana secundaria con esa `key`. No afecta a la primaria.
    CloseWindow { key: u64 },
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

    /// Abre una ventana OS **secundaria** (ver [`App::secondary_view`]). La
    /// `key` la elige la app para reconocerla luego; abrir con una key que ya
    /// existe sólo la enfoca (no duplica). El contenido lo pinta
    /// `App::secondary_view(model, key)` y los eventos (click/tecla/…) reentran
    /// al mismo `update`, así que la ventana comparte el modelo con la primaria.
    /// Cerrala con [`Self::close_window`] o con el botón del SO.
    pub fn open_window(&self, key: u64, title: impl Into<String>, width: u32, height: u32) {
        if let HandleInner::Real(p) = &self.inner {
            let _ = p.send_event(UserEvent::OpenWindow {
                key,
                title: title.into(),
                width,
                height,
            });
        }
    }

    /// Cierra la ventana secundaria con esa `key` (no-op si no existe). La
    /// ventana primaria nunca se cierra por acá — para eso está [`Self::quit`].
    pub fn close_window(&self, key: u64) {
        if let HandleInner::Real(p) = &self.inner {
            let _ = p.send_event(UserEvent::CloseWindow { key });
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

/// Evento de IME normalizado (espeja `winit::event::Ime`). Ver
/// [`App::on_ime`] para el flujo Enabled → Preedit* → Commit/Disabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeEvent {
    /// El IME se activó para esta ventana.
    Enabled,
    /// Texto en composición (aún no confirmado). `cursor` es el rango
    /// `(inicio, fin)` en bytes a resaltar dentro de `text`, si el IME lo
    /// reporta. Cada `Preedit` reemplaza al anterior; uno con `text`
    /// vacío cierra la preedición sin confirmar.
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// Texto confirmado: insertarlo como si se hubiera tecleado.
    Commit(String),
    /// El IME se desactivó (perder foco, cambiar de método).
    Disabled,
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
    /// Ventanas OS secundarias abiertas (opt-in vía [`Handle::open_window`]).
    /// Comparten el `Hal`/`Renderer` y el modelo de la primaria (`state`);
    /// cada una lleva su propia surface + caches de interacción. Vacío en la
    /// inmensa mayoría de las apps (monoventana) — coste cero.
    secondaries: Vec<SecondaryState<A>>,
}

/// Estado por **ventana secundaria**. Espeja los campos de interacción de
/// [`RuntimeState`] pero SIN modelo (vive en la primaria), sin overlay y sin
/// `Hal`/`Renderer` propios (los toma prestados de la primaria al pintar).
struct SecondaryState<A: App> {
    /// La key con la que la app la abrió (la pasa a `secondary_view`).
    key: u64,
    window: Arc<Window>,
    surface: WinitSurface,
    scene: vello::Scene,
    typesetter: llimphi_text::Typesetter,
    layout: LayoutTree,
    cursor: PhysicalPosition<f64>,
    modifiers: Modifiers,
    last_render: Option<SecRenderCache<A::Msg>>,
    hovered: Option<usize>,
    drag: Option<DragState<A::Msg>>,
    last_title: Option<String>,
}

/// Cache de render de una ventana secundaria (como [`RenderCache`] pero sin
/// capa de overlay). Sólo guarda el árbol montado + layout para hit-testear el
/// próximo click/hover; el `hover_idx` actual vive en `SecondaryState::hovered`.
struct SecRenderCache<Msg> {
    mounted: Mounted<Msg>,
    computed: ComputedLayout,
}

struct RuntimeState<A: App> {
    window: Arc<Window>,
    hal: Hal,
    surface: WinitSurface,
    renderer: Renderer,
    scene: vello::Scene,
    /// Compositor de la capa de overlay sobre contenido `gpu_paint` (video).
    /// Sólo entra en juego cuando el árbol principal tiene painters gpu y hay
    /// un overlay activo; resuelve el z-order (menús por encima del video).
    overlay_compositor: llimphi_hal::OverlayCompositor,
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
    /// Nodo hovereado **persistente** entre frames, actualizado SÓLO en
    /// `CursorMoved`. Es contra esto que se detecta el `on_pointer_enter`
    /// (no contra `last_render.hover_idx`, que el render recomputa cada
    /// cuadro): en una app que re-renderiza sin parar (visores `paint_with`)
    /// el render "se comería" la transición de hover antes de que el handler
    /// del mouse la detecte, y el hover-switch de menús no funcionaría.
    hovered: Option<usize>,
    /// Drag activo. Mantiene su propio handler clonado del MountedNode
    /// — así el drag sobrevive aunque el cache se invalide entre
    /// eventos.
    drag: Option<DragState<A::Msg>>,
    /// Foco actual (id de un nodo `View::focusable`). El runtime es la
    /// única fuente de verdad: lo mueve con Tab/Shift+Tab y click-to-focus
    /// y lo notifica vía `App::on_focus`. `None` = nada enfocado.
    focused: Option<u64>,
    /// Último título dinámico aplicado a la ventana (ver [`App::window_title`]).
    /// Evita llamar `set_title` en cada frame cuando no cambió.
    last_title: Option<String>,
    /// Registro de animaciones implícitas (`View::animated`), vivo entre
    /// frames. En cada redraw reconcilia el árbol y, si alguna sigue en curso,
    /// el runtime pide otro frame (ticker autodetenido). Ver
    /// [`llimphi_compositor::AnimRegistry`].
    anim_registry: llimphi_compositor::AnimRegistry,
    /// Último tap (press izquierdo) sobre un nodo con `on_double_tap`: instante
    /// + posición. El próximo press que caiga cerca y a tiempo dispara el
    /// doble-tap. `None` cuando no hay un primer tap pendiente.
    last_tap: Option<(std::time::Instant, PhysicalPosition<f64>)>,
    /// Long-press armado (ver [`PendingLongPress`]). El runtime lo vence por
    /// tiempo en `about_to_wait` y lo cancela en movimiento/release.
    pending_long_press: Option<PendingLongPress<A::Msg>>,
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

/// Un handler de gesto "tipo click" (doble-tap / long-press) ya **resuelto**
/// contra el nodo: o un `Msg` directo, o un handler posicional con la posición
/// local `(lx, ly, w, h)` ya calculada. Se captura en el press para poder
/// dispararlo más tarde (long-press, que vence por tiempo) sin volver a tocar
/// el árbol.
enum GestureResolved<Msg> {
    Direct(Msg),
    At(ClickAtFn<Msg>, f32, f32, f32, f32),
}

impl<Msg: Clone> GestureResolved<Msg> {
    /// Materializa el `Msg` (clona el directo o invoca el handler posicional).
    fn invoke(&self) -> Option<Msg> {
        match self {
            GestureResolved::Direct(m) => Some(m.clone()),
            GestureResolved::At(h, lx, ly, w, ht) => h(*lx, *ly, *w, *ht),
        }
    }
}

/// Long-press **armado**: el press cayó sobre un nodo con `on_long_press`. El
/// runtime lo dispara cuando pasa `deadline` (en `about_to_wait`), salvo que
/// antes el cursor se aleje de `origin` (pasó a drag) o se suelte el botón —
/// en ambos casos se cancela. Es la parte de "arena" del gesto: el árbitro es
/// el tiempo + el movimiento.
struct PendingLongPress<Msg> {
    deadline: std::time::Instant,
    origin: PhysicalPosition<f64>,
    handler: GestureResolved<Msg>,
}

/// Umbral de duración para que un press se convierta en long-press.
const LONG_PRESS_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
/// Si el cursor se aleja más que esto (px físicos) del origen del press, deja
/// de ser long-press (pasó a drag/scroll) y se cancela.
const LONG_PRESS_MOVE_CANCEL: f64 = 8.0;
/// Ventana temporal máxima entre los dos taps de un doble-tap.
const DOUBLE_TAP_WINDOW: std::time::Duration = std::time::Duration::from_millis(400);
/// Distancia máxima (px físicos) entre los dos taps de un doble-tap.
const DOUBLE_TAP_DIST: f64 = 16.0;

/// ¿El press actual (`now`, `pos`) completa un doble-tap con el tap previo
/// `last`? Verdadero si hubo un tap previo dentro de [`DOUBLE_TAP_WINDOW`] y a
/// menos de [`DOUBLE_TAP_DIST`]. Función pura (testeable sin event loop).
fn double_tap_qualifies(
    last: Option<(std::time::Instant, PhysicalPosition<f64>)>,
    now: std::time::Instant,
    pos: PhysicalPosition<f64>,
) -> bool {
    last.is_some_and(|(t, p)| {
        now.duration_since(t) <= DOUBLE_TAP_WINDOW
            && ((p.x - pos.x).powi(2) + (p.y - pos.y).powi(2)).sqrt() <= DOUBLE_TAP_DIST
    })
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
        secondaries: Vec::new(),
    };
    event_loop.run_app(&mut runtime).expect("run app");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn double_tap_ventana_y_distancia() {
        let t0 = Instant::now();
        let p = PhysicalPosition::new(100.0, 100.0);
        // Sin tap previo → nunca califica.
        assert!(!double_tap_qualifies(None, t0, p));
        // Segundo tap a tiempo (100 ms < 400) y cerca (3px < 16) → califica.
        let near = PhysicalPosition::new(102.0, 102.0);
        assert!(double_tap_qualifies(
            Some((t0, p)),
            t0 + Duration::from_millis(100),
            near
        ));
        // A tiempo pero lejos (>16px) → no.
        let far = PhysicalPosition::new(140.0, 100.0);
        assert!(!double_tap_qualifies(
            Some((t0, p)),
            t0 + Duration::from_millis(100),
            far
        ));
        // Cerca pero tarde (>400 ms) → no.
        assert!(!double_tap_qualifies(
            Some((t0, p)),
            t0 + Duration::from_millis(600),
            near
        ));
    }
}
