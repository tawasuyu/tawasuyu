//! llimphi-compositor — el núcleo declarativo de Llimphi, sin winit.
//!
//! Aquí vive el árbol de vista `View<Msg>` (DSL declarativo), su instalación
//! sobre taffy (`mount`), el pintado a `vello::Scene` (`paint`/`paint_gpu`) y
//! el hit-test. Nada de esto necesita una ventana ni `llimphi-hal`: la
//! composición `view → layout → scene` es pura y reutilizable.
//!
//! El runtime que la maneja vive aparte:
//! - `llimphi-ui` la corre sobre winit (`run<A: App>()`).
//! - a futuro, un runtime sobre el framebuffer del kernel `wawa` puede
//!   reusar exactamente este compositor sin arrastrar winit.
//!
//! `wgpu` entra sólo por la firma de [`GpuPaintFn`] (tipos de Device/Queue/
//! Encoder/TextureView); `wgpu` no depende de winit, así que el compositor
//! sigue libre de windowing.

use std::collections::HashMap;
use std::sync::Arc;

use llimphi_layout::taffy::NodeId;
use llimphi_layout::{ComputedLayout, LayoutTree, Style};
use vello::kurbo::{Affine, Point, Rect as KurboRect, RoundedRect, RoundedRectRadii, Stroke};
use vello::peniko::{Color, Fill, Gradient, Image, Mix};

mod anim;
mod render;
mod view;
pub use anim::{ease_out_cubic, Anim, AnimRegistry};
pub use render::*;

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
    /// Peso de fuente CSS: 400 = normal, 700 = bold. parley elige la
    /// variante más cercana de la familia activa (o la sintetiza). Se usa
    /// tanto al **medir** como al **pintar**, así medida y dibujo coinciden.
    /// Default 400.
    pub weight: f32,
    /// Límite de líneas (CSS `-webkit-line-clamp` / Flutter `maxLines`). `None`
    /// = sin límite (envuelve libre). Cuando el texto excede, se trunca: con
    /// [`Self::ellipsis`] la última línea termina en `…`, sin él se corta seco.
    /// Afecta medida (taffy reserva el alto de N líneas) y pintado.
    pub max_lines: Option<usize>,
    /// Si `true` y `max_lines` trunca, la última línea visible termina en `…`.
    /// Sin efecto si `max_lines` es `None`. Default false.
    pub ellipsis: bool,
    /// CSS-style font-family string (acepta lista con fallbacks). `None`
    /// = la fuente default de parley.
    pub font_family: Option<String>,
    /// Múltiplo de interlínea (`line-height` / `font-size`). 1.2 es el
    /// default que usaban todos los callers; puriy lo sobreescribe con el
    /// valor computado de CSS. Se usa tanto al **medir** (para que taffy
    /// reserve el alto correcto) como al **pintar**, así medida y dibujo
    /// coinciden.
    pub line_height: f32,
    /// Colores por rango de **bytes** sobre `content`, para texto multicolor
    /// (syntax highlighting) en una sola pasada de shaping. `None` = color
    /// uniforme (`color`). Cuando es `Some`, el runtime usa
    /// `Typesetter::layout_runs` + `draw_layout_runs`, y `color` actúa como
    /// color por defecto de lo no cubierto por ningún run.
    pub runs: Option<Vec<(usize, usize, Color)>>,
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

/// Handler de rueda **local a un nodo**. Recibe el delta `(dx, dy)` en
/// líneas lógicas (misma normalización que `App::on_wheel`: `dy` positivo
/// = scroll hacia abajo). El runtime lo invoca cuando la rueda gira con el
/// cursor sobre este nodo, ANTES de caer al `App::on_wheel` global: si el
/// handler devuelve `Some(Msg)`, el evento se consume acá. Permite áreas
/// de scroll autocontenidas (el widget `scroll` lo usa) sin que cada app
/// rutee la rueda a mano por su `Model`. Devolver `None` deja pasar el
/// evento al `on_wheel` global.
pub type ScrollFn<Msg> = Arc<dyn Fn(f32, f32) -> Option<Msg> + Send + Sync>;

