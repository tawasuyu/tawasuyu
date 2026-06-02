//! Style engine — parser CSS minimal sobre `cssparser`.
//!
//! Para Fase 2 soportamos sólo:
//! - selectores type (`p`, `div`, `h1`) y universal (`*`)
//! - propiedades `color`, `background-color`, `display`, `font-size`,
//!   `margin`, `padding`
//! - inline `style="..."` en cada elemento
//!
//! No hay cascada con especificidad real ni `!important`. Stylo entero
//! entra en Fase 3 cuando el chrome Llimphi consuma estilos jerárquicos
//! complejos. Por ahora, una pasada de regla→nodo con override por
//! inline style alcanza para renderizar páginas simples (example.com,
//! landing del propio repo).

use std::collections::HashMap;

use markup5ever_rcdom::Handle;

use crate::boxes::{Color, Display};
use crate::dom::{self, DomTree};

/// Estilo computado por nodo. Defaults razonables — un nodo sin reglas
/// que matchen igual produce un box renderizable (texto negro sobre
/// transparente).
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: Color,
    pub background: Option<Color>,
    pub font_size: f32,
    pub font_weight: u16,
    /// CSS `font-style`: normal vs italic/oblique. Heredable.
    pub font_style: FontStyle,
    /// CSS `font-family` como string crudo (acepta lista con fallbacks).
    /// `None` = sin override; usa la fuente default del runtime.
    pub font_family: Option<String>,
    pub margin: Sides<f32>,
    pub padding: Sides<f32>,
    /// Ancho explícito. `Auto` = el default block-fills-parent.
    pub width: LengthVal,
    /// Tope superior — útil para containers narrow ("max-width:800px").
    pub max_width: LengthVal,
    /// Alineación horizontal del texto dentro del box.
    pub text_align: TextAlign,
    /// Altura de línea como multiplicador del font-size. `None` =
    /// default razonable (1.4) en el caller.
    pub line_height: Option<f32>,
    /// Ancho del border en px por lado. `0` = ese lado sin border.
    /// El shorthand `border: 2px solid red` setea los 4 lados; las
    /// propiedades `border-top/right/bottom/left[-width]` los setean
    /// individualmente.
    pub border_widths: Sides<f32>,
    /// Color del border por lado. `None` = ese lado no se dibuja aunque
    /// `width > 0`. Mismo modelo que `border_widths`.
    pub border_colors: Sides<Option<Color>>,
    /// Radio del corner-radius en px por esquina (0 = esquina viva).
    /// El shorthand `border-radius: 8px` setea las 4; las propiedades
    /// `border-top-left-radius` etc. las setean individualmente.
    pub border_radii: Corners<f32>,
    /// `box-shadow` simplificado. `None` = sin sombra.
    pub box_shadow: Option<BoxShadow>,
    /// `z-index` aplicado al stacking order entre siblings positioned
    /// (absolute/fixed). Para nodos en flow normal (static), CSS spec
    /// dice que z-index no aplica y se ignora. `0` = default.
    pub z_index: i32,
    /// `content: ...` para pseudo-elementos `::before`/`::after`.
    /// `None` = no hay content (pseudo-element NO se materializa). Sólo
    /// se consulta en estilos computados para pseudo-elements; en el
    /// estilo del elemento real, content es no-op (matchea spec).
    ///
    /// Es un `Vec` porque `content:` admite concatenación de items:
    /// `content: "Sección " counter(sec) ": " attr(data-title)`.
    pub content: Option<Vec<ContentItem>>,
    /// `counter-reset: name [val] name2 [val2]...`. Cada par crea o
    /// resetea un contador en el scope del nodo. Se aplica antes que
    /// `counter-increment` al entrar al nodo en el DFS.
    pub counter_reset: Vec<(String, i32)>,
    /// `counter-increment: name [delta] name2 [delta2]...`. Cada par
    /// incrementa el contador correspondiente; si no existía, lo crea
    /// implícitamente (CSS spec: el reset implícito es 0).
    pub counter_increment: Vec<(String, i32)>,
    /// `text-decoration-line` reducido al subset que pintamos.
    /// `None` = sin decoración (default HTML, salvo `<a>`/`<u>`/`<s>`).
    pub text_decoration: TextDecorationLine,
    /// Marker que `<li>` pinta delante del contenido. Hereda (CSS spec).
    /// Default `Disc` (CSS default); UA stylesheet override en `<ol>` y
    /// `<ul>` por consistencia.
    pub list_style_type: ListStyleType,
    /// Solo relevante si `display` es `Flex`/`InlineFlex`. Default Row.
    pub flex_direction: FlexDirection,
    /// Distribución horizontal (eje principal) de los hijos flex.
    pub justify_content: JustifyContent,
    /// Alineación vertical (eje cruzado) de los hijos flex.
    pub align_items: AlignItems,
    /// `nowrap` por default (CSS spec).
    pub flex_wrap: FlexWrap,
    /// Separación entre items en el eje principal (px). En CSS estándar,
    /// `column-gap` para row-direction, `row-gap` para column-direction.
    /// Acá los separamos para mapear directo a taffy.
    pub gap_row: f32,
    pub gap_column: f32,
    /// Cómo se cuentan padding/border dentro del width. Default
    /// `ContentBox` (CSS spec); los resets modernos lo fuerzan a
    /// BorderBox.
    pub box_sizing: BoxSizing,
    /// Ancho/alto mínimos.
    pub min_width: LengthVal,
    pub min_height: LengthVal,
    /// Alto máximo (max-width ya existe). `Auto` = sin tope.
    pub max_height: LengthVal,
    /// Overflow del contenido. Default `Visible`.
    pub overflow: Overflow,
    /// Colapsado y wrap del texto.
    pub white_space: WhiteSpace,
    /// Transformación de texto pre-render.
    pub text_transform: TextTransform,
    /// 0..1. Multiplica alpha del background/border al pintar.
    /// `text` queda sin tocar (el spec exige multiplicar todo el
    /// subárbol, pero acá pragmaticamente sólo afecta el propio nodo —
    /// matchea el uso real donde opacity se aplica a overlays).
    pub opacity: f32,
    /// Item-side de flex.
    pub align_self: AlignSelf,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    /// `Auto` = el width del item; `Px/Pct` = base explícita.
    pub flex_basis: LengthVal,
    /// Outline (fuera del border, sin afectar layout).
    pub outline: Outline,
    /// `background-image: linear-gradient(...)`. Cuando es Some, el
    /// chrome lo pinta detrás (o encima del background sólido).
    pub background_gradient: Option<LinearGradient>,
    /// `background-image: url(...)` — URL sin resolver (puede ser
    /// relativa). El engine la resuelve y descarga en `build_node`; el
    /// chrome consume el resultado vía `BoxNode.background_image`.
    pub background_image_url: Option<String>,
    /// CSS `position`. Default Static.
    pub position: Position,
    /// Insets (top/right/bottom/left). `Auto` por default.
    pub inset_top: LengthVal,
    pub inset_right: LengthVal,
    pub inset_bottom: LengthVal,
    pub inset_left: LengthVal,
    /// `vertical-align` para inline / inline-block / cells.
    pub vertical_align: VerticalAlign,
    /// `visibility: hidden` → ocupa espacio pero no se pinta.
    pub visibility: Visibility,
    /// `pointer-events: none` → ignora clics/hover.
    pub pointer_events: PointerEvents,
    /// Sangrado de primera línea de un bloque (en px).
    pub text_indent: f32,
    /// Espacio extra entre palabras (en px). Heredable.
    pub word_spacing: f32,
    /// Sombras del texto. Vacío = ninguna.
    pub text_shadows: Vec<TextShadow>,
    /// Cadena de transformaciones (translate/scale/rotate) aplicadas
    /// en orden. Vacío = identidad.
    pub transforms: Vec<Transform>,
    /// Para `display: grid` — pistas de columnas y filas.
    pub grid_template_columns: Vec<GridTrackSize>,
    pub grid_template_rows: Vec<GridTrackSize>,
    /// `animation: <name> <duration> ...` colapsado en una binding.
    /// `None` = sin animación. **Sólo parseado**: no hay runtime de tween
    /// todavía, así que esto no anima nada (ver Fase B4). El runtime
    /// futuro cruzaría `name` contra [`StyleEngine::keyframes`].
    pub animation: Option<AnimationBinding>,
    /// `transition: <prop> <duration> ...`. Lista separada por coma →
    /// varios bindings. Vacío = sin transición. **Sólo parseado** — sin
    /// runtime de tween no dispara nada (ver Fase B4).
    pub transitions: Vec<TransitionBinding>,
}

/// Estilo del marker de `<li>`. Reducido al subset que el chrome puede
/// pintar como texto plano (sin imágenes ni cuadritos pintados a mano).
/// `Decimal`/`*Alpha`/`*Roman` requieren conocer la posición del `<li>`
/// entre sus hermanos — `boxes::build_node` la calcula y la sustituye.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStyleType {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

/// Línea decorativa que el chrome dibuja sobre/atravesando/debajo del
/// texto del nodo. CSS spec dice que la propiedad NO se hereda — los
/// descendientes inline heredan la decoración por propagación visual,
/// no computacional. Acá la tratamos como heredable porque dibujamos
/// por leaf de texto: sin propagar, `<a>foo <b>bar</b></a>` rendearía
/// `foo` subrayado y `bar` sin subrayar. Override explícito a `None`
/// la suprime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDecorationLine {
    None,
    Underline,
    LineThrough,
    Overline,
}

/// CSS `font-style`. Heredable. `Oblique` lo tratamos igual que
/// `Italic` (parley sintetiza si la fuente no tiene oblique nativo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}

/// Sombra rectangular detrás del box. `blur_px` y `spread_px` se
/// combinan en una expansión efectiva del rect — gaussian blur real
/// queda para cuando el render-pipeline soporte multi-pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_px: f32,
    pub spread_px: f32,
    pub color: Color,
}

/// Valor longitud de CSS reducido al subset que soportamos: `auto`,
/// `Npx`, `N%`. `em`/`rem` se resuelven a px en parse time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthVal {
    Auto,
    Px(f32),
    Pct(f32),
}

/// 4 valores por lado (top/right/bottom/left). Lo usan `margin` y
/// `padding` para no perder información del shorthand CSS — un
/// `padding: 10px 20px` se queda con `top/bottom=10, right/left=20`
/// en vez de colapsarse a un único `f32`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sides<T: Copy> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

/// Eje principal de un contenedor `display: flex`. Default `Row`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

/// Distribución del espacio libre a lo largo del eje principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustifyContent {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// Alineación de los items en el eje cruzado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    Start,
    Center,
    End,
    Stretch,
    Baseline,
}

/// ¿Hijos en una sola línea o wrap a múltiples?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    NoWrap,
    Wrap,
    WrapReverse,
}

/// Modelo de caja CSS: cómo se cuentan `padding` y `border` dentro del
/// `width`/`height`. CSS default `ContentBox` (width = sólo contenido);
/// la mayoría de los resets modernos fuerzan `BorderBox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

/// `align-items` por item — pisa el del contenedor para ese hijo.
/// `Auto` significa heredar del padre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignSelf {
    Auto,
    Start,
    Center,
    End,
    Stretch,
    Baseline,
}

/// Comportamiento de overflow del contenido del box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
}

/// `white-space` controla colapsado de espacios y wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace {
    /// Default: runs internos colapsan a un solo espacio, wrap libre.
    Normal,
    /// Sin wrap; runs internos colapsan.
    NoWrap,
    /// Preserva todo (espacios, tabs, newlines).
    Pre,
    /// Preserva espacios/newlines; wrap permitido en cualquier espacio.
    PreWrap,
    /// Colapsa runs internos a uno, pero preserva newlines.
    PreLine,
}

/// `text-transform` aplica una transformación al texto antes de
/// pintarlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

/// `outline` se pinta fuera del border (sin ocupar layout). Útil para
/// focus rings y debug. `style_active=false` (CSS `none`/`hidden`) lo
/// desactiva aunque haya width/color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outline {
    pub width: f32,
    pub color: Option<Color>,
    pub style_active: bool,
    /// Distancia del border al outline. Default 0.
    pub offset: f32,
}

impl Default for Outline {
    fn default() -> Self {
        Self { width: 0.0, color: None, style_active: true, offset: 0.0 }
    }
}

/// Un stop de `linear-gradient`. `pos` es la fracción (0..1) del eje;
/// si `None`, se distribuye automáticamente entre stops adyacentes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    pub pos: Option<f32>,
}

/// `background-image: linear-gradient(...)`. Subset: ángulo en grados
/// (0 = bottom→top, 90 = left→right), 2+ stops. Conic/radial quedan
/// para más adelante.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    /// Ángulo CSS en grados — 0 = up, 90 = right, 180 = down, 270 = left.
    pub angle_deg: f32,
    pub stops: Vec<GradientStop>,
}

/// CSS `position`. `Static` = el default (no position; los insets
/// se ignoran). `Fixed`/`Sticky` los fakeamos como Absolute/Relative en
/// el chrome — taffy 0.9 sólo expone esos dos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

/// CSS `vertical-align` para inline / inline-block. Mapea a alignment
/// del item en el contexto del padre. `Super`/`Sub` los aproximamos
/// como Top/Bottom respectivamente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
    Super,
    Sub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEvents {
    Auto,
    None,
}

/// Una sombra de texto. CSS permite varias separadas por coma.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_px: f32,
    pub color: Color,
}

/// Una transformación CSS individual. Las cadenas `transform: rotate(45deg)
/// scale(2) translate(10px, 20px)` se aplican en orden de izquierda a
/// derecha como matrices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transform {
    /// Pixeles X/Y.
    Translate(f32, f32),
    /// Factores X/Y (uno solo si CSS da un valor).
    Scale(f32, f32),
    /// Grados (sentido horario en pantalla = sentido CSS).
    Rotate(f32),
}

/// Tamaño de track para `display: grid`. `Fr(N)` = fracción del espacio
/// remanente (CSS unit `fr`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrackSize {
    Auto,
    Px(f32),
    Pct(f32),
    Fr(f32),
}

/// Función de easing de una `transition`/`animation`. El runtime de
/// tween (Fase B4+, todavía NO implementado) la usaría para mapear el
/// progreso lineal `t∈[0,1]` al progreso efectivo. Por ahora sólo se
/// parsea y se guarda en `ComputedStyle` — no anima nada.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EasingFunction {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// `step-start` ≡ `steps(1, start)`.
    StepStart,
    /// `step-end` ≡ `steps(1, end)`.
    StepEnd,
    /// `cubic-bezier(x1, y1, x2, y2)` — los dos puntos de control.
    CubicBezier(f32, f32, f32, f32),
    /// `steps(n, jump-term)`. `jump_start=true` ⇒ `steps(n, start)`
    /// (salto al inicio del intervalo); `false` ⇒ `steps(n, end)`.
    Steps(u32, bool),
}

impl Default for EasingFunction {
    fn default() -> Self {
        // CSS spec: el default de `transition-timing-function` y
        // `animation-timing-function` es `ease`.
        EasingFunction::Ease
    }
}

/// Número de iteraciones de una animación (`animation-iteration-count`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationIterations {
    Count(f32),
    Infinite,
}

/// `animation-direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationDirection {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

/// `animation-fill-mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationFillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

/// `animation-play-state`. `Paused` congela el progreso de la animación en
/// el frame actual (lo consume el runtime de tween en `anim.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationPlayState {
    Running,
    Paused,
}

/// `animation: <name> <duration> <timing> <delay> <iteration> <direction>
/// <fill> <play-state>` colapsado en una sola binding. Si el shorthand
/// lista varias animaciones separadas por coma nos quedamos con la primera.
/// El runtime de tween vive en `anim.rs` (rescatado del frente engine). Los
/// tokens se clasifican por forma, no por posición, así que el orden
/// laxo del wild (`animation: spin 2s linear infinite`) se tolera.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationBinding {
    pub name: String,
    /// Duración en segundos.
    pub duration_s: f32,
    pub timing: EasingFunction,
    /// Retardo en segundos.
    pub delay_s: f32,
    pub iterations: AnimationIterations,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

/// `transition: <property> <duration> <timing> <delay>`. Una lista
/// separada por coma produce varios bindings. `property` queda como
/// string cruda (`opacity`, `transform`, `all`...) — el matching contra
/// las propiedades animables real lo hará el runtime de tween (Fase B4+).
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionBinding {
    pub property: String,
    pub duration_s: f32,
    pub timing: EasingFunction,
    pub delay_s: f32,
}

/// Un paso de un `@keyframes`: el offset normalizado en el timeline
/// (`from` = 0.0, `to` = 1.0, `50%` = 0.5) + las declaraciones crudas
/// (`prop`, `value`) que aplican en ese punto. Guardamos los pares SIN
/// parsear porque el runtime de animación (Fase B4+) todavía no existe;
/// cuando llegue, los re-parseará con la maquinaria de `Decl` para
/// derivar el overlay interpolado entre pasos.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframeStep {
    pub offset: f32,
    pub declarations: Vec<(String, String)>,
}

/// Definición de un `@keyframes name { ... }`. Los pasos quedan ordenados
/// por `offset` ascendente.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Keyframes {
    pub steps: Vec<KeyframeStep>,
}

/// Viewport asumido por el parser para resolver unidades `vw`/`vh`/
/// `vmin`/`vmax` y para evaluar `@media` queries. Por ahora es
/// constante (1280×800 — desktop típico). Cuando puriy soporte resize
/// dinámico del viewport, pasará a ser un parámetro de `StyleEngine`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    /// Factor de escala (DPI lógico) — `window.devicePixelRatio`. 1.0 normal,
    /// 2.0 HiDPI/Retina. Lo consume `evaluate_media_query` para las features
    /// `min/max-resolution` (`Ndppx` / `Ndpi`). Default 1.0.
    pub dpr: f32,
}

pub const DEFAULT_VIEWPORT: Viewport = Viewport { width: 1280.0, height: 800.0, dpr: 1.0 };

thread_local! {
    /// Viewport activo para resolver unidades `vw`/`vh`/`vmin`/`vmax` durante
    /// el parseo de un documento. `Engine::load_html` lo instala con el
    /// viewport real (vía [`ViewportScope`]) antes de parsear hojas y construir
    /// el box tree — incluido el `style="…"` inline que se parsea en
    /// `boxes::build`. Fuera de ese scope (tests que llaman parsers sueltos)
    /// cae a [`DEFAULT_VIEWPORT`], preservando el comportamiento previo.
    static RESOLVE_VIEWPORT: std::cell::Cell<Viewport> = const { std::cell::Cell::new(DEFAULT_VIEWPORT) };
}

/// Guard RAII que instala `vp` como viewport de resolución de longitudes
/// mientras viva, y restaura el anterior al dropear. Reentrante (anida bien).
/// Lo usa `Engine::load_html` para que `50vw`/`100vh` resuelvan contra el
/// tamaño real de la ventana en vez del viewport por defecto.
pub struct ViewportScope(Viewport);

impl ViewportScope {
    pub fn new(vp: Viewport) -> Self {
        let prev = RESOLVE_VIEWPORT.with(|c| c.replace(vp));
        ViewportScope(prev)
    }
}

impl Drop for ViewportScope {
    fn drop(&mut self) {
        RESOLVE_VIEWPORT.with(|c| c.set(self.0));
    }
}

/// Viewport contra el que se resuelven las unidades viewport ahora mismo.
/// `DEFAULT_VIEWPORT` salvo dentro de un [`ViewportScope`] activo.
fn resolve_viewport() -> Viewport {
    RESOLVE_VIEWPORT.with(|c| c.get())
}

impl<T: Copy> Sides<T> {
    pub const fn all(v: T) -> Self {
        Self { top: v, right: v, bottom: v, left: v }
    }
}

impl Default for Sides<f32> {
    fn default() -> Self {
        Self::all(0.0)
    }
}

/// Valores por esquina (top-left, top-right, bottom-right, bottom-left)
/// — usado por `border-radius` per-corner. El shorthand `border-radius`
/// setea las 4; las longhand `border-{top|bottom}-{left|right}-radius`
/// las setean individualmente.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corners<T: Copy> {
    pub top_left: T,
    pub top_right: T,
    pub bottom_right: T,
    pub bottom_left: T,
}

impl<T: Copy> Corners<T> {
    pub const fn all(v: T) -> Self {
        Self { top_left: v, top_right: v, bottom_right: v, bottom_left: v }
    }
}

impl Default for Corners<f32> {
    fn default() -> Self {
        Self::all(0.0)
    }
}

/// Lado de un border (`border-top-width: 2px` → `Top`, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderEdge {
    Top,
    Right,
    Bottom,
    Left,
}

/// Esquina de un border-radius (`border-top-left-radius` → `TopLeft`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderCorner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

fn set_side<T: Copy>(sides: &mut Sides<T>, edge: BorderEdge, v: T) {
    match edge {
        BorderEdge::Top => sides.top = v,
        BorderEdge::Right => sides.right = v,
        BorderEdge::Bottom => sides.bottom = v,
        BorderEdge::Left => sides.left = v,
    }
}

fn set_side_f32(sides: &mut Sides<f32>, edge: BorderEdge, v: f32) {
    set_side(sides, edge, v)
}

fn set_corner(corners: &mut Corners<f32>, corner: BorderCorner, v: f32) {
    match corner {
        BorderCorner::TopLeft => corners.top_left = v,
        BorderCorner::TopRight => corners.top_right = v,
        BorderCorner::BottomRight => corners.bottom_right = v,
        BorderCorner::BottomLeft => corners.bottom_left = v,
    }
}

/// Alineación horizontal del contenido inline dentro de un bloque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::Inline,
            color: Color::BLACK,
            background: None,
            font_size: 16.0,
            font_weight: 400,
            font_style: FontStyle::Normal,
            font_family: None,
            margin: Sides::all(0.0),
            padding: Sides::all(0.0),
            width: LengthVal::Auto,
            max_width: LengthVal::Auto,
            text_align: TextAlign::Left,
            line_height: None,
            border_widths: Sides::all(0.0),
            border_colors: Sides::all(None),
            border_radii: Corners::all(0.0),
            box_shadow: None,
            z_index: 0,
            content: None,
            counter_reset: Vec::new(),
            counter_increment: Vec::new(),
            text_decoration: TextDecorationLine::None,
            list_style_type: ListStyleType::Disc,
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::Start,
            align_items: AlignItems::Stretch,
            flex_wrap: FlexWrap::NoWrap,
            gap_row: 0.0,
            gap_column: 0.0,
            box_sizing: BoxSizing::ContentBox,
            min_width: LengthVal::Auto,
            min_height: LengthVal::Auto,
            max_height: LengthVal::Auto,
            overflow: Overflow::Visible,
            white_space: WhiteSpace::Normal,
            text_transform: TextTransform::None,
            opacity: 1.0,
            align_self: AlignSelf::Auto,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: LengthVal::Auto,
            outline: Outline::default(),
            background_gradient: None,
            background_image_url: None,
            position: Position::Static,
            inset_top: LengthVal::Auto,
            inset_right: LengthVal::Auto,
            inset_bottom: LengthVal::Auto,
            inset_left: LengthVal::Auto,
            vertical_align: VerticalAlign::Baseline,
            visibility: Visibility::Visible,
            pointer_events: PointerEvents::Auto,
            text_indent: 0.0,
            word_spacing: 0.0,
            text_shadows: Vec::new(),
            transforms: Vec::new(),
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            animation: None,
            transitions: Vec::new(),
        }
    }
}

/// Almacena reglas parseadas + función de "computar para nodo".
#[derive(Debug, Clone)]
pub struct StyleEngine {
    rules: Vec<Rule>,
    /// CSS variables declaradas en `:root`/`html`/`*`. Se substituyen en
    /// los values en parse-time (y en values de `style="..."` inline en
    /// compute-time). Scope cascade real queda para una iteración futura
    /// — :root cubre el 80% de los usos en el wild.
    vars: HashMap<String, String>,
    /// Definiciones `@keyframes name { ... }` recogidas de todos los
    /// stylesheets. Las consumiría el runtime de animación (Fase B4+, aún
    /// no implementado) cruzando el `name` de un `AnimationBinding` con
    /// esta tabla. Hoy sólo se parsean y se exponen vía [`Self::keyframes`].
    keyframes: HashMap<String, Keyframes>,
}

impl StyleEngine {
    /// Construye el engine desde el DOM: parsea cada `<style>` inline +
    /// inyecta el UA stylesheet (los defaults HTML que cssparser no
    /// conoce).
    /// Construye el motor de estilos resolviendo `@media` contra `DEFAULT_VIEWPORT`.
    /// El chrome usa [`Self::from_dom_with_viewport`] para el viewport real.
    pub fn from_dom(dom: &DomTree) -> Self {
        Self::from_dom_with_viewport(dom, DEFAULT_VIEWPORT)
    }

    /// Como [`Self::from_dom`] pero evalúa los `@media` del documento contra
    /// `vp` (el tamaño/DPR real de la ventana). Las queries que no matchean se
    /// descartan en el parse, así que la cascada sólo ve las reglas activas.
    /// Sólo ve los `<style>` inline — las hojas externas (`<link>`) las baja el
    /// `Engine` y entran por [`Self::from_sheets_with_viewport`].
    pub fn from_dom_with_viewport(dom: &DomTree, vp: Viewport) -> Self {
        Self::from_sheets_with_viewport(&dom.collect_inline_stylesheets(), vp)
    }

    /// Construye el motor desde una lista de hojas de estilo YA resueltas (su
    /// texto), en orden de cascada. Es el punto por el que el `Engine` mete
    /// tanto los `<style>` inline como los `<link rel="stylesheet">` externos
    /// (ya bajados), preservando el orden de documento. El UA stylesheet va
    /// siempre primero (menor prioridad).
    pub fn from_sheets_with_viewport(sheets: &[String], vp: Viewport) -> Self {
        let mut rules = ua_stylesheet();
        // Primera pasada: recoger `--name: value` de `:root` de todas las
        // hojas para que cualquier `var(--x)` se resuelva sin importar en qué
        // archivo se declaró.
        let mut vars: HashMap<String, String> = HashMap::new();
        for sheet in sheets {
            let cleaned = strip_comments(sheet);
            extract_root_vars(&cleaned, &mut vars);
        }
        // Segunda pasada: recoger `@keyframes` de todas las hojas. Son
        // globales (no caen en la cascada por selector), así que un mapa
        // name→def plano alcanza; conflictos los gana el último.
        let mut keyframes: HashMap<String, Keyframes> = HashMap::new();
        for sheet in sheets {
            let cleaned = strip_comments(sheet);
            extract_keyframes(&cleaned, &mut keyframes);
        }
        for sheet in sheets {
            rules.extend(parse_stylesheet(sheet, &vars, vp));
        }
        Self { rules, vars, keyframes }
    }

    /// Tabla de `@keyframes` parseados (name → definición). Vacía si el
    /// documento no declara animaciones. El runtime de tween (Fase B4+)
    /// la cruzará con `ComputedStyle::animation`; hoy es sólo lectura.
    pub fn keyframes(&self) -> &HashMap<String, Keyframes> {
        &self.keyframes
    }

    /// Computa el estilo de un nodo Element. Aplica en orden: UA →
    /// stylesheets del documento → atributo `style="..."`. El último
    /// gana (cascada simplificada). Sin inheritance — el caller debe
    /// usar [`Self::compute_with_parent`] si necesita propagación.
    pub fn compute(&self, node: &Handle) -> ComputedStyle {
        self.compute_with_parent(node, None)
    }

    /// Variante con inheritance CSS. Si `parent` está dado, las
    /// propiedades heredables (`color`, `font_size`, `font_weight`,
    /// `text_align`, `line_height`) se inicializan con el valor del
    /// padre antes de aplicar reglas y `style=`. Propiedades no
    /// heredables (`background`, `display`, `margin`, `padding`,
    /// `width`, `max_width`) siempre arrancan en el default.
    pub fn compute_with_parent(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
    ) -> ComputedStyle {
        self.compute_with_parent_in_state(node, parent, false)
    }

    /// Variante con hover. Si `hover_active=true`, los selectores con
    /// `:hover` también matchean. Permite computar el "estilo bajo el
    /// mouse" sin un mouse real — el chrome lo usa para precalcular
    /// `hover_fill` en el render. Compat con la API anterior — para
    /// `:focus` usar [`compute_with_parent_for_state`].
    pub fn compute_with_parent_in_state(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
        hover_active: bool,
    ) -> ComputedStyle {
        self.compute_with_parent_for_state(node, parent, hover_active, false)
    }

    /// Computa el estilo del pseudo-element `::before` o `::after` del
    /// nodo. Sólo matchean selectores que terminan con ese pseudo;
    /// reglas para el elemento real se ignoran. Devuelve `None` si el
    /// pseudo no tiene `content` válido — CSS spec dice que un
    /// pseudo-element sin content no se materializa.
    pub fn compute_pseudo(
        &self,
        node: &Handle,
        pseudo: PseudoElement,
        parent: Option<&ComputedStyle>,
    ) -> Option<ComputedStyle> {
        let style = self.compute_internal(node, parent, false, false, Some(pseudo));
        // CSS spec: si `content` no se setea (None) o resuelve a `none`,
        // el pseudo-element NO se genera. Acá `content: None` cubre
        // ambos casos (el parser de content normaliza `none`/`normal` a
        // None, y la ausencia total también queda en None).
        style.content.is_some().then_some(style)
    }

