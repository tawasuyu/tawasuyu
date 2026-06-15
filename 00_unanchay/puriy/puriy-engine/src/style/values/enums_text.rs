//! Enums/structs de texto, fuente, scroll-snap, énfasis, transform-origin, baseline.
//! Tipos de valores CSS extraídos de `values.rs` (regla #1). Sin cambios de lógica.
use super::*;

/// Propiedad-destino de una declaración `currentColor`. Se resuelve al
/// `color` computado del elemento en una pasada final de la cascada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTarget {
    Background,
    BorderAll,
    BorderSide(BorderEdge),
    Outline,
}

/// Estilo del marker de `<li>`. Reducido al subset que el chrome puede
/// pintar como texto plano (sin imágenes ni cuadritos pintados a mano).
/// `Decimal`/`*Alpha`/`*Roman` requieren conocer la posición del `<li>`
/// entre sus hermanos — `boxes::build_node` la calcula y la sustituye.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// `list-style-type: "<string>"` (CSS Lists 3): el marcador es el string
    /// literal, verbatim (el autor controla el espaciado). Fase 7.1216.
    Str(String),
    /// `list-style-type: <custom-ident>` que referencia un `@counter-style`
    /// registrado (CSS Counter Styles 3). Se resuelve en render contra el
    /// registro; si no está definido, cae a `decimal`. Fase 7.1218.
    Named(String),
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

/// CSS `text-decoration-style`. El subset que el chrome sabe pintar:
/// `solid` (línea continua), `double` (dos líneas), `dotted`/`dashed`
/// (patrón de stroke) y `wavy` (aproximada como zig-zag). Default `Solid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecorationStyle {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

/// `cursor` CSS — subset reconocido. Otros valores (`url(...)` y
/// `<x> <y>` fallback, `none`, `progress`, `cell`, `vertical-text`,
/// `alias`, `copy`, `no-drop`, todas las flechas direccionales y los
/// resize compuestos) caen a `Auto` (= no cambia el cursor por
/// elemento). Heredable. Fase 7.240.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Auto,
    Default,
    Pointer,
    Text,
    Wait,
    Help,
    Crosshair,
    Move,
    NotAllowed,
    Grab,
    Grabbing,
    ZoomIn,
    ZoomOut,
    EResize,
    NResize,
    SResize,
    WResize,
    NsResize,
    EwResize,
    NeswResize,
    NwseResize,
    RowResize,
    ColResize,
}

/// `text-overflow` — qué hacer con el texto recortado por un padre
/// con `overflow: hidden` + `white-space: nowrap`. Sólo `Clip` y
/// `Ellipsis` por ahora (`fade` y string custom de CSS3 aparte).
/// Fase 7.241.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}

/// `clip` (CSS2.1 §11.1.2, deprecada pero viva en el patrón a11y
/// "visually-hidden"). Sólo aplica a elementos `position: absolute/fixed`.
/// `Auto` = sin recorte. `Rect` lleva los 4 offsets (`None` = `auto` por
/// lado), medidos desde el borde superior/izquierdo de la caja. NO hereda.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Clip {
    #[default]
    Auto,
    Rect {
        top: Option<f32>,
        right: Option<f32>,
        bottom: Option<f32>,
        left: Option<f32>,
    },
}

/// `scroll-behavior` — animación del scroll programático
/// (`element.scrollTo`, jump por `#anchor`...). Fase 7.242.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollBehavior {
    #[default]
    Auto,
    Smooth,
}

/// `tab-size`: ancho del U+0009 expresado en caracteres o longitud px.
/// CSS permite ambos formatos (`tab-size: 4` o `tab-size: 32px`).
/// Default = 8 caracteres. Fase 7.243.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabSize {
    Chars(u16),
    Px(f32),
}

/// `user-select`: controla si el usuario puede seleccionar texto del
/// elemento. `Auto` = default del UA (texto seleccionable en bloque
/// de texto; no en widgets nativos). Heredable. Fase 7.244.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UserSelect {
    #[default]
    Auto,
    None,
    Text,
    All,
    Contain,
}

/// `overflow-wrap` (alias legacy `word-wrap`): permite que el text shaper
/// quiebre dentro de una palabra cuando la línea no le alcanza. Default
/// `Normal` (sólo quiebra en oportunidades válidas del idioma).
/// Heredable. Fase 7.245.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowWrap {
    #[default]
    Normal,
    BreakWord,
    Anywhere,
}

