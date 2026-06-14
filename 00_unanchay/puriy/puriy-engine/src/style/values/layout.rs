//! Tipos de layout/flex/grid/gradientes/fondo/animación, viewport, Sides/Corners y helpers set_*.
//! Tipos de valores CSS extraídos de `values.rs` (regla #1). Sin cambios de lógica.
use super::*;

/// `font-kerning`. Heredable. Default `Auto`. Fase 7.259.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontKerning {
    #[default]
    Auto,
    Normal,
    None,
}

/// Un entry de `font-feature-settings`: tag de 4 bytes + valor entero
/// (0 = off, 1 = on, N = índice de variante). Fase 7.260.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFeatureSetting {
    /// 4 ASCII chars (case-sensitive por OpenType). Sin validar contra
    /// `[a-zA-Z0-9]` por simplicidad — el shaper hace la verificación final.
    pub tag: [u8; 4],
    pub value: i32,
}

/// Un entry de `font-variation-settings`: tag de 4 bytes + valor
/// número (`wght 700`, `wdth 100`, `slnt -15`...). Fase 7.261.
#[derive(Debug, Clone, PartialEq)]
pub struct FontVariationSetting {
    pub tag: [u8; 4],
    pub value: f32,
}

/// `text-rendering`. Heredable. Default `Auto`. Fase 7.263.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextRendering {
    #[default]
    Auto,
    OptimizeSpeed,
    OptimizeLegibility,
    GeometricPrecision,
}

/// `mix-blend-mode` / `background-blend-mode`. Subset Compositing &
/// Blending 1. Default `Normal`. Plumb. Fase 7.254/7.255.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    PlusLighter,
}

/// `isolation`. NO heredable. `Isolate` fuerza un nuevo stacking context.
/// Fase 7.256.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Isolation {
    #[default]
    Auto,
    Isolate,
}

/// `will-change`: hint individual. `Auto` cuando la lista es vacía.
/// Subset: `scroll-position`, `contents`, o nombre arbitrario de
/// propiedad (almacenado como `Property(String)`). Fase 7.257.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WillChangeHint {
    ScrollPosition,
    Contents,
    /// Nombre de propiedad CSS (ej. `transform`, `opacity`). Se almacena
    /// tal cual lo escribió el autor, en lowercase.
    Property(String),
}

/// `appearance` (CSS UI 4). Default `Auto`. NO heredable. Fase 7.258.
/// El subset cubre los valores de compat más usados; cualquier otro
/// keyword cae a `Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    Auto,
    None,
    /// Hints de compat conservados.
    Textfield,
    MenulistButton,
    Button,
    Checkbox,
    Radio,
}

/// `image-rendering`: hint del sampler al pintar `<img>` y backgrounds.
/// Heredable. Default `Auto`. Fase 7.253. Plumb: el chrome aún no elige
/// `nearest` vs `linear` en función de este flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageRendering {
    #[default]
    Auto,
    /// CSS Images 3 `smooth` — bilinear/trilinear (lo que el GPU haga).
    Smooth,
    /// CSS Images 3 `crisp-edges` — sin antialiasing en escala (ideal pixel art).
    CrispEdges,
    /// CSS Images 4 `pixelated` — nearest-neighbour explícito.
    Pixelated,
}

/// CSS `border-style` reducido al subset que el chrome pinta: `solid`
/// (línea continua), `dashed`/`dotted` (patrón de stroke) y `double` (dos
/// líneas). `none`/`hidden` se modelan aparte (color del lado = `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderLineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    Double,
    /// 3D "carved" — top+left dark, bottom+right light.
    Groove,
    /// 3D opuesto a `Groove` — top+left light, bottom+right dark.
    Ridge,
    /// 3D "hundido" — render como `Groove` (suficiente aprox sin
    /// gradiente real por dentro del lado).
    Inset,
    /// 3D opuesto a `Inset` — render como `Ridge`.
    Outset,
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
/// queda para cuando el render-pipeline soporte multi-pass. `inset`
/// invierte el lado: en vez de pintar afuera, recorta una sombra
/// dentro del box (aproximada con un fill traslúcido del color sobre
/// el área interior).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_px: f32,
    pub spread_px: f32,
    pub color: Color,
    pub inset: bool,
}