/// Variante de [`DragFn`] que **conoce la posición inicial del press**
/// relativa al rect del nodo. Útil cuando el caller necesita identificar
/// qué entidad (Concepto, lemming, etc.) bajo el cursor agarró el drag.
/// Recibe `(phase, dx, dy, initial_lx, initial_ly)`.
pub type DragAtFn<Msg> = Arc<dyn Fn(DragPhase, f32, f32, f32, f32) -> Option<Msg> + Send + Sync>;

/// Fase de un **gesto continuo** (pinch-to-zoom de momento; rotación a futuro).
/// El runtime emite `Begin` al iniciar el gesto, `Update` por cada cambio
/// incremental y `End` al terminar. El camino de Ctrl+rueda (universal, sin
/// trackpad) emite un único `Update` por click de rueda — no hay un "inicio"
/// ni "fin" naturales, así que el handler debe tolerar `Update`s sueltos sin
/// `Begin` previo (es lo común en desktop). El camino de trackpad
/// (`PinchGesture`, sólo macOS/iOS) sí entrega `Begin`/`Update*`/`End`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GesturePhase {
    Begin,
    Update,
    End,
}

/// Handler de gesto de **escala** (pinch-to-zoom). Recibe `(phase, factor,
/// focal_x, focal_y)`:
/// - `factor`: cambio de escala **incremental y multiplicativo** desde el
///   evento anterior — `1.0` = sin cambio, `>1.0` agranda (zoom in), `<1.0`
///   achica (zoom out). El caller acumula con `mi_zoom *= factor` y, si
///   quiere, lo clampa a su rango. En `Begin`/`End` el factor es `1.0`.
/// - `focal_x`/`focal_y`: punto focal del gesto **relativo a la esquina
///   superior-izquierda del rect del nodo** (mismo espacio que los handlers
///   `*_at`). Es el punto que debe quedar fijo bajo el cursor al hacer zoom —
///   el caller lo usa para zoomear "hacia el cursor" en vez de hacia el
///   centro. En Ctrl+rueda es la posición del cursor; en trackpad, idem.
///
/// Devolver `Some(Msg)` dispara una transición; `None` ignora el evento. El
/// runtime lo resuelve con [`hit_test_scale`]: el nodo más al frente bajo el
/// cursor que declare un `on_scale` consume el gesto. Es la base del zoom de
/// los canvases (pineal/cosmos/nakui).
pub type ScaleFn<Msg> = Arc<dyn Fn(GesturePhase, f32, f32, f32) -> Option<Msg> + Send + Sync>;

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
            &wgpu::Device,
            &wgpu::Queue,
            &mut wgpu::CommandEncoder,
            &wgpu::TextureView,
            PaintRect,
            (u32, u32),
        ) + Send
        + Sync,
>;

/// Sombra proyectada detrás del rect del nodo (drop shadow), rasterizada
/// con el `draw_blurred_rounded_rect` nativo de vello. Se pinta **antes**
/// del relleno, así el fill (si es opaco) tapa la parte solapada y la
/// sombra sólo asoma por el desenfoque + el offset. El radio sigue al del
/// nodo (más `spread`).
#[derive(Clone, Copy, Debug)]
pub struct Shadow {
    pub color: Color,
    /// Desviación estándar del gaussiano (qué tan difusa). En px.
    pub blur: f64,
    /// Desplazamiento de la sombra respecto del nodo.
    pub dx: f64,
    pub dy: f64,
    /// Cuánto crece (px) el rect de la sombra respecto del nodo.
    pub spread: f64,
}

impl Shadow {
    /// Sombra con color + blur explícitos, sin offset ni spread.
    pub fn new(color: Color, blur: f64) -> Self {
        Self { color, blur, dx: 0.0, dy: 0.0, spread: 0.0 }
    }