/// `word-break`: política de quiebre de palabra. `BreakAll` (CJK) y
/// `KeepAll` (sólo en separadores reales). Heredable. Fase 7.246.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordBreak {
    #[default]
    Normal,
    BreakAll,
    KeepAll,
    /// CSS Text 4: quiebre por frase (japonés). El shaper aún no lo aplica;
    /// se modela el valor para fidelidad de cascada. Fase 7.917.
    AutoPhrase,
}

/// `hyphens`: control de hyphenation. `Auto` requeriría diccionarios por
/// idioma; quedó como acepto-pero-no-aplico. Heredable. Fase 7.247.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hyphens {
    #[default]
    Manual,
    None,
    Auto,
}

/// `resize`: el usuario puede arrastrar el borde del elemento para
/// redimensionarlo (típicamente `<textarea>`). Default `None`.
/// NO heredable. Fase 7.248.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Resize {
    #[default]
    None,
    Both,
    Horizontal,
    Vertical,
    Block,
    Inline,
}

/// `writing-mode`: orientación de bloque. Default `HorizontalTb`
/// (occidental). Heredable. Fase 7.249. Plumb: el resto de los modos
/// quedan parseados pero el shaper no rota glifos todavía.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMode {
    #[default]
    HorizontalTb,
    VerticalRl,
    VerticalLr,
    SidewaysRl,
    SidewaysLr,
}

/// `direction`: dirección base del texto del elemento. Heredable.
/// Default `Ltr`. Fase 7.250. Plumb: sin BiDi runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}

/// `unicode-bidi`: trato del elemento por el algoritmo BiDi. NO heredable.
/// Default `Normal`. Fase 7.251. Plumb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnicodeBidi {
    #[default]
    Normal,
    Embed,
    Isolate,
    BidiOverride,
    IsolateOverride,
    Plaintext,
}

/// CSS `filter` / `backdrop-filter` function-list item. CSS Filter
/// Effects 1, subset. Cada variante guarda el argumento ya parseado.
/// `none` = lista vacía (no se modela acá). Fase 7.264.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterFn {
    /// `blur(<length>)` en px.
    Blur(f32),
    /// `brightness(<number>|<percentage>)`. 1.0 = sin cambio.
    Brightness(f32),
    /// `contrast(<number>|<percentage>)`. 1.0 = sin cambio.
    Contrast(f32),
    /// `grayscale(<number>|<percentage>)`. 0 = sin cambio, 1 = full.
    Grayscale(f32),
    /// `hue-rotate(<angle>)` en grados.
    HueRotate(f32),
    /// `invert(<number>|<percentage>)`.
    Invert(f32),
    /// `opacity(<number>|<percentage>)`. 1 = sin cambio.
    Opacity(f32),
    /// `saturate(<number>|<percentage>)`. 1 = sin cambio.
    Saturate(f32),
    /// `sepia(<number>|<percentage>)`.
    Sepia(f32),
    /// `drop-shadow(offset-x offset-y [blur] [color])`. Reusa el
    /// `BoxShadow` (inset=false).
    DropShadow(BoxShadow),
}

/// `text-orientation` (CSS Writing Modes 3). Heredable. Default `Mixed`.
/// Fase 7.266.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextOrientation {
    #[default]
    Mixed,
    Upright,
    Sideways,
    /// Legacy `sideways-right` (deprecado, alias de `Sideways`).
    SidewaysRight,
}

/// `overscroll-behavior-x` / `-y`. Default `Auto`. NO heredable.
/// Fase 7.267.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverscrollBehavior {
    #[default]
    Auto,
    Contain,
    None,
}

/// `scroll-snap-type`. Default `None`. NO heredable. Fase 7.268.
/// El axis + strictness se modela como struct (None = sin snap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollSnapAxis {
    X,
    Y,
    Block,
    Inline,
    #[default]
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollSnapStrictness {
    #[default]
    Proximity,
    Mandatory,
}

/// `scroll-snap-type`. `None` = sin snap. Some((axis, strictness)) si
/// se declaró. Fase 7.268.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScrollSnapType(pub Option<(ScrollSnapAxis, ScrollSnapStrictness)>);