/// Valor longitud de CSS reducido al subset que soportamos: `auto`,
/// `Npx`, `N%`. `em`/`rem` se resuelven a px en parse time.
///
/// Fase 7.849 — keywords de tamaño intrínseco (`min-content`/`max-content`/
/// `fit-content`). taffy 0.9 no las modela en `Dimension`, así que el puente
/// (`puriy-llimphi::box_style`) las aproxima a *shrink-to-fit* (la caja se
/// encoge a su contenido en lugar de llenar el contenedor). La distinción
/// fina entre min/max/fit (cuán agresivo es el wrap) NO se modela: las tres
/// caen al mismo ancho-según-contenido, que es la corrección visible respecto
/// del bug previo (un bloque `width: max-content` llenaba el ancho completo).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthVal {
    Auto,
    Px(f32),
    Pct(f32),
    MinContent,
    MaxContent,
    FitContent,
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

/// Distribución de las *líneas* en el eje cruzado (flex multilínea) o de
/// las pistas en grid. CSS `align-content`. `Normal` (default) deja que
/// taffy use su comportamiento por defecto (stretch para flex). No hereda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignContent {
    Normal,
    Start,
    Center,
    End,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
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
    /// Patrón visual del outline (reusa el enum de border). Default `Solid`.
    pub style: BorderLineStyle,
    /// Distancia del border al outline. Default 0.
    pub offset: f32,
}

impl Default for Outline {
    fn default() -> Self {
        Self {
            width: 0.0,
            color: None,
            style_active: true,
            style: BorderLineStyle::Solid,
            offset: 0.0,
        }
    }
}

/// Un stop de gradiente. `pos` es la posición a lo largo del eje:
/// `Pct(n)` = fracción del eje (`n` en 0..100), `Px(n)` = distancia absoluta
/// (px en lineal/radial, grados en cónico). Si `None`, se distribuye
/// automáticamente entre los stops fijos adyacentes (interpolación CSS).
/// Fase 7.228 (antes era `Option<f32>` ya normalizado a 0..1, lo que perdía
/// los px reales que los `repeating-*` necesitan).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    pub pos: Option<LengthVal>,
}

/// Tamaño de un `radial-gradient` — qué borde/esquina toca el círculo en su
/// stop final. Default `FarthestCorner`. Fase 7.226.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadialSize {
    ClosestSide,
    ClosestCorner,
    FarthestSide,
    FarthestCorner,
}

/// Geometría de un `radial-gradient`. El render lo trata como círculo (peniko
/// `Radial` es circular): forma `circle`/`ellipse` no se distingue todavía.
/// `cx`/`cy` = centro (`at <position>`, default 50% 50%). Fase 7.226.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialSpec {
    pub size: RadialSize,
    pub cx: LengthVal,
    pub cy: LengthVal,
}

impl Default for RadialSpec {
    fn default() -> Self {
        Self {
            size: RadialSize::FarthestCorner,
            cx: LengthVal::Pct(50.0),
            cy: LengthVal::Pct(50.0),
        }
    }
}

/// Geometría de un gradiente CSS. Fase 7.227 (antes eran campos sueltos
/// `angle_deg` + `radial: Option`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientGeometry {
    /// `linear-gradient` — ángulo CSS en grados (0 = up, 90 = right, 180 =
    /// down, 270 = left).
    Linear { angle_deg: f32 },
    /// `radial-gradient` — forma/tamaño/centro.
    Radial(RadialSpec),
    /// `conic-gradient` — ángulo inicial `from <angle>` (grados, 0 = up) y
    /// centro (`at <position>`, default 50% 50%).
    Conic { from_deg: f32, cx: LengthVal, cy: LengthVal },
}

/// `background-image: {linear,radial,conic}-gradient(...)`. La `geometry`
/// discrimina el tipo; los `stops` (2+) son comunes a los tres. El nombre
/// histórico `LinearGradient` se conserva (deuda) para no propagar el rename
/// a ~9 archivos.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    pub geometry: GradientGeometry,
    pub stops: Vec<GradientStop>,
    /// `repeating-{linear,radial,conic}-gradient`: el patrón de stops se
    /// tilea a lo largo del eje en vez de extender el color de los extremos
    /// (peniko `Extend::Repeat`). Fase 7.228.
    pub repeating: bool,
}

impl LinearGradient {
    /// Ángulo del gradiente lineal en grados (0 si no es lineal).
    pub fn angle_deg(&self) -> f32 {
        match self.geometry {
            GradientGeometry::Linear { angle_deg } => angle_deg,
            _ => 0.0,
        }
    }