    /// Elevación suave y tasteful: negro translúcido, leve caída hacia
    /// abajo. El default razonable para cards/menús/modales.
    pub fn soft(alpha: u8, blur: f64) -> Self {
        Self {
            color: Color::from_rgba8(0, 0, 0, alpha),
            blur,
            dx: 0.0,
            dy: blur * 0.4,
            spread: 0.0,
        }
    }

    pub fn offset(mut self, dx: f64, dy: f64) -> Self {
        self.dx = dx;
        self.dy = dy;
        self
    }

    pub fn spread(mut self, spread: f64) -> Self {
        self.spread = spread;
        self
    }
}

/// Borde (stroke) pintado sobre el contorno redondeado del nodo, **inset**
/// hacia adentro media línea para que el grosor quede dentro del rect
/// (convención CSS `box-sizing: border-box`). Se pinta después del relleno.
#[derive(Clone, Copy, Debug)]
pub struct Border {
    pub width: f64,
    pub color: Color,
}

impl Border {
    pub fn new(width: f64, color: Color) -> Self {
        Self { width, color }
    }
}

/// Nodo de la vista declarativa. Estilo de layout (taffy) + relleno opcional
/// (vello) + texto opcional (skrifa+vello) + Msg al click opcional + hijos.
pub struct View<Msg> {
    pub style: Style,
    pub fill: Option<Color>,
    /// Relleno cuando el cursor está sobre este nodo. Sin valor (`None`)
    /// = no se reacciona al hover.
    pub hover_fill: Option<Color>,
    pub radius: f64,
    /// Radio **por esquina** (top-left, top-right, bottom-right, bottom-left),
    /// que sobreescribe a `radius` cuando está presente. Permite cards con
    /// sólo las esquinas de arriba redondeadas, pestañas, bocadillos de chat,
    /// etc. (CSS `border-radius` con 4 valores). `None` = usar el `radius`
    /// uniforme. Ver [`View::radius_corners`]. La **sombra** sigue usando un
    /// radio escalar (el blur nativo de vello no acepta radios por esquina);
    /// el **borde** sí respeta las cuatro esquinas.
    pub corner_radii: Option<RoundedRectRadii>,
    /// Sombra proyectada detrás del nodo (drop shadow). `None` = sin sombra
    /// (la mayoría de nodos). Ver [`Shadow`].
    pub shadow: Option<Shadow>,
    /// Relleno con **gradiente**, autoreado en el cuadrado unidad `[0,1]²` y
    /// mapeado al rect del nodo. Gana sobre `fill` como base; `hover_fill`
    /// (un color) lo sigue overrideando en hover. Ver [`View::fill_gradient`].
    pub fill_gradient: Option<Gradient>,
    /// Borde (stroke) sobre el contorno redondeado. Ver [`Border`].
    pub border: Option<Border>,
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
    /// Handler de rueda local. Si está presente y el cursor cae sobre este
    /// nodo, el runtime lo invoca antes del `App::on_wheel` global; un
    /// `Some(Msg)` consume el evento. Base de las áreas de scroll
    /// autocontenidas. Ver [`ScrollFn`].
    pub on_scroll: Option<ScrollFn<Msg>>,
    /// Handler de gesto de **escala** (pinch-to-zoom). Si está presente y el
    /// gesto cae sobre este nodo (Ctrl+rueda en desktop, pinch de trackpad en
    /// macOS), el runtime lo invoca con el factor incremental + el punto focal
    /// local. Base del zoom de canvases. Ver [`ScaleFn`] y [`View::on_scale`].
    pub on_scale: Option<ScaleFn<Msg>>,
    /// Marca este nodo como **enfocable** con el id opaco `u64`. El runtime
    /// mantiene el foco (uno por ventana) y lo mueve con Tab/Shift+Tab en
    /// orden de árbol (pre-orden) y al clickear un nodo enfocable; notifica
    /// a la app vía `App::on_focus` para que pinte el ring y rutee el
    /// teclado. El id lo elige el caller (índice de campo, hash, etc.).
    pub focusable: Option<u64>,
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
    /// Animación **implícita** de las props de paint (fill/radius): cuando el
    /// valor cambia entre frames, el runtime interpola en vez de saltar. `None`
    /// = sin animación (la abrumadora mayoría). La `key` debe ser estable entre
    /// rebuilds. Ver [`Anim`] y [`View::animated`]. Lo consume el runtime vía
    /// [`AnimRegistry::reconcile`] (DESPUÉS de layout, ANTES de paint).
    pub anim: Option<Anim>,
    /// Transformación afín 2D aplicada a este nodo y todo su subtree
    /// **alrededor del centro de su propio rect** (convención CSS
    /// `transform-origin: 50% 50%`). El runtime resuelve el centro en
    /// `paint` (sólo entonces conoce el layout computado) y compone
    /// `T(centro) · transform · T(-centro)` sobre la transformación
    /// acumulada del padre, así nodos anidados transforman en el espacio
    /// ya transformado de su ancestro — igual que CSS. `None` = identidad
    /// (la abrumadora mayoría de nodos). Pensado para `transform`/
    /// `@keyframes` CSS de puriy (rotate/scale/translate). El hit-test
    /// **respeta** el afín (un nodo transformado recibe clicks donde se ve
    /// pintado). Limitación restante: los `painter`/`runs` custom no heredan
    /// el afín, y la posición local que reciben los handlers `*_at` se
    /// reporta en espacio de pantalla, no en el espacio local del nodo.
    pub transform: Option<Affine>,
    /// Texto de **tooltip**: si está, el runtime/cliente puede mostrar un
    /// rótulo flotante cuando el cursor se posa sobre este nodo. Llimphi sólo
    /// transporta el dato hasta el [`MountedNode`]; *quién* lo pinta (un overlay
    /// del runtime, una surface popup del cliente) lo decide el consumidor. El
    /// hit-test de hover ya localiza el nodo bajo el cursor. `None` = sin tip.
    pub tooltip: Option<String>,
    /// Forma del puntero del mouse mientras está sobre este nodo (o un
    /// descendiente sin cursor propio — se hereda del ancestro más cercano que
    /// lo declare). El runtime lo resuelve en el hit-test de hover y lo aplica a
    /// la ventana. `None` = hereda (default flecha en la raíz). Ver [`Cursor`] y
    /// [`View::cursor`]. Llimphi-native (sin winit); el runtime lo mapea.
    pub cursor: Option<Cursor>,
    pub children: Vec<View<Msg>>,
}