/// `scroll-snap-align` (CSS Scroll Snap 1). Default `None` (no snap).
/// El shorthand acepta 1 o 2 valores; con 1 se aplica a block + inline.
/// Fase 7.269.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollSnapAlign {
    #[default]
    None,
    Start,
    End,
    Center,
}

/// `scroll-snap-stop` (CSS Scroll Snap 1). Default `Normal` (el snap
/// puede saltearse). `Always` fuerza parar en cada snap point. Fase 7.270.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollSnapStop {
    #[default]
    Normal,
    Always,
}

/// `touch-action` (CSS Pointer Events 2). Default `Auto`. `Pan { ... }`
/// modela la combinación `pan-x` / `pan-y` / `pinch-zoom`; al menos uno
/// debe estar en `true` (validado por el parser). Fase 7.273.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TouchAction {
    #[default]
    Auto,
    None,
    Manipulation,
    Pan {
        pan_x: bool,
        pan_y: bool,
        pinch_zoom: bool,
    },
}

/// Radio de `circle()`/`ellipse()` de clip-path: una `<length-percentage>`
/// o un keyword de lado. `closest-side`/`farthest-side` resuelven contra la
/// distancia del centro a los bordes de la caja (en el compositor, que tiene
/// el rect). Default de un radio ausente = `ClosestSide`. Fase 7.1222.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipRadius {
    Len(LengthVal),
    ClosestSide,
    FarthestSide,
}

/// `clip-path` (CSS Masking 1). Subset: `inset()`, `circle()`, `ellipse()`.
/// `polygon()` y `path()` quedan fuera por ahora — la mayoría del wild
/// usa formas básicas. `None` (afuera del enum) = `clip-path: none`.
/// Fase 7.274.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipPath {
    /// `inset(<top> [<right> [<bottom> [<left>]]] [round <r>])`. Los
    /// offsets en px desde cada borde; `radius` (opcional) curva las
    /// esquinas. La spec acepta `<length-percentage>`; acá guardamos px.
    Inset { top: f32, right: f32, bottom: f32, left: f32, radius: f32 },
    /// `circle(<radius> [at <x> <y>])`. `radius` es `<length-percentage>` (un
    /// `%` resuelve contra `√(w²+h²)/√2`, la diagonal de la caja) o un keyword
    /// `closest-side`/`farthest-side`. Centro default `50% 50%`. Fase 7.1222:
    /// `radius` pasó de `LengthVal` a `ClipRadius` (admite keywords).
    Circle { radius: ClipRadius, cx: LengthVal, cy: LengthVal },
    /// `ellipse(<rx> <ry> [at <x> <y>])`. `rx`/`ry` son `<length-percentage>`
    /// (`rx%`→ancho, `ry%`→alto) o keywords de lado (sobre el eje respectivo).
    Ellipse { rx: ClipRadius, ry: ClipRadius, cx: LengthVal, cy: LengthVal },
}

/// `mask-image` (CSS Masking 1). Subset: `url(...)`. `image()`,
/// `linear-gradient()` y demás quedan fuera. `None` = `mask-image: none`.
/// Fase 7.275.
#[derive(Debug, Clone, PartialEq)]
pub enum MaskImage {
    Url(String),
}

/// `content-visibility` (CSS Containment 2). Default `Visible`. `Auto`
/// permite al UA skipear el render fuera de viewport (no implementado);
/// `Hidden` lo skipea siempre. NO heredable. Fase 7.276.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContentVisibility {
    #[default]
    Visible,
    Auto,
    Hidden,
}

/// `contain` (CSS Containment 2). Bitset — el shorthand `strict` y
/// `content` se expanden a combinaciones de bits. `none` = todos los
/// bits a 0. Fase 7.277.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContainFlags {
    pub size: bool,
    pub inline_size: bool,
    pub layout: bool,
    pub style: bool,
    pub paint: bool,
}

/// `column-fill` (CSS Multi-column 1). Default `Balance`. Fase 7.281.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnFill {
    Auto,
    #[default]
    Balance,
    BalanceAll,
}

/// `column-span` (CSS Multi-column 1). Default `None`. `All` saca el
/// elemento del flujo multicol. Fase 7.282.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnSpan {
    #[default]
    None,
    All,
}

/// `break-inside` (CSS Fragmentation 3). Default `Auto`. Las variantes
/// `AvoidPage`/`AvoidColumn`/`AvoidRegion` son hints más finos que `Avoid`.
/// Fase 7.283.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakInside {
    #[default]
    Auto,
    Avoid,
    AvoidPage,
    AvoidColumn,
    AvoidRegion,
}