    /// Variante con hover **y** focus. Cuando focus_active=true, los
    /// selectores `:focus` matchean. Útil para precalcular `focus_*`
    /// styles desde el chrome.
    pub fn compute_with_parent_for_state(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
        hover_active: bool,
        focus_active: bool,
    ) -> ComputedStyle {
        self.compute_internal(node, parent, hover_active, focus_active, None)
    }

    fn compute_internal(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
        hover_active: bool,
        focus_active: bool,
        target_pseudo: Option<PseudoElement>,
    ) -> ComputedStyle {
        let mut style = ComputedStyle::default();
        if let Some(p) = parent {
            style.color = p.color;
            style.font_size = p.font_size;
            style.font_weight = p.font_weight;
            style.font_style = p.font_style;
            style.font_family = p.font_family.clone();
            style.text_align = p.text_align;
            style.line_height = p.line_height;
            // text-decoration: tratada heredable para que descendientes
            // inline (`<a>foo <b>bar</b></a>`) mantengan la línea.
            style.text_decoration = p.text_decoration;
            // list-style-type sí es heredable según CSS spec — un `<ol>`
            // con `list-style-type: decimal` debe propagarse a sus `<li>`.
            style.list_style_type = p.list_style_type;
            // white-space y text-transform son heredables (CSS spec).
            // Sin esto, `<p style="text-transform:uppercase">FOO <span>bar</span></p>`
            // dejaría "bar" en minúscula porque el text leaf vive en `<span>`.
            style.white_space = p.white_space;
            style.text_transform = p.text_transform;
            // text-shadow, word-spacing, text-indent, visibility,
            // pointer-events: heredables (CSS spec).
            style.text_shadows = p.text_shadows.clone();
            style.word_spacing = p.word_spacing;
            style.text_indent = p.text_indent;
            style.visibility = p.visibility;
            style.pointer_events = p.pointer_events;
        }
        let Some(local) = dom::element_name(node) else {
            return style;
        };
        // Defaults por tag — `div`/`p`/`h1` son block. `display` no
        // hereda, así que siempre se setea según el tag local.
        style.display = default_display(&local);

        // `font_weight` por tag (h1..h6/b/strong/th = bold) override
        // el heredado — un `<b>` dentro de un `<p>` no-bold sigue
        // siendo bold.
        let weight_default = default_weight(&local);
        if weight_default != 400 {
            style.font_weight = weight_default;
        }
        // `font_style` por tag (em/i/cite/dfn/var/address = italic).
        // Override el heredado por defecto pero NO si el padre ya lo es
        // (`<em><span>foo</span></em>` debe conservar italic en el span).
        if default_italic(&local) {
            style.font_style = FontStyle::Italic;
        }

        // Cascada en dos pasadas:
        //   1. Decls normales, ordenadas por (specificity, source_index).
        //   2. Decls `!important`, ordenadas igual.
        // Cada decl individual lleva su flag — una misma regla puede
        // tener decls normales y `!important` mezcladas.
        let matched: Vec<(u32, usize, &Rule)> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                // Filtramos por pseudo-element: cuando computamos un
                // pseudo, sólo nos interesan reglas con ese mismo
                // pseudo_element en el selector; cuando computamos el
                // elemento real (target_pseudo=None), ignoramos las
                // reglas que generan pseudo-elements (sino sus decls
                // pegarían al padre).
                r.selector.pseudo_element == target_pseudo
                    && r.matches_in_state(node, hover_active, focus_active)
            })
            .map(|(i, r)| (r.selector.specificity(), i, r))
            .collect();
        // Inline `style="..."` no aplica a pseudo-elements (no podés
        // setear `::before` desde el HTML inline). Sólo lo recogemos
        // cuando computamos el elemento real.
        let inline_decls: Vec<Decl> = if target_pseudo.is_some() {
            Vec::new()
        } else {
            dom::attr(node, "style")
                .map(|s| parse_declarations(&s, &self.vars))
                .unwrap_or_default()
        };

        // PASADA 1 — normales.
        let mut normal_apps: Vec<(u32, usize, &Decl)> = Vec::new();
        for (spec, src, rule) in &matched {
            for d in &rule.decls {
                if !d.important {
                    normal_apps.push((*spec, *src, d));
                }
            }
        }
        normal_apps.sort_by_key(|(spec, idx, _)| (*spec, *idx));
        for (_, _, d) in normal_apps {
            d.apply(&mut style);
        }
        // Inline normal (especificidad 1000) cierra la pasada normal.
        for d in &inline_decls {
            if !d.important {
                d.apply(&mut style);
            }
        }

        // PASADA 2 — `!important`. Cualquier important de cualquier
        // regla vence cualquier normal — y entre importants, vuelve a
        // mandar especificidad/orden.
        let mut imp_apps: Vec<(u32, usize, &Decl)> = Vec::new();
        for (spec, src, rule) in &matched {
            for d in &rule.decls {
                if d.important {
                    imp_apps.push((*spec, *src, d));
                }
            }
        }
        imp_apps.sort_by_key(|(spec, idx, _)| (*spec, *idx));
        for (_, _, d) in imp_apps {
            d.apply(&mut style);
        }
        // Inline `!important` (efectiva 10_000 en CSS real, pero acá
        // simplemente cierra la pasada — gana todo lo anterior).
        for d in &inline_decls {
            if d.important {
                d.apply(&mut style);
            }
        }
        style
    }
}

#[derive(Debug, Clone)]
struct Rule {
    selector: Selector,
    decls: Vec<Decl>,
}

/// Resuelve una lista de `ContentItem` a la string final que se pintará
/// como leaf de texto. Counters se buscan en `counters`; ausentes
/// resuelven a `0` (CSS spec: el contador implícito vale 0 si no se
/// resetó). Attrs se leen del `node` (el padre del pseudo-element);
/// ausentes resuelven a `""`.
pub fn resolve_content_items(
    items: &[ContentItem],
    node: &markup5ever_rcdom::Handle,
    counters: &std::collections::HashMap<String, i32>,
) -> String {
    let mut out = String::new();
    for it in items {
        match it {
            ContentItem::Text(s) => out.push_str(s),
            ContentItem::Counter(name) => {
                let v = counters.get(name).copied().unwrap_or(0);
                out.push_str(&v.to_string());
            }
            ContentItem::Attr(name) => {
                if let Some(v) = dom::attr(node, name) {
                    out.push_str(&v);
                }
            }
            // `Url` se materializa como `<img>` sintético en boxes —
            // acá lo saltamos, el caller hace dispatch sobre los items.
            ContentItem::Url(_) => {}
        }
    }
    out
}

/// Item dentro del valor de `content:` para `::before`/`::after`. Un
/// `content:` puede tener varios items concatenados — `Text`/`Counter`/
/// `Attr` se resuelven a string y los runs adyacentes se mergean en un
/// solo text leaf; `Url` se materializa como un `<img>` sintético
/// separado, en línea con los demás items.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentItem {
    /// Literal string entre comillas — el más común.
    Text(String),
    /// `counter(name)` — el valor actual del contador con ese nombre,
    /// formateado como decimal por ahora (CSS spec permite list-style-type
    /// como segundo arg; queda para más adelante).
    Counter(String),
    /// `attr(name)` — el valor del atributo `name` del elemento padre del
    /// pseudo. Strings vacíos si el atributo no existe.
    Attr(String),
    /// `url(...)` — genera un `<img>` sintético inline-block con el
    /// recurso descargado. Si la descarga/decode falla, se omite (no
    /// fallback a texto — CSS spec dice que un url() inválido suprime
    /// la generación del pseudo).
    Url(String),
}

/// Pseudo-elemento attachado al selector. Genera un box hijo sintético
/// del nodo matching, no parte del DOM real. `content: "..."` define
/// qué texto pintar. El chrome lo trata como un text leaf inline
/// regular insertado al inicio (`Before`) o al final (`After`) de los
/// children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
}

/// Selector encadenado — alterna compound + combinador. `compounds[0]`
/// es el ancestro/hermano más lejano; `compounds.last()` es el sujeto.
/// `combinators[i]` es el combinador entre `compounds[i]` y
/// `compounds[i+1]`. `pseudo_element` (si Some) indica que la regla
/// genera un `::before` o `::after` del sujeto en lugar de aplicar al
/// nodo mismo.
#[derive(Debug, Clone)]
struct Selector {
    compounds: Vec<Compound>,
    combinators: Vec<Combinator>,
    pseudo_element: Option<PseudoElement>,
}

impl Selector {
    /// Especificidad CSS — número compuesto `a*100 + b*10 + c` donde:
    /// - `a` = cuentas de `#id` en toda la cadena
    /// - `b` = cuentas de `.class`, `[attr]`, `:pseudo-class`
    /// - `c` = cuentas de tags (`p`, `div`, …); `*` y combinadores no
    ///   suman
    ///
    /// Inline `style="..."` no pasa por acá; el caller le otorga 1000
    /// implícito al aplicarlo después de los selectores.
    fn specificity(&self) -> u32 {
        let mut ids = 0u32;
        let mut classes_etc = 0u32;
        let mut types = 0u32;
        for c in &self.compounds {
            ids += c.ids.len() as u32;
            classes_etc += c.classes.len() as u32;
            classes_etc += c.attrs.len() as u32;
            for p in &c.pseudos {
                match p {
                    // CSS spec: :not(X) aporta la especificidad de X, no
                    // la suya propia. Sumamos las partes del compound
                    // interno.
                    Pseudo::Not(inner) => {
                        ids += inner.ids.len() as u32;
                        classes_etc += inner.classes.len() as u32;
                        classes_etc += inner.attrs.len() as u32;
                        classes_etc += inner.pseudos.len() as u32;
                        if matches!(inner.tag, TagPart::Type(_)) {
                            types += 1;
                        }
                    }
                    _ => classes_etc += 1,
                }
            }
            if matches!(c.tag, TagPart::Type(_)) {
                types += 1;
            }
        }
        ids * 100 + classes_etc * 10 + types
    }
}

/// Combinador CSS entre dos compounds consecutivos.
#[derive(Debug, Clone, Copy)]
enum Combinator {
    /// Whitespace — descendiente cualquier nivel.
    Descendant,
    /// `>` — hijo directo.
    Child,
    /// `+` — hermano adyacente inmediato.
    AdjacentSibling,
    /// `~` — hermano general (posterior, mismo padre).
    GeneralSibling,
}

/// Simple compound — un Tag + 0..N ids/clases/atributos/pseudoclases en
/// cadena (sin espacios). Ejemplos válidos: `a.btn`, `p#hero.alert`,
/// `input[type="checkbox"]`, `li:first-child`, `a[href^="https"]:last-of-type`.
#[derive(Debug, Clone)]
struct Compound {
    tag: TagPart,
    ids: Vec<String>,
    classes: Vec<String>,
    attrs: Vec<AttrMatch>,
    pseudos: Vec<Pseudo>,
}

#[derive(Debug, Clone)]
enum TagPart {
    Universal,
    Type(String),
}

#[derive(Debug, Clone)]
struct AttrMatch {
    name: String,
    op: AttrOp,
    value: String,
}

#[derive(Debug, Clone, Copy)]
enum AttrOp {
    /// `[attr]` — sólo presencia
    Present,
    /// `[attr=value]` — igualdad exacta
    Equals,
    /// `[attr^=value]` — empieza con
    Prefix,
    /// `[attr$=value]` — termina con
    Suffix,
    /// `[attr*=value]` — contiene substring
    Contains,
}

/// Pseudoclases soportadas — la mayoría estructurales (puramente
/// posicionales). `Hover` se evalúa según un flag externo que pasa el
/// caller (`hover_active`); el chrome se encarga de mantenerlo
/// correlacionado con la posición del mouse.
#[derive(Debug, Clone)]
enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    Hover,
    /// `:focus` — flag externo del caller. Sólo aporta a la cascada
    /// cuando el chrome computa el estilo "como si el nodo estuviera
    /// focado"; el engine no sabe qué nodo lo está y deja la decisión
    /// al chrome.
    Focus,
    /// `:nth-child(an+b)` — match si la posición 1-indexed del nodo en
    /// el padre satisface `pos = a*k + b` para algún `k >= 0`.
    NthChild {
        a: i32,
        b: i32,
    },
    /// `:not(simple)` — negación de un compound simple sin combinadores
    /// ni `:not` anidado. Almacenamos el compound interno completo.
    Not(Box<Compound>),
}

impl Compound {
    fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        self.matches_in_state(node, false, false)
    }

    /// Variante con flags de estado externos (`hover_active`,
    /// `focus_active`) — los `:hover` y `:focus` matchean cuando el
    /// caller los activa.
    fn matches_in_state(
        &self,
        node: &markup5ever_rcdom::Handle,
        hover_active: bool,
        focus_active: bool,
    ) -> bool {
        let Some(local) = dom::element_name(node) else {
            return false;
        };
        if let TagPart::Type(t) = &self.tag {
            if !t.eq_ignore_ascii_case(&local) {
                return false;
            }
        }
        for want in &self.ids {
            if dom::attr(node, "id").as_deref() != Some(want.as_str()) {
                return false;
            }
        }
        if !self.classes.is_empty() {
            let attr = dom::attr(node, "class").unwrap_or_default();
            let present: Vec<&str> = attr.split_whitespace().collect();
            for want in &self.classes {
                if !present.iter().any(|c| c == want) {
                    return false;
                }
            }
        }
        for am in &self.attrs {
            if !attr_matches(node, am) {
                return false;
            }
        }
        for p in &self.pseudos {
            if !pseudo_matches(node, p, hover_active, focus_active) {
                return false;
            }
        }
        true
    }
}

fn attr_matches(node: &markup5ever_rcdom::Handle, am: &AttrMatch) -> bool {
    let actual = dom::attr(node, &am.name);
    match am.op {
        AttrOp::Present => actual.is_some(),
        op => {
            let Some(actual) = actual else { return false };
            match op {
                AttrOp::Equals => actual == am.value,
                AttrOp::Prefix => actual.starts_with(&am.value),
                AttrOp::Suffix => actual.ends_with(&am.value),
                AttrOp::Contains => actual.contains(&am.value),
                AttrOp::Present => unreachable!(),
            }
        }
    }
}

fn pseudo_matches(
    node: &markup5ever_rcdom::Handle,
    p: &Pseudo,
    hover_active: bool,
    focus_active: bool,
) -> bool {
    match p {
        Pseudo::Hover => return hover_active,
        Pseudo::Focus => return focus_active,
        Pseudo::Not(c) => return !c.matches_in_state(node, hover_active, focus_active),
        _ => {}
    }
    let Some(parent) = parent_of(node) else { return false };
    let kids = parent.children.borrow();
    let mut elems: Vec<markup5ever_rcdom::Handle> = Vec::new();
    for c in kids.iter() {
        if dom::element_name(c).is_some() {
            elems.push(c.clone());
        }
    }
    let Some(pos) = elems.iter().position(|c| std::rc::Rc::ptr_eq(c, node)) else {
        return false;
    };
    match p {
        Pseudo::Hover | Pseudo::Focus | Pseudo::Not(_) => unreachable!("ya resueltos arriba"),
        Pseudo::FirstChild => pos == 0,
        Pseudo::LastChild => pos + 1 == elems.len(),
        Pseudo::OnlyChild => elems.len() == 1,
        Pseudo::FirstOfType => {
            let my_tag = dom::element_name(node).unwrap_or_default();
            elems[..pos]
                .iter()
                .all(|c| dom::element_name(c).map(|t| t != my_tag).unwrap_or(true))
        }
        Pseudo::LastOfType => {
            let my_tag = dom::element_name(node).unwrap_or_default();
            elems[pos + 1..]
                .iter()
                .all(|c| dom::element_name(c).map(|t| t != my_tag).unwrap_or(true))
        }
        Pseudo::NthChild { a, b } => {
            // `pos` es 0-indexed, CSS usa 1-indexed.
            let p_css = (pos + 1) as i32;
            let diff = p_css - *b;
            if *a == 0 {
                diff == 0
            } else if *a > 0 {
                diff >= 0 && diff % *a == 0
            } else {
                diff <= 0 && diff % *a == 0
            }
        }
    }
}

impl Rule {
    #[allow(dead_code)]
    fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        self.matches_in_state(node, false, false)
    }

    fn matches_in_state(
        &self,
        node: &markup5ever_rcdom::Handle,
        hover_active: bool,
        focus_active: bool,
    ) -> bool {
        let compounds = &self.selector.compounds;
        if compounds.is_empty() {
            return false;
        }
        // El sujeto (último) debe matchear el nodo. Los ancestros/hermanos
        // siguen matcheando sin los flags activos (un `:hover/:focus`
        // sólo aplica al sujeto del selector, no propagamos el estado
        // por la cadena — es suficiente para 90% del CSS real).
        if !compounds.last().unwrap().matches_in_state(node, hover_active, focus_active) {
            return false;
        }
        if compounds.len() == 1 {
            return true;
        }
        // Avanzamos derecha→izquierda, encadenando combinadores. Cada
        // combinador define cómo viajar al "siguiente" candidato:
        //   Descendant/Child  → ancestro
        //   Adjacent/General  → hermano anterior
        let combs = &self.selector.combinators;
        // El combinador entre compounds[i-1] y compounds[i] vive en
        // combs[i-1]. Recorremos desde compounds[len-2] hacia 0.
        let mut subject = node.clone();
        let mut i = compounds.len() - 1;
        while i > 0 {
            let comb = combs[i - 1];
            let target = &compounds[i - 1];
            match comb {
                Combinator::Child => {
                    let Some(p) = parent_of(&subject) else { return false };
                    if !target.matches(&p) {
                        return false;
                    }
                    subject = p;
                }
                Combinator::Descendant => {
                    let mut cur = parent_of(&subject);
                    loop {
                        let Some(n) = cur else { return false };
                        if target.matches(&n) {
                            subject = n;
                            break;
                        }
                        cur = parent_of(&n);
                    }
                }
                Combinator::AdjacentSibling => {
                    let Some(prev) = prev_element_sibling(&subject) else { return false };
                    if !target.matches(&prev) {
                        return false;
                    }
                    subject = prev;
                }
                Combinator::GeneralSibling => {
                    let mut cur = prev_element_sibling(&subject);
                    loop {
                        let Some(n) = cur else { return false };
                        if target.matches(&n) {
                            subject = n;
                            break;
                        }
                        cur = prev_element_sibling(&n);
                    }
                }
            }
            i -= 1;
        }
        true
    }
}

fn parent_of(node: &markup5ever_rcdom::Handle) -> Option<markup5ever_rcdom::Handle> {
    let weak = node.parent.take();
    let restored = weak.clone();
    node.parent.set(restored);
    weak.and_then(|w| w.upgrade())
}

/// Hermano Element anterior (saltea texto/whitespace nodes). Devuelve
/// `None` si no hay padre o si no hay Element previo bajo el mismo padre.
fn prev_element_sibling(
    node: &markup5ever_rcdom::Handle,
) -> Option<markup5ever_rcdom::Handle> {
    let parent = parent_of(node)?;
    let kids = parent.children.borrow();
    let mut last_elem: Option<markup5ever_rcdom::Handle> = None;
    for child in kids.iter() {
        if std::rc::Rc::ptr_eq(child, node) {
            return last_elem;
        }
        if dom::element_name(child).is_some() {
            last_elem = Some(child.clone());
        }
    }
    None
}

/// Una declaración CSS individual + flag `!important`.
#[derive(Debug, Clone)]
struct Decl {
    kind: DeclKind,
    important: bool,
}

#[derive(Debug, Clone)]
enum DeclKind {
    Color(Color),
    Background(Color),
    Display(Display),
    FontSize(f32),
    FontWeight(u16),
    FontStyle(FontStyle),
    FontFamily(String),
    Margin(Sides<f32>),
    MarginTop(f32),
    MarginRight(f32),
    MarginBottom(f32),
    MarginLeft(f32),
    Padding(Sides<f32>),
    PaddingTop(f32),
    PaddingRight(f32),
    PaddingBottom(f32),
    PaddingLeft(f32),
    Width(LengthVal),
    MaxWidth(LengthVal),
    TextAlign(TextAlign),
    LineHeight(f32),
    BorderWidth(f32),
    BorderColor(Color),
    /// `border-style: solid` activa el dibujo del border; `none`/`hidden`
    /// lo desactiva (color → None).
    BorderEnabled(bool),
    /// Variantes per-side: `border-top-width: 2px` setea sólo el top.
    BorderSideWidth(BorderEdge, f32),
    BorderSideColor(BorderEdge, Color),
    BorderSideStyle(BorderEdge, bool),
    BorderRadius(f32),
    /// `border-top-left-radius` etc. — setean una esquina sola.
    BorderCornerRadius(BorderCorner, f32),
    /// `z-index: N`. Aplica sólo a positioned (absolute/fixed/relative);
    /// para `position: static` el chrome lo ignora (matchea spec).
    ZIndex(i32),
    /// `content: ...` para `::before`/`::after`. Lista de items
    /// (string/counter/attr) que se concatenan al inyectar el pseudo.
    /// `None` = `content: none` (suprime el pseudo-element).
    Content(Option<Vec<ContentItem>>),
    /// `counter-reset: name [val] name2 [val2]...`.
    CounterReset(Vec<(String, i32)>),
    /// `counter-increment: name [delta] name2 [delta2]...`.
    CounterIncrement(Vec<(String, i32)>),
    /// `None` = `box-shadow: none` (limpia la sombra).
    BoxShadow(Option<BoxShadow>),
    TextDecoration(TextDecorationLine),
    ListStyleType(ListStyleType),
    FlexDirection(FlexDirection),
    JustifyContent(JustifyContent),
    AlignItems(AlignItems),
    FlexWrap(FlexWrap),
    /// `gap: A B` setea ambos (row=A, column=B); `gap: V` los iguala.
    Gap { row: f32, column: f32 },
    RowGap(f32),
    ColumnGap(f32),
    BoxSizing(BoxSizing),
    MinWidth(LengthVal),
    MinHeight(LengthVal),
    MaxHeight(LengthVal),
    Overflow(Overflow),
    WhiteSpace(WhiteSpace),
    TextTransform(TextTransform),
    Opacity(f32),
    AlignSelf(AlignSelf),
    FlexGrow(f32),
    FlexShrink(f32),
    FlexBasis(LengthVal),
    OutlineWidth(f32),
    OutlineColor(Color),
    OutlineStyle(bool),
    OutlineOffset(f32),
    BackgroundGradient(LinearGradient),
    /// `background-image: none` limpia el gradient (un autor puede
    /// querer overridear un gradient heredado).
    BackgroundGradientNone,
    /// `background-image: url(...)` — URL absoluta o relativa, el engine
    /// la resuelve contra el base del documento en `build_node`.
    BackgroundImageUrl(String),
    Position(Position),
    InsetTop(LengthVal),
    InsetRight(LengthVal),
    InsetBottom(LengthVal),
    InsetLeft(LengthVal),
    VerticalAlign(VerticalAlign),
    Visibility(Visibility),
    PointerEvents(PointerEvents),
    TextIndent(f32),
    WordSpacing(f32),
    TextShadows(Vec<TextShadow>),
    /// Cadena vacía = `transform: none`.
    Transforms(Vec<Transform>),
    GridTemplateColumns(Vec<GridTrackSize>),
    GridTemplateRows(Vec<GridTrackSize>),
    /// `animation: ...`. `None` = `animation: none`.
    Animation(Option<AnimationBinding>),
    /// `transition: ...`. Vec vacío = `transition: none`.
    Transitions(Vec<TransitionBinding>),
}

impl Decl {
    fn apply(&self, s: &mut ComputedStyle) {
        match &self.kind {
            DeclKind::Color(c) => s.color = *c,
            DeclKind::Background(c) => s.background = Some(*c),
            DeclKind::Display(d) => s.display = *d,
            DeclKind::FontSize(v) => s.font_size = *v,
            DeclKind::FontWeight(w) => s.font_weight = *w,
            DeclKind::FontStyle(fs) => s.font_style = *fs,
            DeclKind::FontFamily(ff) => s.font_family = Some(ff.clone()),
            DeclKind::Margin(v) => s.margin = *v,
            DeclKind::MarginTop(v) => s.margin.top = *v,
            DeclKind::MarginRight(v) => s.margin.right = *v,
            DeclKind::MarginBottom(v) => s.margin.bottom = *v,
            DeclKind::MarginLeft(v) => s.margin.left = *v,
            DeclKind::Padding(v) => s.padding = *v,
            DeclKind::PaddingTop(v) => s.padding.top = *v,
            DeclKind::PaddingRight(v) => s.padding.right = *v,
            DeclKind::PaddingBottom(v) => s.padding.bottom = *v,
            DeclKind::PaddingLeft(v) => s.padding.left = *v,
            DeclKind::Width(v) => s.width = *v,
            DeclKind::MaxWidth(v) => s.max_width = *v,
            DeclKind::TextAlign(a) => s.text_align = *a,
            DeclKind::LineHeight(v) => s.line_height = Some(*v),
            DeclKind::BorderWidth(v) => s.border_widths = Sides::all(*v),
            DeclKind::BorderColor(c) => s.border_colors = Sides::all(Some(*c)),
            DeclKind::BorderEnabled(on) => {
                if !*on {
                    s.border_colors = Sides::all(None);
                    s.border_widths = Sides::all(0.0);
                }
            }
            DeclKind::BorderSideWidth(side, v) => set_side_f32(&mut s.border_widths, *side, *v),
            DeclKind::BorderSideColor(side, c) => set_side(&mut s.border_colors, *side, Some(*c)),
            DeclKind::BorderSideStyle(side, on) => {
                if !*on {
                    set_side_f32(&mut s.border_widths, *side, 0.0);
                    set_side(&mut s.border_colors, *side, None);
                }
            }
            DeclKind::BorderRadius(v) => s.border_radii = Corners::all(*v),
            DeclKind::BorderCornerRadius(corner, v) => {
                set_corner(&mut s.border_radii, *corner, *v)
            }
            DeclKind::ZIndex(v) => s.z_index = *v,
            DeclKind::Content(c) => s.content = c.clone(),
            DeclKind::CounterReset(v) => s.counter_reset = v.clone(),
            DeclKind::CounterIncrement(v) => s.counter_increment = v.clone(),
            DeclKind::BoxShadow(v) => s.box_shadow = *v,
            DeclKind::TextDecoration(t) => s.text_decoration = *t,
            DeclKind::ListStyleType(t) => s.list_style_type = *t,
            DeclKind::FlexDirection(d) => s.flex_direction = *d,
            DeclKind::JustifyContent(j) => s.justify_content = *j,
            DeclKind::AlignItems(a) => s.align_items = *a,
            DeclKind::FlexWrap(w) => s.flex_wrap = *w,
            DeclKind::Gap { row, column } => {
                s.gap_row = *row;
                s.gap_column = *column;
            }
            DeclKind::RowGap(v) => s.gap_row = *v,
            DeclKind::ColumnGap(v) => s.gap_column = *v,
            DeclKind::BoxSizing(b) => s.box_sizing = *b,
            DeclKind::MinWidth(v) => s.min_width = *v,
            DeclKind::MinHeight(v) => s.min_height = *v,
            DeclKind::MaxHeight(v) => s.max_height = *v,
            DeclKind::Overflow(o) => s.overflow = *o,
            DeclKind::WhiteSpace(w) => s.white_space = *w,
            DeclKind::TextTransform(t) => s.text_transform = *t,
            DeclKind::Opacity(v) => s.opacity = *v,
            DeclKind::AlignSelf(a) => s.align_self = *a,
            DeclKind::FlexGrow(v) => s.flex_grow = *v,
            DeclKind::FlexShrink(v) => s.flex_shrink = *v,
            DeclKind::FlexBasis(v) => s.flex_basis = *v,
            DeclKind::OutlineWidth(v) => s.outline.width = *v,
            DeclKind::OutlineColor(c) => s.outline.color = Some(*c),
            DeclKind::OutlineStyle(active) => {
                s.outline.style_active = *active;
                if !*active {
                    s.outline.color = None;
                }
            }
            DeclKind::OutlineOffset(v) => s.outline.offset = *v,
            DeclKind::BackgroundGradient(g) => s.background_gradient = Some(g.clone()),
            DeclKind::BackgroundGradientNone => {
                s.background_gradient = None;
                s.background_image_url = None;
            }
            DeclKind::BackgroundImageUrl(u) => s.background_image_url = Some(u.clone()),
            DeclKind::Position(p) => s.position = *p,
            DeclKind::InsetTop(v) => s.inset_top = *v,
            DeclKind::InsetRight(v) => s.inset_right = *v,
            DeclKind::InsetBottom(v) => s.inset_bottom = *v,
            DeclKind::InsetLeft(v) => s.inset_left = *v,
            DeclKind::VerticalAlign(va) => s.vertical_align = *va,
            DeclKind::Visibility(v) => s.visibility = *v,
            DeclKind::PointerEvents(pe) => s.pointer_events = *pe,
            DeclKind::TextIndent(v) => s.text_indent = *v,
            DeclKind::WordSpacing(v) => s.word_spacing = *v,
            DeclKind::TextShadows(shadows) => s.text_shadows = shadows.clone(),
            DeclKind::Transforms(tr) => s.transforms = tr.clone(),
            DeclKind::GridTemplateColumns(t) => s.grid_template_columns = t.clone(),
            DeclKind::GridTemplateRows(t) => s.grid_template_rows = t.clone(),
            DeclKind::Animation(a) => s.animation = a.clone(),
            DeclKind::Transitions(t) => s.transitions = t.clone(),
        }
    }
}