/// Versión "instalada" del árbol: cada nodo tiene su NodeId de taffy, color
/// y handler. Se mantiene en orden de inserción (recorrido pre-orden), así
/// el hit-test puede iterar al revés para honrar el orden de pintado.
///
/// `pub` (con campos `pub`) porque el runtime (llimphi-ui) lee el árbol
/// montado para hit-test y para la pasada GPU directa, pero vive en otro
/// crate. No se construye fuera de [`mount`].
pub struct Mounted<Msg> {
    pub root: NodeId,
    pub nodes: Vec<MountedNode<Msg>>,
    /// Contenido de texto por nodo-hoja, para que el runtime lo mida con
    /// parley durante `compute_with_measure` y taffy reserve el alto real
    /// del texto envuelto (varias líneas) en vez de una sola. Sin esto un
    /// párrafo que envuelve a N líneas se aplastaría en la altura de una
    /// (el bug clásico de "textos aplastados"). Sólo se pueblan hojas con
    /// texto uniforme (sin `runs` multicolor, que el caller dimensiona).
    pub text_measures: HashMap<NodeId, TextMeasure>,
}

/// Datos de un nodo-hoja de texto necesarios para medirlo (shaping +
/// line-break) sin volver a tocar el `View`. Lo consume el runtime en la
/// función de medición que le pasa a [`LayoutTree::compute_with_measure`].
#[derive(Clone)]
pub struct TextMeasure {
    pub content: String,
    pub size_px: f32,
    pub alignment: llimphi_text::Alignment,
    pub italic: bool,
    pub font_family: Option<String>,
    pub line_height: f32,
    pub weight: f32,
    pub max_lines: Option<usize>,
    pub ellipsis: bool,
}