/// `table-layout` (CSS Tables 3). Default `Auto`. Fase 7.284.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableLayout {
    #[default]
    Auto,
    Fixed,
}

/// `border-collapse` (CSS Tables 3). Default `Separate`. Heredable.
/// Fase 7.285.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderCollapse {
    #[default]
    Separate,
    Collapse,
}

/// `caption-side` (CSS Tables 3). Default `Top`. Heredable.
/// Las variantes `inline-start`/`inline-end` (logical) caen a Top/Bottom
/// según `direction`; por simplicidad las aceptamos pero las aplastamos
/// a Top/Bottom. Fase 7.287.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptionSide {
    #[default]
    Top,
    Bottom,
}

/// `empty-cells` (CSS Tables 3). Default `Show`. Heredable. Fase 7.288.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmptyCells {
    #[default]
    Show,
    Hide,
}

/// `break-before` / `break-after` (CSS Fragmentation 3). Default `Auto`.
/// Comparten dominio de valores. Fase 7.289 / 7.290.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakBetween {
    #[default]
    Auto,
    Avoid,
    /// Forzar break (genérico).
    Always,
    /// Variantes específicas por tipo de break.
    AvoidPage,
    Page,
    Left,
    Right,
    Recto,
    Verso,
    AvoidColumn,
    Column,
    AvoidRegion,
    Region,
}

/// `color-scheme` (CSS Color Adjustment 1). Default `Normal` (sin
/// compromiso). El valor `Only(...)` marca el `only` opt-in (un browser
/// fuera del set no puede caer a otro). Heredable. Fase 7.293.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorScheme {
    pub light: bool,
    pub dark: bool,
    /// `only` marca que el UA no debe caer al esquema opuesto.
    pub only: bool,
}

impl ColorScheme {
    /// `normal` = light=false, dark=false, only=false.
    pub const NORMAL: Self = Self { light: false, dark: false, only: false };
}

/// `list-style-position` (CSS Lists 3). Default `Outside`. Fase 7.294.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListStylePosition {
    #[default]
    Outside,
    Inside,
}

/// `quotes` (CSS Generated Content 3). Default `Auto` — la UA elige.
/// `None` (afuera del enum, `Quotes::None`) deja los `open-quote`/
/// `close-quote` mudos. `Pairs(vec)` fija pares concretos por nivel
/// de anidamiento. Fase 7.298.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Quotes {
    #[default]
    Auto,
    None,
    /// Lista `(open, close)` por nivel — el último par se recicla.
    Pairs(Vec<(String, String)>),
}

/// `text-underline-position` (CSS Text Decoration 4). Default `Auto`.
/// Heredable. `Left`/`Right` aplican sólo a writing-modes verticales.
/// Fase 7.299.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextUnderlinePosition {
    #[default]
    Auto,
    FromFont,
    Under,
    Left,
    Right,
}

/// `text-justify` (CSS Text 3). Default `Auto`. Heredable. Fase 7.300.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextJustify {
    #[default]
    Auto,
    None,
    InterWord,
    InterCharacter,
    /// Alias legacy de `InterCharacter`.
    Distribute,
}

/// `print-color-adjust` (CSS Color Adjustment 1). Alias `color-adjust`.
/// Default `Economy`. Heredable. Fase 7.301.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrintColorAdjust {
    #[default]
    Economy,
    Exact,
}

/// `forced-color-adjust` (CSS Color Adjustment 1). Default `Auto`.
/// Heredable. Fase 7.302.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForcedColorAdjust {
    #[default]
    Auto,
    None,
    /// Hint moderno (subset opt-in).
    Preserve,
}

/// `font-variant-caps` (CSS Fonts 4). Default `Normal`. Heredable.
/// Fase 7.304.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontVariantCaps {
    #[default]
    Normal,
    SmallCaps,
    AllSmallCaps,
    PetiteCaps,
    AllPetiteCaps,
    Unicase,
    TitlingCaps,
}

/// `font-variant-numeric` (CSS Fonts 4). Bitset libre — los valores
/// `normal` (todos false) y los individuales se acumulan. Heredable.
/// Fase 7.305.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FontVariantNumeric {
    pub lining_nums: bool,
    pub oldstyle_nums: bool,
    pub proportional_nums: bool,
    pub tabular_nums: bool,
    pub diagonal_fractions: bool,
    pub stacked_fractions: bool,
    pub ordinal: bool,
    pub slashed_zero: bool,
}