fn default_display(tag: &str) -> Display {
    match tag {
        "html" | "body" | "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol"
        | "li" | "header" | "footer" | "section" | "article" | "nav" | "main" | "aside"
        | "form" | "pre" | "blockquote" | "hr" | "figure" | "figcaption" | "details"
        | "summary" | "dialog" | "menu" | "address" | "fieldset" | "legend" | "dl" | "dd"
        | "dt" | "caption" => Display::Block,
        // Tables — semánticamente correctos serían display-table-*, pero
        // tratamos tr como flex-row, td/th como inline-block para que
        // la grilla se rinda razonablemente sin un layout engine de
        // tables completo.
        "table" | "thead" | "tbody" | "tfoot" => Display::Block,
        // `<colgroup>` y `<col>` son metadatos de columna en la spec
        // CSS table layout, NO se renderean como cajas propias — su rol
        // es definir width de columnas (que acá no soportamos). Ocultar
        // evita que tablas con esos elementos muestren espacios fantasma.
        "colgroup" | "col" => Display::None,
        "tr" => Display::Flex,
        "td" | "th" => Display::InlineBlock,
        // Form widgets: inline-block para que respeten width/height
        // pero no rompan el row del padre.
        "button" | "select" | "textarea" | "label" => Display::InlineBlock,
        "head" | "title" | "style" | "script" | "meta" | "link" => Display::None,
        // `<option>` / `<optgroup>`: el chrome los recolecta en
        // `SelectInfo` cuando ve un `<select>` padre y los renderea
        // como popup. Como hijos directos del DOM serían texto suelto.
        "option" | "optgroup" => Display::None,
        // `<svg>`: lo tratamos como inline-block — el engine recolecta
        // las primitivas (rect/circle/line) en `BoxNode.svg` y el chrome
        // las pinta. Sus descendientes (los `<rect>`/`<path>`/etc.) NO
        // entran al box tree.
        "svg" => Display::InlineBlock,
        // `<iframe>` no tiene engine de sub-página todavía, pero
        // mostrarlo como block placeholder (border + label con la URL)
        // es mejor que ocultarlo — el lector ve QUE hay contenido
        // embebido y dónde apunta. El placeholder lo arma boxes.
        "iframe" => Display::Block,
        // canvas/math/video/audio/object/embed: sin renderer todavía.
        // Ocultos para no derramar texto basura en la página.
        "canvas" | "math" | "video" | "audio" | "object" | "embed" => Display::None,
        _ => Display::Inline,
    }
}

/// `true` si el tag se oculta por defecto en la hoja UA (`script`/`style`/
/// `head`/`option`/`colgroup`/`canvas`/...). Lo usa `boxes::build_node` para
/// distinguir el `display:none` "de ruido UA" (que se descarta del box tree)
/// del puesto por el autor (que se retiene como box oculto, para poder
/// mostrarlo con un toggle de clase vía restyle). Fase 7.185.
pub(crate) fn tag_defaults_to_none(tag: &str) -> bool {
    default_display(tag) == Display::None
}

fn default_weight(tag: &str) -> u16 {
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "b" | "strong" | "th" => 700,
        _ => 400,
    }
}

/// Tags que el UA stylesheet pone en italic por default (CSS spec).
fn default_italic(tag: &str) -> bool {
    matches!(
        tag,
        "em" | "i" | "cite" | "dfn" | "var" | "address" | "blockquote"
    )
}

/// UA stylesheet mínimo — defaults HTML5 que cssparser por sí solo no
/// inyecta. Mantén corto: sólo lo necesario para no devolver páginas
/// "blancas" sin reglas autor.
fn ua_stylesheet() -> Vec<Rule> {
    fn ty(s: &str) -> Selector {
        Selector {
            compounds: vec![Compound {
                tag: TagPart::Type(s.into()),
                ids: vec![],
                classes: vec![],
                attrs: vec![],
                pseudos: vec![],
            }],
            combinators: vec![],
            pseudo_element: None,
        }
    }
    fn decl(kind: DeclKind) -> Decl {
        Decl { kind, important: false }
    }
    fn sides_lrtb(t: f32, r: f32, b: f32, l: f32) -> Sides<f32> {
        Sides { top: t, right: r, bottom: b, left: l }
    }
    // Tamaños y márgenes de heading siguen el patrón de Firefox / Chrome
    // (em-based, redondeado a px sobre font-size 16). h1 sólo dentro del
    // primer `<section>`/`<article>` sería 1.5em según spec, pero ese
    // matching contextual queda para más adelante — usamos 2em fijo.
    vec![
        Rule {
            selector: ty("body"),
            decls: vec![
                // Browser real default es `margin: 8px` (no padding). Lo
                // dejamos así para que páginas sin CSS no queden pegadas
                // al borde de la ventana.
                decl(DeclKind::Margin(Sides::all(8.0))),
                // CSS spec default es `font-family: serif`. Browsers
                // mapean "serif" a Times New Roman, Georgia, etc. según
                // el sistema. `parley::FontStack::Source("serif")` ya
                // delega esa resolución a la system font config.
                decl(DeclKind::FontFamily("serif".to_string())),
            ],
        },
        Rule {
            selector: ty("h1"),
            decls: vec![
                decl(DeclKind::FontSize(32.0)),
                decl(DeclKind::Margin(sides_lrtb(21.0, 0.0, 21.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("h2"),
            decls: vec![
                decl(DeclKind::FontSize(24.0)),
                decl(DeclKind::Margin(sides_lrtb(19.0, 0.0, 19.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("h3"),
            decls: vec![
                decl(DeclKind::FontSize(19.0)),
                decl(DeclKind::Margin(sides_lrtb(19.0, 0.0, 19.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("h4"),
            decls: vec![
                decl(DeclKind::FontSize(16.0)),
                decl(DeclKind::Margin(sides_lrtb(21.0, 0.0, 21.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("h5"),
            decls: vec![
                decl(DeclKind::FontSize(13.0)),
                decl(DeclKind::Margin(sides_lrtb(22.0, 0.0, 22.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("h6"),
            decls: vec![
                decl(DeclKind::FontSize(11.0)),
                decl(DeclKind::Margin(sides_lrtb(25.0, 0.0, 25.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("p"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0)))],
        },
        // Listas: padding-left para los bullets/numerales (el marker se
        // pinta antes del contenido, necesita espacio para no chocar
        // con el borde izquierdo del block).
        Rule {
            selector: ty("ul"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0))),
                decl(DeclKind::Padding(sides_lrtb(0.0, 0.0, 0.0, 40.0))),
                decl(DeclKind::ListStyleType(ListStyleType::Disc)),
            ],
        },
        Rule {
            selector: ty("ol"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0))),
                decl(DeclKind::Padding(sides_lrtb(0.0, 0.0, 0.0, 40.0))),
                decl(DeclKind::ListStyleType(ListStyleType::Decimal)),
            ],
        },
        Rule {
            selector: ty("blockquote"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(10.0, 40.0, 10.0, 40.0)))],
        },
        Rule {
            selector: ty("dl"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0)))],
        },
        Rule {
            selector: ty("dd"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(0.0, 0.0, 0.0, 40.0)))],
        },
        Rule {
            selector: ty("pre"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0))),
                decl(DeclKind::WhiteSpace(WhiteSpace::Pre)),
            ],
        },
        Rule {
            selector: ty("hr"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(8.0, 0.0, 8.0, 0.0))),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(192, 192, 192))),
                decl(DeclKind::BorderEnabled(true)),
            ],
        },
        // Color por defecto de los links — azul clásico de navegadores.
        // Esto se cascadea bajo el override del chrome que pinta links
        // con un blue ligeramente más oscuro (30,90,200).
        Rule {
            selector: ty("a"),
            decls: vec![
                decl(DeclKind::Color(Color::rgb(0, 0, 238))),
                decl(DeclKind::TextDecoration(TextDecorationLine::Underline)),
            ],
        },
        // Defaults de text-decoration. `<a>` y `<u>`/`<ins>` van con
        // underline; `<s>`/`<strike>`/`<del>` tachadas. Cualquier autor
        // puede override con `text-decoration: none` en su stylesheet.
        Rule {
            selector: ty("a"),
            decls: vec![Decl {
                kind: DeclKind::TextDecoration(TextDecorationLine::Underline),
                important: false,
            }],
        },
        Rule {
            selector: ty("u"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::Underline))],
        },
        Rule {
            selector: ty("ins"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::Underline))],
        },
        Rule {
            selector: ty("s"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::LineThrough))],
        },
        Rule {
            selector: ty("strike"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::LineThrough))],
        },
        Rule {
            selector: ty("del"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::LineThrough))],
        },
        Rule {
            selector: ty("menu"),
            decls: vec![decl(DeclKind::ListStyleType(ListStyleType::Disc))],
        },
        // Tables: bordes celulares mínimos para que la grilla se vea sin
        // CSS de autor. Browsers reales no dibujan bordes hasta que un
        // stylesheet lo pida, pero acá preferimos mostrarlos por default
        // — la mayoría de páginas con `<table>` sin estilo asumen un
        // "look spreadsheet" y tablas sin bordes salen invisibles.
        Rule {
            selector: ty("table"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(8.0, 0.0, 8.0, 0.0)))],
        },
        Rule {
            selector: ty("td"),
            decls: vec![
                decl(DeclKind::Padding(Sides::all(4.0))),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(204, 204, 204))),
                decl(DeclKind::BorderEnabled(true)),
            ],
        },
        Rule {
            selector: ty("th"),
            decls: vec![
                decl(DeclKind::Padding(Sides::all(4.0))),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(204, 204, 204))),
                decl(DeclKind::BorderEnabled(true)),
                decl(DeclKind::Background(Color::rgb(242, 242, 242))),
            ],
        },
        // `<caption>` es el título de la tabla — centrado encima de las
        // filas. Sin esto el caption queda alineado a la izquierda
        // como cualquier block.
        Rule {
            selector: ty("caption"),
            decls: vec![
                decl(DeclKind::TextAlign(TextAlign::Center)),
                decl(DeclKind::Padding(Sides::all(4.0))),
            ],
        },
        // `<iframe>` placeholder: border gris discreto + padding +
        // margin vertical para que se distinga del flujo. El label
        // con la URL lo inyecta `boxes::build_node`.
        Rule {
            selector: ty("iframe"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(8.0, 0.0, 8.0, 0.0))),
                decl(DeclKind::Padding(Sides::all(8.0))),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(180, 180, 180))),
                decl(DeclKind::BorderEnabled(true)),
                decl(DeclKind::Background(Color::rgb(248, 248, 248))),
                decl(DeclKind::Color(Color::rgb(100, 100, 100))),
            ],
        },
        // <small>/<sub>/<sup>: tamaño relativo. CSS spec usa `smaller`
        // (~83% del padre). Acá usamos 13px como aproximación.
        Rule {
            selector: ty("small"),
            decls: vec![decl(DeclKind::FontSize(13.0))],
        },
        Rule {
            selector: ty("sub"),
            decls: vec![
                decl(DeclKind::FontSize(13.0)),
                decl(DeclKind::VerticalAlign(VerticalAlign::Sub)),
            ],
        },
        Rule {
            selector: ty("sup"),
            decls: vec![
                decl(DeclKind::FontSize(13.0)),
                decl(DeclKind::VerticalAlign(VerticalAlign::Super)),
            ],
        },
        Rule {
            selector: ty("button"),
            decls: vec![
                decl(DeclKind::Padding(Sides { top: 1.0, right: 6.0, bottom: 1.0, left: 6.0 })),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(118, 118, 118))),
                decl(DeclKind::BorderEnabled(true)),
                decl(DeclKind::Background(Color::rgb(239, 239, 239))),
            ],
        },
        Rule {
            selector: ty("input"),
            decls: vec![
                decl(DeclKind::Padding(Sides { top: 1.0, right: 2.0, bottom: 1.0, left: 2.0 })),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(118, 118, 118))),
                decl(DeclKind::BorderEnabled(true)),
                decl(DeclKind::Background(Color::WHITE)),
            ],
        },
    ]
}

// ----- parser -----
//
// Para Fase 2 no usamos cssparser AtRule/QualifiedRule (su API rotó
// entre 0.33→0.35 y nuestro subset cabe en 30 líneas). Si Fase 3 mete
// nesting / `@media` / `!important`, migrar a `cssparser::StyleSheetParser`
// con un visitor.

fn parse_stylesheet(css: &str, vars: &HashMap<String, String>, vp: Viewport) -> Vec<Rule> {
    let css = strip_comments(css);
    parse_rules_block(&css, vars, vp)
}

/// Parsea un bloque de reglas — el cuerpo de un stylesheet completo o
/// el contenido de un `@media` / `@supports`. Soporta:
/// - reglas normales `selector { decls }`
/// - `@media (condition) { ... }` recursivo — eval contra `viewport`
/// - `@supports (prop: value) { ... }` recursivo — eval por parser
/// - `@-rules` desconocidos (`@font-face`, `@keyframes`, etc.) los
///   saltea silenciosamente
fn parse_rules_block(css: &str, vars: &HashMap<String, String>, viewport: Viewport) -> Vec<Rule> {
    let mut out = Vec::new();
    let bytes = css.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Salta whitespace inicial.
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Detecta @-rule.
        if bytes[i] == b'@' {
            let rest = &css[i..];
            let Some(rule_end) = at_rule_end(rest) else {
                break;
            };
            let chunk = &rest[..rule_end];
            i += rule_end;
            // Distinguimos at-rules con bloque `{...}` vs at-rules statement
            // que terminan en `;` (ej: @import, @charset).
            let lower = chunk.trim_start().to_ascii_lowercase();
            if let Some(rest_after) = lower.strip_prefix("@media") {
                let cond = parse_at_rule_condition(chunk, "@media");
                let body = parse_at_rule_body(chunk);
                if evaluate_media_query(cond, viewport) {
                    out.extend(parse_rules_block(body, vars, viewport));
                }
                let _ = rest_after;
                continue;
            }
            if lower.starts_with("@supports") {
                let cond = parse_at_rule_condition(chunk, "@supports");
                let body = parse_at_rule_body(chunk);
                if evaluate_supports_query(cond) {
                    out.extend(parse_rules_block(body, vars, viewport));
                }
                continue;
            }
            // @-rule desconocido: lo saltamos sin parsear.
            continue;
        }
        // Regla normal: `selector { decls }`.
        let Some(brace) = css[i..].find('{') else { break };
        let sel_raw = css[i..i + brace].trim();
        i += brace + 1;
        let Some(close) = matching_close_brace(&css[i..]) else { break };
        let body = &css[i..i + close];
        i += close + 1;
        if sel_raw.is_empty() {
            continue;
        }
        for sel in sel_raw.split(',') {
            let sel = sel.trim();
            let Some(selector) = parse_selector(sel) else {
                continue;
            };
            out.push(Rule { selector, decls: parse_declarations(body, vars) });
        }
    }
    out
}

/// Encuentra el final del @-rule actual. Para at-rules con bloque,
/// devuelve la posición del `}` cerrando (inclusive). Para at-rules
/// statement (ej: `@import url;`), devuelve la posición del `;`
/// (inclusive). Si nada cuadra, None.
fn at_rule_end(s: &str) -> Option<usize> {
    let semi = s.find(';');
    let brace = s.find('{');
    match (semi, brace) {
        (Some(se), Some(br)) if se < br => Some(se + 1),
        (Some(se), None) => Some(se + 1),
        (_, Some(br)) => {
            // Encuentra el `}` que cierra balanceado.
            let body = &s[br + 1..];
            let close = matching_close_brace(body)?;
            Some(br + 1 + close + 1)
        }
        (None, None) => None,
    }
}

/// Dado el chunk completo del at-rule (`@media (cond) { body }`),
/// extrae la condición entre el nombre y el `{`.
fn parse_at_rule_condition<'a>(chunk: &'a str, name: &str) -> &'a str {
    let after_name = chunk.trim_start().get(name.len()..).unwrap_or("");
    let end = after_name.find('{').unwrap_or(after_name.len());
    after_name[..end].trim()
}

/// Extrae el body entre `{` y el `}` cerrando.
fn parse_at_rule_body(chunk: &str) -> &str {
    let Some(open) = chunk.find('{') else {
        return "";
    };
    let after = &chunk[open + 1..];
    let close = matching_close_brace(after).unwrap_or(after.len());
    &after[..close]
}