/// Forma del puntero del mouse. Subconjunto práctico, llimphi-native (el
/// compositor no depende de winit). El runtime (`llimphi-ui`) mapea 1:1 a
/// `winit::window::CursorIcon`. Nombres alineados con CSS/winit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    /// Flecha por defecto.
    Default,
    /// Manito — sobre algo clickeable (links, botones).
    Pointer,
    /// I-beam — sobre texto editable/seleccionable.
    Text,
    /// Cruz — selección precisa (canvas, picker de color).
    Crosshair,
    /// Cuatro flechas — mover un objeto.
    Move,
    /// Mano abierta — agarrable (antes de arrastrar).
    Grab,
    /// Mano cerrada — arrastrando.
    Grabbing,
    /// Prohibido — drop no permitido / acción inválida.
    NotAllowed,
    /// Reloj/espera — operación bloqueante.
    Wait,
    /// Progreso — ocupado pero la UI responde.
    Progress,
    /// Interrogación — ayuda contextual.
    Help,
    /// Resize horizontal (columna / divisor vertical).
    ColResize,
    /// Resize vertical (fila / divisor horizontal).
    RowResize,
    /// Resize este-oeste.
    EwResize,
    /// Resize norte-sur.
    NsResize,
    /// Resize diagonal ↗↙.
    NeswResize,
    /// Resize diagonal ↖↘.
    NwseResize,
    /// Lupa + (zoom in).
    ZoomIn,
    /// Lupa − (zoom out).
    ZoomOut,
}

pub struct MountedNode<Msg> {
    pub id: NodeId,
    pub fill: Option<Color>,
    pub hover_fill: Option<Color>,
    pub radius: f64,
    pub corner_radii: Option<RoundedRectRadii>,
    pub shadow: Option<Shadow>,
    pub fill_gradient: Option<Gradient>,
    pub border: Option<Border>,
    pub text: Option<TextSpec>,
    pub image: Option<Image>,
    pub painter: Option<PaintFn>,
    pub gpu_painter: Option<GpuPaintFn>,
    pub on_click: Option<Msg>,
    pub on_click_at: Option<ClickAtFn<Msg>>,
    pub on_right_click: Option<Msg>,
    pub on_right_click_at: Option<ClickAtFn<Msg>>,
    pub on_middle_click: Option<Msg>,
    pub drag: Option<DragFn<Msg>>,
    pub drag_at: Option<DragAtFn<Msg>>,
    pub drag_payload: Option<u64>,
    pub on_drop: Option<DropFn<Msg>>,
    pub drop_hover_fill: Option<Color>,
    pub clip: bool,
    pub on_pointer_enter: Option<Msg>,
    pub on_pointer_leave: Option<Msg>,
    pub on_scroll: Option<ScrollFn<Msg>>,
    /// Handler de gesto de escala (pinch-to-zoom) de este nodo. Ver
    /// [`View::on_scale`] y [`ScaleFn`].
    pub on_scale: Option<ScaleFn<Msg>>,
    pub focusable: Option<u64>,
    pub alpha: Option<f32>,
    pub anim: Option<Anim>,
    /// Transformación afín 2D del nodo (alrededor del centro de su rect).
    /// Ver [`View::transform`]. `paint` la compone con la del padre.
    pub transform: Option<Affine>,
    /// Texto de tooltip de este nodo (ver [`View::tooltip`]). El consumidor lo
    /// lee tras un hit-test de hover para pintar el rótulo flotante.
    pub tooltip: Option<String>,
    /// Forma del puntero sobre este nodo (ver [`View::cursor`]). El runtime la
    /// resuelve heredando del ancestro más cercano que la declare.
    pub cursor: Option<Cursor>,
    /// Índice (exclusivo) del fin del subárbol en `Mounted::nodes`. Los
    /// descendientes ocupan `[idx + 1, subtree_end)`. Hace de "barrera" en
    /// paint/hit_test para `pop_layer` y para saltar subárboles enteros.
    pub subtree_end: usize,
}