/// `font-variant-ligatures` (CSS Fonts 4). `None` (variante) = todas las
/// ligaduras off; `Normal` (todos false, no_* false) = defaults de la
/// font. Fase 7.306.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontVariantLigatures {
    #[default]
    Normal,
    None,
    /// Combinación de habilitaciones/deshabilitaciones explícitas.
    Custom(LigatureSet),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LigatureSet {
    pub common_ligatures: bool,
    pub no_common_ligatures: bool,
    pub discretionary_ligatures: bool,
    pub no_discretionary_ligatures: bool,
    pub historical_ligatures: bool,
    pub no_historical_ligatures: bool,
    pub contextual: bool,
    pub no_contextual: bool,
}

/// `font-variant-east-asian` (CSS Fonts 4). Bitset libre — `normal` =
/// todos false. Las variantes JIS78/JIS83/.../Simplified/Traditional son
/// mutuamente excluyentes, igual que `full-width`/`proportional-width`;
/// el parser rechaza combinaciones inválidas. Heredable. Fase 7.307.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FontVariantEastAsian {
    pub jis78: bool,
    pub jis83: bool,
    pub jis90: bool,
    pub jis04: bool,
    pub simplified: bool,
    pub traditional: bool,
    pub full_width: bool,
    pub proportional_width: bool,
    pub ruby: bool,
}

/// `font-variant-position` (CSS Fonts 4). Default `Normal`. Heredable.
/// Fase 7.308.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontVariantPosition {
    #[default]
    Normal,
    Sub,
    Super,
}

/// `text-emphasis-style` (CSS Text Decoration 4). Default `None` (sin
/// marca). `Mark` modela `[filled|open] && [dot|circle|...]`. `Custom`
/// guarda el string literal (sólo 1 grapheme válido según spec, pero
/// no validamos). Fase 7.309.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TextEmphasisStyle {
    #[default]
    None,
    Mark {
        fill: TextEmphasisFill,
        shape: TextEmphasisShape,
    },
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEmphasisFill {
    #[default]
    Filled,
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEmphasisShape {
    #[default]
    Dot,
    Circle,
    DoubleCircle,
    Triangle,
    Sesame,
}

/// `text-emphasis-position` (CSS Text Decoration 4). Default `Over Right`.
/// Combina eje (over/under) + lado (right/left). Fase 7.311.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextEmphasisPosition {
    pub over: bool,
    /// `right` (true) o `left` (false).
    pub right: bool,
}

impl Default for TextEmphasisPosition {
    fn default() -> Self {
        Self { over: true, right: true }
    }
}

/// `ruby-position` (CSS Ruby 1). Default `Alternate` (over normalmente,
/// under cuando hay dos anotaciones). Heredable. Fase 7.313.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RubyPosition {
    Over,
    Under,
    InterCharacter,
    #[default]
    Alternate,
}

/// `transform-origin` (CSS Transforms 1). Punto pivote para `transform`.
/// `x`/`y` en `LengthVal` (`Px(n)` u `Pct(p)`) — el chrome resolvería el
/// % contra el border-box del elemento. `z` en px (`Pct` no se permite
/// en el eje Z). Default CSS: `50% 50% 0`. Fase 7.314.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformOrigin {
    pub x: LengthVal,
    pub y: LengthVal,
    pub z: f32,
}

impl Default for TransformOrigin {
    fn default() -> Self {
        Self { x: LengthVal::Pct(50.0), y: LengthVal::Pct(50.0), z: 0.0 }
    }
}

/// `transform-style` (CSS Transforms 2). Define si los hijos viven en su
/// propio plano (Flat) o componen en 3D con sus padres (Preserve3d).
/// Default `Flat`. Fase 7.315.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransformStyle {
    #[default]
    Flat,
    Preserve3d,
}

/// `perspective-origin` (CSS Transforms 2). Punto desde el que se mira
/// a los hijos cuando hay `perspective: <length>`. Default `50% 50%`.
/// Fase 7.317.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerspectiveOrigin {
    pub x: LengthVal,
    pub y: LengthVal,
}