/// Busca el `}` que cierra balanceadamente — respeta nesting (`{ ... }`
/// dentro del body cuentan).
fn matching_close_brace(s: &str) -> Option<usize> {
    let mut depth: usize = 1;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Pasada previa al parseo real: encuentra bloques `:root { ... }`,
/// `html { ... }` o `* { ... }` y recoge cualquier declaración `--name:
/// value` en el mapa global de variables. Los conflictos (mismo nombre
/// en dos bloques) los gana el último — se acerca bastante a la cascada
/// CSS para vars declaradas en root.
fn extract_root_vars(css: &str, vars: &mut HashMap<String, String>) {
    let mut i = 0;
    while i < css.len() {
        let Some(brace) = css[i..].find('{') else { break };
        let sel_raw = css[i..i + brace].trim();
        let body_start = i + brace + 1;
        let Some(close) = css[body_start..].find('}') else { break };
        let body = &css[body_start..body_start + close];
        i = body_start + close + 1;
        let mut is_root = false;
        for sel in sel_raw.split(',') {
            let sel = sel.trim();
            if sel == ":root" || sel == "html" || sel == "*" {
                is_root = true;
                break;
            }
        }
        if !is_root {
            continue;
        }
        for chunk in body.split(';') {
            let Some((prop, value)) = chunk.split_once(':') else {
                continue;
            };
            let prop = prop.trim();
            if let Some(name) = prop.strip_prefix("--") {
                vars.insert(name.to_string(), value.trim().to_string());
            }
        }
    }
}

/// Pasada análoga a [`extract_root_vars`] pero para `@keyframes`. Escanea
/// el CSS crudo buscando `@keyframes name { ... }` (también los prefijos
/// vendor `@-webkit-keyframes` / `@-moz-keyframes`) y los acumula en el
/// mapa. Conflictos (mismo `name` en dos sitios) los gana el último.
fn extract_keyframes(css: &str, out: &mut HashMap<String, Keyframes>) {
    // `to_ascii_lowercase` preserva el largo en bytes (ASCII case sólo),
    // así que los índices del lowercase indexan el `css` original sin
    // desfase — necesario para conservar el case del `name` y los values.
    let lower = css.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find('@') {
        let at = from + rel;
        let lrest = &lower[at..];
        let prefix = if lrest.starts_with("@keyframes") {
            "@keyframes"
        } else if lrest.starts_with("@-webkit-keyframes") {
            "@-webkit-keyframes"
        } else if lrest.starts_with("@-moz-keyframes") {
            "@-moz-keyframes"
        } else {
            from = at + 1;
            continue;
        };
        let after = &css[at + prefix.len()..];
        let Some(brace_rel) = after.find('{') else { break };
        let name = after[..brace_rel]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        let body_start = at + prefix.len() + brace_rel + 1;
        let Some(close) = matching_close_brace(&css[body_start..]) else {
            break;
        };
        let body = &css[body_start..body_start + close];
        from = body_start + close + 1;
        if name.is_empty() {
            continue;
        }
        let kf = parse_keyframes_body(body);
        if !kf.steps.is_empty() {
            out.insert(name, kf);
        }
    }
}

/// Parsea el cuerpo de un `@keyframes`: una secuencia de bloques
/// `selector { decls }` donde `selector` es una lista de offsets
/// (`from`/`to`/`N%`) separados por coma. Los pasos quedan ordenados por
/// offset ascendente.
fn parse_keyframes_body(body: &str) -> Keyframes {
    let mut steps: Vec<KeyframeStep> = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < body.len() {
        while i < body.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= body.len() {
            break;
        }
        let Some(brace) = body[i..].find('{') else { break };
        let selector_raw = body[i..i + brace].trim();
        let inner_start = i + brace + 1;
        let Some(close) = matching_close_brace(&body[inner_start..]) else {
            break;
        };
        let inner = &body[inner_start..inner_start + close];
        i = inner_start + close + 1;
        let decls = parse_keyframe_declarations(inner);
        if decls.is_empty() {
            continue;
        }
        for tok in selector_raw.split(',') {
            if let Some(offset) = parse_keyframe_offset(tok.trim()) {
                steps.push(KeyframeStep { offset, declarations: decls.clone() });
            }
        }
    }
    steps.sort_by(|a, b| {
        a.offset.partial_cmp(&b.offset).unwrap_or(std::cmp::Ordering::Equal)
    });
    Keyframes { steps }
}

/// `from` → 0.0, `to` → 1.0, `N%` → N/100. Cualquier otra cosa → None.
fn parse_keyframe_offset(tok: &str) -> Option<f32> {
    let t = tok.trim().to_ascii_lowercase();
    match t.as_str() {
        "from" => Some(0.0),
        "to" => Some(1.0),
        _ => t.strip_suffix('%').and_then(|n| n.trim().parse::<f32>().ok()).map(|p| p / 100.0),
    }
}

/// Pares `prop: value` crudos del cuerpo de un keyframe. No sustituye
/// `var(...)` ni valida la propiedad — eso lo hará el runtime de tween
/// cuando exista (Fase B4+).
fn parse_keyframe_declarations(inner: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for chunk in inner.split(';') {
        let Some((prop, value)) = chunk.split_once(':') else {
            continue;
        };
        let prop = prop.trim();
        let value = value.trim();
        if prop.is_empty() || value.is_empty() {
            continue;
        }
        out.push((prop.to_ascii_lowercase(), value.to_string()));
    }
    out
}

/// Parsea una duración CSS (`2s`, `200ms`, `0.3s`) a segundos. `0` sin
/// unidad → 0.0. Sin unidad reconocida → None (así un token numérico puro
/// no se confunde con una duración al clasificar el shorthand).
fn parse_time(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix("ms") {
        return num.trim().parse::<f32>().ok().map(|v| v / 1000.0);
    }
    if let Some(num) = s.strip_suffix('s') {
        return num.trim().parse::<f32>().ok();
    }
    if s == "0" {
        return Some(0.0);
    }
    None
}

/// Parsea una `<timing-function>`: keywords (`ease`/`linear`/`ease-in`/
/// `ease-out`/`ease-in-out`/`step-start`/`step-end`), `cubic-bezier(...)`
/// y `steps(n, term)`. None si no encaja.
fn parse_easing(s: &str) -> Option<EasingFunction> {
    let t = s.trim().to_ascii_lowercase();
    match t.as_str() {
        "linear" => return Some(EasingFunction::Linear),
        "ease" => return Some(EasingFunction::Ease),
        "ease-in" => return Some(EasingFunction::EaseIn),
        "ease-out" => return Some(EasingFunction::EaseOut),
        "ease-in-out" => return Some(EasingFunction::EaseInOut),
        "step-start" => return Some(EasingFunction::StepStart),
        "step-end" => return Some(EasingFunction::StepEnd),
        _ => {}
    }
    if let Some(args) = t.strip_prefix("cubic-bezier(").and_then(|r| r.strip_suffix(')')) {
        let nums: Vec<f32> = args.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        if nums.len() == 4 {
            return Some(EasingFunction::CubicBezier(nums[0], nums[1], nums[2], nums[3]));
        }
        return None;
    }
    if let Some(args) = t.strip_prefix("steps(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = args.split(',').map(|p| p.trim()).collect();
        let n: u32 = parts.first()?.parse().ok()?;
        let jump_start = parts
            .get(1)
            .map(|p| *p == "start" || *p == "jump-start")
            .unwrap_or(false);
        return Some(EasingFunction::Steps(n, jump_start));
    }
    None
}

/// Tokeniza un value por whitespace de nivel superior, respetando
/// paréntesis: `cubic-bezier(.1, .2, .3, .4)` queda como un único token.
fn split_top_level_ws(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut depth: i32 = 0;
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Separa por comas de nivel superior, respetando paréntesis. Usado para
/// las listas de `transition`/`animation` múltiples.
fn split_top_level_comma(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth: i32 = 0;
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// `animation: <name> <duration> <timing> <delay> <iteration> <direction>
/// <fill>`. Clasifica cada token por forma, no por posición. `none` →
/// `Animation(None)`. Lista separada por coma → nos quedamos con la
/// primera animación parseable (no hay runtime multi-animación todavía).
fn parse_animation(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(DeclKind::Animation(None));
    }
    let seg = split_top_level_comma(v).into_iter().next()?;
    Some(DeclKind::Animation(parse_one_animation(&seg)))
}

fn parse_one_animation(seg: &str) -> Option<AnimationBinding> {
    let tokens = split_top_level_ws(seg.trim());
    if tokens.is_empty() {
        return None;
    }
    let mut name: Option<String> = None;
    let mut duration: Option<f32> = None;
    let mut delay: Option<f32> = None;
    let mut timing: Option<EasingFunction> = None;
    let mut iterations: Option<AnimationIterations> = None;
    let mut direction: Option<AnimationDirection> = None;
    let mut fill: Option<AnimationFillMode> = None;
    let mut play_state: Option<AnimationPlayState> = None;
    for tok in &tokens {
        let lt = tok.to_ascii_lowercase();
        // Duración primero, delay después (orden posicional de los dos
        // valores de tiempo — único caso donde la posición importa).
        if let Some(t) = parse_time(tok) {
            if duration.is_none() {
                duration = Some(t);
            } else if delay.is_none() {
                delay = Some(t);
            }
            continue;
        }
        if lt == "infinite" {
            iterations = Some(AnimationIterations::Infinite);
            continue;
        }
        // Número puro sin unidad → iteration-count (`parse_time` ya
        // descartó los que llevan `s`/`ms`).
        if let Ok(n) = lt.parse::<f32>() {
            iterations = Some(AnimationIterations::Count(n));
            continue;
        }
        if timing.is_none() {
            if let Some(e) = parse_easing(&lt) {
                timing = Some(e);
                continue;
            }
        }
        match lt.as_str() {
            "normal" => {
                direction = Some(AnimationDirection::Normal);
                continue;
            }
            "reverse" => {
                direction = Some(AnimationDirection::Reverse);
                continue;
            }
            "alternate" => {
                direction = Some(AnimationDirection::Alternate);
                continue;
            }
            "alternate-reverse" => {
                direction = Some(AnimationDirection::AlternateReverse);
                continue;
            }
            "forwards" => {
                fill = Some(AnimationFillMode::Forwards);
                continue;
            }
            "backwards" => {
                fill = Some(AnimationFillMode::Backwards);
                continue;
            }
            "both" => {
                fill = Some(AnimationFillMode::Both);
                continue;
            }
            "running" => {
                play_state = Some(AnimationPlayState::Running);
                continue;
            }
            "paused" => {
                play_state = Some(AnimationPlayState::Paused);
                continue;
            }
            // `none` acá sería `animation-name: none` o `fill-mode: none` —
            // ambiguo y raro en shorthand; lo tratamos como "sin nombre".
            "none" => continue,
            _ => {}
        }
        if name.is_none() {
            name = Some(tok.clone());
        }
    }
    let name = name?;
    Some(AnimationBinding {
        name,
        duration_s: duration.unwrap_or(0.0),
        timing: timing.unwrap_or_default(),
        delay_s: delay.unwrap_or(0.0),
        iterations: iterations.unwrap_or(AnimationIterations::Count(1.0)),
        direction: direction.unwrap_or(AnimationDirection::Normal),
        fill_mode: fill.unwrap_or(AnimationFillMode::None),
        play_state: play_state.unwrap_or(AnimationPlayState::Running),
    })
}

/// `transition: <property> <duration> <timing> <delay>`. Lista separada
/// por coma → varios bindings. `none` → lista vacía.
fn parse_transition(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(DeclKind::Transitions(Vec::new()));
    }
    let mut out = Vec::new();
    for seg in split_top_level_comma(v) {
        if let Some(b) = parse_one_transition(&seg) {
            out.push(b);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(DeclKind::Transitions(out))
    }
}

fn parse_one_transition(seg: &str) -> Option<TransitionBinding> {
    let tokens = split_top_level_ws(seg.trim());
    if tokens.is_empty() {
        return None;
    }
    let mut property: Option<String> = None;
    let mut duration: Option<f32> = None;
    let mut delay: Option<f32> = None;
    let mut timing: Option<EasingFunction> = None;
    for tok in &tokens {
        let lt = tok.to_ascii_lowercase();
        if let Some(t) = parse_time(tok) {
            if duration.is_none() {
                duration = Some(t);
            } else if delay.is_none() {
                delay = Some(t);
            }
            continue;
        }
        if timing.is_none() {
            if let Some(e) = parse_easing(&lt) {
                timing = Some(e);
                continue;
            }
        }
        // El primer token que no es tiempo ni easing es la propiedad
        // (`opacity`, `transform`, `all`, `background-color`...).
        if property.is_none() {
            property = Some(lt);
        }
    }
    Some(TransitionBinding {
        property: property.unwrap_or_else(|| "all".to_string()),
        duration_s: duration.unwrap_or(0.0),
        timing: timing.unwrap_or_default(),
        delay_s: delay.unwrap_or(0.0),
    })
}

/// Reemplaza `var(--name)` y `var(--name, fallback)` en `value` por el
/// valor recogido en `vars`. Si la variable no existe y hay fallback, lo
/// usa; sino, sustituye por cadena vacía. La sustitución es recursiva
/// (un value de var puede a su vez contener `var(...)`).
fn substitute_vars(value: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("var(") {
        out.push_str(&rest[..start]);
        let inside_start = start + 4;
        // Buscar el `)` que cierra, respetando nesting de paréntesis
        // (para tolerar `var(--x, calc(1px))` aunque no parseemos calc).
        let mut depth = 1usize;
        let bytes = rest[inside_start..].as_bytes();
        let mut close_pos: Option<usize> = None;
        for (i, &c) in bytes.iter().enumerate() {
            match c {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close_pos = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close_pos else {
            // Paréntesis colgado — devolvemos lo que quedaba pegado.
            out.push_str(&rest[start..]);
            return out;
        };
        let args = &rest[inside_start..inside_start + close];
        let (name, fallback) = match args.split_once(',') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (args.trim(), None),
        };
        let var_name = name.strip_prefix("--").unwrap_or("");
        let replacement = vars
            .get(var_name)
            .cloned()
            .or_else(|| fallback.map(|s| s.to_string()))
            .unwrap_or_default();
        // Recursión: el value resuelto puede contener más var().
        out.push_str(&substitute_vars(&replacement, vars));
        rest = &rest[inside_start + close + 1..];
    }
    out.push_str(rest);
    out
}

/// Parsea un selector encadenado. Soporta:
/// - simples compound: `*`, `tag`, `.class`, `#id`, `a.btn`, `p#hero.alert`
/// - selectores de atributo: `[href]`, `[type="text"]`, `[href^="https"]`,
///   `[src$=".png"]`, `[class*="foo"]`
/// - pseudoclases estructurales: `:first-child`, `:last-child`,
///   `:only-child`, `:first-of-type`, `:last-of-type`
/// - combinadores: descendiente (whitespace), hijo directo `>`,
///   hermano adyacente `+`, hermano general `~`
///
/// Pseudoclases de estado (`:hover`, `:focus`, `:active`), `:not(...)`,
/// `:nth-child(...)` y pseudo-elementos (`::before`) siguen sin soporte —
/// el selector entero se ignora si los menciona.
fn parse_selector(sel: &str) -> Option<Selector> {
    let sel = sel.trim();
    // Strip pseudo-element del final (`::before`/`::after`). CSS también
    // acepta la sintaxis legacy `:before`/`:after` con un sólo `:` —
    // las aceptamos por compatibilidad. Pueden venir adheridas al
    // último compound (`p::before`) o solas (`::before` matchea
    // implícitamente al universal).
    let (sel, pseudo_element) = strip_pseudo_element(sel);
    if sel.is_empty() {
        let compound = Compound {
            tag: TagPart::Universal,
            ids: vec![],
            classes: vec![],
            attrs: vec![],
            pseudos: vec![],
        };
        return Some(Selector {
            compounds: vec![compound],
            combinators: vec![],
            pseudo_element,
        });
    }
    // Tokenizamos: cada compound es una secuencia sin espacios ni
    // combinadores; los combinadores ('>', '+', '~') están separados por
    // whitespace en CSS canónico o pegados. Normalizamos respetando lo
    // que viva dentro de `[...]` o `(...)`.
    let normalized = normalize_combinators(sel);
    let mut compounds: Vec<Compound> = Vec::new();
    let mut combinators: Vec<Combinator> = Vec::new();
    let mut pending_combinator: Option<Combinator> = None;
    let mut first = true;
    for tok in normalized.split_whitespace() {
        match tok {
            ">" => pending_combinator = Some(Combinator::Child),
            "+" => pending_combinator = Some(Combinator::AdjacentSibling),
            "~" => pending_combinator = Some(Combinator::GeneralSibling),
            _ => {
                let compound = parse_compound(tok)?;
                if first {
                    first = false;
                } else {
                    combinators.push(pending_combinator.take().unwrap_or(Combinator::Descendant));
                }
                compounds.push(compound);
            }
        }
    }
    if compounds.is_empty() {
        return None;
    }
    if pending_combinator.is_some() {
        return None;
    }
    Some(Selector { compounds, combinators, pseudo_element })
}

/// Si `sel` termina con `::before`/`::after` (o legacy `:before`/`:after`),
/// devuelve `(prefix, Some(PseudoElement))`. Sino devuelve `(sel, None)`.
fn strip_pseudo_element(sel: &str) -> (&str, Option<PseudoElement>) {
    let lower = sel.to_ascii_lowercase();
    for (suffix, pe) in [
        ("::before", PseudoElement::Before),
        ("::after", PseudoElement::After),
        (":before", PseudoElement::Before),
        (":after", PseudoElement::After),
    ] {
        if let Some(prefix) = lower.strip_suffix(suffix) {
            // Cuidado: `:before` no debe matchear cuando es parte de
            // `:before-leaf` (no es un pseudo válido en CSS). Pero al
            // ser sufijo exacto del string, esto no aplica acá. Sí
            // garantizamos que el prefijo no termine en alfanumérico
            // (caso `p:beforex` — el parseo falla al no encontrar
            // pseudoclase válida y lo rechazamos abajo). Acá basta.
            return (&sel[..prefix.len()], Some(pe));
        }
    }
    (sel, None)
}

/// Inserta espacios alrededor de `>`/`+`/`~` para que `split_whitespace`
/// los aísle como tokens propios. Si caen dentro de `[…]` o `(…)` los
/// dejamos intactos — `[href*="a>b"]` o `:not(a+b)` deben pasar al
/// compound parser sin romperse.
fn normalize_combinators(sel: &str) -> String {
    let mut out = String::with_capacity(sel.len() + 4);
    let mut in_bracket = false;
    let mut paren_depth: u32 = 0;
    for c in sel.chars() {
        match c {
            '[' => {
                in_bracket = true;
                out.push(c);
            }
            ']' => {
                in_bracket = false;
                out.push(c);
            }
            '(' => {
                paren_depth += 1;
                out.push(c);
            }
            ')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                out.push(c);
            }
            '>' | '+' | '~' if !in_bracket && paren_depth == 0 => {
                out.push(' ');
                out.push(c);
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// Parsea un compound: opcional tag/`*` seguido de cualquier número de
/// `.class`, `#id`, `[attr...]`, `:pseudo`. Devuelve `None` si encuentra
/// caracteres no esperados, una pseudo no soportada, o `::pseudo-element`.
fn parse_compound(sel: &str) -> Option<Compound> {
    let bytes = sel.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0;
    // Tag opcional (puede ser `*` o un nombre).
    let tag = if bytes[0] == b'*' {
        i = 1;
        TagPart::Universal
    } else if is_ident_byte(bytes[0]) {
        let start = i;
        while i < bytes.len() && is_ident_byte(bytes[i]) {
            i += 1;
        }
        TagPart::Type(sel[start..i].to_string())
    } else {
        TagPart::Universal
    };
    let mut ids = Vec::new();
    let mut classes = Vec::new();
    let mut attrs = Vec::new();
    let mut pseudos = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'.' | b'#' => {
                let marker = bytes[i];
                i += 1;
                let start = i;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let ident = sel[start..i].to_string();
                if marker == b'.' {
                    classes.push(ident);
                } else {
                    ids.push(ident);
                }
            }
            b'[' => {
                let inner_start = i + 1;
                let rel_close = sel[inner_start..].find(']')?;
                let inner = &sel[inner_start..inner_start + rel_close];
                attrs.push(parse_attr_match(inner)?);
                i = inner_start + rel_close + 1;
            }
            b':' => {
                i += 1;
                // `::pseudo-element` (e.g. ::before) — rechazamos.
                if i < bytes.len() && bytes[i] == b':' {
                    return None;
                }
                let start = i;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let name = sel[start..i].to_ascii_lowercase();
                // Funcionales: `:nth-child(...)`, `:not(...)`. Detectamos
                // y consumimos los argumentos.
                if i < bytes.len() && bytes[i] == b'(' {
                    let arg_start = i + 1;
                    let rel_close = sel[arg_start..].find(')')?;
                    let arg = &sel[arg_start..arg_start + rel_close];
                    let p = match name.as_str() {
                        "nth-child" => {
                            let (a, b) = parse_nth_arg(arg)?;
                            Pseudo::NthChild { a, b }
                        }
                        "not" => {
                            let inner = parse_compound(arg)?;
                            // Anti-recursión: `:not(:not(...))` rechazamos.
                            if inner
                                .pseudos
                                .iter()
                                .any(|p| matches!(p, Pseudo::Not(_)))
                            {
                                return None;
                            }
                            Pseudo::Not(Box::new(inner))
                        }
                        _ => return None,
                    };
                    pseudos.push(p);
                    i = arg_start + rel_close + 1;
                    continue;
                }
                let p = match name.as_str() {
                    "first-child" => Pseudo::FirstChild,
                    "last-child" => Pseudo::LastChild,
                    "only-child" => Pseudo::OnlyChild,
                    "first-of-type" => Pseudo::FirstOfType,
                    "last-of-type" => Pseudo::LastOfType,
                    "hover" => Pseudo::Hover,
                    "focus" | "focus-visible" | "focus-within" => Pseudo::Focus,
                    _ => return None,
                };
                pseudos.push(p);
            }
            _ => return None,
        }
    }
    if matches!(tag, TagPart::Universal)
        && ids.is_empty()
        && classes.is_empty()
        && attrs.is_empty()
        && pseudos.is_empty()
        && sel != "*"
    {
        return None;
    }
    Some(Compound { tag, ids, classes, attrs, pseudos })
}

/// Parsea el interior de `[...]`: `name`, `name=val`, `name="val"`,
/// `name^=val`, `name$=val`, `name*=val`. Devuelve `None` si el formato
/// no encaja.
fn parse_attr_match(inner: &str) -> Option<AttrMatch> {
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    let ops: &[(&str, AttrOp)] = &[
        ("^=", AttrOp::Prefix),
        ("$=", AttrOp::Suffix),
        ("*=", AttrOp::Contains),
        ("=", AttrOp::Equals),
    ];
    for (sym, op) in ops {
        if let Some(pos) = inner.find(sym) {
            let name = inner[..pos].trim().to_string();
            if name.is_empty() {
                return None;
            }
            let raw = inner[pos + sym.len()..].trim();
            let value = raw.trim_matches(|c| c == '"' || c == '\'').to_string();
            return Some(AttrMatch { name, op: *op, value });
        }
    }
    Some(AttrMatch {
        name: inner.to_string(),
        op: AttrOp::Present,
        value: String::new(),
    })
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn strip_comments(css: &str) -> String {
    // Operamos a nivel byte para detectar `/*…*/`, pero copiamos slices
    // de la `&str` original para preservar UTF-8 multi-byte (un push de
    // bytes individuales `as char` rompe runs no-ASCII como "▸").
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    let mut chunk_start = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // Volcamos el chunk pendiente antes del comentario.
            out.push_str(&css[chunk_start..i]);
            if let Some(end) = css[i + 2..].find("*/") {
                i += 2 + end + 2;
                chunk_start = i;
                continue;
            }
            return out;
        }
        i += 1;
    }
    out.push_str(&css[chunk_start..]);
    out
}

fn parse_declarations(css: &str, vars: &HashMap<String, String>) -> Vec<Decl> {
    // Cada decl separada por `;`. Detectamos `!important` recortando
    // el sufijo del value antes de pasarlo al parser de tipo. La
    // shorthand `border:` se expande inline a 1..3 decls atómicas.
    let mut out = Vec::new();
    for chunk in css.split(';') {
        let Some((prop, value)) = chunk.split_once(':') else {
            continue;
        };
        let prop = prop.trim();
        // Las declaraciones de variables (`--name: value`) ya se
        // recogieron en la pasada de `extract_root_vars`. Acá las
        // saltamos para no intentar parsearlas como propiedades reales.
        if prop.starts_with("--") {
            continue;
        }
        let value = value.trim();
        let (value, important) = match strip_important(value) {
            Some(stripped) => (stripped, true),
            None => (value, false),
        };
        // Sustituye `var(--name)` antes de parsear. `substitute_vars` es
        // cheap si el value no contiene `var(` (early-out al primer find).
        let substituted = substitute_vars(value, vars);
        let value = substituted.as_str();
        if prop.eq_ignore_ascii_case("border") {
            out.extend(parse_border_shorthand(value, important));
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "") {
            out.extend(parse_border_side_shorthand(edge, value, important));
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "-width") {
            if let Some(w) = parse_length_px(value) {
                out.push(Decl { kind: DeclKind::BorderSideWidth(edge, w), important });
            }
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "-color") {
            if let Some(c) = parse_color(value) {
                out.push(Decl { kind: DeclKind::BorderSideColor(edge, c), important });
            }
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "-style") {
            if let Some(s) = parse_border_style(value) {
                out.push(Decl { kind: DeclKind::BorderSideStyle(edge, s), important });
            }
            continue;
        }
        if let Some(corner) = match_border_corner_prop(prop) {
            if let Some(r) = parse_length_px(value) {
                out.push(Decl { kind: DeclKind::BorderCornerRadius(corner, r), important });
            }
            continue;
        }
        if prop.eq_ignore_ascii_case("flex") {
            out.extend(parse_flex_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("outline") {
            out.extend(parse_outline_shorthand(value, important));
            continue;
        }
        if let Some(kind) = decl_kind_from_pair(prop, value) {
            out.push(Decl { kind, important });
        }
    }
    out
}

/// Si `value` termina en `!important` (con o sin espacios), devuelve la
/// porción antes del bang. Sino, `None`.
fn strip_important(value: &str) -> Option<&str> {
    let v = value.trim_end();
    if v.len() < "!important".len() {
        return None;
    }
    let tail = &v[v.len() - "!important".len()..];
    if tail.eq_ignore_ascii_case("!important") {
        Some(v[..v.len() - "!important".len()].trim_end())
    } else {
        None
    }
}

fn decl_kind_from_pair(prop: &str, value: &str) -> Option<DeclKind> {
    match prop.to_ascii_lowercase().as_str() {
        "color" => parse_color(value).map(DeclKind::Color),
        "background-color" | "background" => parse_color(value).map(DeclKind::Background),
        "display" => parse_display(value).map(DeclKind::Display),
        "font-size" => parse_length_px(value).map(DeclKind::FontSize),
        "font-weight" => parse_weight(value).map(DeclKind::FontWeight),
        "font-style" => parse_font_style(value).map(DeclKind::FontStyle),
        "font-family" => Some(DeclKind::FontFamily(value.trim().to_string())),
        "margin" => parse_sides(value).map(DeclKind::Margin),
        "margin-top" => parse_length_px(value).map(DeclKind::MarginTop),
        "margin-right" => parse_length_px(value).map(DeclKind::MarginRight),
        "margin-bottom" => parse_length_px(value).map(DeclKind::MarginBottom),
        "margin-left" => parse_length_px(value).map(DeclKind::MarginLeft),
        "padding" => parse_sides(value).map(DeclKind::Padding),
        "padding-top" => parse_length_px(value).map(DeclKind::PaddingTop),
        "padding-right" => parse_length_px(value).map(DeclKind::PaddingRight),
        "padding-bottom" => parse_length_px(value).map(DeclKind::PaddingBottom),
        "padding-left" => parse_length_px(value).map(DeclKind::PaddingLeft),
        "width" => parse_length_or_pct(value).map(DeclKind::Width),
        "max-width" => parse_length_or_pct(value).map(DeclKind::MaxWidth),
        "text-align" => parse_text_align(value).map(DeclKind::TextAlign),
        "line-height" => parse_line_height(value).map(DeclKind::LineHeight),
        "border-width" => parse_length_px(value).map(DeclKind::BorderWidth),
        "border-color" => parse_color(value).map(DeclKind::BorderColor),
        "border-style" => parse_border_style(value).map(DeclKind::BorderEnabled),
        "border-radius" => parse_length_px(value).map(DeclKind::BorderRadius),
        "z-index" => {
            // `auto` → 0; sino int. Negativos OK.
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ZIndex(0))
            } else {
                v.parse::<i32>().ok().map(DeclKind::ZIndex)
            }
        }
        "content" => Some(DeclKind::Content(parse_content_value(value))),
        "counter-reset" => Some(DeclKind::CounterReset(parse_counter_list(value, 0))),
        "counter-increment" => Some(DeclKind::CounterIncrement(parse_counter_list(value, 1))),
        "box-shadow" => Some(DeclKind::BoxShadow(parse_box_shadow(value))),
        "text-decoration" | "text-decoration-line" => {
            parse_text_decoration(value).map(DeclKind::TextDecoration)
        }
        "list-style-type" => parse_list_style_type(value).map(DeclKind::ListStyleType),
        // `list-style` shorthand reducido: sólo capturamos el `-type`.
        // Image y position los ignoramos — `none` desactiva el marker
        // entero (matchea el comportamiento del browser).
        "list-style" => parse_list_style_shorthand(value).map(DeclKind::ListStyleType),
        "flex-direction" => parse_flex_direction(value).map(DeclKind::FlexDirection),
        "flex-wrap" => parse_flex_wrap(value).map(DeclKind::FlexWrap),
        "justify-content" => parse_justify_content(value).map(DeclKind::JustifyContent),
        "align-items" => parse_align_items(value).map(DeclKind::AlignItems),
        "gap" => parse_gap(value).map(|(r, c)| DeclKind::Gap { row: r, column: c }),
        "row-gap" => parse_length_px(value).map(DeclKind::RowGap),
        "column-gap" => parse_length_px(value).map(DeclKind::ColumnGap),
        "box-sizing" => parse_box_sizing(value).map(DeclKind::BoxSizing),
        "min-width" => parse_length_or_pct(value).map(DeclKind::MinWidth),
        "min-height" => parse_length_or_pct(value).map(DeclKind::MinHeight),
        "max-height" => parse_length_or_pct(value).map(DeclKind::MaxHeight),
        "overflow" | "overflow-x" | "overflow-y" => {
            parse_overflow(value).map(DeclKind::Overflow)
        }
        "white-space" => parse_white_space(value).map(DeclKind::WhiteSpace),
        "text-transform" => parse_text_transform(value).map(DeclKind::TextTransform),
        "opacity" => parse_opacity(value).map(DeclKind::Opacity),
        "align-self" => parse_align_self(value).map(DeclKind::AlignSelf),
        "flex-grow" => value.trim().parse::<f32>().ok().map(DeclKind::FlexGrow),
        "flex-shrink" => value.trim().parse::<f32>().ok().map(DeclKind::FlexShrink),
        "flex-basis" => parse_length_or_pct(value).map(DeclKind::FlexBasis),
        // `flex` y `outline` son shorthands múltiples — se expanden en
        // `parse_declarations` antes de llegar acá.
        "flex" | "outline" => None,
        "outline-width" => parse_length_px(value).map(DeclKind::OutlineWidth),
        "outline-color" => parse_color(value).map(DeclKind::OutlineColor),
        "outline-style" => parse_border_style(value).map(DeclKind::OutlineStyle),
        "outline-offset" => parse_length_px(value).map(DeclKind::OutlineOffset),
        "background-image" => parse_background_image(value),
        "position" => parse_position(value).map(DeclKind::Position),
        "top" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetTop),
        "right" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetRight),
        "bottom" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetBottom),
        "left" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetLeft),
        "vertical-align" => parse_vertical_align(value).map(DeclKind::VerticalAlign),
        "visibility" => parse_visibility(value).map(DeclKind::Visibility),
        "pointer-events" => parse_pointer_events(value).map(DeclKind::PointerEvents),
        "text-indent" => parse_length_px(value).map(DeclKind::TextIndent),
        "word-spacing" => parse_length_px(value).map(DeclKind::WordSpacing),
        "text-shadow" => parse_text_shadows(value).map(DeclKind::TextShadows),
        "transform" => parse_transforms(value).map(DeclKind::Transforms),
        "grid-template-columns" => {
            parse_grid_template(value).map(DeclKind::GridTemplateColumns)
        }
        "grid-template-rows" => parse_grid_template(value).map(DeclKind::GridTemplateRows),
        "animation" => parse_animation(value),
        "transition" => parse_transition(value),
        // `grid-gap` (legacy) = `gap`.
        "grid-gap" => parse_gap(value).map(|(r, c)| DeclKind::Gap { row: r, column: c }),
        "grid-row-gap" => parse_length_px(value).map(DeclKind::RowGap),
        "grid-column-gap" => parse_length_px(value).map(DeclKind::ColumnGap),
        // `border: 1px solid #ccc` — shorthand. Devolvemos un único
        // DeclKind sintético: en realidad ya hay 3 sub-decls que el
        // caller debe emitir, así que delegamos a una ruta especial vía
        // parse_declarations (ver más arriba). Acá no podemos producir
        // varios, así que ignoramos — la entrada se rellena en
        // parse_declarations cuando ve `border`.
        "border" => None,
        _ => None,
    }
}

/// Parsea el argumento de `:nth-child(...)`. Soporta:
/// - palabras clave: `odd` (= `2n+1`), `even` (= `2n`)
/// - número entero: `3` → `(0, 3)` (sólo la 3a)
/// - `n` → `(1, 0)` (todos), `-n` → `(-1, 0)`
/// - `an` → `(a, 0)`; `an+b` y `an-b` → `(a, ±b)`
/// - `-n+b` → `(-1, b)`
///
/// Devuelve `Some((a, b))` o `None` si el formato no encaja.
fn parse_nth_arg(arg: &str) -> Option<(i32, i32)> {
    let s: String = arg.chars().filter(|c| !c.is_whitespace()).collect();
    let s = s.to_ascii_lowercase();
    if s == "odd" {
        return Some((2, 1));
    }
    if s == "even" {
        return Some((2, 0));
    }
    // Caso entero puro: "3" o "-3".
    if let Ok(n) = s.parse::<i32>() {
        return Some((0, n));
    }
    // Buscar la 'n' que separa coeficiente de constante.
    let n_pos = s.find('n')?;
    let coeff_str = &s[..n_pos];
    let rest = &s[n_pos + 1..];
    let a: i32 = match coeff_str {
        "" => 1,
        "-" => -1,
        "+" => 1,
        other => other.parse().ok()?,
    };
    let b: i32 = if rest.is_empty() { 0 } else { rest.parse().ok()? };
    Some((a, b))
}

/// Parsea `box-shadow: <offset-x> <offset-y> [blur] [spread] <color>`
/// o `box-shadow: none`. Devuelve `None` (= no-shadow) si:
/// - value es exactamente `none`, o
/// - falta el offset-x/offset-y, o
/// - no se reconoce el color.
///
/// `inset` y múltiples sombras separadas por coma no soportadas — el
/// resto del declaration se ignora silenciosamente.
fn parse_box_shadow(value: &str) -> Option<BoxShadow> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") || v.is_empty() {
        return None;
    }
    // Toma sólo la primera sombra (si hay coma).
    let first = v.split(',').next().unwrap_or(v).trim();
    let mut lengths: Vec<f32> = Vec::with_capacity(4);
    let mut color: Option<Color> = None;
    for tok in first.split_whitespace() {
        if tok.eq_ignore_ascii_case("inset") {
            // No soportado todavía — abortamos.
            return None;
        }
        if let Some(l) = parse_length_px(tok) {
            lengths.push(l);
            continue;
        }
        if let Some(c) = parse_color(tok) {
            color = Some(c);
            continue;
        }
    }
    if lengths.len() < 2 {
        return None;
    }
    Some(BoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_px: lengths.get(2).copied().unwrap_or(0.0),
        spread_px: lengths.get(3).copied().unwrap_or(0.0),
        color: color.unwrap_or(Color::rgb(0, 0, 0)),
    })
}

fn parse_border_style(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "solid" | "dashed" | "dotted" | "double" => Some(true),
        "none" | "hidden" => Some(false),
        _ => None,
    }
}

/// Parsea el shorthand `border: <width> <style> <color>` (componentes en
/// cualquier orden). Devuelve hasta 3 decls. Si falta el style, se asume
/// `solid`. Cualquier "none" en la posición de style desactiva el border.
fn parse_border_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut width: Option<f32> = None;
    let mut color: Option<Color> = None;
    let mut style_on: Option<bool> = None;
    for tok in value.split_whitespace() {
        if let Some(w) = parse_length_px(tok) {
            width = Some(w);
            continue;
        }
        if let Some(c) = parse_color(tok) {
            color = Some(c);
            continue;
        }
        if let Some(s) = parse_border_style(tok) {
            style_on = Some(s);
            continue;
        }
    }
    // Defaults razonables: si hay width+color sin style, asumimos solid.
    if style_on.is_none() && (width.is_some() || color.is_some()) {
        style_on = Some(true);
    }
    let mut out = Vec::new();
    if let Some(on) = style_on {
        out.push(Decl { kind: DeclKind::BorderEnabled(on), important });
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::BorderWidth(w), important });
    }
    if let Some(c) = color {
        out.push(Decl { kind: DeclKind::BorderColor(c), important });
    }
    out
}

/// Match propiedades `border-{top|right|bottom|left}{suffix}`. `suffix`
/// puede ser "" (shorthand), "-width", "-color", o "-style". Devuelve
/// el `BorderEdge` matcheado, o `None` si no aplica.
fn match_border_side_prop(prop: &str, suffix: &str) -> Option<BorderEdge> {
    let lc = prop.to_ascii_lowercase();
    for (name, edge) in [
        ("border-top", BorderEdge::Top),
        ("border-right", BorderEdge::Right),
        ("border-bottom", BorderEdge::Bottom),
        ("border-left", BorderEdge::Left),
    ] {
        if lc.len() == name.len() + suffix.len()
            && lc.starts_with(name)
            && lc[name.len()..].eq_ignore_ascii_case(suffix)
        {
            return Some(edge);
        }
    }
    None
}

/// Match propiedades `border-{top|bottom}-{left|right}-radius`.
fn match_border_corner_prop(prop: &str) -> Option<BorderCorner> {
    match prop.to_ascii_lowercase().as_str() {
        "border-top-left-radius" => Some(BorderCorner::TopLeft),
        "border-top-right-radius" => Some(BorderCorner::TopRight),
        "border-bottom-right-radius" => Some(BorderCorner::BottomRight),
        "border-bottom-left-radius" => Some(BorderCorner::BottomLeft),
        _ => None,
    }
}

/// Shorthand `border-top: <width> <style> <color>` (componentes en
/// cualquier orden, sólo afecta a un lado). Mismo formato que `border:`
/// pero las decls resultantes son las variantes per-side.
fn parse_border_side_shorthand(edge: BorderEdge, value: &str, important: bool) -> Vec<Decl> {
    let mut width: Option<f32> = None;
    let mut color: Option<Color> = None;
    let mut style_on: Option<bool> = None;
    for tok in value.split_whitespace() {
        if let Some(w) = parse_length_px(tok) {
            width = Some(w);
            continue;
        }
        if let Some(c) = parse_color(tok) {
            color = Some(c);
            continue;
        }
        if let Some(s) = parse_border_style(tok) {
            style_on = Some(s);
            continue;
        }
    }
    if style_on.is_none() && (width.is_some() || color.is_some()) {
        style_on = Some(true);
    }
    let mut out = Vec::new();
    if let Some(on) = style_on {
        out.push(Decl { kind: DeclKind::BorderSideStyle(edge, on), important });
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::BorderSideWidth(edge, w), important });
    }
    if let Some(c) = color {
        out.push(Decl { kind: DeclKind::BorderSideColor(edge, c), important });
    }
    out
}

/// Parsea `text-decoration` o `text-decoration-line`. Acepta el shorthand
/// con varios tokens — busca el primer keyword reconocido como line y
/// devuelve eso. Estilos (`dotted`/`wavy`) y color se ignoran (sólo
/// pintamos línea sólida del color del texto).
fn parse_text_decoration(value: &str) -> Option<TextDecorationLine> {
    for tok in value.split_whitespace() {
        match tok.to_ascii_lowercase().as_str() {
            "none" => return Some(TextDecorationLine::None),
            "underline" => return Some(TextDecorationLine::Underline),
            "line-through" => return Some(TextDecorationLine::LineThrough),
            "overline" => return Some(TextDecorationLine::Overline),
            _ => {}
        }
    }
    None
}

/// Parsea `list-style-type: <keyword>`. Acepta los aliases comunes
/// (`lower-latin` = `lower-alpha`, `upper-latin` = `upper-alpha`).
/// Keywords no soportados (`georgian`, `hebrew`, …) caen a `None` y la
/// declaración se ignora — el caller mantiene el valor anterior.
fn parse_list_style_type(s: &str) -> Option<ListStyleType> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(ListStyleType::None),
        "disc" => Some(ListStyleType::Disc),
        "circle" => Some(ListStyleType::Circle),
        "square" => Some(ListStyleType::Square),
        "decimal" => Some(ListStyleType::Decimal),
        "lower-alpha" | "lower-latin" => Some(ListStyleType::LowerAlpha),
        "upper-alpha" | "upper-latin" => Some(ListStyleType::UpperAlpha),
        "lower-roman" => Some(ListStyleType::LowerRoman),
        "upper-roman" => Some(ListStyleType::UpperRoman),
        _ => None,
    }
}