    /// La geometría radial si el gradiente es `radial-gradient`.
    pub fn radial(&self) -> Option<RadialSpec> {
        match self.geometry {
            GradientGeometry::Radial(spec) => Some(spec),
            _ => None,
        }
    }
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

/// `object-fit` de un reemplazado (`<img>`): cómo encaja la imagen en la
/// caja cuando el tamaño de la caja (CSS `width`/`height`) difiere del
/// intrínseco. `Fill` estira a la caja (default CSS), `Contain`/`Cover`
/// preservan aspecto (cabe / cubre), `None` usa el tamaño natural,
/// `ScaleDown` = el menor entre `None` y `Contain`. Fase 7.230.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

/// `background-size`. `Auto` = tamaño natural de la imagen; `Cover`/`Contain`
/// escalan preservando aspecto (la más grande / la más chica que cubre / cabe);
/// `Explicit` da ancho/alto, donde cada eje puede ser `Auto` (= derivado del
/// otro por aspecto). El chrome resuelve % y aspecto contra el rect del box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundSize {
    Auto,
    Cover,
    Contain,
    Explicit { x: LengthVal, y: LengthVal },
}

/// `background-position`. `x`/`y` son el offset del origen del primer tile.
/// `Pct(p)` tiene semántica de alineación CSS (el punto `p%` de la imagen se
/// alinea con el `p%` del box) — la resuelve el chrome; `Px(n)` es un offset
/// directo desde la esquina superior-izquierda.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundPosition {
    pub x: LengthVal,
    pub y: LengthVal,
}

/// `background-repeat`. `space`/`round` se aproximan a `Repeat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundRepeat {
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

/// `background-origin`: el área de posicionamiento del background — contra qué
/// caja se anclan `background-position`, los `%` y `cover`/`contain`. Default
/// CSS `PaddingBox`. El chrome insetea el rect del border-box según el valor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundOrigin {
    BorderBox,
    PaddingBox,
    ContentBox,
}

/// `background-clip`: hasta qué caja se recorta el pintado del background.
/// Default CSS `BorderBox`. `Text` recorta el background a las glifos del
/// texto (Fase 7.208): el chrome lo propaga a las hojas de texto y rellena
/// los glifos con el gradiente en vez de pintar el fondo como rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundClip {
    BorderBox,
    PaddingBox,
    ContentBox,
    Text,
}

/// La imagen de una capa de background: o un gradiente, o una URL sin
/// resolver (el engine la descarga en `build_node`). Una capa siempre tiene
/// imagen — sin imagen no hay nada que pintar.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundImage {
    Url(String),
    Gradient(LinearGradient),
}

/// Una capa de background ADICIONAL (más allá de la capa 0, que vive en los
/// campos `background_*` sueltos de `ComputedStyle`). CSS pinta la PRIMERA
/// capa de la lista arriba; estas capas extra son las 2..N de una lista
/// `background: a, b, c` separada por coma y van por DEBAJO de la capa 0.
#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundLayer {
    pub image: BackgroundImage,
    pub size: BackgroundSize,
    pub position: BackgroundPosition,
    pub repeat: BackgroundRepeat,
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
    /// Traslación en PORCENTAJE del tamaño usado del propio elemento (X% del
    /// ancho, Y% del alto) — p. ej. `translate(-50%, -50%)`. Los valores son
    /// porcentajes (50.0 = 50%); se resuelven contra la caja en tiempo de
    /// composición (puriy-llimphi los pasa como `transform_rel` a Llimphi).
    TranslatePct(f32, f32),
    /// Factores X/Y (uno solo si CSS da un valor).
    Scale(f32, f32),
    /// Grados (sentido horario en pantalla = sentido CSS).
    Rotate(f32),
    /// Sesgo X/Y en grados (`skew`/`skewX`/`skewY`).
    Skew(f32, f32),
    /// `matrix(a, b, c, d, e, f)` — afín 2D completa. `a..d` son unitless;
    /// `e`/`f` son la traslación en px (se escalan por zoom en el render).
    Matrix(f32, f32, f32, f32, f32, f32),
}

/// Una "breadth" de track: el componente que va en cada lado de `minmax()` o
/// como track suelto (salvo las funciones). NO anida — la gramática CSS de
/// `minmax()` no admite `minmax`/`fit-content` adentro, así que se mantiene
/// `Copy` sin `Box`. `Fr` sólo es válido como `max` (en `min` degrada a auto).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrackBreadth {
    Auto,
    Px(f32),
    Pct(f32),
    Fr(f32),
    MinContent,
    MaxContent,
}

