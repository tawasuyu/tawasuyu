//! Enums/structs SVG/paint, máscaras, container, block-step, footnotes, ruby, offset.
//! Tipos de valores CSS extraídos de `values.rs` (regla #1). Sin cambios de lógica.
use super::*;

/// `background-attachment` (CSS Backgrounds 3). Vec paralelo a las
/// capas de background. NO hereda. Fase 7.362.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundAttachment {
    #[default]
    Scroll,
    Fixed,
    Local,
}

/// `caret-shape` (CSS UI 4). Heredable. Default `Auto`. Fase 7.363.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaretShape {
    #[default]
    Auto,
    Bar,
    Block,
    Underscore,
}

/// `baseline-source` (CSS Inline Layout 3). NO hereda. Default `Auto`.
/// Fase 7.364.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BaselineSource {
    #[default]
    Auto,
    First,
    Last,
}

/// `alignment-baseline` (SVG 2). NO hereda. Default `Baseline`.
/// Fase 7.365.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignmentBaseline {
    #[default]
    Baseline,
    TextBottom,
    Alphabetic,
    Ideographic,
    Middle,
    Central,
    Mathematical,
    TextTop,
    Bottom,
    Center,
    Top,
}

/// `dominant-baseline` (SVG 2). Heredable. Default `Auto`. Fase 7.366.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DominantBaseline {
    #[default]
    Auto,
    TextBottom,
    Alphabetic,
    Ideographic,
    Middle,
    Central,
    Mathematical,
    Hanging,
    TextTop,
}

/// `paint-order` (SVG 2). Heredable. Default `Normal` (= `fill stroke
/// markers`). Cuando se especifican `<paint-fragment>+` los faltantes
/// se completan en orden canónico. Fase 7.367.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaintOrder {
    pub one: PaintFragment,
    pub two: PaintFragment,
    pub three: PaintFragment,
}

impl Default for PaintOrder {
    fn default() -> Self {
        Self {
            one: PaintFragment::Fill,
            two: PaintFragment::Stroke,
            three: PaintFragment::Markers,
        }
    }
}

impl PaintOrder {
    pub fn is_normal(self) -> bool {
        self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintFragment {
    Fill,
    Stroke,
    Markers,
}

/// `marker-side` (CSS Lists 3). Heredable. Default `MatchSelf`.
/// Fase 7.368.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarkerSide {
    #[default]
    MatchSelf,
    MatchParent,
}

/// `<paint>` (SVG 2). Default depende de la propiedad: `fill` arranca
/// en `Color(Color::BLACK)`; `stroke` arranca en `None`. Heredable.
/// Fases 7.369–7.370. `None` = sin pintura; `CurrentColor` = se
/// resuelve contra `color` del elemento; `Color(c)` literal; `Url(s)`
/// a un paint server (gradient/pattern/marker).
#[derive(Debug, Clone, PartialEq)]
pub enum SvgPaint {
    None,
    CurrentColor,
    Color(Color),
    Url(String),
}

impl Default for SvgPaint {
    fn default() -> Self {
        SvgPaint::None
    }
}

/// `stroke-linecap` (SVG 2). Heredable. Default `Butt`. Fase 7.374.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrokeLinecap {
    #[default]
    Butt,
    Round,
    Square,
}

/// `stroke-linejoin` (SVG 2). Heredable. Default `Miter`. Fase 7.375.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrokeLinejoin {
    #[default]
    Miter,
    Round,
    Bevel,
    Arcs,
    MiterClip,
}

/// `fill-rule` / `clip-rule` (SVG 2). Heredable. Default `Nonzero`.
/// Fases 7.379–7.380.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    Nonzero,
    Evenodd,
}

/// `color-interpolation` (SVG 2). Heredable. Default `SRgb`.
/// Fase 7.381.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorInterpolation {
    Auto,
    #[default]
    SRgb,
    LinearRgb,
}

/// `shape-rendering` (SVG 2). Heredable. Default `Auto`. Fase 7.382.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShapeRendering {
    #[default]
    Auto,
    OptimizeSpeed,
    CrispEdges,
    GeometricPrecision,
}