/// Shorthand `list-style: [type] [position] [image]` muy reducido. Sólo
/// extraemos el primer token que matchee un `-type` keyword. `list-style:
/// none` desactiva el marker (matchea browsers — `none` ahí setea ambos
/// `-type` e `-image` a none, y como no tenemos `-image`, alcanza con
/// poner `-type` en `None`).
fn parse_list_style_shorthand(s: &str) -> Option<ListStyleType> {
    for tok in s.split_whitespace() {
        if let Some(t) = parse_list_style_type(tok) {
            return Some(t);
        }
    }
    None
}

fn parse_text_align(s: &str) -> Option<TextAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => Some(TextAlign::Left),
        "center" => Some(TextAlign::Center),
        "right" | "end" => Some(TextAlign::Right),
        "justify" => Some(TextAlign::Justify),
        _ => None,
    }
}

/// Acepta `auto`, `Npx`, `Nrem`/`Nem` (→ px), `N%`. Sin unidad y
/// distinto de `0` → falla (a diferencia de `parse_length_px`, que
/// asume px).
fn parse_length_or_pct(s: &str) -> Option<LengthVal> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") {
        return Some(LengthVal::Auto);
    }
    if let Some(num) = s.strip_suffix('%') {
        return num.trim().parse::<f32>().ok().map(LengthVal::Pct);
    }
    if let Some(inner) = strip_calc(s) {
        return parse_calc_expr(inner);
    }
    parse_length_px(s).map(LengthVal::Px)
}

/// Parsea el value de `content:` para pseudo-elements. Soporta una
/// secuencia de items separados por whitespace: strings quoted,
/// `counter(name)` y `attr(name)`. Devuelve `None` para `none`/`normal`
/// (que suprime el pseudo-element) o si encuentra algo no reconocible.
fn parse_content_value(value: &str) -> Option<Vec<ContentItem>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("normal") {
        return None;
    }
    let mut items = Vec::new();
    let mut chars = v.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '"' || c == '\'' {
            let item = parse_string_literal(&mut chars)?;
            items.push(ContentItem::Text(item));
            continue;
        }
        // Identificador: `counter(...)` o `attr(...)` (case-insensitive).
        let ident = read_ident(&mut chars);
        if ident.is_empty() {
            return None;
        }
        let lower = ident.to_ascii_lowercase();
        // Comer paréntesis de apertura.
        if chars.next() != Some('(') {
            return None;
        }
        let arg = read_until(&mut chars, ')')?;
        let arg = arg.trim();
        // `counter(name[, list-style])`: nos quedamos con el name; el
        // list-style queda para más adelante.
        let name = arg.split(',').next().unwrap_or("").trim();
        if name.is_empty() {
            return None;
        }
        match lower.as_str() {
            "counter" => items.push(ContentItem::Counter(name.to_string())),
            "attr" => items.push(ContentItem::Attr(name.to_string())),
            "url" => {
                // El arg de url() puede venir entre comillas o sin.
                // arg ya fue trimmeado del paréntesis exterior; acá
                // strippeamos comillas si las hay y devolvemos el resto
                // sin trim adicional (las URLs pueden tener espacios
                // encodeados pero no whitespace literal interno).
                let raw = arg.trim();
                let clean = raw
                    .trim_start_matches(['"', '\''].as_ref())
                    .trim_end_matches(['"', '\''].as_ref())
                    .trim()
                    .to_string();
                if clean.is_empty() {
                    return None;
                }
                items.push(ContentItem::Url(clean));
            }
            _ => return None, // `counters(...)` no soportado aún.
        }
    }
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

/// Lee una string literal (incluyendo las comillas) de `chars` —
/// consume hasta encontrar la comilla de cierre matching. Soporta
/// escape `\X` que vuelca X tal cual. Devuelve None si la string queda
/// sin cerrar.
fn parse_string_literal(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<String> {
    let quote = chars.next()?;
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(esc) = chars.next() {
                out.push(esc);
                continue;
            }
            return None;
        }
        if c == quote {
            return Some(out);
        }
        out.push(c);
    }
    None
}

/// Lee chars mientras sean alfanuméricos, `-` o `_`. Devuelve el ident
/// como String (vacío si el siguiente char no era válido).
fn read_ident(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut out = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
            chars.next();
        } else {
            break;
        }
    }
    out
}

/// Lee chars hasta el delimitador `end` (exclusivo) — lo consume. Devuelve
/// el contenido. None si no encuentra el delim.
fn read_until(chars: &mut std::iter::Peekable<std::str::Chars>, end: char) -> Option<String> {
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == end {
            return Some(out);
        }
        out.push(c);
    }
    None
}

/// Parsea `counter-reset` o `counter-increment`. Devuelve pares
/// `(name, value)` — para reset el default es `0`, para increment es
/// `1`. Si el value es `none`, devuelve vec vacío.
fn parse_counter_list(value: &str, default: i32) -> Vec<(String, i32)> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    let mut out: Vec<(String, i32)> = Vec::new();
    let toks: Vec<&str> = v.split_whitespace().collect();
    let mut i = 0;
    while i < toks.len() {
        let name = toks[i];
        if !is_valid_counter_name(name) {
            // Token no nombre — skip (parser tolerante).
            i += 1;
            continue;
        }
        let value = toks
            .get(i + 1)
            .and_then(|t| t.parse::<i32>().ok());
        if let Some(v) = value {
            out.push((name.to_string(), v));
            i += 2;
        } else {
            out.push((name.to_string(), default));
            i += 1;
        }
    }
    out
}

fn is_valid_counter_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Si `s` matchea `calc(...)` (case-insensitive), devuelve el contenido
/// entre paréntesis. Sino `None`.
fn strip_calc(s: &str) -> Option<&str> {
    let lower = s.to_ascii_lowercase();
    let stripped = lower.strip_prefix("calc(")?.strip_suffix(')')?;
    // Recortamos del original (mantiene casing del inner por si tiene
    // hex colors en el futuro — hoy sólo números/units, no importa).
    let start = "calc(".len();
    Some(&s[start..s.len() - 1])
        .filter(|_| !stripped.is_empty())
}

/// Parsea un expression `calc()` mínimo: `<term> <+|-> <term>` o un
/// único `<term>`. Resuelve en parse time, conservando `Pct` cuando hay
/// mezcla (caso `calc(100% - 20px)` queda como `Pct(100)` y se pierde
/// el offset — taffy no soporta calc nativo y aproximarlo a más
/// precisión requeriría conocer el container, que no tenemos acá).
fn parse_calc_expr(inner: &str) -> Option<LengthVal> {
    let toks = tokenize_calc(inner);
    if toks.is_empty() || toks.len() % 2 == 0 {
        // Sin tokens, o longitud par (1+op+term tiene que ser impar).
        return None;
    }
    let mut acc = parse_calc_term(toks[0])?;
    let mut i = 1;
    while i + 1 < toks.len() {
        let op = toks[i];
        let rhs = parse_calc_term(toks[i + 1])?;
        acc = combine_calc(acc, op, rhs)?;
        i += 2;
    }
    Some(acc)
}

/// Tokens del calc: separamos números+unidad de operadores `+`/`-`/`*`/
/// `/`. CSS spec requiere whitespace alrededor de `+`/`-` (no de `*`/`/`).
/// Por simplicidad sólo soportamos `+` y `-` con whitespace.
fn tokenize_calc(s: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b' ' || c == b'\t' || c == b'\n' {
            if i > start {
                out.push(&s[start..i]);
            }
            // Detectar operador como token único si está rodeado de spaces.
            if i + 1 < bytes.len() && (bytes[i + 1] == b'+' || bytes[i + 1] == b'-') {
                // Skip leading spaces hasta el operador.
                let op_start = i + 1;
                if op_start + 1 < bytes.len()
                    && (bytes[op_start + 1] == b' ' || bytes[op_start + 1] == b'\t')
                {
                    out.push(&s[op_start..op_start + 1]);
                    i = op_start + 1;
                    start = i;
                    continue;
                }
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < bytes.len() {
        out.push(&s[start..]);
    }
    out
}

fn parse_calc_term(tok: &str) -> Option<LengthVal> {
    let tok = tok.trim();
    if let Some(num) = tok.strip_suffix('%') {
        return num.trim().parse::<f32>().ok().map(LengthVal::Pct);
    }
    parse_length_px(tok).map(LengthVal::Px)
}

fn combine_calc(a: LengthVal, op: &str, b: LengthVal) -> Option<LengthVal> {
    let sign = match op {
        "+" => 1.0,
        "-" => -1.0,
        _ => return None,
    };
    match (a, b) {
        (LengthVal::Px(x), LengthVal::Px(y)) => Some(LengthVal::Px(x + sign * y)),
        (LengthVal::Pct(x), LengthVal::Pct(y)) => Some(LengthVal::Pct(x + sign * y)),
        // Mezcla pct/px: conservamos el pct ignorando el offset px.
        // Aproximación pragmática — taffy no soporta calc nativo y un
        // valor mixto requeriría el container width, no disponible acá.
        (LengthVal::Pct(p), LengthVal::Px(_)) | (LengthVal::Px(_), LengthVal::Pct(p)) => {
            Some(LengthVal::Pct(p))
        }
        _ => None,
    }
}

/// Acepta multiplicador adimensional (`1.5`, `1.6`), `Npx`, `Nem`/`Nrem`.
/// Devuelve siempre un multiplicador (px se divide por 16; `em`/`rem`
/// salen como ya están). Imperfecto pero alcanza para Fase 4.
fn parse_line_height(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix("px") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v / 16.0);
    }
    if let Some(num) = s.strip_suffix("rem") {
        return num.trim().parse().ok();
    }
    if let Some(num) = s.strip_suffix("em") {
        return num.trim().parse().ok();
    }
    s.parse::<f32>().ok()
}

/// Versión pública para que `boxes` parsee colors de attrs SVG.
pub(crate) fn parse_color_named_or_hex(s: &str) -> Option<Color> {
    parse_color(s)
}

pub(crate) fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    // hex #RRGGBB / #RGB / #RRGGBBAA / #RGBA
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::rgb(r, g, b));
        }
        if hex.len() == 3 {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            return Some(Color::rgb(r, g, b));
        }
        if hex.len() == 8 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            return Some(Color { r, g, b, a });
        }
        if hex.len() == 4 {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
            return Some(Color { r, g, b, a });
        }
    }
    // rgb()/rgba() — coma legacy o whitespace moderno, con alpha por
    // 4to arg o sufijo `/ alpha`.
    if let Some(args) = strip_fn(s, "rgba").or_else(|| strip_fn(s, "rgb")) {
        return parse_rgb_func(args);
    }
    if let Some(args) = strip_fn(s, "hsla").or_else(|| strip_fn(s, "hsl")) {
        return parse_hsl_func(args);
    }
    // Nombres comunes.
    NAMED_COLORS.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, c)| *c)
}

/// Si `s` es de la forma `name(…)`, devuelve los argumentos crudos
/// (sin paréntesis). Tolera espacios entre el nombre y `(`. Match del
/// nombre case-insensitive.
fn strip_fn<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let s = s.trim();
    if !s.get(..name.len())?.eq_ignore_ascii_case(name) {
        return None;
    }
    let rest = s[name.len()..].trim_start();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.trim())
}

/// Parsea los argumentos de `rgb(…)` o `rgba(…)`. Acepta sintaxis
/// legacy (separador coma, alpha como 4to arg) y moderna (whitespace
/// + `/ alpha`). Cada canal RGB tolera entero 0-255 o porcentaje. El
/// alpha tolera fracción 0-1 o porcentaje.
fn parse_rgb_func(args: &str) -> Option<Color> {
    let (rgb, alpha) = split_color_args(args)?;
    if rgb.len() != 3 {
        return None;
    }
    let r = parse_color_chan(rgb[0])?;
    let g = parse_color_chan(rgb[1])?;
    let b = parse_color_chan(rgb[2])?;
    let a = match alpha {
        Some(a_str) => parse_alpha(a_str)?,
        None => 255,
    };
    Some(Color { r, g, b, a })
}

/// Parsea `hsl(…)` / `hsla(…)`. H = grados (0-360, se wrappea), S/L =
/// porcentaje (0-100). Alpha igual que rgba.
fn parse_hsl_func(args: &str) -> Option<Color> {
    let (parts, alpha) = split_color_args(args)?;
    if parts.len() != 3 {
        return None;
    }
    let h = parse_hue(parts[0])?;
    let s = parse_pct(parts[1])?;
    let l = parse_pct(parts[2])?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    let a = match alpha {
        Some(a_str) => parse_alpha(a_str)?,
        None => 255,
    };
    Some(Color { r, g, b, a })
}

/// Tokeniza los args de un color function. Devuelve `(canales, alpha?)`.
/// Resuelve coma vs whitespace y la sintaxis moderna `r g b / a`.
fn split_color_args(args: &str) -> Option<(Vec<&str>, Option<&str>)> {
    let args = args.trim();
    // Sintaxis moderna: `R G B / A`. La barra separa el alpha.
    if let Some(slash) = args.find('/') {
        let main = args[..slash].trim();
        let alpha = args[slash + 1..].trim();
        let parts: Vec<&str> = main.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }
        return Some((parts, Some(alpha)));
    }
    // Legacy: comas separan TODO (incluido el alpha).
    if args.contains(',') {
        let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
        if parts.len() == 4 {
            let (rgb, a) = parts.split_at(3);
            return Some((rgb.to_vec(), Some(a[0])));
        }
        return Some((parts, None));
    }
    // Moderna sin alpha: solo whitespace.
    let parts: Vec<&str> = args.split_whitespace().collect();
    Some((parts, None))
}

/// Canal RGB: entero 0-255 o porcentaje 0%-100%.
fn parse_color_chan(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct.clamp(0.0, 100.0) * 2.55).round() as u8);
    }
    s.parse::<i32>().ok().map(|n| n.clamp(0, 255) as u8)
}

/// Alpha: fracción 0.0-1.0 o porcentaje 0%-100%.
fn parse_alpha(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct.clamp(0.0, 100.0) * 2.55).round() as u8);
    }
    let f: f32 = s.parse().ok()?;
    Some((f.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Hue: `Ndeg` o número crudo (grados implícitos). `Nrad`/`Nturn` no
/// soportados — caen a `None` y la función devuelve `None`.
fn parse_hue(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix("deg").unwrap_or(s);
    s.trim().parse().ok()
}

/// Porcentaje 0%-100% → fracción 0.0-1.0.
fn parse_pct(s: &str) -> Option<f32> {
    let s = s.trim().strip_suffix('%')?;
    let pct: f32 = s.trim().parse().ok()?;
    Some((pct / 100.0).clamp(0.0, 1.0))
}

/// HSL→RGB estándar (CSS Color Module L3). h en grados, s/l en 0..1.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h_prime.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_u8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r1), to_u8(g1), to_u8(b1))
}

/// Parsea un value tipo `margin: <1..4 longitudes>`. Devuelve `None` si
/// algún token no es longitud válida o si hay menos de 1 / más de 4.
fn parse_sides(value: &str) -> Option<Sides<f32>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let parsed: Vec<f32> = parts
        .iter()
        .map(|t| parse_length_px(t))
        .collect::<Option<Vec<_>>>()?;
    Some(match parsed.as_slice() {
        [a] => Sides::all(*a),
        [v, h] => Sides { top: *v, right: *h, bottom: *v, left: *h },
        [t, h, b] => Sides { top: *t, right: *h, bottom: *b, left: *h },
        [t, r, b, l] => Sides { top: *t, right: *r, bottom: *b, left: *l },
        _ => return None,
    })
}

const NAMED_COLORS: &[(&str, Color)] = &[
    ("black", Color::BLACK),
    ("white", Color::WHITE),
    ("red", Color::rgb_const(255, 0, 0)),
    ("green", Color::rgb_const(0, 128, 0)),
    ("blue", Color::rgb_const(0, 0, 255)),
    ("gray", Color::rgb_const(128, 128, 128)),
    ("grey", Color::rgb_const(128, 128, 128)),
    ("silver", Color::rgb_const(192, 192, 192)),
    ("maroon", Color::rgb_const(128, 0, 0)),
    ("yellow", Color::rgb_const(255, 255, 0)),
    ("olive", Color::rgb_const(128, 128, 0)),
    ("lime", Color::rgb_const(0, 255, 0)),
    ("aqua", Color::rgb_const(0, 255, 255)),
    ("cyan", Color::rgb_const(0, 255, 255)),
    ("teal", Color::rgb_const(0, 128, 128)),
    ("navy", Color::rgb_const(0, 0, 128)),
    ("fuchsia", Color::rgb_const(255, 0, 255)),
    ("magenta", Color::rgb_const(255, 0, 255)),
    ("purple", Color::rgb_const(128, 0, 128)),
    ("orange", Color::rgb_const(255, 165, 0)),
    ("pink", Color::rgb_const(255, 192, 203)),
    ("brown", Color::rgb_const(165, 42, 42)),
    ("gold", Color::rgb_const(255, 215, 0)),
    ("indigo", Color::rgb_const(75, 0, 130)),
    ("violet", Color::rgb_const(238, 130, 238)),
    ("crimson", Color::rgb_const(220, 20, 60)),
    ("darkblue", Color::rgb_const(0, 0, 139)),
    ("darkgreen", Color::rgb_const(0, 100, 0)),
    ("darkred", Color::rgb_const(139, 0, 0)),
    ("darkgray", Color::rgb_const(169, 169, 169)),
    ("lightgray", Color::rgb_const(211, 211, 211)),
    ("lightblue", Color::rgb_const(173, 216, 230)),
    ("lightgreen", Color::rgb_const(144, 238, 144)),
    ("transparent", Color::TRANSPARENT),
];

fn parse_weight(s: &str) -> Option<u16> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        "lighter" => Some(300),
        "bolder" => Some(700),
        num => num.parse().ok(),
    }
}

fn parse_font_style(s: &str) -> Option<FontStyle> {
    // CSS spec: normal | italic | oblique [<angle>?]. Tratamos oblique
    // como italic — parley/fontique sintetizan si la fuente no tiene
    // oblique nativo.
    let v = s.trim().to_ascii_lowercase();
    if v == "normal" {
        Some(FontStyle::Normal)
    } else if v == "italic" || v.starts_with("oblique") {
        Some(FontStyle::Italic)
    } else {
        None
    }
}

fn parse_display(s: &str) -> Option<Display> {
    match s.trim().to_ascii_lowercase().as_str() {
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "inline-block" => Some(Display::InlineBlock),
        "flex" => Some(Display::Flex),
        "inline-flex" => Some(Display::InlineFlex),
        "grid" => Some(Display::Grid),
        "inline-grid" => Some(Display::InlineGrid),
        "none" => Some(Display::None),
        _ => None,
    }
}

fn parse_flex_direction(s: &str) -> Option<FlexDirection> {
    match s.trim().to_ascii_lowercase().as_str() {
        "row" => Some(FlexDirection::Row),
        "row-reverse" => Some(FlexDirection::RowReverse),
        "column" => Some(FlexDirection::Column),
        "column-reverse" => Some(FlexDirection::ColumnReverse),
        _ => None,
    }
}

fn parse_flex_wrap(s: &str) -> Option<FlexWrap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
}

fn parse_justify_content(s: &str) -> Option<JustifyContent> {
    match s.trim().to_ascii_lowercase().as_str() {
        "start" | "flex-start" | "left" => Some(JustifyContent::Start),
        "center" => Some(JustifyContent::Center),
        "end" | "flex-end" | "right" => Some(JustifyContent::End),
        "space-between" => Some(JustifyContent::SpaceBetween),
        "space-around" => Some(JustifyContent::SpaceAround),
        "space-evenly" => Some(JustifyContent::SpaceEvenly),
        _ => None,
    }
}

fn parse_align_items(s: &str) -> Option<AlignItems> {
    match s.trim().to_ascii_lowercase().as_str() {
        "start" | "flex-start" => Some(AlignItems::Start),
        "center" => Some(AlignItems::Center),
        "end" | "flex-end" => Some(AlignItems::End),
        "stretch" => Some(AlignItems::Stretch),
        "baseline" => Some(AlignItems::Baseline),
        _ => None,
    }
}

/// `gap: V` ⇒ row=V, column=V. `gap: R C` ⇒ row=R, column=C. Coincide
/// con la semántica CSS shorthand (primer valor = row, segundo = column).
fn parse_gap(value: &str) -> Option<(f32, f32)> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    match parts.as_slice() {
        [v] => {
            let v = parse_length_px(v)?;
            Some((v, v))
        }
        [r, c] => Some((parse_length_px(r)?, parse_length_px(c)?)),
        _ => None,
    }
}

fn parse_box_sizing(s: &str) -> Option<BoxSizing> {
    match s.trim().to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

fn parse_overflow(s: &str) -> Option<Overflow> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Overflow::Visible),
        // hidden/clip/auto/scroll todos los tratamos como Hidden por
        // ahora (no soportamos scroll real; clip y hidden cortan igual).
        "hidden" | "clip" | "auto" | "scroll" => Some(Overflow::Hidden),
        _ => None,
    }
}

fn parse_white_space(s: &str) -> Option<WhiteSpace> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpace::Normal),
        "nowrap" => Some(WhiteSpace::NoWrap),
        "pre" => Some(WhiteSpace::Pre),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "pre-line" => Some(WhiteSpace::PreLine),
        _ => None,
    }
}

fn parse_text_transform(s: &str) -> Option<TextTransform> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        "capitalize" => Some(TextTransform::Capitalize),
        _ => None,
    }
}

/// Acepta `0..1` o `0%..100%`. Clampa.
pub(crate) fn parse_opacity(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct / 100.0).clamp(0.0, 1.0));
    }
    s.parse::<f32>().ok().map(|v| v.clamp(0.0, 1.0))
}

fn parse_align_self(s: &str) -> Option<AlignSelf> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(AlignSelf::Auto),
        "start" | "flex-start" => Some(AlignSelf::Start),
        "center" => Some(AlignSelf::Center),
        "end" | "flex-end" => Some(AlignSelf::End),
        "stretch" => Some(AlignSelf::Stretch),
        "baseline" => Some(AlignSelf::Baseline),
        _ => None,
    }
}

/// `flex: <grow> [<shrink>] [<basis>]`. Casos especiales:
/// - `flex: none` → `0 0 auto`
/// - `flex: auto` → `1 1 auto`
/// - `flex: <number>` → `N 1 0%` (basis 0%, common preset)
/// Devuelve 3 decls atómicas (grow + shrink + basis).
fn parse_flex_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let v = value.trim().to_ascii_lowercase();
    let (grow, shrink, basis) = if v == "none" {
        (0.0_f32, 0.0_f32, LengthVal::Auto)
    } else if v == "auto" {
        (1.0_f32, 1.0_f32, LengthVal::Auto)
    } else if v == "initial" {
        (0.0_f32, 1.0_f32, LengthVal::Auto)
    } else {
        let parts: Vec<&str> = value.split_whitespace().collect();
        match parts.as_slice() {
            [g] => {
                // `flex: 1` ⇒ `1 1 0%`
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                (g, 1.0, LengthVal::Pct(0.0))
            }
            [g, s_or_b] => {
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                // El segundo puede ser shrink (número solo) o basis (longitud).
                if let Some(b) = parse_length_or_pct(s_or_b) {
                    (g, 1.0, b)
                } else if let Some(s) = s_or_b.parse::<f32>().ok() {
                    (g, s, LengthVal::Pct(0.0))
                } else {
                    return Vec::new();
                }
            }
            [g, s, b] => {
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                let Some(s) = s.parse::<f32>().ok() else {
                    return Vec::new();
                };
                let Some(b) = parse_length_or_pct(b) else {
                    return Vec::new();
                };
                (g, s, b)
            }
            _ => return Vec::new(),
        }
    };
    vec![
        Decl { kind: DeclKind::FlexGrow(grow), important },
        Decl { kind: DeclKind::FlexShrink(shrink), important },
        Decl { kind: DeclKind::FlexBasis(basis), important },
    ]
}

/// `outline: <width> <style> <color>`. Tokens en cualquier orden.
fn parse_outline_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut width: Option<f32> = None;
    let mut color: Option<Color> = None;
    let mut style_active: Option<bool> = None;
    for tok in value.split_whitespace() {
        if width.is_none() {
            if let Some(w) = parse_length_px(tok) {
                width = Some(w);
                continue;
            }
        }
        if style_active.is_none() {
            if let Some(active) = parse_border_style(tok) {
                style_active = Some(active);
                continue;
            }
        }
        if color.is_none() {
            if let Some(c) = parse_color(tok) {
                color = Some(c);
                continue;
            }
        }
    }
    let mut out = Vec::new();
    let active = style_active.unwrap_or(true);
    if !active {
        // `outline-style: none` apaga: width=0 + color=None.
        out.push(Decl { kind: DeclKind::OutlineStyle(false), important });
        return out;
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::OutlineWidth(w), important });
    }
    if let Some(c) = color {
        out.push(Decl { kind: DeclKind::OutlineColor(c), important });
    }
    if style_active.is_some() {
        out.push(Decl { kind: DeclKind::OutlineStyle(true), important });
    }
    out
}

/// `background-image: linear-gradient(...)` o `none`. Devuelve un
/// `DeclKind` listo (Background o BackgroundGradient o None).
fn parse_background_image(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(DeclKind::BackgroundGradientNone);
    }
    if let Some(args) = strip_fn(v, "linear-gradient") {
        return parse_linear_gradient(args).map(DeclKind::BackgroundGradient);
    }
    if let Some(args) = strip_fn(v, "url") {
        // url('foo') / url("foo") / url(foo) — trimea comillas.
        let raw = args.trim();
        let unquoted = raw
            .strip_prefix('"').and_then(|s| s.strip_suffix('"'))
            .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(raw);
        let url = unquoted.trim();
        if url.is_empty() {
            return None;
        }
        return Some(DeclKind::BackgroundImageUrl(url.to_string()));
    }
    // Otros gradientes (`radial-gradient`, `conic-gradient`) o `cross-fade`
    // no soportados — silencio.
    None
}

/// Parsea el contenido de `linear-gradient(...)`. Sintaxis aceptada:
/// - `linear-gradient(<angle>?, <stop>, <stop>, ...)`
/// - `linear-gradient(to <side>?, <stop>, <stop>, ...)`
/// `<angle>` en `Ndeg` o `Nturn` (turn × 360 = grados). Default 180
/// (top→bottom). `to right`=90, `to left`=270, `to top`=0, `to bottom`=180,
/// combinaciones diagonales (`to top right`=45) también. Stops: `<color>
/// <pos>?` donde pos es `N%` o `Npx`.
fn parse_linear_gradient(args: &str) -> Option<LinearGradient> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    if parts.len() < 2 {
        return None;
    }
    let (angle_deg, stops_start) = parse_gradient_direction(parts[0]);
    let stops_start_idx = if angle_deg.is_some() { 1 } else { 0 };
    let angle_deg = angle_deg.unwrap_or(180.0);
    let mut stops: Vec<GradientStop> = Vec::new();
    for raw in &parts[stops_start_idx..] {
        if let Some(s) = parse_gradient_stop(raw) {
            stops.push(s);
        }
    }
    if stops.len() < 2 {
        return None;
    }
    let _ = stops_start;
    Some(LinearGradient { angle_deg, stops })
}

/// Si el token es una dirección/ángulo válido devuelve `(Some(deg),
/// true)`; si no encaja, `(None, false)` para que el caller lo trate
/// como stop.
fn parse_gradient_direction(s: &str) -> (Option<f32>, bool) {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("to ") {
        let deg = match rest.trim() {
            "top" => 0.0,
            "right" => 90.0,
            "bottom" => 180.0,
            "left" => 270.0,
            "top right" | "right top" => 45.0,
            "bottom right" | "right bottom" => 135.0,
            "bottom left" | "left bottom" => 225.0,
            "top left" | "left top" => 315.0,
            _ => return (None, false),
        };
        return (Some(deg), true);
    }
    if let Some(num) = lower.strip_suffix("deg") {
        if let Ok(v) = num.trim().parse::<f32>() {
            return (Some(v), true);
        }
    }
    if let Some(num) = lower.strip_suffix("turn") {
        if let Ok(v) = num.trim().parse::<f32>() {
            return (Some(v * 360.0), true);
        }
    }
    (None, false)
}

fn parse_gradient_stop(s: &str) -> Option<GradientStop> {
    let s = s.trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.as_slice() {
        [c] => Some(GradientStop { color: parse_color(c)?, pos: None }),
        [c, p] => {
            let color = parse_color(c)?;
            let pos = if let Some(pct) = p.strip_suffix('%') {
                pct.trim().parse::<f32>().ok().map(|v| (v / 100.0).clamp(0.0, 1.0))
            } else if let Some(px) = parse_length_px(p) {
                // Aproximación: tratamos px como 0..1 dividiendo por 100.
                // En el wild la mayoría usa %, así que esta heurística
                // raramente importa.
                Some((px / 100.0).clamp(0.0, 1.0))
            } else {
                None
            };
            Some(GradientStop { color, pos })
        }
        _ => None,
    }
}