/// Tamaño de track para `display: grid`. `Fr(N)` = fracción del espacio
/// remanente (CSS unit `fr`). `Minmax`/`FitContent`/`Min·MaxContent` se
/// mapean a los track-sizing nativos de taffy en el puente (Fase 7.916).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrackSize {
    Auto,
    Px(f32),
    Pct(f32),
    Fr(f32),
    MinContent,
    MaxContent,
    /// `fit-content(<len-px>)` — clamp al length dado.
    FitContent(f32),
    /// `minmax(<min>, <max>)`.
    Minmax(GridTrackBreadth, GridTrackBreadth),
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

impl Default for AnimationBinding {
    fn default() -> Self {
        Self {
            name: String::new(),
            duration_s: 0.0,
            timing: EasingFunction::Ease,
            delay_s: 0.0,
            iterations: AnimationIterations::Count(1.0),
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
        }
    }
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

/// Una entrada de la lista `src:` de un `@font-face` (CSS Fonts 4). O bien una
/// fuente externa (`url(...)`), o una instalada en el sistema (`local(...)`).
/// `format`/`tech` son las pistas opcionales (`format("woff2")`,
/// `tech(color-COLRv1)`) que el cargador usa para elegir/descartar sin bajar.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontSrc {
    /// `url(...)` sin las comillas ni la función — la URL cruda a resolver.
    pub url: Option<String>,
    /// `local(...)` — nombre de una familia instalada en el sistema.
    pub local: Option<String>,
    /// `format(...)` — `woff2`/`truetype`/`opentype`/… (sin comillas).
    pub format: Option<String>,
    /// `tech(...)` — capacidad requerida (`color-COLRv1`, `variations`, …).
    pub tech: Option<String>,
}

/// Definición de un `@font-face { ... }` (CSS Fonts 4). Se recoge globalmente
/// (como `@keyframes`) y se expone para el cargador de fuentes — que cruzará
/// `family` con el `font-family` computado y, si matchea, bajará la primera
/// `src` compatible y la registrará en parley. Hoy sólo se parsea y se expone
/// vía [`super::super::StyleEngine::font_faces`]; el cargador es trabajo futuro.
/// Los descriptores tipográficos (weight/style/stretch/unicode-range/…) se
/// guardan crudos (`Option<String>`) — el matcher los interpretará al cargar.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontFaceRule {
    /// `font-family` — la clave bajo la que esta fuente se referencia. Requerida
    /// (un `@font-face` sin family se descarta).
    pub family: String,
    /// `src:` — lista de fuentes candidatas en orden de preferencia.
    pub sources: Vec<FontSrc>,
    /// `font-weight` — un peso (`700`) o un rango (`100 900`).
    pub weight: Option<String>,
    /// `font-style` — `normal`/`italic`/`oblique [<angle>]?`.
    pub style: Option<String>,
    /// `font-stretch` / `font-width` — `condensed`/`75%`/rango.
    pub stretch: Option<String>,
    /// `font-display` — `auto|block|swap|fallback|optional`.
    pub display: Option<String>,
    /// `unicode-range` — el subset cubierto (`U+0000-00FF, U+2000-206F`).
    pub unicode_range: Option<String>,
    /// `font-feature-settings`.
    pub feature_settings: Option<String>,
    /// `font-variation-settings`.
    pub variation_settings: Option<String>,
    /// `ascent-override` — métrica forzada (`90%`).
    pub ascent_override: Option<String>,
    /// `descent-override`.
    pub descent_override: Option<String>,
    /// `line-gap-override`.
    pub line_gap_override: Option<String>,
    /// `size-adjust` — escala del glyph (`110%`).
    pub size_adjust: Option<String>,
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
pub(crate) fn resolve_viewport() -> Viewport {
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

pub(crate) fn set_side<T: Copy>(sides: &mut Sides<T>, edge: BorderEdge, v: T) {
    match edge {
        BorderEdge::Top => sides.top = v,
        BorderEdge::Right => sides.right = v,
        BorderEdge::Bottom => sides.bottom = v,
        BorderEdge::Left => sides.left = v,
    }
}

pub(crate) fn set_side_f32(sides: &mut Sides<f32>, edge: BorderEdge, v: f32) {
    set_side(sides, edge, v)
}

pub(crate) fn set_corner(corners: &mut Corners<f32>, corner: BorderCorner, v: f32) {
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