/// `vector-effect` (SVG 2). NO hereda. Default `None`. Fase 7.383.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VectorEffect {
    #[default]
    None,
    NonScalingStroke,
    NonScalingSize,
    NonRotation,
    FixedPosition,
}

/// `text-anchor` (SVG 2). Heredable. Default `Start`. Fase 7.389.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAnchor {
    #[default]
    Start,
    Middle,
    End,
}

/// `color-rendering` (SVG 2). Heredable. Default `Auto`. Fase 7.390.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorRendering {
    #[default]
    Auto,
    OptimizeSpeed,
    OptimizeQuality,
}

/// `color-interpolation-filters` (SVG 2). Heredable. Default
/// `LinearRgb` (la spec difiere de `color-interpolation`, que default
/// a `sRGB`). Fase 7.391.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorInterpolationFilters {
    Auto,
    SRgb,
    #[default]
    LinearRgb,
}

/// `glyph-orientation-vertical` (SVG 1.1 deprecated, parseado por
/// compatibilidad). Heredable. Default `Auto`. Sólo se aceptan los
/// 4 ángulos rectos. Fase 7.392.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlyphOrientationVertical {
    #[default]
    Auto,
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

/// `transform-box` (CSS Transforms 2). NO hereda. Default `ViewBox`
/// para coincidir con el reset SVG (el resto del web la trata como
/// `border-box` por compat — todavía no diferenciamos). Fase 7.393.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransformBox {
    ContentBox,
    BorderBox,
    FillBox,
    StrokeBox,
    #[default]
    ViewBox,
}

/// Referencia a un `<marker>` SVG: `None` = `marker-*: none`;
/// `Some(s)` = IRI tal como vino (`url(#mid)`). Heredable. Fases
/// 7.394–7.397.
pub type MarkerRef = Option<String>;

/// `mask-type` (CSS Masking 1). Default `Luminance` (spec). NO hereda.
/// Fase 7.398.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskType {
    #[default]
    Luminance,
    Alpha,
}

/// `mask-mode` (CSS Masking 1). Default `MatchSource` (toma del
/// `mask-image` su modo nativo). NO hereda. Fase 7.399.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskMode {
    Alpha,
    Luminance,
    #[default]
    MatchSource,
}

/// `mask-clip` (CSS Masking 1). Default `BorderBox`. NO hereda. Acepta
/// los 5 `<geometry-box>` + `NoClip`. Fase 7.400.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskClip {
    #[default]
    BorderBox,
    PaddingBox,
    ContentBox,
    FillBox,
    StrokeBox,
    ViewBox,
    NoClip,
}

/// `mask-composite` (CSS Masking 1). Default `Add` (spec). NO hereda.
/// Fase 7.401.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskComposite {
    #[default]
    Add,
    Subtract,
    Intersect,
    Exclude,
}

/// `mask-origin` (CSS Masking 1). Default `BorderBox`. NO hereda.
/// `<geometry-box>` puro (sin `no-clip`). Fase 7.402.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskOrigin {
    #[default]
    BorderBox,
    PaddingBox,
    ContentBox,
    FillBox,
    StrokeBox,
    ViewBox,
}

/// `container-type` (CSS Containment 3). Default `Normal` (no es un
/// query container). NO hereda. Fase 7.407.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContainerType {
    #[default]
    Normal,
    Size,
    InlineSize,
}

/// `hyphenate-limit-chars` (CSS Text 4). Triple `<total> <start> <end>`
/// donde cada campo puede ser `auto` (`None`) o un entero ≥1. Default
/// `(None, None, None)`. HEREDA. Fase 7.430.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HyphenateLimitChars {
    /// Largo mínimo de la palabra completa para permitir hifenado.
    pub total: Option<u32>,
    /// Mínimo de caracteres antes del hyphen.
    pub start: Option<u32>,
    /// Mínimo de caracteres después del hyphen.
    pub end: Option<u32>,
}

/// `text-size-adjust` (CSS Text Inline 3). Default `Auto`. HEREDA. Fase 7.431.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TextSizeAdjust {
    #[default]
    Auto,
    None,
    /// `<pct>` — porcentaje (100% = sin ajuste). Plumb: no aplicamos.
    Pct(f32),
}