/// Acepta `12px`, `1.5rem` (tratada como em*16), `0`. Sin unidad → px.
/// `Nvw`/`Nvh`/`Nvmin`/`Nvmax` resuelven contra el viewport activo
/// ([`resolve_viewport`]): el real bajo un `ViewportScope` (carga normal),
/// `DEFAULT_VIEWPORT` fuera de él (parsers sueltos en tests).
fn parse_length_px(s: &str) -> Option<f32> {
    let s = s.trim();
    if s == "0" {
        return Some(0.0);
    }
    if let Some(num) = s.strip_suffix("px") {
        return num.trim().parse().ok();
    }
    if let Some(num) = s.strip_suffix("rem") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * 16.0);
    }
    if let Some(num) = s.strip_suffix("em") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * 16.0);
    }
    if let Some(num) = s.strip_suffix("vmin") {
        let v: f32 = num.trim().parse().ok()?;
        let vp = resolve_viewport();
        return Some(v * vp.width.min(vp.height) / 100.0);
    }
    if let Some(num) = s.strip_suffix("vmax") {
        let v: f32 = num.trim().parse().ok()?;
        let vp = resolve_viewport();
        return Some(v * vp.width.max(vp.height) / 100.0);
    }
    if let Some(num) = s.strip_suffix("vw") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * resolve_viewport().width / 100.0);
    }
    if let Some(num) = s.strip_suffix("vh") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * resolve_viewport().height / 100.0);
    }
    s.parse().ok()
}

/// `length`, `%` o `auto`. Variante para insets que sí admiten `auto`.
fn parse_length_or_pct_or_auto(s: &str) -> Option<LengthVal> {
    parse_length_or_pct(s.trim())
}

fn parse_position(s: &str) -> Option<Position> {
    match s.trim().to_ascii_lowercase().as_str() {
        "static" => Some(Position::Static),
        "relative" => Some(Position::Relative),
        "absolute" => Some(Position::Absolute),
        "fixed" => Some(Position::Fixed),
        "sticky" => Some(Position::Sticky),
        _ => None,
    }
}

fn parse_vertical_align(s: &str) -> Option<VerticalAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        "baseline" => Some(VerticalAlign::Baseline),
        "top" | "text-top" => Some(VerticalAlign::Top),
        "middle" => Some(VerticalAlign::Middle),
        "bottom" | "text-bottom" => Some(VerticalAlign::Bottom),
        "super" => Some(VerticalAlign::Super),
        "sub" => Some(VerticalAlign::Sub),
        _ => None,
    }
}

fn parse_visibility(s: &str) -> Option<Visibility> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Visibility::Visible),
        // `collapse` lo tratamos igual que hidden (sólo aplica a
        // tablas/flex en CSS spec, aproximación segura).
        "hidden" | "collapse" => Some(Visibility::Hidden),
        _ => None,
    }
}

fn parse_pointer_events(s: &str) -> Option<PointerEvents> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(PointerEvents::Auto),
        "none" => Some(PointerEvents::None),
        _ => None,
    }
}

/// `text-shadow: <x> <y> [blur] <color>[, <x> <y> [blur] <color>]*`.
/// `none` → vector vacío. Devuelve None si ningún shadow es válido.
fn parse_text_shadows(value: &str) -> Option<Vec<TextShadow>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for sh in v.split(',') {
        if let Some(s) = parse_one_text_shadow(sh) {
            out.push(s);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn parse_one_text_shadow(s: &str) -> Option<TextShadow> {
    let mut lengths: Vec<f32> = Vec::with_capacity(3);
    let mut color: Option<Color> = None;
    for tok in s.split_whitespace() {
        if let Some(l) = parse_length_px(tok) {
            lengths.push(l);
            continue;
        }
        if let Some(c) = parse_color(tok) {
            color = Some(c);
            continue;
        }
    }
    if lengths.len() < 2 {
        return None;
    }
    Some(TextShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_px: lengths.get(2).copied().unwrap_or(0.0),
        color: color.unwrap_or(Color::BLACK),
    })
}

/// `transform: none` o cadena de funciones (`rotate(45deg) scale(2)
/// translate(10px, 20px)`). Acepta `translate(x)`, `translate(x, y)`,
/// `translateX(x)`, `translateY(y)`, `scale(s)`, `scale(sx, sy)`,
/// `scaleX(sx)`, `scaleY(sy)`, `rotate(Ndeg|Nrad|Nturn)`.
pub(crate) fn parse_transforms(value: &str) -> Option<Vec<Transform>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    let mut rest = v;
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let open = rest.find('(')?;
        let name = rest[..open].trim().to_ascii_lowercase();
        let mut depth = 1usize;
        let bytes = rest[open + 1..].as_bytes();
        let mut close = None;
        for (i, &c) in bytes.iter().enumerate() {
            match c {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let args = &rest[open + 1..open + 1 + close];
        let tr = parse_transform_fn(&name, args)?;
        out.push(tr);
        rest = &rest[open + 1 + close + 1..];
    }
    Some(out)
}

fn parse_transform_fn(name: &str, args: &str) -> Option<Transform> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    match name {
        "translate" => match parts.as_slice() {
            [x] => Some(Transform::Translate(parse_length_px(x)?, 0.0)),
            [x, y] => Some(Transform::Translate(parse_length_px(x)?, parse_length_px(y)?)),
            _ => None,
        },
        "translatex" => Some(Transform::Translate(parse_length_px(parts[0])?, 0.0)),
        "translatey" => Some(Transform::Translate(0.0, parse_length_px(parts[0])?)),
        "scale" => match parts.as_slice() {
            [s] => {
                let v = s.parse::<f32>().ok()?;
                Some(Transform::Scale(v, v))
            }
            [sx, sy] => {
                Some(Transform::Scale(sx.parse().ok()?, sy.parse().ok()?))
            }
            _ => None,
        },
        "scalex" => Some(Transform::Scale(parts[0].parse().ok()?, 1.0)),
        "scaley" => Some(Transform::Scale(1.0, parts[0].parse().ok()?)),
        "rotate" => {
            let arg = parts[0];
            let deg = if let Some(n) = arg.strip_suffix("deg") {
                n.trim().parse::<f32>().ok()?
            } else if let Some(n) = arg.strip_suffix("rad") {
                let v: f32 = n.trim().parse().ok()?;
                v.to_degrees()
            } else if let Some(n) = arg.strip_suffix("turn") {
                let v: f32 = n.trim().parse().ok()?;
                v * 360.0
            } else {
                // Sin unidad: asumir deg.
                arg.parse::<f32>().ok()?
            };
            Some(Transform::Rotate(deg))
        }
        _ => None,
    }
}

/// `grid-template-columns: <track-list>`. Subset soportado:
/// - `auto`
/// - `Npx` / `N%`
/// - `Nfr`
/// - `repeat(N, <track>)` con repeat de un solo track
/// Tokens separados por whitespace.
fn parse_grid_template(value: &str) -> Option<Vec<GridTrackSize>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out: Vec<GridTrackSize> = Vec::new();
    // Tokenize: respeta nesting de paréntesis para repeat(N, X).
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in v.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    for tok in tokens {
        if let Some(inner) = strip_fn(&tok, "repeat") {
            let parts: Vec<&str> = inner.splitn(2, ',').collect();
            if parts.len() != 2 {
                continue;
            }
            let count: i32 = parts[0].trim().parse().ok()?;
            let track = parse_one_grid_track(parts[1].trim())?;
            for _ in 0..count.max(0) {
                out.push(track);
            }
        } else if let Some(t) = parse_one_grid_track(&tok) {
            out.push(t);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn parse_one_grid_track(s: &str) -> Option<GridTrackSize> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") {
        return Some(GridTrackSize::Auto);
    }
    if let Some(num) = s.strip_suffix("fr") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(GridTrackSize::Fr(v));
    }
    if let Some(lv) = parse_length_or_pct(s) {
        return Some(match lv {
            LengthVal::Px(v) => GridTrackSize::Px(v),
            LengthVal::Pct(v) => GridTrackSize::Pct(v),
            LengthVal::Auto => GridTrackSize::Auto,
        });
    }
    None
}

/// Evalúa una condición de `@media` contra el viewport por defecto. Subset:
/// `(max-width: Npx)`, `(min-width: Npx)`, encadenados por ` and `.
/// `screen`/`all` se ignoran (siempre true).
/// Evalúa una media query (`@media` en CSS y `window.matchMedia()` en JS) contra
/// el viewport actual. Soporta listas separadas por `,` (OR), `not`/`only`,
/// el combinador ` and `, tipos de media (`screen`/`all`/`print`/`speech`) y
/// las features: `min/max/exact-width`, `min/max/exact-height`, `orientation`
/// (portrait/landscape), `min/max/exact-resolution` (`Ndppx`/`Ndpi`/`Nx` vs
/// `vp.dpr`) y `prefers-color-scheme`/`prefers-reduced-motion` (reportamos
/// light / no-reduce). Features desconocidas se ignoran (no descalifican), igual
/// que el comportamiento previo, para no romper CSS que las use de forma
/// progresiva. Pública porque el chrome (`puriy-llimphi`) la reusa para resolver
/// `matchMedia` contra el viewport real de la ventana.
pub fn evaluate_media_query(condition: &str, vp: Viewport) -> bool {
    let cond = condition.trim().to_ascii_lowercase();
    if cond.is_empty() {
        return true;
    }
    // Media query LIST: separada por comas, matchea si CUALQUIER componente lo hace.
    if cond.contains(',') {
        return cond.split(',').any(|q| evaluate_media_query(q, vp));
    }
    // `not` a nivel de query invierte el resultado completo.
    if let Some(rest) = cond.strip_prefix("not ") {
        return !evaluate_media_query_terms(rest.trim(), vp);
    }
    evaluate_media_query_terms(&cond, vp)
}

/// Evalúa los términos unidos por ` and ` de una query ya sin `,`/`not` de tope.
fn evaluate_media_query_terms(cond: &str, vp: Viewport) -> bool {
    for part in cond.split(" and ").map(|s| s.trim()) {
        if part.is_empty() {
            continue;
        }
        // Tipos de media.
        if part == "all" || part == "screen" {
            continue;
        }
        if part == "print" || part == "speech" || part == "tty" {
            return false;
        }
        let part = part.strip_prefix("only ").unwrap_or(part).trim();
        // Esperamos `(feature)` o `(feature: value)`.
        let Some(inner) = part.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
            // Token no reconocido (tipo de media raro): no matchea.
            return false;
        };
        if !evaluate_media_feature(inner.trim(), vp) {
            return false;
        }
    }
    true
}

/// Evalúa UNA feature `(feature)` o `(feature: value)` contra el viewport.
fn evaluate_media_feature(inner: &str, vp: Viewport) -> bool {
    let Some((feature, val)) = inner.split_once(':').map(|(a, b)| (a.trim(), b.trim())) else {
        // Feature booleana (sin valor): matchea si la capacidad "existe".
        return matches!(inner, "color" | "grid" | "hover" | "pointer");
    };
    match feature {
        "max-width" => parse_length_px(val).is_some_and(|l| vp.width <= l),
        "min-width" => parse_length_px(val).is_some_and(|l| vp.width >= l),
        "width" => parse_length_px(val).is_some_and(|l| (vp.width - l).abs() < 0.5),
        "max-height" => parse_length_px(val).is_some_and(|l| vp.height <= l),
        "min-height" => parse_length_px(val).is_some_and(|l| vp.height >= l),
        "height" => parse_length_px(val).is_some_and(|l| (vp.height - l).abs() < 0.5),
        "orientation" => match val {
            "portrait" => vp.height >= vp.width,
            "landscape" => vp.width > vp.height,
            _ => false,
        },
        "min-resolution" => parse_resolution_dppx(val).is_some_and(|r| vp.dpr >= r),
        "max-resolution" => parse_resolution_dppx(val).is_some_and(|r| vp.dpr <= r),
        "resolution" => parse_resolution_dppx(val).is_some_and(|r| (vp.dpr - r).abs() < 0.01),
        "min-aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| vp.width / vp.height >= r)
        }
        "max-aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| vp.width / vp.height <= r)
        }
        "aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| (vp.width / vp.height - r).abs() < 0.01)
        }
        // Preferencias del usuario: reportamos tema claro y sin reducción.
        "prefers-color-scheme" => val == "light" || val == "no-preference",
        "prefers-reduced-motion" => val == "no-preference",
        "prefers-contrast" => val == "no-preference",
        "hover" => val == "hover",
        "any-hover" => val == "hover",
        "pointer" => val == "fine",
        "any-pointer" => val == "fine",
        // Feature desconocida: no descalifica (comportamiento previo lenient).
        _ => true,
    }
}

/// Parsea un aspect-ratio de media query a un float `ancho/alto`. Acepta la
/// forma `W/H` (`16/9`) y el número suelto (`1.5`). `None` si no parsea o el
/// alto es cero.
fn parse_aspect_ratio(val: &str) -> Option<f32> {
    let v = val.trim();
    if let Some((w, h)) = v.split_once('/') {
        let w: f32 = w.trim().parse().ok()?;
        let h: f32 = h.trim().parse().ok()?;
        if h == 0.0 {
            return None;
        }
        Some(w / h)
    } else {
        v.parse::<f32>().ok()
    }
}

/// Parsea una resolución de media query a `dppx` (dots per px). Acepta
/// `Ndppx`, `Nx` (alias de dppx) y `Ndpi` (96dpi = 1dppx). `None` si no parsea.
fn parse_resolution_dppx(val: &str) -> Option<f32> {
    let v = val.trim();
    if let Some(n) = v.strip_suffix("dppx").or_else(|| v.strip_suffix('x')) {
        n.trim().parse::<f32>().ok()
    } else if let Some(n) = v.strip_suffix("dpi") {
        n.trim().parse::<f32>().ok().map(|d| d / 96.0)
    } else if let Some(n) = v.strip_suffix("dpcm") {
        n.trim().parse::<f32>().ok().map(|d| d / 96.0 * 2.54)
    } else {
        None
    }
}

/// Evalúa una condición `@supports (prop: value)` ⇒ true si nuestro
/// parser puede convertirla a algún DeclKind. Subset minimal: no
/// soporta `and`/`or`/`not` por ahora.
fn evaluate_supports_query(condition: &str) -> bool {
    let cond = condition.trim();
    let Some(inner) = cond.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
        return false;
    };
    let Some((prop, val)) = inner.split_once(':') else {
        return false;
    };
    decl_kind_from_pair(prop.trim(), val.trim()).is_some()
}