impl Default for PerspectiveOrigin {
    fn default() -> Self {
        Self { x: LengthVal::Pct(50.0), y: LengthVal::Pct(50.0) }
    }
}

/// `backface-visibility` (CSS Transforms 2). `Hidden` esconde el elemento
/// cuando una rotación 3D lo voltea (cara mirando para atrás). Default
/// `Visible`. Fase 7.318.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackfaceVisibility {
    #[default]
    Visible,
    Hidden,
}

/// `scrollbar-width` (CSS Scrollbars 1). Heredable. Default `Auto`.
/// `None` = sin barra. Fase 7.319.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollbarWidth {
    #[default]
    Auto,
    Thin,
    None,
}

/// `scrollbar-color: <thumb> <track>` (CSS Scrollbars 1). El valor `auto`
/// se modela como `Option::None` del field padre — esta struct sólo
/// existe cuando ambos colores fueron declarados. Fase 7.320.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbarColorPair {
    pub thumb: Color,
    pub track: Color,
}

/// `scrollbar-gutter` (CSS Overflow 3). NO hereda. Default `Auto`.
/// `Stable` reserva el espacio aunque la barra no esté montada;
/// `stable_both_edges` además duplica el gutter en el lado opuesto.
/// Fase 7.321.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScrollbarGutter {
    pub stable: bool,
    pub both_edges: bool,
}

impl ScrollbarGutter {
    pub const AUTO: Self = Self { stable: false, both_edges: false };
    pub const STABLE: Self = Self { stable: true, both_edges: false };
    pub const STABLE_BOTH: Self = Self { stable: true, both_edges: true };
}

/// `overflow-anchor` (CSS Scroll Anchoring 1). NO hereda. Default
/// `Auto` (el browser decide). `None` desactiva el reanclaje en este
/// subárbol. Fase 7.322.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowAnchor {
    #[default]
    Auto,
    None,
}

/// `overflow-clip-margin` (CSS Overflow 4). Extiende el clip de
/// `overflow: clip` afuera del padding-box. `length` siempre en px no
/// negativo; `visual_box` indica la caja desde la que se mide
/// (default `PaddingBox` en CSS). Fase 7.323.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverflowClipMargin {
    pub visual_box: VisualBox,
    pub length: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisualBox {
    ContentBox,
    #[default]
    PaddingBox,
    BorderBox,
}

/// `text-align-last` (CSS Text 4). Alineación de la última línea de un
/// bloque (la única para `text-align: justify`). Heredable. Default
/// `Auto`. Fase 7.324.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlignLast {
    #[default]
    Auto,
    Start,
    End,
    Left,
    Right,
    Center,
    Justify,
}

/// `text-wrap` (CSS Text 4). Heredable. Default `Wrap`. Fase 7.325.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextWrap {
    #[default]
    Wrap,
    Nowrap,
    Balance,
    Pretty,
    Stable,
}

/// `line-break` (CSS Text 3). Estrictez del breaker para CJK y
/// puntuación pegada. Heredable. Default `Auto`. Fase 7.326.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineBreak {
    #[default]
    Auto,
    Loose,
    Normal,
    Strict,
    Anywhere,
}

/// `hanging-punctuation` (CSS Text 4). Combinación de flags. La spec
/// permite `none | [first || force-end | allow-end || last]`. `force_end`
/// y `allow_end` son mutuamente excluyentes (modelado: bool `force_end`
/// + bool `allow_end`; sólo uno puede ser true a la vez). Heredable.
/// Fase 7.327.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HangingPunctuation {
    pub first: bool,
    pub force_end: bool,
    pub allow_end: bool,
    pub last: bool,
}

impl HangingPunctuation {
    pub const fn is_none(self) -> bool {
        !self.first && !self.force_end && !self.allow_end && !self.last
    }
}

/// `text-decoration-skip-ink` (CSS Text Decoration 4). Heredable.
/// Default `Auto`. Fase 7.328.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecorationSkipInk {
    #[default]
    Auto,
    None,
    All,
}

/// `font-optical-sizing` (CSS Fonts 4). `Auto` deja que el shaper
/// setee el axis `opsz` según el tamaño; `None` lo fija al default.
/// Heredable. Fase 7.329.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontOpticalSizing {
    #[default]
    Auto,
    None,
}