/// `font-variant-emoji` (CSS Fonts 4). Selecciona la presentación cuando
/// un codepoint tiene tanto glifo emoji a color como texto monocromo.
/// Default `Normal`. HEREDA. Fase 7.433.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontVariantEmoji {
    #[default]
    Normal,
    Text,
    Emoji,
    Unicode,
}

/// `block-step-size` (CSS Inline Layout 3). Tamaño de la cuadrícula vertical
/// (`<length>`). Default `None` (sin alineación a cuadrícula). NO hereda.
/// Fase 7.454.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BlockStepSize {
    #[default]
    None,
    Length(f32),
}

/// `block-step-insert` (CSS Inline Layout 3). Dónde se inserta el espacio
/// extra para alinear a la cuadrícula vertical. Default `MarginBox`. NO hereda.
/// Fase 7.455.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockStepInsert {
    #[default]
    MarginBox,
    PaddingBox,
}

/// `block-step-align` (CSS Inline Layout 3). Cómo se distribuye el espacio
/// dentro del block-step. Default `Auto`. NO hereda. Fase 7.456.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockStepAlign {
    #[default]
    Auto,
    Center,
    Start,
    End,
}

/// `block-step-round` (CSS Inline Layout 3). Redondeo al múltiplo de
/// `block-step-size`. Default `Up`. NO hereda. Fase 7.457.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockStepRound {
    #[default]
    Up,
    Down,
    Nearest,
}

/// `position-visibility` (CSS Anchor Positioning 1). Política de visibilidad
/// de un elemento posicionado contra su anchor cuando éste queda fuera del
/// viewport o de su containing block. Default `Always`. NO hereda. Fase 7.459.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionVisibility {
    #[default]
    Always,
    AnchorsVisible,
    NoOverflow,
}

/// `position-try-order` (CSS Anchor Positioning 1). Orden de prueba de las
/// posiciones fallback. Default `Normal` (= en orden declarado). NO hereda.
/// Fase 7.460.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionTryOrder {
    #[default]
    Normal,
    MostWidth,
    MostHeight,
    MostBlockSize,
    MostInlineSize,
}

/// `animation-range-{start,end}` (CSS Animations 2). Rango temporal del
/// scroll/view-timeline en el que la animación está activa. `Normal` = 0%/100%
/// del timeline. `Length(<length-or-pct>)` = offset numérico. `Named { phase,
/// offset }` = fase + offset opcional (`cover 20%`, `entry 0%`). El offset es
/// porcentaje del rango de la fase. Default `Normal`. NO hereda. Fase 7.464/465.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum AnimationRange {
    #[default]
    Normal,
    Length(LengthVal),
    Named {
        phase: AnimationRangePhase,
        /// Offset porcentual relativo a la fase. `None` = default de la fase
        /// (start → 0%, end → 100%).
        offset_pct: Option<f32>,
    },
}

/// Fase nombrada de un `animation-range-{start,end}`. CSS Animations 2 sobre
/// view-timeline. Fase 7.464/465.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationRangePhase {
    Cover,
    Contain,
    Entry,
    Exit,
    EntryCrossing,
    ExitCrossing,
}

/// `transition-behavior` (CSS Transitions 2). `Normal` = sólo props
/// interpolables; `AllowDiscrete` permite transiciones en propiedades
/// discretas (`display`, `visibility`, ...). Default `Normal`. NO hereda.
/// Fase 7.467.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransitionBehavior {
    #[default]
    Normal,
    AllowDiscrete,
}

/// `interpolate-size` (CSS Values 5). `NumericOnly` = el chrome interpola
/// sólo entre dos `<length-percentage>` numéricos; `AllowKeywords` extiende
/// la interpolación a `auto`/`min-content`/`max-content`/`fit-content`.
/// Default `NumericOnly`. **HEREDA**. Fase 7.468.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InterpolateSize {
    #[default]
    NumericOnly,
    AllowKeywords,
}