/// Indica que `cssparser` está enlazado aunque el subset actual no use
/// la API completa — la presencia del crate evita que `cargo` lo
/// pruebe y deja el camino abierto para Fase 3.
#[doc(hidden)]
pub fn _cssparser_anchor() {
    let _ = cssparser::ParserInput::new("");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_hex_color() {
        assert_eq!(parse_color("#ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("red"), Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn parsea_length() {
        assert_eq!(parse_length_px("12px"), Some(12.0));
        assert_eq!(parse_length_px("1.5em"), Some(24.0));
        assert_eq!(parse_length_px("0"), Some(0.0));
        assert_eq!(parse_length_px("xyz"), None);
    }

    #[test]
    fn parse_content_value_acepta_string_quoted() {
        assert_eq!(
            parse_content_value(r#""hola""#),
            Some(vec![ContentItem::Text("hola".into())])
        );
        assert_eq!(
            parse_content_value(r#"'mundo'"#),
            Some(vec![ContentItem::Text("mundo".into())])
        );
        assert_eq!(parse_content_value("none"), None);
        assert_eq!(parse_content_value("normal"), None);
        // Sin comillas y sin counter()/attr() → None.
        assert_eq!(parse_content_value("foo"), None);
    }

    #[test]
    fn parse_content_value_respeta_escapes() {
        assert_eq!(
            parse_content_value(r#""linea1\nlinea2""#),
            Some(vec![ContentItem::Text("linea1nlinea2".into())]) // \n no especial
        );
        assert_eq!(
            parse_content_value(r#""con \"quote\" adentro""#),
            Some(vec![ContentItem::Text(r#"con "quote" adentro"#.into())])
        );
    }

    #[test]
    fn parse_content_value_concat_counter_attr() {
        let items = parse_content_value(r#""Sección " counter(sec) ": " attr(data-title)"#)
            .expect("debería parsear");
        assert_eq!(
            items,
            vec![
                ContentItem::Text("Sección ".into()),
                ContentItem::Counter("sec".into()),
                ContentItem::Text(": ".into()),
                ContentItem::Attr("data-title".into()),
            ]
        );
    }

    #[test]
    fn parse_counter_list_acepta_pares_y_defaults() {
        assert_eq!(
            parse_counter_list("section 0 chapter 5", 0),
            vec![("section".into(), 0), ("chapter".into(), 5)]
        );
        // Default cuando no hay valor explícito.
        assert_eq!(
            parse_counter_list("h2", 1),
            vec![("h2".into(), 1)]
        );
        assert_eq!(parse_counter_list("none", 0), Vec::<(String, i32)>::new());
    }

    #[test]
    fn pseudo_element_extrae_del_selector() {
        let html = r##"<html><head><style>
            p::before { content: "PRE " }
            p::after { content: " POST" }
            p:before { content: "legacy" }
        </style></head><body><p>x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let before = eng.compute_pseudo(&p, PseudoElement::Before, None);
        let after = eng.compute_pseudo(&p, PseudoElement::After, None);
        // `:before` legacy también matchea Before pero llega después; el
        // último gana en empate de especificidad.
        assert_eq!(
            before.and_then(|s| s.content),
            Some(vec![ContentItem::Text("legacy".into())])
        );
        assert_eq!(
            after.and_then(|s| s.content),
            Some(vec![ContentItem::Text(" POST".into())])
        );
    }

    #[test]
    fn pseudo_element_sin_content_no_se_materializa() {
        // Una regla `::before` sin content → compute_pseudo devuelve None.
        let html = r##"<html><head><style>
            p::before { color: red }
        </style></head><body><p>x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert!(eng.compute_pseudo(&p, PseudoElement::Before, None).is_none());
    }

    #[test]
    fn reglas_pseudo_no_pegan_al_elemento_real() {
        // `p::before { color: red }` NO debe afectar el color de `<p>`
        // — sólo de su `::before`.
        let html = r##"<html><head><style>
            p::before { content: "X"; color: red }
            p { color: blue }
        </style></head><body><p>texto</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.color, Color::rgb(0, 0, 255)); // blue, no red
    }

    #[test]
    fn parsea_z_index() {
        let html = r##"<html><head><style>
            .a { z-index: 5 }
            .b { z-index: -2 }
            .c { z-index: auto }
        </style></head><body>
            <div class="a"></div>
            <div class="b"></div>
            <div class="c"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 3);
        assert_eq!(eng.compute(&divs[0]).z_index, 5);
        assert_eq!(eng.compute(&divs[1]).z_index, -2);
        assert_eq!(eng.compute(&divs[2]).z_index, 0); // auto → 0
    }

    #[test]
    fn parsea_calc_solo_px() {
        // calc(10px + 5px) resuelve a Px(15) en parse time.
        assert_eq!(parse_length_or_pct("calc(10px + 5px)"), Some(LengthVal::Px(15.0)));
        assert_eq!(parse_length_or_pct("calc(20px - 5px)"), Some(LengthVal::Px(15.0)));
    }

    #[test]
    fn parsea_calc_solo_pct() {
        assert_eq!(parse_length_or_pct("calc(80% - 10%)"), Some(LengthVal::Pct(70.0)));
        assert_eq!(parse_length_or_pct("calc(50% + 20%)"), Some(LengthVal::Pct(70.0)));
    }

    #[test]
    fn parsea_calc_mixto_pierde_offset_px() {
        // Mezcla pct + px: conservamos el Pct e ignoramos el px (no
        // tenemos container width acá; taffy no soporta calc nativo).
        // Esto es una limitación documentada del soporte de calc.
        assert_eq!(parse_length_or_pct("calc(100% - 20px)"), Some(LengthVal::Pct(100.0)));
        assert_eq!(parse_length_or_pct("calc(50% + 10px)"), Some(LengthVal::Pct(50.0)));
    }

    #[test]
    fn parsea_calc_invalido_devuelve_none() {
        // Tokens incompletos / mismatched parens / op desconocido.
        assert!(parse_length_or_pct("calc(10px +)").is_none());
        assert!(parse_length_or_pct("calc(10px * 2)").is_none());
        assert!(parse_length_or_pct("calc(10px").is_none());
    }

    #[test]
    fn parsea_regla_simple() {
        let rules = parse_stylesheet("p { color: red; font-size: 14px; }", &HashMap::new(), DEFAULT_VIEWPORT);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.compounds.len(), 1);
        assert!(matches!(
            &rules[0].selector.compounds[0].tag,
            TagPart::Type(t) if t == "p"
        ));
        assert_eq!(rules[0].decls.len(), 2);
    }

    #[test]
    fn selector_compound_matchea() {
        // `a.btn` matchea sólo `<a class="btn">`.
        let html = r##"<html><head><style>a.btn{color:red}</style></head><body>
                <a class="btn" href="#">click</a>
                <a href="#">otro</a>
                <span class="btn">no soy a</span>
            </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut anchors = Vec::new();
        let mut spans = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("a") => anchors.push(n.clone()),
                Some("span") => spans.push(n.clone()),
                _ => {}
            }
        });
        assert_eq!(anchors.len(), 2);
        assert_eq!(spans.len(), 1);
        // anchors[0] tiene class="btn" — `.btn { color: red }` pisa
        // el azul-de-link del UA stylesheet.
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(255, 0, 0));
        // anchors[1] sin class — sólo aplica el UA, que pinta `<a>`
        // con el azul clásico de browser (0, 0, 238).
        assert_eq!(eng.compute(&anchors[1]).color, Color::rgb(0, 0, 238));
        // span.btn no es <a> — no aplica el UA de link.
        assert_eq!(eng.compute(&spans[0]).color, Color::BLACK);
    }

    #[test]
    fn selector_hijo_directo_matchea() {
        // `ul > li` matchea `<li>` que es hijo *directo* de `<ul>`. Un
        // `<li>` dentro de `<ol>` adentro de `<ul>` no debe matchear.
        let html = r#"<html><head><style>ul > li{color:#0a0}</style></head>
            <body>
              <ul><li>directo</li></ul>
              <ol><li>indirecto</li></ol>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 2);
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_hermano_adyacente_matchea() {
        // `h2 + p` matchea sólo el primer `<p>` inmediatamente después
        // de un `<h2>`.
        let html = r#"<html><head><style>h2+p{color:#00f}</style></head>
            <body>
              <h2>t</h2><p>uno</p><p>dos</p>
              <p>aislado</p>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_hermano_general_matchea() {
        // `h2 ~ p` matchea TODOS los `<p>` hermanos posteriores a un `<h2>`.
        let html = r#"<html><head><style>h2~p{color:#00f}</style></head>
            <body>
              <p>antes</p><h2>t</h2><p>uno</p><span>x</span><p>dos</p>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        // El primero está antes del h2 → no aplica.
        assert_eq!(eng.compute(&ps[0]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[1]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn selector_descendiente_matchea() {
        // `.menu li` matchea sólo los `<li>` dentro de `.menu`.
        let html = r#"<html><head><style>.menu li{color:#00aa00}</style></head>
            <body>
              <ul class="menu"><li>uno</li><li>dos</li></ul>
              <ul><li>tres</li></ul>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 3);
        // Los dos primeros viven en .menu → verde
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[1]).color, Color::rgb(0, 0xaa, 0));
        // El tercero no
        assert_eq!(eng.compute(&lis[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_class_matchea() {
        let html = r#"<html><head><style>.alert{color:red}</style></head><body><p class="alert">x</p><p>y</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ps: Vec<_> = {
            let mut acc = Vec::new();
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::element_name(n).as_deref() == Some("p") {
                    acc.push(n.clone());
                }
            });
            acc
        };
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_id_matchea() {
        let html = r#"<html><head><style>#hero{color:#0000ff}</style></head><body><p id="hero">x</p><p>y</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_presente() {
        // `[href]` matchea cualquier elemento con atributo `href`.
        let html = r#"<html><head><style>[href]{color:red}</style></head>
            <body><a href="x">link</a><a>sin</a><span>no a</span></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut elems = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if matches!(
                crate::dom::element_name(n).as_deref(),
                Some("a") | Some("span")
            ) {
                elems.push(n.clone());
            }
        });
        // a[href] → rojo (la regla `[href]{color:red}` con
        // especificidad 10 pisa el UA `a{color:#00ee}`); a sin href no
        // matchea pero recibe el UA = azul-link; span → BLACK default.
        assert_eq!(eng.compute(&elems[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&elems[1]).color, Color::rgb(0, 0, 238));
        assert_eq!(eng.compute(&elems[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_equals() {
        // `input[type="checkbox"]` matchea sólo el checkbox.
        let html = r##"<html><head><style>input[type="checkbox"]{color:#00aa00}</style></head>
            <body>
              <input type="checkbox">
              <input type="text">
              <input>
            </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut inputs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("input") {
                inputs.push(n.clone());
            }
        });
        assert_eq!(inputs.len(), 3);
        assert_eq!(eng.compute(&inputs[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&inputs[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&inputs[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_prefix_suffix_contains() {
        let html = r##"<html><head><style>
            a[href^="https"]{color:#00f}
            img[src$=".png"]{color:#0f0}
            div[class*="warn"]{color:#f00}
        </style></head>
        <body>
            <a href="https://x">seguro</a>
            <a href="http://x">inseguro</a>
            <img src="logo.png">
            <img src="logo.jpg">
            <div class="banner warn-strong">!!</div>
            <div class="banner">--</div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut anchors = Vec::new();
        let mut imgs = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| match crate::dom::element_name(n).as_deref() {
            Some("a") => anchors.push(n.clone()),
            Some("img") => imgs.push(n.clone()),
            Some("div") => divs.push(n.clone()),
            _ => {}
        });
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(0, 0, 255));
        // anchors[1] no matchea `[href^="https"]` pero recibe el UA
        // de `<a>` (azul 0,0,238).
        assert_eq!(eng.compute(&anchors[1]).color, Color::rgb(0, 0, 238));
        assert_eq!(eng.compute(&imgs[0]).color, Color::rgb(0, 255, 0));
        assert_eq!(eng.compute(&imgs[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&divs[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&divs[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_first_last_only_child() {
        let html = r#"<html><head><style>
            li:first-child{color:#00f}
            li:last-child{background:#0f0}
            p:only-child{color:#f0f}
        </style></head>
        <body>
          <ul><li>a</li><li>b</li><li>c</li></ul>
          <section><p>solo</p></section>
          <section><p>uno</p><p>dos</p></section>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| match crate::dom::element_name(n).as_deref() {
            Some("li") => lis.push(n.clone()),
            Some("p") => ps.push(n.clone()),
            _ => {}
        });
        // li:first-child sólo el primero
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
        // li:last-child sólo el tercero (background)
        assert!(eng.compute(&lis[0]).background.is_none());
        assert_eq!(eng.compute(&lis[2]).background, Some(Color::rgb(0, 255, 0)));
        // p:only-child el primero (único en su section), no los otros dos
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(255, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_first_last_of_type() {
        let html = r#"<html><head><style>
            p:first-of-type{color:#00f}
            p:last-of-type{color:#0a0}
        </style></head>
        <body>
          <div>x</div>
          <p>uno</p>
          <span>y</span>
          <p>dos</p>
          <p>tres</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        // primer <p> → azul (es :first-of-type aunque haya <div> y <span> antes)
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        // del medio → ninguno (last gana cascada al último pero a este ninguno)
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        // último <p> → verde
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0, 0xaa, 0));
    }

    #[test]
    fn parsea_width_max_width() {
        let s = parse_stylesheet(
            "p { width: 80%; max-width: 800px } div { width: auto }",
            &HashMap::new(),
            DEFAULT_VIEWPORT,
        );
        assert_eq!(s.len(), 2);
        assert!(matches!(s[0].decls[0].kind, DeclKind::Width(LengthVal::Pct(80.0))));
        assert!(matches!(s[0].decls[1].kind, DeclKind::MaxWidth(LengthVal::Px(800.0))));
        assert!(matches!(s[1].decls[0].kind, DeclKind::Width(LengthVal::Auto)));
    }

    #[test]
    fn parsea_text_align() {
        let s = parse_stylesheet(
            "h1 { text-align: center } p { text-align: right }",
            &HashMap::new(),
            DEFAULT_VIEWPORT,
        );
        assert!(matches!(s[0].decls[0].kind, DeclKind::TextAlign(TextAlign::Center)));
        assert!(matches!(s[1].decls[0].kind, DeclKind::TextAlign(TextAlign::Right)));
    }

    #[test]
    fn parsea_line_height() {
        let s = parse_stylesheet(
            "p { line-height: 1.5 } h1 { line-height: 32px }",
            &HashMap::new(),
            DEFAULT_VIEWPORT,
        );
        // 1.5 → 1.5
        assert!(matches!(s[0].decls[0].kind, DeclKind::LineHeight(v) if (v - 1.5).abs() < 1e-6));
        // 32px sobre font-size 16px estimado → 2.0
        assert!(matches!(s[1].decls[0].kind, DeclKind::LineHeight(v) if (v - 2.0).abs() < 1e-6));
    }

    #[test]
    fn computa_width_y_text_align() {
        let html = r#"<html><head><style>
            .narrow{max-width:600px;text-align:center;line-height:1.6}
        </style></head><body><div class="narrow">x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let st = eng.compute(&div);
        assert_eq!(st.max_width, LengthVal::Px(600.0));
        assert_eq!(st.text_align, TextAlign::Center);
        assert!((st.line_height.unwrap() - 1.6).abs() < 1e-6);
    }

    #[test]
    fn hereda_color_y_font_size_del_padre() {
        // `<p style="color:red; font-size:20px">foo <em>bar</em></p>` —
        // el `<em>` no tiene regla propia pero hereda color y tamaño.
        let html = r#"<html><body><p style="color:red; font-size:20px">foo<em>bar</em></p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, None);
        assert_eq!(p_style.color, Color::rgb(255, 0, 0));
        let em = dom.find("em").unwrap();
        let em_style = eng.compute_with_parent(&em, Some(&p_style));
        assert_eq!(em_style.color, Color::rgb(255, 0, 0));
        assert!((em_style.font_size - 20.0).abs() < 1e-6);
    }

    #[test]
    fn no_hereda_propiedades_no_heredables() {
        // background y margin/padding NO heredan.
        let html = r#"<html><body><div style="background:red; margin:30px"><p>x</p></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let div_style = eng.compute_with_parent(&div, None);
        assert_eq!(div_style.background, Some(Color::rgb(255, 0, 0)));
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, Some(&div_style));
        assert_eq!(p_style.background, None);
        // margin del <p> es 12px (UA default), no 30px del padre.
        assert!((p_style.margin.top - 12.0).abs() < 1e-6);
        assert!((p_style.margin.bottom - 12.0).abs() < 1e-6);
    }

    #[test]
    fn font_weight_bold_local_no_propaga_a_padre_no_bold() {
        // Un `<b>` dentro de `<p>` no-bold sigue siendo bold.
        let html = "<html><body><p>foo<b>bar</b></p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, None);
        assert_eq!(p_style.font_weight, 400);
        let b = dom.find("b").unwrap();
        let b_style = eng.compute_with_parent(&b, Some(&p_style));
        assert_eq!(b_style.font_weight, 700);
    }

    #[test]
    fn box_tree_propaga_color_a_hoja_de_texto() {
        // Verifica el bug original: el text leaf debe heredar el color
        // del `<p>` padre.
        let html = r#"<html><body><p style="color: #00ff00">verde</p></body></html>"#;
        let eng = crate::Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut leaf_colors = Vec::new();
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("verde") {
                leaf_colors.push(b.color);
            }
        });
        assert_eq!(leaf_colors.len(), 1);
        assert_eq!(leaf_colors[0], Color::rgb(0, 0xff, 0));
    }

    #[test]
    fn specificity_calculada_correctamente() {
        // `body p` = 0,0,2 → 2
        let s1 = parse_selector("body p").unwrap();
        assert_eq!(s1.specificity(), 2);
        // `.menu li` = 0,1,1 → 11
        let s2 = parse_selector(".menu li").unwrap();
        assert_eq!(s2.specificity(), 11);
        // `#hero` = 1,0,0 → 100
        let s3 = parse_selector("#hero").unwrap();
        assert_eq!(s3.specificity(), 100);
        // `a.btn[href^="https"]:first-child` = 0,3,1 → 31
        let s4 = parse_selector(r#"a.btn[href^="https"]:first-child"#).unwrap();
        assert_eq!(s4.specificity(), 31);
        // `nav > a#x.y` = 1,1,2 → 112
        let s5 = parse_selector("nav > a#x.y").unwrap();
        assert_eq!(s5.specificity(), 112);
    }

    #[test]
    fn id_vence_a_tag_aunque_llegue_antes() {
        // `#hero { color: blue }` está ANTES que `body p { color: red }`
        // en el stylesheet — sin especificidad, el último (rojo) ganaba.
        // Con especificidad, el #id (100 > 2) gana azul.
        let html = r#"<html><head><style>
            #hero { color: blue }
            body p { color: red }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn clase_vence_a_tag() {
        // `.alert` (10) > `p` (1) aunque ambos matcheen.
        let html = r#"<html><head><style>
            .alert { color: red }
            p { color: blue }
        </style></head><body><p class="alert">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn inline_style_vence_a_id() {
        // Inline tiene especificidad implícita 1000 — gana sobre `#hero`.
        let html = r##"<html><head><style>
            #hero { color: blue }
        </style></head><body><p id="hero" style="color: green">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn empate_de_especificidad_gana_el_ultimo() {
        // Dos selectores con misma especificidad: gana el que llega después.
        let html = r#"<html><head><style>
            p { color: red }
            p { color: blue }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn important_vence_normal_de_mayor_especificidad() {
        // `body p { color: red !important }` (spec=2) debe vencer a
        // `#hero { color: blue }` (spec=100) — important rompe la
        // jerarquía de especificidad dentro del mismo origen.
        let html = r#"<html><head><style>
            body p { color: red !important }
            #hero { color: blue }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn important_inline_vence_important_de_id() {
        // Inline !important vence cualquier !important de selector.
        let html = r##"<html><head><style>
            #hero { color: red !important }
        </style></head><body><p id="hero" style="color: green !important">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn normal_inline_pierde_contra_important_de_regla() {
        // Inline normal (1000) pierde contra !important de cualquier selector.
        let html = r##"<html><head><style>
            p { color: red !important }
        </style></head><body><p style="color: green">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn parsea_border_shorthand() {
        let html = r#"<html><head><style>
            .a { border: 2px solid #ff0000 }
            .b { border: 1px dashed blue !important }
            .c { border: none }
            .d { border-radius: 8px }
        </style></head><body>
          <div class="a"></div><div class="b"></div>
          <div class="c"></div><div class="d"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 4);
        let a = eng.compute(&divs[0]);
        assert!((a.border_widths.top - 2.0).abs() < 1e-6);
        assert_eq!(a.border_colors.top, Some(Color::rgb(255, 0, 0)));
        let b = eng.compute(&divs[1]);
        assert!((b.border_widths.top - 1.0).abs() < 1e-6);
        assert_eq!(b.border_colors.top, Some(Color::rgb(0, 0, 255)));
        let c = eng.compute(&divs[2]);
        assert_eq!(c.border_colors.top, None); // `none` deshabilita
        assert!((c.border_widths.top - 0.0).abs() < 1e-6);
        let d = eng.compute(&divs[3]);
        assert!((d.border_radii.top_left - 8.0).abs() < 1e-6);
    }

    #[test]
    fn parsea_border_per_side() {
        // `border-top: 2px solid red` setea sólo el top; `border-bottom-color`
        // sólo el color del bottom; `border-right-width` sólo el ancho derecho.
        let html = r#"<html><head><style>
            div {
                border-top: 2px solid #ff0000;
                border-bottom-color: #0000ff;
                border-bottom-width: 4px;
                border-bottom-style: solid;
                border-right-width: 1px;
                border-right-color: #00ff00;
                border-right-style: solid;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let s = eng.compute(&div);
        // Top: del shorthand
        assert!((s.border_widths.top - 2.0).abs() < 1e-6);
        assert_eq!(s.border_colors.top, Some(Color::rgb(255, 0, 0)));
        // Bottom: 3 longhand
        assert!((s.border_widths.bottom - 4.0).abs() < 1e-6);
        assert_eq!(s.border_colors.bottom, Some(Color::rgb(0, 0, 255)));
        // Right: 3 longhand
        assert!((s.border_widths.right - 1.0).abs() < 1e-6);
        assert_eq!(s.border_colors.right, Some(Color::rgb(0, 0xff, 0)));
        // Left: no se tocó
        assert_eq!(s.border_widths.left, 0.0);
        assert_eq!(s.border_colors.left, None);
    }

    #[test]
    fn parsea_border_radius_per_corner() {
        let html = r#"<html><head><style>
            div {
                border-top-left-radius: 4px;
                border-top-right-radius: 8px;
                border-bottom-right-radius: 12px;
                border-bottom-left-radius: 16px;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let s = eng.compute(&div);
        assert!((s.border_radii.top_left - 4.0).abs() < 1e-6);
        assert!((s.border_radii.top_right - 8.0).abs() < 1e-6);
        assert!((s.border_radii.bottom_right - 12.0).abs() < 1e-6);
        assert!((s.border_radii.bottom_left - 16.0).abs() < 1e-6);
    }

    #[test]
    fn parsea_border_propiedades_individuales() {
        let html = r#"<html><head><style>
            div { border-width: 3px; border-color: #00ff00; border-style: solid; border-radius: 5px }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let st = eng.compute(&div);
        assert!((st.border_widths.top - 3.0).abs() < 1e-6);
        assert_eq!(st.border_colors.top, Some(Color::rgb(0, 0xff, 0)));
        assert!((st.border_radii.top_left - 5.0).abs() < 1e-6);
    }

    #[test]
    fn hover_state_activa_regla_solo_cuando_corresponde() {
        // `.btn:hover { background: red }`: matchea con hover_active=true,
        // no matchea sin él.
        let html = r##"<html><head><style>
            .btn:hover { background: #ff0000 }
            .btn { background: #ffffff }
        </style></head><body><a class="btn">x</a></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let base = eng.compute_with_parent_in_state(&a, None, false);
        let hover = eng.compute_with_parent_in_state(&a, None, true);
        assert_eq!(base.background, Some(Color::rgb(255, 255, 255)));
        assert_eq!(hover.background, Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn hover_pseudo_aporta_a_specificity() {
        // `.btn:hover` debe tener specificity 0,2,0 → 20 (clase 10 + pseudo 10)
        let s = parse_selector(".btn:hover").unwrap();
        assert_eq!(s.specificity(), 20);
    }

    #[test]
    fn box_tree_expone_hover_background() {
        let html = r##"<html><head><style>
            .btn { background: white }
            .btn:hover { background: #ffaa00 }
        </style></head><body><a class="btn">x</a></body></html>"##;
        let eng = crate::Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut hover_bgs = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                hover_bgs.push(b.hover_background);
            }
        });
        assert_eq!(hover_bgs.len(), 1);
        assert_eq!(hover_bgs[0], Some(Color::rgb(0xff, 0xaa, 0)));
    }

    #[test]
    fn parsea_box_shadow_completo() {
        let html = r#"<html><head><style>
            .a { box-shadow: 2px 4px 8px 1px #000000 }
            .b { box-shadow: 1px 2px red }
            .c { box-shadow: none }
        </style></head><body>
          <div class="a"></div><div class="b"></div><div class="c"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let a = eng.compute(&divs[0]).box_shadow.unwrap();
        assert!((a.offset_x - 2.0).abs() < 1e-6);
        assert!((a.offset_y - 4.0).abs() < 1e-6);
        assert!((a.blur_px - 8.0).abs() < 1e-6);
        assert!((a.spread_px - 1.0).abs() < 1e-6);
        assert_eq!(a.color, Color::BLACK);
        let b = eng.compute(&divs[1]).box_shadow.unwrap();
        assert_eq!(b.color, Color::rgb(255, 0, 0));
        assert!((b.blur_px - 0.0).abs() < 1e-6);
        assert!((b.spread_px - 0.0).abs() < 1e-6);
        let c = eng.compute(&divs[2]).box_shadow;
        assert!(c.is_none());
    }

    #[test]
    fn parse_nth_arg_acepta_formatos_comunes() {
        assert_eq!(parse_nth_arg("odd"), Some((2, 1)));
        assert_eq!(parse_nth_arg("even"), Some((2, 0)));
        assert_eq!(parse_nth_arg("3"), Some((0, 3)));
        assert_eq!(parse_nth_arg("n"), Some((1, 0)));
        assert_eq!(parse_nth_arg("2n"), Some((2, 0)));
        assert_eq!(parse_nth_arg("2n+1"), Some((2, 1)));
        assert_eq!(parse_nth_arg("3n -2"), Some((3, -2)));
        assert_eq!(parse_nth_arg("-n+3"), Some((-1, 3)));
        assert_eq!(parse_nth_arg("xyz"), None);
    }

    #[test]
    fn selector_nth_child_aplica() {
        // `li:nth-child(odd)` matchea li 1, 3 (1-indexed).
        let html = r#"<html><head><style>
            li:nth-child(odd) { color: #f00 }
            li:nth-child(2n) { color: #00f }
        </style></head><body><ul>
          <li>a</li><li>b</li><li>c</li><li>d</li>
        </ul></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 4);
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0xff, 0, 0)); // odd
        assert_eq!(eng.compute(&lis[1]).color, Color::rgb(0, 0, 0xff)); // even (2n)
        assert_eq!(eng.compute(&lis[2]).color, Color::rgb(0xff, 0, 0)); // odd
        assert_eq!(eng.compute(&lis[3]).color, Color::rgb(0, 0, 0xff)); // even
    }

    #[test]
    fn selector_nth_child_n_fija() {
        // `:nth-child(3)` matchea SÓLO la tercera.
        let html = r#"<html><head><style>
            li:nth-child(3) { color: #0a0 }
        </style></head><body><ul>
          <li>1</li><li>2</li><li>3</li><li>4</li>
        </ul></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&lis[0]).color, Color::BLACK);
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&lis[2]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[3]).color, Color::BLACK);
    }

    #[test]
    fn selector_not_excluye() {
        // `p:not(.skip)` matchea todos los <p> excepto los con class skip.
        let html = r#"<html><head><style>
            p:not(.skip) { color: #f00 }
        </style></head><body>
          <p>uno</p>
          <p class="skip">dos</p>
          <p>tres</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0xff, 0, 0));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0xff, 0, 0));
    }

    #[test]
    fn specificity_not_aporta_la_del_argumento() {
        // `:not(#x)` aporta 100 (la del #id interno).
        let s = parse_selector(":not(#x)").unwrap();
        assert_eq!(s.specificity(), 100);
        // `a:not(.b)` aporta 1 (tag) + 10 (.b interno) = 11.
        let s = parse_selector("a:not(.b)").unwrap();
        assert_eq!(s.specificity(), 11);
    }

    #[test]
    fn not_anidado_se_rechaza() {
        // `:not(:not(p))` debe ignorarse, no soportamos recursión.
        assert!(parse_selector(":not(:not(p))").is_none());
    }

    #[test]
    fn cascada_inline_sobrescribe() {
        let html = "<html><head><style>p { color: red }</style></head><body><p style='color:blue'>x</p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let style = eng.compute(&p);
        assert_eq!(style.color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn parsea_text_decoration() {
        assert_eq!(parse_text_decoration("underline"), Some(TextDecorationLine::Underline));
        assert_eq!(parse_text_decoration("line-through"), Some(TextDecorationLine::LineThrough));
        assert_eq!(parse_text_decoration("overline"), Some(TextDecorationLine::Overline));
        assert_eq!(parse_text_decoration("none"), Some(TextDecorationLine::None));
        // Shorthand con varios tokens: capturamos el line, ignoramos color/estilo.
        assert_eq!(
            parse_text_decoration("underline dotted red"),
            Some(TextDecorationLine::Underline)
        );
        assert_eq!(parse_text_decoration("solid red"), None);
    }

    #[test]
    fn ua_aplica_underline_a_link() {
        let html = "<html><body><a href='/x'>click</a></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let style = eng.compute(&a);
        assert_eq!(style.text_decoration, TextDecorationLine::Underline);
    }

    #[test]
    fn ua_aplica_line_through_a_del() {
        let html = "<html><body><del>removed</del></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("del").unwrap();
        let style = eng.compute(&d);
        assert_eq!(style.text_decoration, TextDecorationLine::LineThrough);
    }

    #[test]
    fn text_decoration_se_hereda_a_descendiente_inline() {
        // <a>foo <b>bar</b></a>: el `<b>` debe heredar underline desde `<a>`.
        let html =
            "<html><body><a href='/x'>foo <b>bar</b></a></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let a_style = eng.compute(&a);
        let b = dom.find("b").unwrap();
        let b_style = eng.compute_with_parent(&b, Some(&a_style));
        assert_eq!(b_style.text_decoration, TextDecorationLine::Underline);
    }

    #[test]
    fn parsea_list_style_type() {
        assert_eq!(parse_list_style_type("disc"), Some(ListStyleType::Disc));
        assert_eq!(parse_list_style_type("circle"), Some(ListStyleType::Circle));
        assert_eq!(parse_list_style_type("square"), Some(ListStyleType::Square));
        assert_eq!(parse_list_style_type("decimal"), Some(ListStyleType::Decimal));
        assert_eq!(parse_list_style_type("lower-alpha"), Some(ListStyleType::LowerAlpha));
        assert_eq!(parse_list_style_type("lower-latin"), Some(ListStyleType::LowerAlpha));
        assert_eq!(parse_list_style_type("UPPER-ROMAN"), Some(ListStyleType::UpperRoman));
        assert_eq!(parse_list_style_type("none"), Some(ListStyleType::None));
        assert_eq!(parse_list_style_type("georgian"), None);
    }

    #[test]
    fn parsea_list_style_shorthand() {
        // Cuando aparece un keyword reconocido, se captura.
        assert_eq!(parse_list_style_shorthand("square inside"), Some(ListStyleType::Square));
        assert_eq!(parse_list_style_shorthand("none"), Some(ListStyleType::None));
        // Sin keywords reconocibles, devolvemos None y el caller mantiene
        // el valor anterior.
        assert_eq!(parse_list_style_shorthand("url(foo.png)"), None);
    }

    #[test]
    fn ua_aplica_decimal_a_ol_y_disc_a_ul() {
        let html = "<html><body><ol><li>x</li></ol><ul><li>y</li></ul></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ol = dom.find("ol").unwrap();
        let ul = dom.find("ul").unwrap();
        assert_eq!(eng.compute(&ol).list_style_type, ListStyleType::Decimal);
        assert_eq!(eng.compute(&ul).list_style_type, ListStyleType::Disc);
    }

    #[test]
    fn list_style_type_hereda_de_padre_a_li() {
        // El `<ol>` recibe `decimal` por UA; el `<li>` no tiene regla
        // propia pero hereda el valor.
        let html = "<html><body><ol><li>x</li></ol></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ol = dom.find("ol").unwrap();
        let ol_style = eng.compute_with_parent(&ol, None);
        let li = dom.find("li").unwrap();
        let li_style = eng.compute_with_parent(&li, Some(&ol_style));
        assert_eq!(li_style.list_style_type, ListStyleType::Decimal);
    }

    #[test]
    fn text_decoration_none_override_padre() {
        let html = "<html><head><style>a { text-decoration: none }</style></head><body><a href='/x'>plain</a></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let style = eng.compute(&a);
        assert_eq!(style.text_decoration, TextDecorationLine::None);
    }

    #[test]
    fn parsea_rgb_legacy_y_moderno() {
        // Legacy con comas.
        assert_eq!(parse_color("rgb(255, 0, 0)"), Some(Color::rgb(255, 0, 0)));
        // Moderno con whitespace.
        assert_eq!(parse_color("rgb(0 128 255)"), Some(Color::rgb(0, 128, 255)));
        // Porcentajes.
        assert_eq!(parse_color("rgb(100%, 0%, 50%)"), Some(Color::rgb(255, 0, 128)));
        // Sobre/sub-rango → clamp.
        assert_eq!(parse_color("rgb(300, -10, 128)"), Some(Color::rgb(255, 0, 128)));
    }

    #[test]
    fn parsea_rgba_y_slash_alpha() {
        // Alpha como 4to arg (legacy).
        assert_eq!(parse_color("rgba(255, 0, 0, 0.5)"), Some(Color { r: 255, g: 0, b: 0, a: 128 }));
        // Alpha como porcentaje.
        assert_eq!(parse_color("rgba(0, 0, 0, 50%)"), Some(Color { r: 0, g: 0, b: 0, a: 128 }));
        // Sintaxis moderna `R G B / A`.
        assert_eq!(parse_color("rgb(255 0 0 / 0.5)"), Some(Color { r: 255, g: 0, b: 0, a: 128 }));
        // `rgba` también acepta moderno.
        assert_eq!(parse_color("rgba(0 255 0 / 100%)"), Some(Color::rgb(0, 255, 0)));
    }

    #[test]
    fn parsea_hsl_basico() {
        // hsl(0, 100%, 50%) = rojo puro.
        let red = parse_color("hsl(0, 100%, 50%)").unwrap();
        assert_eq!(red, Color::rgb(255, 0, 0));
        // hsl(120, 100%, 50%) = verde puro.
        let green = parse_color("hsl(120, 100%, 50%)").unwrap();
        assert_eq!(green, Color::rgb(0, 255, 0));
        // hsl(240, 100%, 50%) = azul puro.
        let blue = parse_color("hsl(240, 100%, 50%)").unwrap();
        assert_eq!(blue, Color::rgb(0, 0, 255));
        // hsl(0, 0%, 50%) = gris medio.
        let gray = parse_color("hsl(0, 0%, 50%)").unwrap();
        assert_eq!(gray, Color::rgb(128, 128, 128));
    }

    #[test]
    fn parsea_hsla_con_alpha() {
        let c = parse_color("hsla(0, 100%, 50%, 0.5)").unwrap();
        assert_eq!(c, Color { r: 255, g: 0, b: 0, a: 128 });
        // Moderno con slash.
        let c2 = parse_color("hsl(120 100% 50% / 0.25)").unwrap();
        assert_eq!(c2, Color { r: 0, g: 255, b: 0, a: 64 });
    }

    #[test]
    fn parsea_hex_8_y_4_chars() {
        // #RRGGBBAA.
        assert_eq!(parse_color("#ff000080"), Some(Color { r: 255, g: 0, b: 0, a: 128 }));
        // #RGBA expande cada nibble * 17.
        assert_eq!(parse_color("#f00f"), Some(Color { r: 255, g: 0, b: 0, a: 255 }));
        assert_eq!(parse_color("#0008"), Some(Color { r: 0, g: 0, b: 0, a: 136 }));
    }

    #[test]
    fn named_colors_extendidos() {
        assert_eq!(parse_color("orange"), Some(Color::rgb(255, 165, 0)));
        assert_eq!(parse_color("navy"), Some(Color::rgb(0, 0, 128)));
        assert_eq!(parse_color("teal"), Some(Color::rgb(0, 128, 128)));
        assert_eq!(parse_color("CRIMSON"), Some(Color::rgb(220, 20, 60))); // case-insensitive
        assert_eq!(parse_color("lightblue"), Some(Color::rgb(173, 216, 230)));
        // Alias.
        assert_eq!(parse_color("grey"), parse_color("gray"));
        assert_eq!(parse_color("cyan"), parse_color("aqua"));
        assert_eq!(parse_color("magenta"), parse_color("fuchsia"));
    }

    #[test]
    fn parsea_sides_shorthand_1_2_3_4() {
        assert_eq!(parse_sides("10px"), Some(Sides::all(10.0)));
        assert_eq!(
            parse_sides("10px 20px"),
            Some(Sides { top: 10.0, right: 20.0, bottom: 10.0, left: 20.0 }),
        );
        assert_eq!(
            parse_sides("10px 20px 30px"),
            Some(Sides { top: 10.0, right: 20.0, bottom: 30.0, left: 20.0 }),
        );
        assert_eq!(
            parse_sides("10px 20px 30px 40px"),
            Some(Sides { top: 10.0, right: 20.0, bottom: 30.0, left: 40.0 }),
        );
        // 5 valores → inválido.
        assert_eq!(parse_sides("1px 2px 3px 4px 5px"), None);
        // Token no-longitud → inválido.
        assert_eq!(parse_sides("10px bad 20px"), None);
    }

    #[test]
    fn margin_shorthand_aplica_4_lados() {
        let html = r#"<html><head><style>
            div { margin: 5px 10px 15px 20px }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.margin.top, 5.0);
        assert_eq!(s.margin.right, 10.0);
        assert_eq!(s.margin.bottom, 15.0);
        assert_eq!(s.margin.left, 20.0);
    }

    #[test]
    fn padding_shorthand_2_valores_eje_vertical_horizontal() {
        let html = r#"<html><head><style>
            div { padding: 8px 16px }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.padding.top, 8.0);
        assert_eq!(s.padding.bottom, 8.0);
        assert_eq!(s.padding.left, 16.0);
        assert_eq!(s.padding.right, 16.0);
    }

    #[test]
    fn margin_individual_pisa_shorthand_por_cascada() {
        // El shorthand setea todo a 10px, después `margin-top: 50px` lo pisa.
        let html = r#"<html><head><style>
            div { margin: 10px; margin-top: 50px }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.margin.top, 50.0);
        assert_eq!(s.margin.right, 10.0);
        assert_eq!(s.margin.bottom, 10.0);
        assert_eq!(s.margin.left, 10.0);
    }

    #[test]
    fn parsea_display_flex_y_inline_flex() {
        assert_eq!(parse_display("flex"), Some(Display::Flex));
        assert_eq!(parse_display("inline-flex"), Some(Display::InlineFlex));
        assert_eq!(parse_display("FLEX"), Some(Display::Flex));
    }

    #[test]
    fn parsea_flex_direction() {
        assert_eq!(parse_flex_direction("row"), Some(FlexDirection::Row));
        assert_eq!(parse_flex_direction("column"), Some(FlexDirection::Column));
        assert_eq!(parse_flex_direction("row-reverse"), Some(FlexDirection::RowReverse));
        assert_eq!(parse_flex_direction("column-reverse"), Some(FlexDirection::ColumnReverse));
        assert_eq!(parse_flex_direction("diagonal"), None);
    }

    #[test]
    fn parsea_justify_y_align() {
        // Aceptamos los alias `flex-start`/`flex-end` ↔ `start`/`end`.
        assert_eq!(parse_justify_content("flex-start"), Some(JustifyContent::Start));
        assert_eq!(parse_justify_content("space-between"), Some(JustifyContent::SpaceBetween));
        assert_eq!(parse_justify_content("space-around"), Some(JustifyContent::SpaceAround));
        assert_eq!(parse_align_items("flex-end"), Some(AlignItems::End));
        assert_eq!(parse_align_items("stretch"), Some(AlignItems::Stretch));
        assert_eq!(parse_align_items("baseline"), Some(AlignItems::Baseline));
    }

    #[test]
    fn parsea_flex_wrap() {
        assert_eq!(parse_flex_wrap("nowrap"), Some(FlexWrap::NoWrap));
        assert_eq!(parse_flex_wrap("wrap"), Some(FlexWrap::Wrap));
        assert_eq!(parse_flex_wrap("wrap-reverse"), Some(FlexWrap::WrapReverse));
    }

    #[test]
    fn parsea_gap_1_y_2_valores() {
        assert_eq!(parse_gap("12px"), Some((12.0, 12.0)));
        assert_eq!(parse_gap("4px 8px"), Some((4.0, 8.0)));
        assert_eq!(parse_gap("0"), Some((0.0, 0.0)));
        assert_eq!(parse_gap("a b c"), None);
    }

    #[test]
    fn computa_flex_container_completo() {
        let html = r#"<html><head><style>
            .row {
                display: flex;
                flex-direction: row;
                justify-content: space-between;
                align-items: center;
                gap: 16px 24px;
                flex-wrap: wrap;
            }
        </style></head><body><div class="row"><span>a</span><span>b</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.display, Display::Flex);
        assert_eq!(s.flex_direction, FlexDirection::Row);
        assert_eq!(s.justify_content, JustifyContent::SpaceBetween);
        assert_eq!(s.align_items, AlignItems::Center);
        assert_eq!(s.flex_wrap, FlexWrap::Wrap);
        assert_eq!(s.gap_row, 16.0);
        assert_eq!(s.gap_column, 24.0);
    }

    #[test]
    fn row_gap_y_column_gap_individuales_pisan_shorthand() {
        let html = r#"<html><head><style>
            div {
                display: flex;
                gap: 10px;
                row-gap: 30px;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        // row-gap pisa la mitad del shorthand; column-gap del shorthand sigue (10).
        assert_eq!(s.gap_row, 30.0);
        assert_eq!(s.gap_column, 10.0);
    }

    #[test]
    fn css_var_basico_sobre_root() {
        let html = r#"<html><head><style>
            :root { --primary: #ff0000 }
            p { color: var(--primary) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn css_var_con_fallback() {
        // `--missing` no existe → usa el fallback `blue`.
        let html = r#"<html><head><style>
            p { color: var(--missing, blue) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn css_var_se_declara_en_html_y_asterisco() {
        // Variables declaradas en `html` y `*` también valen (no solo `:root`).
        let html = r#"<html><head><style>
            html { --a: #aa0000 }
            * { --b: 5px }
            p { color: var(--a); margin: var(--b) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.color, Color::rgb(0xaa, 0, 0));
        assert_eq!(s.margin.top, 5.0);
    }

    #[test]
    fn css_var_recursiva() {
        // `--secondary` se define como `var(--primary)` — la sustitución
        // debe resolver hasta el valor base.
        let html = r#"<html><head><style>
            :root {
                --primary: rgb(0, 200, 100);
                --secondary: var(--primary);
            }
            p { color: var(--secondary) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 200, 100));
    }

    #[test]
    fn css_var_en_inline_style() {
        // `style="..."` también debe resolver var().
        let html = r#"<html><head><style>
            :root { --hi: hsl(120, 100%, 50%) }
        </style></head><body>
          <p style="background: var(--hi)">x</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).background, Some(Color::rgb(0, 255, 0)));
    }

    #[test]
    fn css_var_inexistente_sin_fallback_borra_declaracion() {
        // `var(--nope)` sin fallback resuelve a "" — el parser de color
        // rechaza el value y la decl se ignora silenciosamente.
        // El color debe quedar en el default BLACK heredado.
        let html = r#"<html><head><style>
            p { color: var(--nope) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::BLACK);
    }

    #[test]
    fn css_var_multiple_en_un_value() {
        // Shorthand `border: var(--w) solid var(--c)`.
        let html = r#"<html><head><style>
            :root { --w: 3px; --c: orange }
            div { border: var(--w) solid var(--c) }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert!((s.border_widths.top - 3.0).abs() < 1e-6);
        assert_eq!(s.border_colors.top, Some(Color::rgb(255, 165, 0)));
    }

    #[test]
    fn parsea_box_sizing() {
        assert_eq!(parse_box_sizing("content-box"), Some(BoxSizing::ContentBox));
        assert_eq!(parse_box_sizing("border-box"), Some(BoxSizing::BorderBox));
        assert_eq!(parse_box_sizing("WeIrD"), None);
    }

    #[test]
    fn computa_min_max_sizes() {
        let html = r#"<html><head><style>
            div {
                min-width: 100px;
                min-height: 50px;
                max-height: 200px;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert!(matches!(s.min_width, LengthVal::Px(100.0)));
        assert!(matches!(s.min_height, LengthVal::Px(50.0)));
        assert!(matches!(s.max_height, LengthVal::Px(200.0)));
    }

    #[test]
    fn parsea_overflow_alias() {
        assert_eq!(parse_overflow("visible"), Some(Overflow::Visible));
        assert_eq!(parse_overflow("hidden"), Some(Overflow::Hidden));
        assert_eq!(parse_overflow("auto"), Some(Overflow::Hidden));
        assert_eq!(parse_overflow("scroll"), Some(Overflow::Hidden));
        assert_eq!(parse_overflow("clip"), Some(Overflow::Hidden));
    }

    #[test]
    fn parsea_white_space_y_se_hereda() {
        let html = r#"<html><head><style>
            pre { white-space: pre }
        </style></head><body><pre>line1
line2</pre></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let pre = dom.find("pre").unwrap();
        let s = eng.compute(&pre);
        assert_eq!(s.white_space, WhiteSpace::Pre);
    }

    #[test]
    fn parsea_text_transform_y_se_hereda() {
        let html = r#"<html><head><style>
            p { text-transform: uppercase }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.text_transform, TextTransform::Uppercase);
    }

    #[test]
    fn parsea_opacity_clampa() {
        assert_eq!(parse_opacity("0.5"), Some(0.5));
        assert_eq!(parse_opacity("100%"), Some(1.0));
        assert_eq!(parse_opacity("0"), Some(0.0));
        assert_eq!(parse_opacity("2"), Some(1.0)); // clamp arriba
        assert_eq!(parse_opacity("-0.5"), Some(0.0)); // clamp abajo
    }

    #[test]
    fn parsea_align_self() {
        assert_eq!(parse_align_self("auto"), Some(AlignSelf::Auto));
        assert_eq!(parse_align_self("flex-end"), Some(AlignSelf::End));
        assert_eq!(parse_align_self("stretch"), Some(AlignSelf::Stretch));
    }

    #[test]
    fn parsea_flex_shorthand_presets() {
        let decls = parse_flex_shorthand("none", false);
        assert_eq!(decls.len(), 3);
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 0.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 0.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Auto)));

        let decls = parse_flex_shorthand("auto", false);
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 1.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 1.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Auto)));

        let decls = parse_flex_shorthand("1", false);
        // `flex: 1` ⇒ `1 1 0%`
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 1.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 1.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Pct(0.0))));
    }

    #[test]
    fn parsea_flex_shorthand_3_valores() {
        let decls = parse_flex_shorthand("2 0 200px", false);
        assert_eq!(decls.len(), 3);
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 2.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 0.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Px(200.0))));
    }

    #[test]
    fn parsea_outline_shorthand() {
        let decls = parse_outline_shorthand("2px solid orange", false);
        let mut has_w = false; let mut has_s = false; let mut has_c = false;
        for d in &decls {
            match &d.kind {
                DeclKind::OutlineWidth(w) => { has_w = (*w - 2.0).abs() < 1e-6; }
                DeclKind::OutlineStyle(active) => { has_s = *active; }
                DeclKind::OutlineColor(c) => { has_c = *c == Color::rgb(255, 165, 0); }
                _ => {}
            }
        }
        assert!(has_w && has_s && has_c);

        let decls = parse_outline_shorthand("none", false);
        assert_eq!(decls.len(), 1);
        assert!(matches!(decls[0].kind, DeclKind::OutlineStyle(false)));
    }

    #[test]
    fn parsea_linear_gradient_basico() {
        let g = parse_linear_gradient("to right, #f00, #00f").unwrap();
        assert!((g.angle_deg - 90.0).abs() < 1e-6);
        assert_eq!(g.stops.len(), 2);
        assert_eq!(g.stops[0].color, Color::rgb(255, 0, 0));
        assert_eq!(g.stops[1].color, Color::rgb(0, 0, 255));

        let g = parse_linear_gradient("45deg, red 0%, blue 100%").unwrap();
        assert!((g.angle_deg - 45.0).abs() < 1e-6);
        assert_eq!(g.stops[0].pos, Some(0.0));
        assert_eq!(g.stops[1].pos, Some(1.0));

        // Default 180 (top→bottom) cuando no se da dirección.
        let g = parse_linear_gradient("red, blue").unwrap();
        assert!((g.angle_deg - 180.0).abs() < 1e-6);
    }

    #[test]
    fn parsea_background_image_gradient_y_none() {
        // `background-image: linear-gradient(...)` produce un Gradient.
        let html = r#"<html><head><style>
            div { background-image: linear-gradient(to right, red, blue) }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert!(s.background_gradient.is_some());

        // `background-image: none` deshabilita.
        let html2 = r#"<html><head><style>
            div { background-image: linear-gradient(red, blue); background-image: none }
        </style></head><body><div></div></body></html>"#;
        let dom2 = DomTree::parse(html2);
        let eng2 = StyleEngine::from_dom(&dom2);
        let d2 = dom2.find("div").unwrap();
        assert!(eng2.compute(&d2).background_gradient.is_none());
    }

    #[test]
    fn parsea_padding_individual_4_lados() {
        let html = r#"<html><head><style>
            div {
                padding-top: 1px;
                padding-right: 2px;
                padding-bottom: 3px;
                padding-left: 4px;
            }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.padding.top, 1.0);
        assert_eq!(s.padding.right, 2.0);
        assert_eq!(s.padding.bottom, 3.0);
        assert_eq!(s.padding.left, 4.0);
    }

    #[test]
    fn parsea_position_y_insets() {
        let html = r#"<html><head><style>
            div { position: absolute; top: 10px; left: 50%; bottom: auto; right: 20px }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.position, Position::Absolute);
        assert!(matches!(s.inset_top, LengthVal::Px(10.0)));
        assert!(matches!(s.inset_left, LengthVal::Pct(50.0)));
        assert!(matches!(s.inset_bottom, LengthVal::Auto));
        assert!(matches!(s.inset_right, LengthVal::Px(20.0)));

        let dom2 = DomTree::parse(r#"<html><body><nav style="position:sticky"></nav></body></html>"#);
        let eng2 = StyleEngine::from_dom(&dom2);
        let n = dom2.find("nav").unwrap();
        assert_eq!(eng2.compute(&n).position, Position::Sticky);
    }

    #[test]
    fn parsea_transforms_cadena() {
        let t = parse_transforms("translate(10px, 20px) scale(2) rotate(45deg)").unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0], Transform::Translate(10.0, 20.0));
        assert_eq!(t[1], Transform::Scale(2.0, 2.0));
        assert_eq!(t[2], Transform::Rotate(45.0));

        let t = parse_transforms("translateX(5px) scaleY(0.5) rotate(0.5turn)").unwrap();
        assert_eq!(t[0], Transform::Translate(5.0, 0.0));
        assert_eq!(t[1], Transform::Scale(1.0, 0.5));
        assert_eq!(t[2], Transform::Rotate(180.0));

        assert!(parse_transforms("none").unwrap().is_empty());
    }

    #[test]
    fn parsea_text_shadow_simple_y_multiple() {
        let sh = parse_text_shadows("2px 3px 4px red").unwrap();
        assert_eq!(sh.len(), 1);
        assert_eq!(sh[0].offset_x, 2.0);
        assert_eq!(sh[0].offset_y, 3.0);
        assert_eq!(sh[0].blur_px, 4.0);
        assert_eq!(sh[0].color, Color::rgb(255, 0, 0));

        let sh = parse_text_shadows("1px 1px black, -1px -1px white").unwrap();
        assert_eq!(sh.len(), 2);
        assert_eq!(sh[0].color, Color::BLACK);
        assert_eq!(sh[1].color, Color::WHITE);
        assert_eq!(sh[1].offset_x, -1.0);

        let sh = parse_text_shadows("none").unwrap();
        assert!(sh.is_empty());
    }

    #[test]
    fn parsea_vertical_align() {
        assert_eq!(parse_vertical_align("baseline"), Some(VerticalAlign::Baseline));
        assert_eq!(parse_vertical_align("middle"), Some(VerticalAlign::Middle));
        assert_eq!(parse_vertical_align("text-top"), Some(VerticalAlign::Top));
        assert_eq!(parse_vertical_align("super"), Some(VerticalAlign::Super));
    }

    #[test]
    fn parsea_visibility_y_pointer_events_heredan() {
        let html = r#"<html><head><style>
            .h { visibility: hidden; pointer-events: none }
        </style></head><body>
          <div class="h"><p>oculto</p></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let p = dom.find("p").unwrap();
        let d_style = eng.compute_with_parent(&d, None);
        let p_style = eng.compute_with_parent(&p, Some(&d_style));
        assert_eq!(p_style.visibility, Visibility::Hidden);
        assert_eq!(p_style.pointer_events, PointerEvents::None);
    }

    #[test]
    fn parsea_text_indent_y_word_spacing_heredan() {
        let html = r#"<html><head><style>
            p { text-indent: 30px; word-spacing: 5px }
        </style></head><body><p>x <span>y</span></p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let span = dom.find("span").unwrap();
        let p_style = eng.compute(&p);
        let span_style = eng.compute_with_parent(&span, Some(&p_style));
        assert_eq!(p_style.text_indent, 30.0);
        assert_eq!(p_style.word_spacing, 5.0);
        assert_eq!(span_style.word_spacing, 5.0);
        assert_eq!(span_style.text_indent, 30.0);
    }

    #[test]
    fn parsea_display_grid_y_template() {
        let html = r#"<html><head><style>
            .grid {
                display: grid;
                grid-template-columns: 100px 1fr 2fr;
                grid-template-rows: repeat(3, auto);
                grid-gap: 8px 16px;
            }
        </style></head><body><div class="grid"></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.display, Display::Grid);
        assert_eq!(s.grid_template_columns.len(), 3);
        assert!(matches!(s.grid_template_columns[0], GridTrackSize::Px(100.0)));
        assert!(matches!(s.grid_template_columns[1], GridTrackSize::Fr(1.0)));
        assert!(matches!(s.grid_template_columns[2], GridTrackSize::Fr(2.0)));
        assert_eq!(s.grid_template_rows.len(), 3);
        assert!(matches!(s.grid_template_rows[0], GridTrackSize::Auto));
        assert_eq!(s.gap_row, 8.0);
        assert_eq!(s.gap_column, 16.0);
    }

    #[test]
    fn unidades_viewport_resuelven() {
        assert_eq!(parse_length_px("50vw"), Some(640.0));
        assert_eq!(parse_length_px("25vh"), Some(200.0));
        assert_eq!(parse_length_px("10vmin"), Some(80.0));
        assert_eq!(parse_length_px("10vmax"), Some(128.0));
    }

    #[test]
    fn viewport_scope_cambia_y_restaura_la_resolucion() {
        // Fuera de scope: DEFAULT_VIEWPORT (1280×800).
        assert_eq!(parse_length_px("50vw"), Some(640.0));
        {
            let _g = ViewportScope::new(Viewport { width: 800.0, height: 600.0, dpr: 1.0 });
            assert_eq!(parse_length_px("50vw"), Some(400.0));
            assert_eq!(parse_length_px("50vh"), Some(300.0));
            assert_eq!(parse_length_px("50vmin"), Some(300.0));
            assert_eq!(parse_length_px("50vmax"), Some(400.0));
            // Anida: el scope interno gana y el externo se recupera al salir.
            {
                let _g2 = ViewportScope::new(Viewport { width: 200.0, height: 200.0, dpr: 1.0 });
                assert_eq!(parse_length_px("50vw"), Some(100.0));
            }
            assert_eq!(parse_length_px("50vw"), Some(400.0));
        }
        // Al dropear el guard, vuelve a DEFAULT.
        assert_eq!(parse_length_px("50vw"), Some(640.0));
    }

    #[test]
    fn media_query_filtra_segun_viewport() {
        assert!(!evaluate_media_query("(max-width: 600px)", DEFAULT_VIEWPORT));
        assert!(evaluate_media_query("(min-width: 1024px)", DEFAULT_VIEWPORT));
        assert!(evaluate_media_query(
            "(min-width: 800px) and (max-width: 1920px)",
            DEFAULT_VIEWPORT,
        ));
        assert!(!evaluate_media_query("print", DEFAULT_VIEWPORT));
        assert!(evaluate_media_query("screen", DEFAULT_VIEWPORT));

        let html = r#"<html><head><style>
            @media (max-width: 600px) { p { color: red } }
            @media (min-width: 1024px) { p { color: blue } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn media_query_orientation_resolution_y_combinadores() {
        let portrait = Viewport { width: 400.0, height: 900.0, dpr: 1.0 };
        let landscape = Viewport { width: 900.0, height: 400.0, dpr: 1.0 };
        let retina = Viewport { width: 900.0, height: 400.0, dpr: 2.0 };

        // orientation.
        assert!(evaluate_media_query("(orientation: portrait)", portrait));
        assert!(!evaluate_media_query("(orientation: portrait)", landscape));
        assert!(evaluate_media_query("(orientation: landscape)", landscape));

        // resolution (dppx / x / dpi).
        assert!(evaluate_media_query("(min-resolution: 2dppx)", retina));
        assert!(!evaluate_media_query("(min-resolution: 2dppx)", landscape));
        assert!(evaluate_media_query("(min-resolution: 2x)", retina));
        assert!(evaluate_media_query("(min-resolution: 192dpi)", retina));
        assert!(evaluate_media_query("(max-resolution: 1dppx)", landscape));

        // Lista OR (`,`): matchea si cualquiera lo hace.
        assert!(evaluate_media_query("(max-width: 100px), (orientation: landscape)", landscape));
        assert!(!evaluate_media_query("(max-width: 100px), (max-height: 100px)", landscape));

        // `not` invierte la query completa.
        assert!(evaluate_media_query("not (max-width: 100px)", landscape));
        assert!(!evaluate_media_query("not (orientation: landscape)", landscape));

        // Preferencias: reportamos tema claro y sin reducción de movimiento.
        assert!(evaluate_media_query("(prefers-color-scheme: light)", landscape));
        assert!(!evaluate_media_query("(prefers-color-scheme: dark)", landscape));
        assert!(evaluate_media_query("(prefers-reduced-motion: no-preference)", landscape));

        // `and` mezclando dimensión + orientación + resolución.
        assert!(evaluate_media_query(
            "screen and (min-width: 800px) and (orientation: landscape) and (min-resolution: 2dppx)",
            retina,
        ));
        assert!(!evaluate_media_query(
            "screen and (min-width: 800px) and (min-resolution: 2dppx)",
            landscape, // dpr 1.0 → falla la última
        ));

        // aspect-ratio (W/H y número). landscape = 900/400 = 2.25.
        assert!(evaluate_media_query("(min-aspect-ratio: 16/9)", landscape)); // 2.25 >= 1.77
        assert!(!evaluate_media_query("(min-aspect-ratio: 16/9)", portrait)); // 0.44 < 1.77
        assert!(evaluate_media_query("(max-aspect-ratio: 1/1)", portrait)); // 0.44 <= 1.0
        assert!(!evaluate_media_query("(max-aspect-ratio: 1/1)", landscape)); // 2.25 > 1.0
        assert!(evaluate_media_query("(min-aspect-ratio: 2)", landscape)); // 2.25 >= 2

        // Feature desconocida no descalifica (lenient, igual que antes).
        assert!(evaluate_media_query("(quantum-foam: 3)", landscape));
    }

    #[test]
    fn from_dom_with_viewport_selecciona_media_por_ancho_real() {
        let html = r#"<html><head><style>
            p { color: green }
            @media (max-width: 600px) { p { color: red } }
            @media (min-width: 601px) { p { color: blue } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);

        // Viewport angosto → gana la regla red.
        let eng = StyleEngine::from_dom_with_viewport(&dom, Viewport { width: 500.0, height: 800.0, dpr: 1.0 });
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0), "ancho 500 → red");

        // Viewport ancho → gana la regla blue.
        let eng = StyleEngine::from_dom_with_viewport(&dom, Viewport { width: 1200.0, height: 800.0, dpr: 1.0 });
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255), "ancho 1200 → blue");

        // `from_dom` sin viewport cae en DEFAULT_VIEWPORT (1280) → blue.
        let eng = StyleEngine::from_dom(&dom);
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255), "default 1280 → blue");
    }

    #[test]
    fn ua_body_lleva_margin_8() {
        // Cualquier página sin CSS de autor debe arrancar con el body
        // margin: 8px (default del browser real).
        let html = "<html><body>x</body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let body = dom.find("body").unwrap();
        let s = eng.compute(&body);
        assert_eq!(s.margin, Sides::all(8.0));
    }

    #[test]
    fn ua_h3_h4_h5_h6_tienen_tamanos_propios() {
        // Antes h3+ caían al default 16 (igual que `<p>`). Ahora cada
        // nivel tiene tamaño y margin propios.
        for (tag, expected) in
            [("h3", 19.0), ("h4", 16.0), ("h5", 13.0), ("h6", 11.0)]
        {
            let html = format!("<html><body><{tag}>x</{tag}></body></html>");
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            let node = dom.find(tag).unwrap();
            let s = eng.compute(&node);
            assert_eq!(s.font_size, expected, "{tag} font-size");
        }
    }

    #[test]
    fn ua_ul_y_ol_padding_left_para_bullets() {
        let html = "<html><body><ul><li>x</li></ul></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ul = dom.find("ul").unwrap();
        let s = eng.compute(&ul);
        assert_eq!(s.padding.left, 40.0);
    }

    #[test]
    fn ua_a_color_azul_default() {
        let html = "<html><body><a href=#>link</a></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let s = eng.compute(&a);
        assert_eq!(s.color, Color::rgb(0, 0, 238));
    }

    #[test]
    fn ua_svg_inline_block_canvas_none() {
        // SVG ahora se renderiza (primitivas básicas vía vello), así que
        // queda como inline-block. canvas/math/video/etc. siguen
        // ocultos hasta que tengan renderer.
        let html = "<html><body><svg></svg><canvas></canvas></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let svg = dom.find("svg").unwrap();
        let canvas = dom.find("canvas").unwrap();
        assert_eq!(eng.compute(&svg).display, Display::InlineBlock);
        assert_eq!(eng.compute(&canvas).display, Display::None);
    }

    #[test]
    fn ua_table_layout_minimo() {
        let html = "<html><body><table><tr><td>a</td><td>b</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let table = dom.find("table").unwrap();
        let tr = dom.find("tr").unwrap();
        let td = dom.find("td").unwrap();
        assert_eq!(eng.compute(&table).display, Display::Block);
        // tr es Flex row para que td/td queden lado a lado.
        assert_eq!(eng.compute(&tr).display, Display::Flex);
        // td es InlineBlock para que el row de flex no le dé 100% width.
        assert_eq!(eng.compute(&td).display, Display::InlineBlock);
    }

    #[test]
    fn ua_table_cells_tienen_border_y_padding() {
        // Tablas sin CSS de autor deben mostrar bordes para que la grilla
        // se vea — sino tablas sin estilo (Wikipedia raw, RFC docs, etc.)
        // colapsan visualmente.
        let html = "<html><body><table><tr><th>h</th><td>d</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let th = dom.find("th").unwrap();
        let td = dom.find("td").unwrap();
        let s_th = eng.compute(&th);
        let s_td = eng.compute(&td);
        assert_eq!(s_th.border_widths.top, 1.0);
        assert!(s_th.border_colors.top.is_some());
        assert_eq!(s_td.border_widths.top, 1.0);
        assert_eq!(s_th.padding, Sides::all(4.0));
        assert_eq!(s_td.padding, Sides::all(4.0));
        // `<th>` lleva un bg gris claro para destacarlo como header.
        assert_eq!(s_th.background, Some(Color::rgb(242, 242, 242)));
    }

    #[test]
    fn ua_colgroup_y_col_ocultos() {
        // `<colgroup><col>` son metadatos de columna — no se renderean.
        let html = "<html><body><table><colgroup><col><col></colgroup><tr><td>x</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let colgroup = dom.find("colgroup").unwrap();
        let col = dom.find("col").unwrap();
        assert_eq!(eng.compute(&colgroup).display, Display::None);
        assert_eq!(eng.compute(&col).display, Display::None);
    }

    #[test]
    fn ua_caption_centrado() {
        let html = "<html><body><table><caption>Tabla X</caption><tr><td>a</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let cap = dom.find("caption").unwrap();
        let s = eng.compute(&cap);
        assert_eq!(s.display, Display::Block);
        assert_eq!(s.text_align, TextAlign::Center);
    }

    #[test]
    fn ua_sub_y_sup_aplican_vertical_align() {
        let html = "<html><body><p>H<sub>2</sub>O y E=mc<sup>2</sup></p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let sub = dom.find("sub").unwrap();
        let sup = dom.find("sup").unwrap();
        assert_eq!(eng.compute(&sub).vertical_align, VerticalAlign::Sub);
        assert_eq!(eng.compute(&sup).vertical_align, VerticalAlign::Super);
    }

    #[test]
    fn supports_query_filtra_por_parser() {
        assert!(evaluate_supports_query("(display: flex)"));
        assert!(evaluate_supports_query("(color: red)"));
        assert!(!evaluate_supports_query("(display: garbage)"));

        let html = r#"<html><head><style>
            @supports (display: flex) { p { color: green } }
            @supports (display: garbage) { p { color: red } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    // === Fase B1: @keyframes ===

    #[test]
    fn keyframes_from_to_se_parsean() {
        let html = r#"<html><head><style>
            @keyframes fade {
                from { opacity: 0; }
                to { opacity: 1; }
            }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let kf = eng.keyframes().get("fade").expect("keyframes fade ausente");
        assert_eq!(kf.steps.len(), 2);
        assert_eq!(kf.steps[0].offset, 0.0);
        assert_eq!(kf.steps[0].declarations, vec![("opacity".into(), "0".into())]);
        assert_eq!(kf.steps[1].offset, 1.0);
        assert_eq!(kf.steps[1].declarations, vec![("opacity".into(), "1".into())]);
    }

    #[test]
    fn keyframes_porcentajes_y_orden() {
        // Pasos declarados fuera de orden deben quedar ordenados por offset.
        let html = r#"<html><head><style>
            @keyframes slide {
                100% { left: 100px; }
                0% { left: 0px; }
                50% { left: 40px; top: 10px; }
            }
        </style></head><body></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let kf = eng.keyframes().get("slide").unwrap();
        let offsets: Vec<f32> = kf.steps.iter().map(|s| s.offset).collect();
        assert_eq!(offsets, vec![0.0, 0.5, 1.0]);
        // El paso del 50% conserva las dos declaraciones en orden.
        assert_eq!(
            kf.steps[1].declarations,
            vec![("left".into(), "40px".into()), ("top".into(), "10px".into())]
        );
    }

    #[test]
    fn keyframes_selector_multiple_comparte_decls() {
        // `0%, 100% { ... }` genera dos pasos con las mismas decls.
        let html = r#"<html><head><style>
            @keyframes pulse {
                0%, 100% { transform: scale(1); }
                50% { transform: scale(1.2); }
            }
        </style></head><body></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let kf = eng.keyframes().get("pulse").unwrap();
        assert_eq!(kf.steps.len(), 3);
        assert_eq!(kf.steps[0].offset, 0.0);
        assert_eq!(kf.steps[2].offset, 1.0);
        assert_eq!(kf.steps[0].declarations, kf.steps[2].declarations);
    }

    #[test]
    fn keyframes_prefijo_vendor_y_no_rompe_reglas_normales() {
        // `@-webkit-keyframes` se captura igual; y las reglas normales
        // alrededor del at-rule siguen aplicándose.
        let html = r#"<html><head><style>
            p { color: red; }
            @-webkit-keyframes spin { from { opacity: 0 } to { opacity: 1 } }
            p { color: green; }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        assert!(eng.keyframes().contains_key("spin"));
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    // === Fase B2: animation shorthand ===

    fn anim_de(decl: &str) -> AnimationBinding {
        let html = format!("<html><body><p style=\"{decl}\">x</p></body></html>");
        let dom = DomTree::parse(&html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        eng.compute(&p).animation.expect("animation ausente")
    }

    #[test]
    fn animation_shorthand_completo() {
        let a = anim_de("animation: spin 2s ease-in-out 0.5s infinite alternate forwards");
        assert_eq!(a.name, "spin");
        assert_eq!(a.duration_s, 2.0);
        assert_eq!(a.timing, EasingFunction::EaseInOut);
        assert_eq!(a.delay_s, 0.5);
        assert_eq!(a.iterations, AnimationIterations::Infinite);
        assert_eq!(a.direction, AnimationDirection::Alternate);
        assert_eq!(a.fill_mode, AnimationFillMode::Forwards);
    }

    #[test]
    fn animation_orden_laxo_y_defaults() {
        // Tokens en orden no canónico + count numérico + ms.
        let a = anim_de("animation: 200ms linear 3 fade");
        assert_eq!(a.name, "fade");
        assert!((a.duration_s - 0.2).abs() < 1e-6);
        assert_eq!(a.timing, EasingFunction::Linear);
        assert_eq!(a.iterations, AnimationIterations::Count(3.0));
        assert_eq!(a.delay_s, 0.0);
        assert_eq!(a.direction, AnimationDirection::Normal);
        assert_eq!(a.fill_mode, AnimationFillMode::None);
    }

    #[test]
    fn animation_cubic_bezier_no_se_parte_por_comas() {
        let a = anim_de("animation: bounce 1s cubic-bezier(0.1, 0.7, 1.0, 0.1)");
        assert_eq!(a.name, "bounce");
        assert_eq!(a.duration_s, 1.0);
        assert_eq!(a.timing, EasingFunction::CubicBezier(0.1, 0.7, 1.0, 0.1));
    }

    #[test]
    fn animation_none_limpia() {
        let html = r#"<html><body><p style="animation: none">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).animation, None);
    }

    // === Fase B3: transition shorthand ===

    fn trans_de(decl: &str) -> Vec<TransitionBinding> {
        let html = format!("<html><body><p style=\"{decl}\">x</p></body></html>");
        let dom = DomTree::parse(&html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        eng.compute(&p).transitions
    }

    #[test]
    fn transition_simple() {
        let t = trans_de("transition: opacity 200ms ease");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].property, "opacity");
        assert!((t[0].duration_s - 0.2).abs() < 1e-6);
        assert_eq!(t[0].timing, EasingFunction::Ease);
        assert_eq!(t[0].delay_s, 0.0);
    }

    #[test]
    fn transition_lista_multiple() {
        let t = trans_de("transition: opacity 200ms ease, transform 0.3s ease-in 0.1s");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].property, "opacity");
        assert_eq!(t[1].property, "transform");
        assert!((t[1].duration_s - 0.3).abs() < 1e-6);
        assert_eq!(t[1].timing, EasingFunction::EaseIn);
        assert!((t[1].delay_s - 0.1).abs() < 1e-6);
    }

    #[test]
    fn transition_default_property_es_all() {
        // Sin nombre de propiedad, default `all` (CSS spec).
        let t = trans_de("transition: 1s");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].property, "all");
        assert_eq!(t[0].duration_s, 1.0);
        assert_eq!(t[0].timing, EasingFunction::Ease);
    }

    #[test]
    fn transition_steps_y_none() {
        let t = trans_de("transition: width 2s steps(4, end)");
        assert_eq!(t[0].timing, EasingFunction::Steps(4, false));

        let html = r#"<html><body><p style="transition: none">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert!(eng.compute(&p).transitions.is_empty());
    }
}