/// `font-synthesis-*` (CSS Fonts 4). Cada eje permite la síntesis
/// (true, default) o la desactiva (false). El shorthand `font-synthesis`
/// (Fase 7.333) setea los tres a la vez. Heredable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontSynthesis {
    pub weight: bool,
    pub style: bool,
    pub small_caps: bool,
    /// `font-synthesis-position` (Fase 7.470). CSS Fonts 4 extiende el
    /// shorthand a un 4º axis que controla la síntesis de `font-variant-
    /// position` (sub/super). Default `true`.
    pub position: bool,
}

impl Default for FontSynthesis {
    fn default() -> Self {
        Self { weight: true, style: true, small_caps: true, position: true }
    }
}

impl FontSynthesis {
    /// `font-synthesis: none` apaga los cuatro.
    pub const NONE: Self = Self {
        weight: false,
        style: false,
        small_caps: false,
        position: false,
    };
}

/// `font-size-adjust` (CSS Fonts 5). `None` = sin ajuste; `Value(metric,
/// number)` = ajustar la métrica del fallback al `number * font-size`
/// del referente; `FromFont(metric)` = usar el valor que provee la
/// fuente para esa métrica. Heredable. Fase 7.334.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontSizeAdjust {
    None,
    Value(FontMetric, f32),
    FromFont(FontMetric),
}

impl Default for FontSizeAdjust {
    fn default() -> Self {
        FontSizeAdjust::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontMetric {
    /// Métrica default cuando no se da una explícitamente.
    #[default]
    ExHeight,
    CapHeight,
    ChWidth,
    IcWidth,
    IcHeight,
}

/// `image-orientation` (CSS Images 3). `FromImage` rota según EXIF;
/// `None` ignora EXIF; `Angle(deg, flip)` aplica un ángulo + flip
/// horizontal opcional. Heredable. Fase 7.335.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageOrientation {
    FromImage,
    None,
    /// `<angle> [flip]?` — el flip se aplica antes de la rotación.
    Angle { degrees: f32, flip: bool },
}

impl Default for ImageOrientation {
    fn default() -> Self {
        ImageOrientation::FromImage
    }
}

/// `animation-timeline` (CSS Animations 2 / Scroll-driven Animations 1).
/// `Auto` usa el monotonic timer del documento (el default
/// implícito); `None` desactiva la animación; `Named(s)` la enlaza a
/// un scroll/view-timeline declarado en otro lado. `Scroll`/`View` son
/// las notaciones funcionales anónimas (`scroll()` / `view()`). NO hereda.
/// Fases 7.339, scroll/view nuevos.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TimelineRef {
    #[default]
    Auto,
    None,
    Named(String),
    /// `scroll([<scroller>] [<axis>])`: timeline anónimo anclado al scroll
    /// de un contenedor (`nearest`/`root`/`self`, default `nearest`) sobre
    /// un eje (default `Block`).
    Scroll { scroller: ScrollScroller, axis: TimelineAxis },
    /// `view([<axis>] [<inset>])`: timeline anónimo de visibilidad. `inset`
    /// se guarda opaco (`Option<String>`).
    View { axis: TimelineAxis, inset: Option<String> },
}

/// Contenedor de scroll para `scroll()` de `animation-timeline`.
/// Default `Nearest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollScroller {
    #[default]
    Nearest,
    Root,
    SelfElement,
}

/// `scroll-timeline-axis` / `view-timeline-axis` (CSS Scroll-driven
/// Animations 1). Default `Block` (el eje block del writing-mode).
/// `X`/`Y` son aliases físicos. Fases 7.341, 7.343.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimelineAxis {
    #[default]
    Block,
    Inline,
    X,
    Y,
}

/// `white-space-collapse` (CSS Text 4). Heredable. Default `Collapse`.
/// Fase 7.344.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteSpaceCollapse {
    #[default]
    Collapse,
    Preserve,
    PreserveBreaks,
    BreakSpaces,
}

/// `text-wrap-mode` (CSS Text 4). Heredable. Default `Wrap`.
/// Fase 7.345.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextWrapMode {
    #[default]
    Wrap,
    Nowrap,
}

/// `text-wrap-style` (CSS Text 4). Heredable. Default `Auto`.
/// Fase 7.346.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextWrapStyle {
    #[default]
    Auto,
    Balance,
    Pretty,
    Stable,
}