/// `interactivity` (CSS UI 4). `Auto` = el elemento responde a input
/// normalmente; `Inert` = el elemento (y sus descendientes) NO reciben
/// input ni foco — inert se propaga por herencia. Default `Auto`.
/// **HEREDA** (no en spec strict — la herencia se logra normativamente
/// porque inert se propaga al subtree completo; modelamos como property
/// heredable para evitar recorrer ancestors al evaluar input). Fase 7.473.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interactivity {
    #[default]
    Auto,
    Inert,
}

/// `animation-composition` (CSS Animations 2). Cómo componer un efecto
/// animado con el valor "underlying" en curso. Default `Replace`. NO
/// hereda. Fase 7.481.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationComposition {
    #[default]
    Replace,
    Add,
    Accumulate,
}

/// `reading-flow` (CSS Display 4). Reordena el "focus order" (tabbing /
/// AT) en contenedores flex y grid. Default `Normal`. NO hereda.
/// Fase 7.484.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadingFlow {
    #[default]
    Normal,
    FlexVisual,
    FlexFlow,
    GridRows,
    GridColumns,
    GridOrder,
}

/// `image-resolution` (CSS Images 4). Resolución intrínseca aplicada a
/// imágenes raster — `FromImage` deja la metadata del archivo;
/// `Resolution(dppx)` la sobreescribe. Default `FromImage`. **HEREDA**.
/// Fase 7.485.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageResolution {
    FromImage,
    Resolution { dppx: f32, snap: bool },
}

impl Default for ImageResolution {
    fn default() -> Self {
        Self::FromImage
    }
}

/// `bookmark-state` (CSS GCPM). Estado inicial del marcador PDF cuando
/// el viewer lo abre. Default `Open`. NO hereda. Fase 7.487.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BookmarkState {
    #[default]
    Open,
    Closed,
}

/// `footnote-display` (CSS GCPM 4). Default `Block`. Fase 7.490.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FootnoteDisplay {
    #[default]
    Block,
    Inline,
    Compact,
}

/// `footnote-policy` (CSS GCPM 4). Política de quiebre de página al
/// emitir la nota. Default `Auto`. Fase 7.491.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FootnotePolicy {
    #[default]
    Auto,
    Line,
    Block,
}

/// `marker-knockout-{left,right}` (CSS GCPM 4). Default `Auto`.
/// Fase 7.492/493.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarkerKnockout {
    #[default]
    Auto,
    None,
}

/// `leading-trim` (CSS Inline 3). Default `Normal`. Fase 7.494.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeadingTrim {
    #[default]
    Normal,
    Start,
    End,
    Both,
}

/// `initial-letter-align` (CSS Inline 3). Default `Auto`. Fase 7.495.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InitialLetterAlign {
    #[default]
    Auto,
    Alphabetic,
    Hanging,
    Ideographic,
    BorderBox,
}

/// `border-image-repeat` (CSS Backgrounds 3) — cómo se tilea el slice
/// medio del border-image. Default `Stretch`. Fase 7.503.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderImageRepeat {
    #[default]
    Stretch,
    Repeat,
    Round,
    Space,
}

/// `text-emphasis-skip` (CSS Text Decoration 4). Default `Spaces`.
/// Fase 7.513.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEmphasisSkip {
    #[default]
    Spaces,
    Punctuation,
    Symbols,
    Narrow,
}

/// `float` (CSS2.1 §9.5 + Logical Properties). Saca la caja del flujo y la
/// pega a un lado. `InlineStart`/`InlineEnd` son los valores lógicos (en LTR
/// horizontal mapean a `Left`/`Right`). Default `None`. NO hereda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Float {
    #[default]
    None,
    Left,
    Right,
    InlineStart,
    InlineEnd,
}

/// `clear` (CSS2.1 §9.5.2 + Logical Properties). Pide que el borde del margen
/// del elemento quede por debajo de los floats del lado indicado. `Both` cubre
/// ambos lados. `InlineStart`/`InlineEnd` son lógicos. Default `None`. NO hereda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clear {
    #[default]
    None,
    Left,
    Right,
    Both,
    InlineStart,
    InlineEnd,
}