/// `text-spacing-trim` (CSS Text 4). Heredable. Default `Normal`.
/// Fase 7.347.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextSpacingTrim {
    #[default]
    Normal,
    SpaceAll,
    SpaceFirst,
    TrimStart,
}

/// `text-box-trim` (CSS Inline Layout 3). Heredable. Default `None`.
/// Fase 7.348.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextBoxTrim {
    #[default]
    None,
    TrimStart,
    TrimEnd,
    TrimBoth,
}

/// `math-style` (CSS MathML 3 Core). Heredable. Default `Normal`.
/// Fase 7.349.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathStyle {
    #[default]
    Normal,
    Compact,
}

/// `math-depth` (CSS MathML 3 Core). `Auto` = el browser ajusta; `Add(n)`
/// suma `n` al heredado; `Value(n)` lo fija absoluto. Heredable.
/// Default `Auto`. Fase 7.350. NOTA: el `add` con signo se modela con
/// `i32` (negativo permitido).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathDepth {
    Auto,
    Add(i32),
    Value(i32),
}

impl Default for MathDepth {
    fn default() -> Self {
        MathDepth::Auto
    }
}

/// `math-shift` (CSS MathML 3 Core). Heredable. Default `Normal`.
/// Fase 7.351.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathShift {
    #[default]
    Normal,
    Compact,
}

/// `field-sizing` (CSS Basic UI 4). NO hereda. Default `Fixed`.
/// `Content` permite que `<input>`/`<textarea>` se autoencojan al
/// contenido (caso de uso: `textarea` sin scroll horizontal). Fase 7.352.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldSizing {
    #[default]
    Fixed,
    Content,
}

/// `overlay` (CSS Position 4). Propiedad controlada por el UA (sólo
/// animable); indica si el elemento está en la top-layer. NO hereda.
/// Default `None`. Fase 7.905 — plumb opaco (no afecta layout en puriy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overlay {
    #[default]
    None,
    Auto,
}

/// `dynamic-range-limit` (CSS Color HDR 1): techo de luminancia HDR del
/// contenido. HEREDA. Default `NoLimit`. Fase 7.905 — plumb opaco (puriy no
/// hace tone-mapping HDR).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynamicRangeLimit {
    Standard,
    #[default]
    NoLimit,
    High,
    Constrained,
    ConstrainedHigh,
}

/// `text-box-edge` (CSS Inline Layout 3). `Auto` deja al browser elegir
/// según script/fuente. Caso con 1 o 2 keywords (`<text-edge> [<text-edge>]?`).
/// `Edge { over, under }` cubre el caso de 1 keyword (over==under) o 2
/// keywords explícitos. Heredable. Default `Auto`. Fase 7.353.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextBoxEdge {
    Auto,
    Edge { over: TextEdge, under: TextEdge },
}

impl Default for TextBoxEdge {
    fn default() -> Self {
        TextBoxEdge::Auto
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEdge {
    #[default]
    Text,
    Cap,
    Ex,
    Ideographic,
    IdeographicInk,
    /// Sólo válido en el lado bajo (`under`); sintetizado por el parser.
    Alphabetic,
}

/// `anchor-scope` (CSS Anchor Positioning 1). `None` = sin scope
/// explícito; `All` = todos los anchors del subárbol; `Names` = lista.
/// Heredable. Fase 7.356.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorScope {
    None,
    All,
    Names(Vec<String>),
}

impl Default for AnchorScope {
    fn default() -> Self {
        AnchorScope::None
    }
}

/// `font-palette` (CSS Fonts 4). `Light` y `Dark` son keywords;
/// `Named(s)` hace match contra `@font-palette-values --x`. Heredable.
/// Default `Normal`. Fase 7.359.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FontPalette {
    #[default]
    Normal,
    Light,
    Dark,
    Named(String),
}

/// `font-variant-alternates` (CSS Fonts 4). Default `Normal`. Soporte
/// MVP: solo el flag `historical_forms` + opcionalmente nombres de
/// alternates funcionales (`stylistic(--x)`, `styleset(--y)`, etc).
/// Heredable. Fase 7.360.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FontVariantAlternates {
    pub historical_forms: bool,
    /// Tuplas `(<function-name>, <ident>)` ej: `("stylistic", "--swash")`.
    pub functional: Vec<(String, String)>,
}

impl FontVariantAlternates {
    pub fn is_normal(&self) -> bool {
        !self.historical_forms && self.functional.is_empty()
    }
}