/// Componente de colocación de `masonry-auto-flow` (CSS Grid 3 draft).
/// `Pack` rellena huecos; `Next` respeta el orden de la pista. Default `Pack`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MasonryPlacement {
    #[default]
    Pack,
    Next,
}

/// Componente de orden de `masonry-auto-flow` (CSS Grid 3 draft).
/// `DefiniteFirst` coloca primero los ítems con posición definida; `Ordered`
/// respeta el orden del documento. Default `DefiniteFirst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MasonryOrder {
    #[default]
    DefiniteFirst,
    Ordered,
}

/// `masonry-auto-flow` (CSS Grid 3 draft): `[ pack | next ] ||
/// [ definite-first | ordered ]`. Default `pack definite-first`. NO hereda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MasonryAutoFlow {
    pub placement: MasonryPlacement,
    pub order: MasonryOrder,
}

/// `float-defer` (CSS Page Floats 3). `None` = `none` (sin diferir);
/// `Last` = `last`; `By(n)` = diferir N fragmentos. Default `None`.
/// Fase 7.519.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FloatDefer {
    #[default]
    None,
    Last,
    By(i32),
}

/// `float-reference` (CSS Page Floats 3). Default `Inline`. Fase 7.520.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FloatReference {
    #[default]
    Inline,
    Column,
    Region,
    Page,
}

/// `box-decoration-break` (CSS Fragmentation 4). Default `Slice`.
/// Fase 7.522.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxDecorationBreak {
    #[default]
    Slice,
    Clone,
}

/// `line-snap` (CSS Line Grid). Default `None`. Fase 7.523.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineSnap {
    #[default]
    None,
    Baseline,
    Contain,
}

/// `line-grid` (CSS Line Grid). Default `Match`. Fase 7.524.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineGrid {
    #[default]
    Match,
    Create,
}

/// `ruby-merge` (CSS Ruby 1). Default `Separate`. Fase 7.527.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RubyMerge {
    #[default]
    Separate,
    Collapse,
    Auto,
}

/// `speak-as` (CSS Speech 1). Default `Normal`. Fase 7.529.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpeakAs {
    #[default]
    Normal,
    SpellOut,
    Digits,
    LiteralPunctuation,
    NoPunctuation,
}

/// `overflow-clip-box` (CSS Overflow legacy). Default `PaddingBox`.
/// Fase 7.548.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowClipBox {
    #[default]
    PaddingBox,
    ContentBox,
}

/// `mask-border-repeat` (CSS Masking 1). Cómo se escala/repite la imagen del
/// borde-máscara. Default `Stretch`. Fase 7.553.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskBorderRepeat {
    #[default]
    Stretch,
    Repeat,
    Round,
    Space,
}

/// `mask-border-mode` (CSS Masking 1). Default `Alpha`. Fase 7.554.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskBorderMode {
    Luminance,
    #[default]
    Alpha,
}

/// `caret-animation` (CSS UI 4). Default `Auto` (parpadea). Fase 7.555.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaretAnimation {
    #[default]
    Auto,
    Manual,
}

/// `scroll-marker-group` (CSS Overflow 5). Default `None`. Fase 7.556.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollMarkerGroup {
    #[default]
    None,
    Before,
    After,
}

/// `scroll-initial-target` (CSS Overflow 5). Default `None`. Fase 7.557.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollInitialTarget {
    #[default]
    None,
    Nearest,
}

/// `speak` (CSS 2.1 aural). Default `Normal`. Fase 7.572. Distinto de
/// `speak-as` (CSS Speech 1), que ya vive en `SpeakAs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Speak {
    #[default]
    Normal,
    None,
    SpellOut,
}

/// `text-decoration-skip-box` (CSS Text Decor 4). Default `None`. Fase 7.575.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecorationSkipBox {
    #[default]
    None,
    All,
}

/// `text-decoration-skip-inset` (CSS Text Decor 4). Default `None`. Fase 7.578.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecorationSkipInset {
    #[default]
    None,
    Auto,
}

/// `text-group-align` (CSS Text 4). Default `None`. Fase 7.583.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextGroupAlign {
    #[default]
    None,
    Start,
    End,
    Left,
    Right,
    Center,
}

/// `continue` (CSS Overflow 4). Default `Auto`. Fase 7.584.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Continue {
    #[default]
    Auto,
    Discard,
}

/// `region-fragment` (CSS Regions 1). Default `Auto`. Fase 7.587.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegionFragment {
    #[default]
    Auto,
    Break,
}

/// `marquee-style` (CSS Marquee). Default `Scroll`. Fase 7.589.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarqueeStyle {
    #[default]
    Scroll,
    Slide,
    Alternate,
}

/// `marquee-direction` (CSS Marquee). Default `Forward`. Fase 7.590.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarqueeDirection {
    #[default]
    Forward,
    Reverse,
}

/// `marquee-speed` (CSS Marquee). Default `Normal`. Fase 7.591.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarqueeSpeed {
    Slow,
    #[default]
    Normal,
    Fast,
}

/// `hyphenate-limit-last` (CSS Text 4). Default `None`. Fase 7.560.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HyphenateLimitLast {
    #[default]
    None,
    Always,
    Column,
    Page,
    Spread,
}

/// `offset-rotate` (CSS Motion Path 1). Default `auto` (la dirección del
/// path orienta el elemento). `reverse` = `auto + 180deg`. NO hereda.
/// Fase 7.449.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffsetRotate {
    /// `auto` flag — orientación seguida del path.
    pub auto: bool,
    /// `reverse` flag — `auto + 180deg`.
    pub reverse: bool,
    /// `<angle>` aditivo en grados. 0 si no se especifica.
    pub angle_deg: f32,
}

impl Default for OffsetRotate {
    fn default() -> Self {
        Self { auto: true, reverse: false, angle_deg: 0.0 }
    }
}

/// `ruby-overhang` (CSS Ruby 1). Permite que el ruby sobresalga sobre
/// caracteres adyacentes para mejor balance. Default `Auto`. HEREDA.
/// Fase 7.453.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RubyOverhang {
    #[default]
    Auto,
    None,
}

/// `text-combine-upright` (CSS Writing Modes 3). Combina caracteres
/// horizontales en un cuadrado en escritura vertical. Default `None`.
/// NO hereda. Fase 7.447.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextCombineUpright {
    #[default]
    None,
    All,
    /// `digits <integer>?` (default 2). `0` = sin combinar dígitos.
    Digits(u32),
}

/// `ruby-align` (CSS Ruby 1). Distribución del texto de ruby contra la
/// base. Default `SpaceAround`. HEREDA. Fase 7.448.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RubyAlign {
    Start,
    Center,
    SpaceBetween,
    #[default]
    SpaceAround,
}

/// `grid-auto-flow` (CSS Grid 1). Cómo se colocan los ítems implícitos.
/// Default `Row`. NO hereda. Fase 7.441.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridAutoFlow {
    #[default]
    Row,
    Column,
    RowDense,
    ColumnDense,
}

/// `contain-intrinsic-*` (CSS Containment 3). Tamaño intrínseco declarado
/// para un elemento `contain: size` (o `content-visibility: auto`). El
/// prefijo `auto` indica "usá el último recordado, si no, este length".
/// Default `None`. NO hereda. Fase 7.434-7.438.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ContainIntrinsicSize {
    /// `none` — sin tamaño intrínseco declarado.
    #[default]
    None,
    /// `<length>` puro.
    Length(f32),
    /// `auto none`.
    AutoNone,
    /// `auto <length>`.
    AutoLength(f32),
}

impl ContainFlags {
    /// `strict` = `size layout style paint`.
    pub const STRICT: Self = Self {
        size: true,
        inline_size: false,
        layout: true,
        style: true,
        paint: true,
    };
    /// `content` = `layout style paint` (sin `size`).
    pub const CONTENT: Self = Self {
        size: false,
        inline_size: false,
        layout: true,
        style: true,
        paint: true,
    };
    /// `true` si NINGÚN bit está activo (equiv. `contain: none`).
    pub const fn is_none(self) -> bool {
        !self.size && !self.inline_size && !self.layout && !self.style && !self.paint
    }
}

