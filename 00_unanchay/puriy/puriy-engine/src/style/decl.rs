//! Declaraciones CSS: el tipo `Decl` (propiedad parseada + `!important`) y el
//! enum `DeclKind` con todas las propiedades soportadas, más el `impl Decl` que
//! aplica cada declaración sobre un `ComputedStyle`. Extraído de `style/mod.rs`
//! (regla #1). Comparte los tipos del módulo `style` vía `use super::*`.
use super::*;

/// Una declaración CSS individual + flag `!important`.
#[derive(Debug, Clone)]
pub(crate) struct Decl {
    pub(crate) kind: DeclKind,
    pub(crate) important: bool,
}

/// Keyword CSS-wide (`inherit`/`initial`/`unset`/`revert`). `revert` se
/// trata como `unset` (no implementamos rollback por origen de cascada).
/// Ver Fase 7.225.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WideKw {
    Inherit,
    Initial,
    Unset,
}

/// Propiedad-destino de un keyword CSS-wide. Subset curado: los usos reales
/// de `inherit`/`initial` se concentran en estas propiedades (resets tipo
/// `* { box-sizing: inherit }`, `button { color: inherit }`). Ver Fase 7.225.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WideProp {
    Color,
    Background,
    FontSize,
    FontWeight,
    FontStyle,
    FontFamily,
    LineHeight,
    TextAlign,
    TextDecoration,
    Visibility,
    Display,
    BoxSizing,
    BorderColor,
}

impl WideProp {
    /// Todas las propiedades del subset curado. Lo consume el shorthand `all`
    /// (Fase 7.851) para expandir `all: <wide-kw>` a un `Wide` por longhand.
    pub(crate) const ALL: [WideProp; 13] = [
        WideProp::Color,
        WideProp::Background,
        WideProp::FontSize,
        WideProp::FontWeight,
        WideProp::FontStyle,
        WideProp::FontFamily,
        WideProp::LineHeight,
        WideProp::TextAlign,
        WideProp::TextDecoration,
        WideProp::Visibility,
        WideProp::Display,
        WideProp::BoxSizing,
        WideProp::BorderColor,
    ];

    /// `true` si la propiedad hereda por defecto (define qué hace `unset`).
    pub(crate) fn is_inherited(self) -> bool {
        matches!(
            self,
            WideProp::Color
                | WideProp::FontSize
                | WideProp::FontWeight
                | WideProp::FontStyle
                | WideProp::FontFamily
                | WideProp::LineHeight
                | WideProp::TextAlign
                | WideProp::TextDecoration
                | WideProp::Visibility
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) enum DeclKind {
    Color(Color),
    /// Keyword CSS-wide (`inherit`/`initial`/`unset`/`revert`). Se resuelve
    /// en `compute_with_parent` (necesita el estilo del padre + el default).
    Wide {
        prop: WideProp,
        kw: WideKw,
    },
    Background(Color),
    Display(Display),
    FontSize(f32),
    /// `font-size` relativo: multiplicador (`em`/`%`/`larger`/`smaller`)
    /// resuelto al cierre contra el font-size heredado. Ver Fase 7.223.
    FontSizeRel(f32),
    FontWeight(u16),
    FontStyle(FontStyle),
    FontFamily(String),
    Margin(Sides<f32>),
    MarginTop(f32),
    MarginRight(f32),
    MarginBottom(f32),
    MarginLeft(f32),
    /// `margin-left/right: auto` — flag de centrado horizontal.
    MarginLeftAuto(bool),
    MarginRightAuto(bool),
    /// `margin-top/bottom: auto` — flag de centrado/empuje vertical. Sólo tiene
    /// efecto cuando el padre es flex/grid (en block flow CSS lo computa a 0);
    /// la resolución contra el contexto se hace al construir el box.
    MarginTopAuto(bool),
    MarginBottomAuto(bool),
    Padding(Sides<f32>),
    PaddingTop(f32),
    PaddingRight(f32),
    PaddingBottom(f32),
    PaddingLeft(f32),
    Width(LengthVal),
    Height(LengthVal),
    MaxWidth(LengthVal),
    TextAlign(TextAlign),
    LineHeight(f32),
    /// `line-height: normal` (Fase 7.831). Resetea a `None` (≈1.2,
    /// font-dependent). NO es lo mismo que un número — por eso variante aparte.
    LineHeightNormal,
    BorderWidth(f32),
    BorderColor(Color),
    /// `border-style: solid` activa el dibujo del border; `none`/`hidden`
    /// lo desactiva (color → None).
    BorderEnabled(bool),
    /// `border-style` uniforme con el patrón visual (`dashed`/`dotted`/
    /// `double`/`solid`). Complementa a `BorderEnabled` (que sólo togglea).
    BorderStyleKind(BorderLineStyle),
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
    /// Lista de sombras (vacía = `box-shadow: none`). Se aplica con
    /// asignación total — sustituye la lista previa, no acumula.
    BoxShadows(Vec<BoxShadow>),
    TextDecoration(TextDecorationLine),
    /// `text-decoration-color`. `None` = `currentColor`.
    TextDecorationColor(Option<Color>),
    TextDecorationStyle(TextDecorationStyle),
    /// `None` = `auto`/`from-font` (grosor derivado del font-size).
    TextDecorationThickness(Option<f32>),
    /// `None` = `auto` (offset default).
    TextUnderlineOffset(Option<f32>),
    ListStyleType(ListStyleType),
    FlexDirection(FlexDirection),
    JustifyContent(JustifyContent),
    AlignItems(AlignItems),
    AlignContent(AlignContent),
    JustifyItems(AlignItems),
    JustifySelf(AlignSelf),
    FlexWrap(FlexWrap),
    /// `gap: A B` setea ambos (row=A, column=B); `gap: V` los iguala.
    Gap { row: f32, column: f32 },
    RowGap(f32),
    ColumnGap(f32),
    BoxSizing(BoxSizing),
    MinWidth(LengthVal),
    MinHeight(LengthVal),
    MaxHeight(LengthVal),
    /// `None` = `aspect-ratio: auto` (resetea).
    AspectRatio(Option<f32>),
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
    /// Patrón visual del outline (`dashed`/`dotted`/`double`/`solid`).
    OutlineStylePattern(BorderLineStyle),
    OutlineOffset(f32),
    BackgroundGradient(LinearGradient),
    /// `background-image: none` limpia el gradient (un autor puede
    /// querer overridear un gradient heredado).
    BackgroundGradientNone,
    /// `background-image: url(...)` — URL absoluta o relativa, el engine
    /// la resuelve contra el base del documento en `build_node`.
    BackgroundImageUrl(String),
    BackgroundSize(BackgroundSize),
    BackgroundPosition(BackgroundPosition),
    BackgroundRepeat(BackgroundRepeat),
    /// Capas de background EXTRA (debajo de la capa 0), de un `background:`
    /// o `background-image:` con varias capas separadas por coma. La
    /// shorthand siempre la emite (posiblemente vacía) para resetear.
    BackgroundExtraLayers(Vec<BackgroundLayer>),
    BackgroundOrigin(BackgroundOrigin),
    BackgroundClip(BackgroundClip),
    Position(Position),
    InsetTop(LengthVal),
    InsetRight(LengthVal),
    InsetBottom(LengthVal),
    InsetLeft(LengthVal),
    VerticalAlign(VerticalAlign),
    Visibility(Visibility),
    PointerEvents(PointerEvents),
    ObjectFit(ObjectFit),
    ObjectPosition(BackgroundPosition),
    /// `None` = `caret-color: auto`.
    CaretColor(Option<Color>),
    /// `None` = `accent-color: auto`.
    AccentColor(Option<Color>),
    Cursor(Cursor),
    TextOverflow(TextOverflow),
    ScrollBehavior(ScrollBehavior),
    TabSize(TabSize),
    UserSelect(UserSelect),
    OverflowWrap(OverflowWrap),
    WordBreak(WordBreak),
    Hyphens(Hyphens),
    Resize(Resize),
    WritingMode(WritingMode),
    Direction(Direction),
    UnicodeBidi(UnicodeBidi),
    /// `font-stretch` ya normalizado a multiplicador (0.5..=2.0).
    FontStretch(f32),
    ImageRendering(ImageRendering),
    MixBlendMode(BlendMode),
    /// Lista paralela a las capas de background. Vacío = resetear.
    BackgroundBlendMode(Vec<BlendMode>),
    Isolation(Isolation),
    /// `will-change: auto` = lista vacía.
    WillChange(Vec<WillChangeHint>),
    Appearance(Appearance),
    FontKerning(FontKerning),
    /// Vacío = `font-feature-settings: normal`.
    FontFeatureSettings(Vec<FontFeatureSetting>),
    /// Vacío = `font-variation-settings: normal`.
    FontVariationSettings(Vec<FontVariationSetting>),
    /// `None` = `font-language-override: normal`.
    FontLanguageOverride(Option<String>),
    TextRendering(TextRendering),
    /// Vacío = `filter: none`.
    Filter(Vec<FilterFn>),
    /// Vacío = `backdrop-filter: none`.
    BackdropFilter(Vec<FilterFn>),
    TextOrientation(TextOrientation),
    OverscrollBehaviorX(OverscrollBehavior),
    OverscrollBehaviorY(OverscrollBehavior),
    ScrollSnapType(ScrollSnapType),
    ScrollSnapAlignBlock(ScrollSnapAlign),
    ScrollSnapAlignInline(ScrollSnapAlign),
    ScrollSnapStop(ScrollSnapStop),
    /// `scroll-padding` shorthand. Sides T/R/B/L con `LengthVal` (acepta
    /// `auto`).
    ScrollPadding(Sides<LengthVal>),
    ScrollPaddingTop(LengthVal),
    ScrollPaddingRight(LengthVal),
    ScrollPaddingBottom(LengthVal),
    ScrollPaddingLeft(LengthVal),
    /// `scroll-margin` shorthand. Sides T/R/B/L en px (la spec no acepta %).
    ScrollMargin(Sides<f32>),
    ScrollMarginTop(f32),
    ScrollMarginRight(f32),
    ScrollMarginBottom(f32),
    ScrollMarginLeft(f32),
    TouchAction(TouchAction),
    /// `None` = `clip-path: none`.
    ClipPath(Option<ClipPath>),
    /// `None` = `mask-image: none`.
    MaskImage(Option<MaskImage>),
    ContentVisibility(ContentVisibility),
    Contain(ContainFlags),
    /// `None` = `column-count: auto`.
    ColumnCount(Option<u32>),
    /// `LengthVal::Auto` = `column-width: auto`.
    ColumnWidth(LengthVal),
    ColumnRuleWidth(f32),
    /// `None` = `column-rule-color: currentColor`.
    ColumnRuleColor(Option<Color>),
    /// `style: none/hidden` → flag `style_active=false` (apaga el dibujo).
    ColumnRuleStyleActive(bool),
    /// Patrón visual del column-rule (`dashed`/`dotted`/`double`/`solid`).
    ColumnRuleStylePattern(BorderLineStyle),
    /// CSS Gap Decorations 1 (Fase 7.920) — espejo de `column-rule-*` para el
    /// eje de filas. `RowRule{Width,Color,StyleActive,StylePattern}`.
    RowRuleWidth(f32),
    /// `None` = `row-rule-color: currentColor`.
    RowRuleColor(Option<Color>),
    RowRuleStyleActive(bool),
    RowRuleStylePattern(BorderLineStyle),
    ColumnFill(ColumnFill),
    ColumnSpan(ColumnSpan),
    BreakInside(BreakInside),
    TableLayout(TableLayout),
    BorderCollapse(BorderCollapse),
    /// `border-spacing` shorthand: emite tupla horizontal+vertical.
    BorderSpacing { h: f32, v: f32 },
    CaptionSide(CaptionSide),
    EmptyCells(EmptyCells),
    BreakBefore(BreakBetween),
    BreakAfter(BreakBetween),
    Orphans(u32),
    Widows(u32),
    ColorScheme(ColorScheme),
    ListStylePosition(ListStylePosition),
    /// `None` = `list-style-image: none`.
    ListStyleImage(Option<String>),
    /// `counter-set: name [N] ...`. Mismo shape que `counter-reset`.
    CounterSet(Vec<(String, i32)>),
    Quotes(Quotes),
    TextUnderlinePosition(TextUnderlinePosition),
    TextJustify(TextJustify),
    PrintColorAdjust(PrintColorAdjust),
    ForcedColorAdjust(ForcedColorAdjust),
    /// `None` = `line-clamp: none` (sin truncado).
    LineClamp(Option<u32>),
    FontVariantCaps(FontVariantCaps),
    FontVariantNumeric(FontVariantNumeric),
    FontVariantLigatures(FontVariantLigatures),
    FontVariantEastAsian(FontVariantEastAsian),
    FontVariantPosition(FontVariantPosition),
    TextEmphasisStyle(TextEmphasisStyle),
    /// `None` = `text-emphasis-color: currentColor`.
    TextEmphasisColor(Option<Color>),
    TextEmphasisPosition(TextEmphasisPosition),
    RubyPosition(RubyPosition),
    TransformOrigin(TransformOrigin),
    TransformStyle(TransformStyle),
    /// `None` = `perspective: none` (sin proyección).
    Perspective(Option<f32>),
    PerspectiveOrigin(PerspectiveOrigin),
    BackfaceVisibility(BackfaceVisibility),
    ScrollbarWidth(ScrollbarWidth),
    /// `None` = `scrollbar-color: auto`.
    ScrollbarColor(Option<ScrollbarColorPair>),
    ScrollbarGutter(ScrollbarGutter),
    OverflowAnchor(OverflowAnchor),
    /// `None` = `overflow-clip-margin: 0px` (sin extensión).
    OverflowClipMargin(Option<OverflowClipMargin>),
    TextAlignLast(TextAlignLast),
    TextWrap(TextWrap),
    LineBreak(LineBreak),
    HangingPunctuation(HangingPunctuation),
    TextDecorationSkipInk(TextDecorationSkipInk),
    FontOpticalSizing(FontOpticalSizing),
    /// Sólo el axis `weight` de `font-synthesis-*`.
    FontSynthesisWeight(bool),
    FontSynthesisStyle(bool),
    FontSynthesisSmallCaps(bool),
    /// `font-synthesis` shorthand emite los 3 axes a la vez.
    FontSynthesisAll(FontSynthesis),
    FontSizeAdjust(FontSizeAdjust),
    ImageOrientation(ImageOrientation),
    AnimationTimeline(TimelineRef),
    /// `None` = `scroll-timeline-name: none`.
    ScrollTimelineName(Option<String>),
    ScrollTimelineAxis(TimelineAxis),
    /// `None` = `view-timeline-name: none`.
    ViewTimelineName(Option<String>),
    ViewTimelineAxis(TimelineAxis),
    WhiteSpaceCollapse(WhiteSpaceCollapse),
    TextWrapMode(TextWrapMode),
    TextWrapStyle(TextWrapStyle),
    /// `wrap-before` / `wrap-after` / `wrap-inside` (CSS Text 4). Plumb. Fase 7.927.
    WrapBefore(WrapBetween),
    WrapAfter(WrapBetween),
    WrapInside(WrapInside),
    TextSpacingTrim(TextSpacingTrim),
    TextBoxTrim(TextBoxTrim),
    MathStyle(MathStyle),
    MathDepth(MathDepth),
    MathShift(MathShift),
    FieldSizing(FieldSizing),
    /// `overlay` (CSS Position 4). NO hereda. Plumb opaco. Fase 7.905.
    Overlay(Overlay),
    /// `dynamic-range-limit` (CSS Color HDR 1). HEREDA. Plumb opaco. Fase 7.905.
    DynamicRangeLimit(DynamicRangeLimit),
    TextBoxEdge(TextBoxEdge),
    /// `anchor-name`. Vec vacío = `none`.
    AnchorName(Vec<String>),
    /// `None` = `position-anchor: auto`.
    PositionAnchor(Option<String>),
    AnchorScope(AnchorScope),
    /// `None` = `view-transition-name: none`.
    ViewTransitionName(Option<String>),
    /// `view-transition-class`. Vec vacío = `none`.
    ViewTransitionClass(Vec<String>),
    FontPalette(FontPalette),
    FontVariantAlternates(FontVariantAlternates),
    /// `background-attachment`. Lista paralela a las capas; vacía =
    /// resetear a `[Scroll]`.
    BackgroundAttachment(Vec<BackgroundAttachment>),
    CaretShape(CaretShape),
    BaselineSource(BaselineSource),
    AlignmentBaseline(AlignmentBaseline),
    DominantBaseline(DominantBaseline),
    PaintOrder(PaintOrder),
    MarkerSide(MarkerSide),
    Fill(SvgPaint),
    Stroke(SvgPaint),
    FillOpacity(f32),
    StrokeOpacity(f32),
    StrokeWidth(LengthVal),
    StrokeLinecap(StrokeLinecap),
    StrokeLinejoin(StrokeLinejoin),
    StrokeMiterlimit(f32),
    /// Vec vacío = `stroke-dasharray: none`.
    StrokeDasharray(Vec<LengthVal>),
    StrokeDashoffset(LengthVal),
    FillRule(FillRule),
    ClipRule(FillRule),
    ColorInterpolation(ColorInterpolation),
    ShapeRendering(ShapeRendering),
    VectorEffect(VectorEffect),
    /// `None` = `flood-color: currentColor`.
    FloodColor(Option<Color>),
    FloodOpacity(f32),
    /// `None` = `lighting-color: currentColor`.
    LightingColor(Option<Color>),
    /// `None` = `stop-color: currentColor`.
    StopColor(Option<Color>),
    StopOpacity(f32),
    TextAnchor(TextAnchor),
    ColorRendering(ColorRendering),
    ColorInterpolationFilters(ColorInterpolationFilters),
    GlyphOrientationVertical(GlyphOrientationVertical),
    TransformBox(TransformBox),
    /// `None` = `marker-start: none`.
    MarkerStart(MarkerRef),
    /// `None` = `marker-mid: none`.
    MarkerMid(MarkerRef),
    /// `None` = `marker-end: none`.
    MarkerEnd(MarkerRef),
    MaskType(MaskType),
    MaskMode(MaskMode),
    MaskClip(MaskClip),
    MaskComposite(MaskComposite),
    MaskOrigin(MaskOrigin),
    MaskRepeat(BackgroundRepeat),
    MaskPosition(BackgroundPosition),
    MaskSize(BackgroundSize),
    /// Vec vacío = `container-name: none`.
    ContainerName(Vec<String>),
    ContainerType(ContainerType),
    /// `offset-path` (Fase 7.427). `None` = `none`; `Some(s)` guarda el
    /// valor crudo (parse opaco). NO hereda. Plumb.
    OffsetPath(Option<String>),
    /// `offset-distance` (Fase 7.428). `length | <pct>`. Default `Px(0)`.
    /// NO hereda. Plumb.
    OffsetDistance(LengthVal),
    /// `hyphenate-character` (Fase 7.429). `None` = `auto`. HEREDA. Plumb.
    HyphenateCharacter(Option<String>),
    /// `hyphenate-limit-chars` (Fase 7.430). HEREDA. Plumb.
    HyphenateLimitChars(HyphenateLimitChars),
    /// `text-size-adjust` (Fase 7.431). HEREDA. Plumb.
    TextSizeAdjust(TextSizeAdjust),
    /// `line-height-step` (Fase 7.432). HEREDA. Plumb.
    LineHeightStep(f32),
    /// `font-variant-emoji` (Fase 7.433). HEREDA. Plumb.
    FontVariantEmoji(FontVariantEmoji),
    /// `contain-intrinsic-width` (Fase 7.434). NO hereda. Plumb.
    ContainIntrinsicWidth(ContainIntrinsicSize),
    /// `contain-intrinsic-height` (Fase 7.435). NO hereda. Plumb.
    ContainIntrinsicHeight(ContainIntrinsicSize),
    /// `background-position-x` (Fase 7.439). Reescribe sólo el eje X
    /// de `background_position`. NO hereda. Plumb.
    BackgroundPositionX(LengthVal),
    /// `background-position-y` (Fase 7.440). Reescribe sólo el eje Y
    /// de `background_position`. NO hereda. Plumb.
    BackgroundPositionY(LengthVal),
    /// `grid-auto-flow` (Fase 7.441). NO hereda. Plumb.
    GridAutoFlow(GridAutoFlow),
    /// `grid-auto-columns` (Fase 7.442). NO hereda. Plumb.
    GridAutoColumns(Vec<GridTrackSize>),
    /// `grid-auto-rows` (Fase 7.443). NO hereda. Plumb.
    GridAutoRows(Vec<GridTrackSize>),
    /// `shape-outside` (Fase 7.444). `None` = `none`. NO hereda. Plumb.
    ShapeOutside(Option<String>),
    /// `shape-margin` (Fase 7.445). NO hereda. Plumb.
    ShapeMargin(LengthVal),
    /// `shape-image-threshold` (Fase 7.446). Clamp [0..1]. NO hereda. Plumb.
    ShapeImageThreshold(f32),
    /// `text-combine-upright` (Fase 7.447). NO hereda. Plumb.
    TextCombineUpright(TextCombineUpright),
    /// `ruby-align` (Fase 7.448). HEREDA. Plumb.
    RubyAlign(RubyAlign),
    /// `offset-rotate` (Fase 7.449). NO hereda. Plumb.
    OffsetRotate(OffsetRotate),
    /// `offset-anchor` (Fase 7.450). `None` = `auto`. NO hereda. Plumb.
    OffsetAnchor(Option<BackgroundPosition>),
    /// `offset-position` (Fase 7.451). `None` = `auto`. NO hereda. Plumb.
    OffsetPosition(Option<BackgroundPosition>),
    /// `object-view-box` (Fase 7.452). `None` = `none`. NO hereda. Plumb.
    ObjectViewBox(Option<String>),
    /// `ruby-overhang` (Fase 7.453). HEREDA. Plumb.
    RubyOverhang(RubyOverhang),
    /// `block-step-size` (Fase 7.454). NO hereda. Plumb.
    BlockStepSize(BlockStepSize),
    /// `block-step-insert` (Fase 7.455). NO hereda. Plumb.
    BlockStepInsert(BlockStepInsert),
    /// `block-step-align` (Fase 7.456). NO hereda. Plumb.
    BlockStepAlign(BlockStepAlign),
    /// `block-step-round` (Fase 7.457). NO hereda. Plumb.
    BlockStepRound(BlockStepRound),
    /// `position-visibility` (Fase 7.459). NO hereda. Plumb.
    PositionVisibility(PositionVisibility),
    /// `position-try-order` (Fase 7.460). NO hereda. Plumb.
    PositionTryOrder(PositionTryOrder),
    /// `position-try-fallbacks` (Fase 7.461). Vec vacío = `none`. NO hereda. Plumb.
    PositionTryFallbacks(Vec<String>),
    /// `position-area` (Fase 7.463). `None` = `none`. NO hereda. Plumb.
    PositionArea(Option<String>),
    /// `animation-range-start` (Fase 7.464). NO hereda. Plumb.
    AnimationRangeStart(AnimationRange),
    /// `animation-range-end` (Fase 7.465). NO hereda. Plumb.
    AnimationRangeEnd(AnimationRange),
    /// `transition-behavior` (Fase 7.467). NO hereda. Plumb.
    TransitionBehavior(TransitionBehavior),
    /// `interpolate-size` (Fase 7.468). HEREDA. Plumb.
    InterpolateSize(InterpolateSize),
    /// `view-timeline-inset` (Fase 7.469). Par `(start, end)`. NO hereda. Plumb.
    ViewTimelineInset(LengthVal, LengthVal),
    /// `font-synthesis-position` (Fase 7.470). HEREDA. Plumb.
    FontSynthesisPosition(bool),
    /// `interactivity` (Fase 7.473). HEREDA. Plumb.
    Interactivity(Interactivity),
    /// `cx` (Fase 7.474). Geometría SVG. NO hereda. Plumb.
    Cx(LengthVal),
    /// `cy` (Fase 7.475). Geometría SVG. NO hereda. Plumb.
    Cy(LengthVal),
    /// `r` (Fase 7.476). Radio del círculo SVG. NO hereda. Plumb.
    R(LengthVal),
    /// `rx` (Fase 7.477). Radio elipse SVG. `Auto` = `auto`. NO hereda. Plumb.
    Rx(LengthVal),
    /// `ry` (Fase 7.478). Radio elipse SVG. `Auto` = `auto`. NO hereda. Plumb.
    Ry(LengthVal),
    /// `x` (SVG 2). Posición SVG como prop CSS. NO hereda. Plumb.
    X(LengthVal),
    /// `y` (SVG 2). Posición SVG como prop CSS. NO hereda. Plumb.
    Y(LengthVal),
    /// `baseline-shift` (SVG / CSS Inline 3). NO hereda. Plumb.
    BaselineShift(BaselineShift),
    /// `solid-color` (SVG 2 `<solidcolor>`). NO hereda. Plumb.
    SolidColor(Color),
    /// `solid-opacity` (SVG 2 `<solidcolor>`). Alpha 0-1. NO hereda. Plumb.
    SolidOpacity(f32),
    /// `order` (Fase 7.479). Orden flex/grid. Default 0. NO hereda. Plumb.
    Order(i32),
    /// `path-length` (Fase 7.480). SVG. `None` = `none`. NO hereda. Plumb.
    PathLength(Option<f32>),
    /// `animation-composition` (Fase 7.481). NO hereda. Plumb.
    AnimationComposition(AnimationComposition),
    /// `timeline-scope` (Fase 7.482). Vec vacío = `none`. NO hereda. Plumb.
    TimelineScope(Vec<String>),
    /// `reading-order` (Fase 7.483). CSS Inline 3. Default 0. NO hereda. Plumb.
    ReadingOrder(i32),
    /// `reading-flow` (Fase 7.484). CSS Display 4. NO hereda. Plumb.
    ReadingFlow(ReadingFlow),
    /// `image-resolution` (Fase 7.485). HEREDA. Plumb.
    ImageResolution(ImageResolution),
    /// `bookmark-level` (Fase 7.486). `None` = `none`. NO hereda. Plumb.
    BookmarkLevel(Option<u32>),
    /// `bookmark-state` (Fase 7.487). NO hereda. Plumb.
    BookmarkState(BookmarkState),
    /// `bookmark-label` (Fase 7.488). `None` = `content(text)`. NO hereda. Plumb.
    BookmarkLabel(Option<String>),
    /// `string-set` (Fase 7.489). `None` = `none`. NO hereda. Plumb.
    StringSet(Option<String>),
    /// `footnote-display` (Fase 7.490). NO hereda. Plumb.
    FootnoteDisplay(FootnoteDisplay),
    /// `footnote-policy` (Fase 7.491). NO hereda. Plumb.
    FootnotePolicy(FootnotePolicy),
    /// `marker-knockout-left` (Fase 7.492). NO hereda. Plumb.
    MarkerKnockoutLeft(MarkerKnockout),
    /// `marker-knockout-right` (Fase 7.493). NO hereda. Plumb.
    MarkerKnockoutRight(MarkerKnockout),
    /// `leading-trim` (Fase 7.494). HEREDA. Plumb.
    LeadingTrim(LeadingTrim),
    /// `initial-letter-align` (Fase 7.495). HEREDA. Plumb.
    InitialLetterAlign(InitialLetterAlign),
    /// `text-autospace` (Fase 7.496). `None` = `normal`. HEREDA. Plumb.
    TextAutospace(Option<String>),
    /// `white-space-trim` (Fase 7.497). `None` = `none`. HEREDA. Plumb.
    WhiteSpaceTrim(Option<String>),
    /// `view-transition-group` (Fase 7.498). `None` = `normal`. NO hereda. Plumb.
    ViewTransitionGroup(Option<String>),
    /// `inset-area` (Fase 7.499). `None` = `none`. NO hereda. Plumb.
    InsetArea(Option<String>),
    /// `view-transition-image-pair` (Fase 7.500). `None` = `auto`. NO hereda. Plumb.
    ViewTransitionImagePair(Option<String>),
    /// `animation-trigger` (Fase 7.501). Shorthand opaco. NO hereda. Plumb.
    AnimationTrigger(Option<String>),
    /// `border-image-source` (Fase 7.502). `None` = `none`. NO hereda. Plumb.
    BorderImageSource(Option<String>),
    /// `border-image-repeat` (Fase 7.503). NO hereda. Plumb.
    BorderImageRepeat(BorderImageRepeat, BorderImageRepeat),
    /// `border-image-slice` (Fase 7.504). `None` = `100%`. NO hereda. Plumb.
    BorderImageSlice(Option<String>),
    /// `border-image-width` (Fase 7.505). `None` = `1`. NO hereda. Plumb.
    BorderImageWidth(Option<String>),
    /// `border-image-outset` (Fase 7.506). `None` = `0`. NO hereda. Plumb.
    BorderImageOutset(Option<String>),
    /// `border-image` shorthand (Fase 7.507). `None` = `none`. NO hereda. Plumb.
    BorderImage(Option<String>),
    /// `grid-template-areas` (Fase 7.508). `None` = `none`. NO hereda. Plumb.
    GridTemplateAreas(Option<String>),
    /// `grid-row-start` (Fase 7.509). `None` = `auto`. NO hereda. Plumb.
    GridRowStart(Option<String>),
    /// `grid-row-end` (Fase 7.510). `None` = `auto`. NO hereda. Plumb.
    GridRowEnd(Option<String>),
    /// `grid-column-start` (Fase 7.511). `None` = `auto`. NO hereda. Plumb.
    GridColumnStart(Option<String>),
    /// `grid-column-end` (Fase 7.512). `None` = `auto`. NO hereda. Plumb.
    GridColumnEnd(Option<String>),
    /// `text-emphasis-skip` (Fase 7.513). HEREDA. Plumb.
    TextEmphasisSkip(TextEmphasisSkip),
    /// `animation-name` (Fase 7.514). `None` = `none` (desactiva la
    /// binding). NO hereda. Plumb.
    AnimationName(Option<String>),
    /// `animation-duration` (Fase 7.515). Segundos. NO hereda. Plumb.
    AnimationDuration(f32),
    /// `animation-timing-function` (Fase 7.516). NO hereda. Plumb.
    AnimationTimingFunction(EasingFunction),
    /// `animation-iteration-count` (Fase 7.517). NO hereda. Plumb.
    AnimationIterationCount(AnimationIterations),
    /// `animation-fill-mode` (Fase 7.518). NO hereda. Plumb.
    AnimationFillMode(AnimationFillMode),
    /// `animation-direction` (Fase 7.816). NO hereda. Plumb.
    AnimationDirection(AnimationDirection),
    /// `animation-play-state` (Fase 7.817). NO hereda. Plumb.
    AnimationPlayState(AnimationPlayState),
    /// `animation-delay` (Fase 7.818). Segundos. NO hereda. Plumb.
    AnimationDelay(f32),
    /// `transition-property` longhand (Fase 7.822). `None` = `none` (limpia
    /// la lista de transiciones); `Some(prop)` fija la propiedad del 1er
    /// binding. NO hereda. Plumb.
    TransitionPropertyFirst(Option<String>),
    /// `transition-duration` longhand (Fase 7.823). Segundos sobre el 1er
    /// binding. NO hereda. Plumb.
    TransitionDurationFirst(f32),
    /// `transition-timing-function` longhand (Fase 7.824). NO hereda. Plumb.
    TransitionTimingFirst(EasingFunction),
    /// `transition-delay` longhand (Fase 7.825). Segundos. NO hereda. Plumb.
    TransitionDelayFirst(f32),
    /// Prop individual `translate` (Fase 7.826). `None` = `none`. NO hereda.
    Translate(Option<Transform>),
    /// Prop individual `rotate` (Fase 7.827). `None` = `none`. NO hereda.
    Rotate(Option<Transform>),
    /// Prop individual `scale` (Fase 7.828). `None` = `none`. NO hereda.
    Scale(Option<Transform>),
    /// `float` (CSS2.1 §9.5). NO hereda. Plumb.
    Float(Float),
    /// `clear` (CSS2.1 §9.5.2). NO hereda. Plumb.
    Clear(Clear),
    /// `page` (CSS Paged Media 3). `None` = `auto`; `Some(name)` = `@page`
    /// nombrado. NO hereda. Plumb.
    Page(Option<String>),
    /// `clip` (CSS2.1, deprecada). NO hereda. Plumb.
    Clip(Clip),
    /// `d` (SVG 2 §6) como prop CSS. `None` = `none`; `Some(raw)` = `path(...)`.
    /// NO hereda. Plumb opaco.
    D(Option<String>),
    /// `masonry-auto-flow` (CSS Grid 3 draft). NO hereda. Plumb.
    MasonryAutoFlow(MasonryAutoFlow),
    /// `justify-tracks` (CSS Grid 3 draft). NO hereda. Plumb.
    JustifyTracks(Vec<JustifyContent>),
    /// `align-tracks` (CSS Grid 3 draft). NO hereda. Plumb.
    AlignTracks(Vec<AlignContent>),
    /// `grid-template-columns: subgrid` (CSS Grid 2). Flag plumb (la
    /// maquinaria de subgrid de layout no está). NO hereda.
    GridTemplateColumnsSubgrid(bool),
    /// `grid-template-rows: subgrid` (CSS Grid 2). Flag plumb. NO hereda.
    GridTemplateRowsSubgrid(bool),
    /// `float-defer` (Fase 7.519). NO hereda. Plumb.
    FloatDefer(FloatDefer),
    /// `float-reference` (Fase 7.520). NO hereda. Plumb.
    FloatReference(FloatReference),
    /// `float-offset` (Fase 7.521). px. NO hereda. Plumb.
    FloatOffset(f32),
    /// `box-decoration-break` (Fase 7.522). NO hereda. Plumb.
    BoxDecorationBreak(BoxDecorationBreak),
    /// `line-snap` (Fase 7.523). HEREDA. Plumb.
    LineSnap(LineSnap),
    /// `line-grid` (Fase 7.524). HEREDA. Plumb.
    LineGrid(LineGrid),
    /// `initial-letter` shorthand (Fase 7.525). `None` = `normal`. HEREDA. Plumb.
    InitialLetter(Option<String>),
    /// `highlight` (Fase 7.526). `None` = `none`. HEREDA. Plumb.
    Highlight(Option<String>),
    /// `ruby-merge` (Fase 7.527). HEREDA. Plumb.
    RubyMerge(RubyMerge),
    /// `text-spacing` shorthand (Fase 7.528). `None` = `normal`. HEREDA. Plumb.
    TextSpacing(Option<String>),
    /// `speak-as` (Fase 7.529). HEREDA. Plumb.
    SpeakAs(SpeakAs),
    /// `voice-balance` (Fase 7.530). -100..100. HEREDA. Plumb.
    VoiceBalance(f32),
    /// `voice-pitch` (Fase 7.531). `None` = `medium`. HEREDA. Plumb.
    VoicePitch(Option<String>),
    /// `voice-rate` (Fase 7.532). `None` = `normal`. HEREDA. Plumb.
    VoiceRate(Option<String>),
    /// `voice-volume` (Fase 7.533). `None` = `medium`. HEREDA. Plumb.
    VoiceVolume(Option<String>),
    /// `voice-family` (Fase 7.919). `None` = `preserve`. Plumb opaco. CSS Speech 1.
    VoiceFamily(Option<String>),
    /// `voice-stress` (Fase 7.919). `None` = `normal`. Plumb opaco. CSS Speech 1.
    VoiceStress(Option<String>),
    /// `voice-duration` (Fase 7.919). `None` = `auto`. Plumb opaco. CSS Speech 1.
    VoiceDuration(Option<String>),
    /// `pause-before` (Fase 7.534). `None` = `none`. HEREDA. Plumb.
    PauseBefore(Option<String>),
    /// `pause-after` (Fase 7.535). `None` = `none`. HEREDA. Plumb.
    PauseAfter(Option<String>),
    /// `rest-before` (Fase 7.536). `None` = `none`. HEREDA. Plumb.
    RestBefore(Option<String>),
    /// `rest-after` (Fase 7.537). `None` = `none`. HEREDA. Plumb.
    RestAfter(Option<String>),
    /// `cue-fade-duration` (Fase 7.538). Segundos. NO hereda. Plumb.
    CueFadeDuration(f32),
    /// `cue-before` (Fase 7.539). `None` = `none`. NO hereda. Plumb.
    CueBefore(Option<String>),
    /// `cue-after` (Fase 7.540). `None` = `none`. NO hereda. Plumb.
    CueAfter(Option<String>),
    /// `cue` shorthand (Fase 7.541). `None` = `none`. NO hereda. Plumb.
    Cue(Option<String>),
    /// `navigation-up` (Fase 7.542). `None` = `auto`. NO hereda. Plumb.
    NavigationUp(Option<String>),
    /// `glyph-orientation-horizontal` (Fase 7.543). Grados. HEREDA. Plumb.
    GlyphOrientationHorizontal(f32),
    /// `navigation-down` (Fase 7.544). `None` = `auto`. NO hereda. Plumb.
    NavigationDown(Option<String>),
    /// `navigation-left` (Fase 7.545). `None` = `auto`. NO hereda. Plumb.
    NavigationLeft(Option<String>),
    /// `navigation-right` (Fase 7.546). `None` = `auto`. NO hereda. Plumb.
    NavigationRight(Option<String>),
    /// `counter-increment-style` (Fase 7.547). `None` = `decimal`. NO hereda. Plumb.
    CounterIncrementStyle(Option<String>),
    /// `overflow-clip-box` (Fase 7.548). NO hereda. Plumb.
    OverflowClipBox(OverflowClipBox),
    /// `mask-border-source` (Fase 7.549). `None` = `none`. NO hereda. Plumb.
    MaskBorderSource(Option<String>),
    /// `mask-border-slice` (Fase 7.550). `None` = `0`. NO hereda. Plumb.
    MaskBorderSlice(Option<String>),
    /// `mask-border-width` (Fase 7.551). `None` = `auto`. NO hereda. Plumb.
    MaskBorderWidth(Option<String>),
    /// `mask-border-outset` (Fase 7.552). `None` = `0`. NO hereda. Plumb.
    MaskBorderOutset(Option<String>),
    /// `mask-border-repeat` (Fase 7.553). NO hereda. Plumb.
    MaskBorderRepeat(MaskBorderRepeat),
    /// `mask-border-mode` (Fase 7.554). NO hereda. Plumb.
    MaskBorderMode(MaskBorderMode),
    /// `mask-border` shorthand (Fase 7.909). `None` = `none`. Parse opaco
    /// (igual que `border-image`). NO hereda. Plumb.
    MaskBorder(Option<String>),
    /// `caret-animation` (Fase 7.555). HEREDA. Plumb.
    CaretAnimation(CaretAnimation),
    /// `scroll-marker-group` (Fase 7.556). NO hereda. Plumb.
    ScrollMarkerGroup(ScrollMarkerGroup),
    /// `scroll-initial-target` (Fase 7.557). NO hereda. Plumb.
    ScrollInitialTarget(ScrollInitialTarget),
    /// `corner-shape` (Fase 7.558). `None` = `round`. NO hereda. Plumb.
    CornerShape(Option<String>),
    /// `hyphenate-limit-lines` (Fase 7.559). `None` = `no-limit`. HEREDA. Plumb.
    HyphenateLimitLines(Option<u32>),
    /// `hyphenate-limit-last` (Fase 7.560). HEREDA. Plumb.
    HyphenateLimitLast(HyphenateLimitLast),
    /// `hyphenate-limit-zone` (Fase 7.561). `None` = `0`. HEREDA. Plumb.
    HyphenateLimitZone(Option<String>),
    /// `interest-target` (Fase 7.562). `None` = sin target. NO hereda. Plumb.
    InterestTarget(Option<String>),
    /// `scroll-start` y sus longhands lógicos (CSS Scroll Snap 2). `None` =
    /// `auto`. NO hereda. Plumb (no se consume en el scroll). Fase 7.928.
    ScrollStart(Option<String>),
    ScrollStartBlock(Option<String>),
    ScrollStartInline(Option<String>),
    /// `scroll-start-target` + longhands lógicos. `None` = `none`. Fase 7.928.
    ScrollStartTarget(Option<String>),
    ScrollStartTargetBlock(Option<String>),
    ScrollStartTargetInline(Option<String>),
    /// `interest-delay-start` (Fase 7.563). `None` = `normal`. NO hereda. Plumb.
    InterestDelayStart(Option<String>),
    /// `interest-delay-end` (Fase 7.564). `None` = `normal`. NO hereda. Plumb.
    InterestDelayEnd(Option<String>),
    /// `azimuth` (Fase 7.565). `None` = `center`. HEREDA. Plumb.
    Azimuth(Option<String>),
    /// `elevation` (Fase 7.566). `None` = `level`. HEREDA. Plumb.
    Elevation(Option<String>),
    /// `richness` (Fase 7.567). 0–100. HEREDA. Plumb.
    Richness(f32),
    /// `stress` (Fase 7.568). 0–100. HEREDA. Plumb.
    Stress(f32),
    /// `pitch` (Fase 7.569). `None` = `medium`. HEREDA. Plumb.
    Pitch(Option<String>),
    /// `speech-rate` (Fase 7.570). `None` = `medium`. HEREDA. Plumb.
    SpeechRate(Option<String>),
    /// `volume` (Fase 7.571). `None` = `medium`. HEREDA. Plumb.
    Volume(Option<String>),
    /// `speak` (Fase 7.572). HEREDA. Plumb.
    Speak(Speak),
    /// `speak-header` (CSS 2.1 aural). `None` = `once`. HEREDA. Plumb. Fase 7.930.
    SpeakHeader(Option<String>),
    /// `pitch-range` (CSS 2.1 aural). 0–100. HEREDA. Plumb. Fase 7.930.
    PitchRange(f32),
    /// `margin-trim` (CSS Box 4). `None` = `none`. NO hereda. Plumb. Fase 7.931.
    MarginTrim(Option<String>),
    /// `margin-break` (CSS Fragmentation 4). `None` = `auto`. NO hereda. Plumb.
    MarginBreak(Option<String>),
    /// `input-security` (CSS UI 4). `None` = `auto`. NO hereda. Plumb. Fase 7.931.
    InputSecurity(Option<String>),
    /// `border-boundary` (CSS Round Display 1). `None` = `none`. NO hereda. Plumb.
    BorderBoundary(Option<String>),
    /// `shape-inside` (CSS Shapes 2). `None` = `auto`. NO hereda. Plumb. Fase 7.932.
    ShapeInside(Option<String>),
    /// `speak-punctuation` (CSS 2.1 aural). `None` = `none`. HEREDA. Plumb. Fase 7.932.
    SpeakPunctuation(Option<String>),
    /// `speak-numeral` (CSS 2.1 aural). `None` = `continuous`. HEREDA. Plumb. Fase 7.932.
    SpeakNumeral(Option<String>),
    /// `play-during` (Fase 7.573). `None` = `auto`. NO hereda. Plumb.
    PlayDuring(Option<String>),
    /// `text-decoration-skip` (Fase 7.574). `None` = `auto`. HEREDA. Plumb.
    TextDecorationSkip(Option<String>),
    /// `text-decoration-skip-box` (Fase 7.575). HEREDA. Plumb.
    TextDecorationSkipBox(TextDecorationSkipBox),
    /// `text-decoration-skip-self` (Fase 7.576). `None` = `auto`. HEREDA. Plumb.
    TextDecorationSkipSelf(Option<String>),
    /// `text-decoration-skip-spaces` (Fase 7.577). `None` = `start end`.
    /// HEREDA. Plumb.
    TextDecorationSkipSpaces(Option<String>),
    /// `text-decoration-skip-inset` (Fase 7.578). HEREDA. Plumb.
    TextDecorationSkipInset(TextDecorationSkipInset),
    /// `-webkit-text-stroke-width` (Fase 7.579). px. HEREDA. Plumb.
    WebkitTextStrokeWidth(f32),
    /// `-webkit-text-stroke-color` (Fase 7.580). `None` = `currentColor`.
    /// HEREDA. Plumb.
    WebkitTextStrokeColor(Option<String>),
    /// `-webkit-text-fill-color` (Fase 7.581). `None` = `currentColor`.
    /// HEREDA. Plumb.
    WebkitTextFillColor(Option<String>),
    /// `font-smooth` (Fase 7.582). `None` = `auto`. HEREDA. Plumb.
    FontSmooth(Option<String>),
    /// `text-group-align` (Fase 7.583). NO hereda. Plumb.
    TextGroupAlign(TextGroupAlign),
    /// `continue` (Fase 7.584). NO hereda. Plumb.
    Continue(Continue),
    /// `block-ellipsis` (Fase 7.585). `None` = `none`. HEREDA. Plumb.
    BlockEllipsis(Option<String>),
    /// `max-lines` (Fase 7.586). `None` = `none`. NO hereda. Plumb.
    MaxLines(Option<u32>),
    /// `region-fragment` (Fase 7.587). NO hereda. Plumb.
    RegionFragment(RegionFragment),
    /// `overflow-style` (Fase 7.588). `None` = `auto`. NO hereda. Plumb.
    OverflowStyle(Option<String>),
    /// `marquee-style` (Fase 7.589). NO hereda. Plumb.
    MarqueeStyle(MarqueeStyle),
    /// `marquee-direction` (Fase 7.590). NO hereda. Plumb.
    MarqueeDirection(MarqueeDirection),
    /// `marquee-speed` (Fase 7.591). NO hereda. Plumb.
    MarqueeSpeed(MarqueeSpeed),
    /// `marquee-loop` (Fase 7.592). `None` = `infinite`. NO hereda. Plumb.
    MarqueeLoop(Option<i32>),
    /// `marquee-increment` (Fase 7.593). `None` = `6px`. NO hereda. Plumb.
    MarqueeIncrement(Option<String>),
    /// `nav-index` (Fase 7.594). `None` = `auto`. NO hereda. Plumb.
    NavIndex(Option<String>),
    /// `nav-up` (Fase 7.595). `None` = `auto`. NO hereda. Plumb.
    NavUp(Option<String>),
    /// `nav-down` (Fase 7.596). `None` = `auto`. NO hereda. Plumb.
    NavDown(Option<String>),
    /// `nav-left` (Fase 7.597). `None` = `auto`. NO hereda. Plumb.
    NavLeft(Option<String>),
    /// `nav-right` (Fase 7.598). `None` = `auto`. NO hereda. Plumb.
    NavRight(Option<String>),
    /// `-webkit-box-orient` (Fase 7.599). `None` = `inline-axis`. NO hereda. Plumb.
    WebkitBoxOrient(Option<String>),
    /// `-webkit-box-direction` (Fase 7.600). `None` = `normal`. NO hereda. Plumb.
    WebkitBoxDirection(Option<String>),
    /// `-webkit-box-align` (Fase 7.601). `None` = `stretch`. NO hereda. Plumb.
    WebkitBoxAlign(Option<String>),
    /// `-webkit-box-pack` (Fase 7.602). `None` = `start`. NO hereda. Plumb.
    WebkitBoxPack(Option<String>),
    /// `-webkit-box-flex` (Fase 7.603). NO hereda. Plumb.
    WebkitBoxFlex(f32),
    /// `-webkit-box-ordinal-group` (Fase 7.604). `None` = `1`. NO hereda. Plumb.
    WebkitBoxOrdinalGroup(Option<u32>),
    /// `-webkit-font-smoothing` (Fase 7.605). `None` = `auto`. HEREDA. Plumb.
    WebkitFontSmoothing(Option<String>),
    /// `-moz-osx-font-smoothing` (Fase 7.606). `None` = `auto`. HEREDA. Plumb.
    MozOsxFontSmoothing(Option<String>),
    /// `-webkit-tap-highlight-color` (Fase 7.607). NO hereda. Plumb.
    WebkitTapHighlightColor(Option<String>),
    /// `zoom` (Fase 7.608). `None` = `normal`. NO hereda. Plumb.
    Zoom(Option<String>),
    /// `column-break-before` (Fase 7.614). `None` = `auto`. NO hereda. Plumb.
    ColumnBreakBefore(Option<String>),
    /// `column-break-after` (Fase 7.615). `None` = `auto`. NO hereda. Plumb.
    ColumnBreakAfter(Option<String>),
    /// `column-break-inside` (Fase 7.616). `None` = `auto`. NO hereda. Plumb.
    ColumnBreakInside(Option<String>),
    /// `user-modify` (Fase 7.617). `None` = `read-only`. HEREDA. Plumb.
    UserModify(Option<String>),
    /// `-webkit-touch-callout` (Fase 7.618). `None` = `default`. HEREDA. Plumb.
    WebkitTouchCallout(Option<String>),
    /// `-webkit-user-drag` (Fase 7.619). `None` = `auto`. NO hereda. Plumb.
    WebkitUserDrag(Option<String>),
    /// `-webkit-rtl-ordering` (Fase 7.620). `None` = `logical`. HEREDA. Plumb.
    WebkitRtlOrdering(Option<String>),
    /// `-webkit-text-security` (Fase 7.621). `None` = `none`. HEREDA. Plumb.
    WebkitTextSecurity(Option<String>),
    /// `-webkit-nbsp-mode` (Fase 7.622). `None` = `normal`. HEREDA. Plumb.
    WebkitNbspMode(Option<String>),
    /// `-webkit-locale` (Fase 7.623). `None` = `auto`. HEREDA. Plumb.
    WebkitLocale(Option<String>),
    /// `-webkit-column-axis` (Fase 7.624). `None` = `auto`. NO hereda. Plumb.
    WebkitColumnAxis(Option<String>),
    /// `-webkit-column-progression` (Fase 7.625). `None` = `normal`. NO hereda. Plumb.
    WebkitColumnProgression(Option<String>),
    /// `-webkit-app-region` (Fase 7.626). `None` = `none`. NO hereda. Plumb.
    WebkitAppRegion(Option<String>),
    /// `-webkit-highlight` (Fase 7.627). `None` = `none`. HEREDA. Plumb.
    WebkitHighlight(Option<String>),
    /// `-webkit-box-reflect` (Fase 7.628). `None` = `none`. NO hereda. Plumb.
    WebkitBoxReflect(Option<String>),
    /// `-webkit-mask-composite` (Fase 7.644). `None` = `add`. NO hereda. Plumb.
    WebkitMaskComposite(Option<String>),
    /// `-webkit-mask-position-x` (Fase 7.645). `None` = `center`. NO hereda. Plumb.
    WebkitMaskPositionX(Option<String>),
    /// `-webkit-mask-position-y` (Fase 7.646). `None` = `center`. NO hereda. Plumb.
    WebkitMaskPositionY(Option<String>),
    /// `-webkit-mask-repeat-x` (Fase 7.647). `None` = `repeat`. NO hereda. Plumb.
    WebkitMaskRepeatX(Option<String>),
    /// `-webkit-mask-repeat-y` (Fase 7.648). `None` = `repeat`. NO hereda. Plumb.
    WebkitMaskRepeatY(Option<String>),
    /// `-webkit-margin-start` (Fase 7.649). `None` = `0`. NO hereda. Plumb.
    WebkitMarginStart(Option<String>),
    /// `-webkit-margin-end` (Fase 7.650). `None` = `0`. NO hereda. Plumb.
    WebkitMarginEnd(Option<String>),
    /// `-webkit-margin-before` (Fase 7.651). `None` = `0`. NO hereda. Plumb.
    WebkitMarginBefore(Option<String>),
    /// `-webkit-margin-after` (Fase 7.652). `None` = `0`. NO hereda. Plumb.
    WebkitMarginAfter(Option<String>),
    /// `-webkit-padding-start` (Fase 7.653). `None` = `0`. NO hereda. Plumb.
    WebkitPaddingStart(Option<String>),
    /// `-webkit-padding-end` (Fase 7.654). `None` = `0`. NO hereda. Plumb.
    WebkitPaddingEnd(Option<String>),
    /// `-webkit-padding-before` (Fase 7.655). `None` = `0`. NO hereda. Plumb.
    WebkitPaddingBefore(Option<String>),
    /// `-webkit-padding-after` (Fase 7.656). `None` = `0`. NO hereda. Plumb.
    WebkitPaddingAfter(Option<String>),
    /// `-webkit-logical-width` (Fase 7.657). `None` = `auto`. NO hereda. Plumb.
    WebkitLogicalWidth(Option<String>),
    /// `-webkit-logical-height` (Fase 7.658). `None` = `auto`. NO hereda. Plumb.
    WebkitLogicalHeight(Option<String>),
    /// `-webkit-transform-origin-x` (Fase 7.664). `None` = `50%`. NO hereda. Plumb.
    WebkitTransformOriginX(Option<String>),
    /// `-webkit-transform-origin-y` (Fase 7.665). `None` = `50%`. NO hereda. Plumb.
    WebkitTransformOriginY(Option<String>),
    /// `-webkit-transform-origin-z` (Fase 7.666). `None` = `0`. NO hereda. Plumb.
    WebkitTransformOriginZ(Option<String>),
    /// `-webkit-perspective-origin-x` (Fase 7.667). `None` = `50%`. NO hereda. Plumb.
    WebkitPerspectiveOriginX(Option<String>),
    /// `-webkit-perspective-origin-y` (Fase 7.668). `None` = `50%`. NO hereda. Plumb.
    WebkitPerspectiveOriginY(Option<String>),
    /// `-webkit-min-logical-width` (Fase 7.669). `None` = `auto`. NO hereda. Plumb.
    WebkitMinLogicalWidth(Option<String>),
    /// `-webkit-max-logical-width` (Fase 7.670). `None` = `none`. NO hereda. Plumb.
    WebkitMaxLogicalWidth(Option<String>),
    /// `-webkit-min-logical-height` (Fase 7.671). `None` = `auto`. NO hereda. Plumb.
    WebkitMinLogicalHeight(Option<String>),
    /// `-webkit-max-logical-height` (Fase 7.672). `None` = `none`. NO hereda. Plumb.
    WebkitMaxLogicalHeight(Option<String>),
    /// `-webkit-background-composite` (Fase 7.673). `None` = `source-over`. NO hereda. Plumb.
    WebkitBackgroundComposite(Option<String>),
    /// `-webkit-border-before` (Fase 7.674). `None` = `none`. NO hereda. Plumb.
    WebkitBorderBefore(Option<String>),
    /// `-webkit-border-after` (Fase 7.675). `None` = `none`. NO hereda. Plumb.
    WebkitBorderAfter(Option<String>),
    /// `-webkit-border-start` (Fase 7.676). `None` = `none`. NO hereda. Plumb.
    WebkitBorderStart(Option<String>),
    /// `-webkit-border-end` (Fase 7.677). `None` = `none`. NO hereda. Plumb.
    WebkitBorderEnd(Option<String>),
    /// `-webkit-border-horizontal-spacing` (Fase 7.678). `None` = `0`. HEREDA. Plumb.
    WebkitBorderHorizontalSpacing(Option<String>),
    /// `-webkit-flow-into` (Fase 7.679). `None` = `none`. NO hereda. Plumb.
    WebkitFlowInto(Option<String>),
    /// `-webkit-flow-from` (Fase 7.680). `None` = `none`. NO hereda. Plumb.
    WebkitFlowFrom(Option<String>),
    /// `-webkit-region-break-before` (Fase 7.681). `None` = `auto`. NO hereda. Plumb.
    WebkitRegionBreakBefore(Option<String>),
    /// `-webkit-region-break-after` (Fase 7.682). `None` = `auto`. NO hereda. Plumb.
    WebkitRegionBreakAfter(Option<String>),
    /// `-webkit-region-break-inside` (Fase 7.683). `None` = `auto`. NO hereda. Plumb.
    WebkitRegionBreakInside(Option<String>),
    /// `-webkit-border-before-color` (Fase 7.698). `None` = `currentcolor`. NO hereda. Plumb.
    WebkitBorderBeforeColor(Option<String>),
    /// `-webkit-border-before-style` (Fase 7.699). `None` = `none`. NO hereda. Plumb.
    WebkitBorderBeforeStyle(Option<String>),
    /// `-webkit-border-before-width` (Fase 7.700). `None` = `medium`. NO hereda. Plumb.
    WebkitBorderBeforeWidth(Option<String>),
    /// `-webkit-border-after-color` (Fase 7.701). `None` = `currentcolor`. NO hereda. Plumb.
    WebkitBorderAfterColor(Option<String>),
    /// `-webkit-border-after-style` (Fase 7.702). `None` = `none`. NO hereda. Plumb.
    WebkitBorderAfterStyle(Option<String>),
    /// `-webkit-border-after-width` (Fase 7.703). `None` = `medium`. NO hereda. Plumb.
    WebkitBorderAfterWidth(Option<String>),
    /// `-webkit-border-start-color` (Fase 7.704). `None` = `currentcolor`. NO hereda. Plumb.
    WebkitBorderStartColor(Option<String>),
    /// `-webkit-border-start-style` (Fase 7.705). `None` = `none`. NO hereda. Plumb.
    WebkitBorderStartStyle(Option<String>),
    /// `-webkit-border-start-width` (Fase 7.706). `None` = `medium`. NO hereda. Plumb.
    WebkitBorderStartWidth(Option<String>),
    /// `-webkit-border-end-color` (Fase 7.707). `None` = `currentcolor`. NO hereda. Plumb.
    WebkitBorderEndColor(Option<String>),
    /// `-webkit-border-end-style` (Fase 7.708). `None` = `none`. NO hereda. Plumb.
    WebkitBorderEndStyle(Option<String>),
    /// `-webkit-border-end-width` (Fase 7.709). `None` = `medium`. NO hereda. Plumb.
    WebkitBorderEndWidth(Option<String>),
    /// `-webkit-margin-top-collapse` (Fase 7.730). `None` = `collapse`. NO hereda. Plumb.
    WebkitMarginTopCollapse(Option<String>),
    /// `-webkit-margin-bottom-collapse` (Fase 7.731). `None` = `collapse`. NO hereda. Plumb.
    WebkitMarginBottomCollapse(Option<String>),
    /// `-webkit-margin-collapse` (Fase 7.732). `None` = `collapse`. NO hereda. Plumb.
    WebkitMarginCollapse(Option<String>),
    /// `-webkit-border-vertical-spacing` (Fase 7.733). `None` = `0`. HEREDA. Plumb.
    WebkitBorderVerticalSpacing(Option<String>),
    /// `-webkit-mask-source-type` (Fase 7.734). `None` = `alpha`. NO hereda. Plumb.
    WebkitMaskSourceType(Option<String>),
    /// `-webkit-marquee-direction` (Fase 7.750). `None` = `auto`. NO hereda. Plumb.
    WebkitMarqueeDirection(Option<String>),
    /// `-webkit-marquee-increment` (Fase 7.751). `None` = `6px`. NO hereda. Plumb.
    WebkitMarqueeIncrement(Option<String>),
    /// `-webkit-marquee-repetition` (Fase 7.752). `None` = `infinite`. NO hereda. Plumb.
    WebkitMarqueeRepetition(Option<String>),
    /// `-webkit-marquee-speed` (Fase 7.753). `None` = `normal`. NO hereda. Plumb.
    WebkitMarqueeSpeed(Option<String>),
    /// `-webkit-marquee-style` (Fase 7.754). `None` = `scroll`. NO hereda. Plumb.
    WebkitMarqueeStyle(Option<String>),
    /// `-webkit-overflow-scrolling` (Fase 7.755). `None` = `auto`. NO hereda. Plumb.
    WebkitOverflowScrolling(Option<String>),
    /// `-webkit-line-grid` (Fase 7.756). `None` = `none`. NO hereda. Plumb.
    WebkitLineGrid(Option<String>),
    /// `-webkit-cursor-visibility` (Fase 7.757). `None` = `auto`. NO hereda. Plumb.
    WebkitCursorVisibility(Option<String>),
    /// `-webkit-border-fit` (Fase 7.758). `None` = `border`. NO hereda. Plumb.
    WebkitBorderFit(Option<String>),
    /// `-webkit-color-correction` (Fase 7.759). `None` = `default`. HEREDA. Plumb.
    WebkitColorCorrection(Option<String>),
    TextIndent(f32),
    WordSpacing(f32),
    LetterSpacing(f32),
    TextShadows(Vec<TextShadow>),
    /// Cadena vacía = `transform: none`.
    Transforms(Vec<Transform>),
    GridTemplateColumns(Vec<GridTrackSize>),
    GridTemplateRows(Vec<GridTrackSize>),
    /// `animation: ...`. `None` = `animation: none`.
    Animation(Option<AnimationBinding>),
    /// `transition: ...`. Vec vacío = `transition: none`.
    Transitions(Vec<TransitionBinding>),
    /// `<prop>: currentColor` — se difiere y resuelve contra el `color`
    /// final del elemento en `compute_internal` (la cascada empuja el
    /// target acá vía `apply`). Ver Fase 7.210.
    CurrentColor(ColorTarget),
    // === Fase 7.966-7.985 — plumb opaco de props legacy/de nicho ===
    SpatialNavigationAction(Option<String>),
    SpatialNavigationContain(Option<String>),
    SpatialNavigationFunction(Option<String>),
    WrapFlow(Option<String>),
    WrapThrough(Option<String>),
    FlowInto(Option<String>),
    FlowFrom(Option<String>),
    MarkBefore(Option<String>),
    MarkAfter(Option<String>),
    TextAlignAll(Option<String>),
    MinZoom(Option<String>),
    MaxZoom(Option<String>),
    UserZoom(Option<String>),
    ViewportFit(Option<String>),
    ImeMode(Option<String>),
    Kerning(Option<String>),
    EnableBackground(Option<String>),
    ColorProfile(Option<String>),
    VoiceRange(Option<String>),
    TextSecurity(Option<String>),
    // === Fase 7.986-7.1005 — plumb opaco (CSS Shapes/Inline/Line-Layout) ===
    ShapePadding(Option<String>),
    LineFitEdge(Option<String>),
    InlineSizing(Option<String>),
    BoxSnap(Option<String>),
    CopyInto(Option<String>),
    LineStacking(Option<String>),
    LineStackingRuby(Option<String>),
    LineStackingShift(Option<String>),
    LineStackingStrategy(Option<String>),
    InlineBoxAlign(Option<String>),
    AlignmentAdjust(Option<String>),
    TextHeight(Option<String>),
    DropInitialSize(Option<String>),
    DropInitialValue(Option<String>),
    DropInitialBeforeAlign(Option<String>),
    DropInitialAfterAlign(Option<String>),
    DropInitialBeforeAdjust(Option<String>),
    DropInitialAfterAdjust(Option<String>),
    BlockProgression(Option<String>),
    SnapHeight(Option<String>),
    // === Fase 7.1031-7.1034 — CSS Scroll Snap v0 (deprecado, shipped) ===
    ScrollSnapPointsX(Option<String>),
    ScrollSnapPointsY(Option<String>),
    ScrollSnapDestination(Option<String>),
    ScrollSnapCoordinate(Option<String>),
    // === Fase 7.1035-7.1042 — Gecko -moz- propiedades reales (plumb opaco) ===
    MozOrient(Option<String>),
    MozUserFocus(Option<String>),
    MozUserInput(Option<String>),
    MozWindowDragging(Option<String>),
    MozFloatEdge(Option<String>),
    MozForceBrokenImageIcon(Option<String>),
    MozImageRegion(Option<String>),
    MozBinding(Option<String>),
    // === Fase 7.1043-7.1047 — Gecko -moz-outline-radius (plumb opaco) ===
    MozOutlineRadius(Option<String>),
    MozOutlineRadiusTopleft(Option<String>),
    MozOutlineRadiusTopright(Option<String>),
    MozOutlineRadiusBottomleft(Option<String>),
    MozOutlineRadiusBottomright(Option<String>),
    // === Fase 7.1048-7.1051 — SVG/masking/scroll-snap-type v0 (plumb opaco) ===
    BufferedRendering(Option<String>),
    MaskSourceType(Option<String>),
    ScrollSnapTypeX(Option<String>),
    ScrollSnapTypeY(Option<String>),
    // === Fase 7.1052-7.1057 — IE -ms- propiedades legacy reales (plumb opaco) ===
    MsOverflowStyle(Option<String>),
    MsScrollChaining(Option<String>),
    MsContentZooming(Option<String>),
    MsScrollRails(Option<String>),
    MsFlexAlign(Option<String>),
    MsFlexPack(Option<String>),
    // === Fase 7.1058-7.1062 — Gecko -moz- misc reales (plumb opaco) ===
    MozContextProperties(Option<String>),
    MozStackSizing(Option<String>),
    MozTextBlink(Option<String>),
    MozDefaultAppearance(Option<String>),
    MozBoxFlexgroup(Option<String>),
    // === Fase 7.1063-7.1072 — CSS Fill and Stroke 3 stroke-* (plumb opaco) ===
    StrokeAlign(Option<String>),
    StrokeBreak(Option<String>),
    StrokeColorCss(Option<String>),
    StrokeImage(Option<String>),
    StrokeOrigin(Option<String>),
    StrokePosition(Option<String>),
    StrokeRepeat(Option<String>),
    StrokeSize(Option<String>),
    StrokeDashCorner(Option<String>),
    StrokeDashJustify(Option<String>),
    // === Fase 7.1073-7.1079 — CSS Fill and Stroke 3 fill-* (plumb opaco) ===
    FillBreak(Option<String>),
    FillColorCss(Option<String>),
    FillImage(Option<String>),
    FillOrigin(Option<String>),
    FillPosition(Option<String>),
    FillSize(Option<String>),
    FillRepeat(Option<String>),
    // === Fase 7.1080-7.1087 — animation-trigger-* longhands (plumb opaco) ===
    AnimationTriggerBehavior(Option<String>),
    AnimationTriggerTimeline(Option<String>),
    AnimationTriggerRange(Option<String>),
    AnimationTriggerRangeStart(Option<String>),
    AnimationTriggerRangeEnd(Option<String>),
    AnimationTriggerExitRange(Option<String>),
    AnimationTriggerExitRangeStart(Option<String>),
    AnimationTriggerExitRangeEnd(Option<String>),
    // === Fase 7.1088-7.1089 — WebKit -webkit-box-* legacy (plumb opaco) ===
    WebkitBoxLines(Option<String>),
    WebkitBoxFlexGroup(Option<String>),
    // === Fase 7.1093-7.1100 — IE scrollbar-*-color legacy (plumb opaco) ===
    ScrollbarBaseColor(Option<String>),
    ScrollbarFaceColor(Option<String>),
    ScrollbarTrackColor(Option<String>),
    ScrollbarArrowColor(Option<String>),
    ScrollbarShadowColor(Option<String>),
    ScrollbarHighlightColor(Option<String>),
    Scrollbar3dlightColor(Option<String>),
    ScrollbarDarkshadowColor(Option<String>),
    // === Fase 7.1101-7.1108 — IE -ms-grid (IE10 grid) (plumb opaco) ===
    MsGridColumns(Option<String>),
    MsGridRows(Option<String>),
    MsGridColumn(Option<String>),
    MsGridRow(Option<String>),
    MsGridColumnSpan(Option<String>),
    MsGridRowSpan(Option<String>),
    MsGridColumnAlign(Option<String>),
    MsGridRowAlign(Option<String>),
    // === Fase 7.1109-7.1116 — IE -ms- exclusions/regions/text (plumb opaco) ===
    MsTouchSelect(Option<String>),
    MsTextAutospace(Option<String>),
    MsWrapFlow(Option<String>),
    MsWrapMargin(Option<String>),
    MsWrapThrough(Option<String>),
    MsFlowInto(Option<String>),
    MsFlowFrom(Option<String>),
    MsHyphenateLimitChars(Option<String>),
    // === Fase 7.1121-7.1122 — WebKit misc (plumb opaco) ===
    WebkitMaskAttachment(Option<String>),
    WebkitTextDecorationsInEffect(Option<String>),
    // === Fase 7.1123-7.1128 — CSS Borders 4 border-clip/border-limit (plumb opaco) ===
    BorderClip(Option<String>),
    BorderClipTop(Option<String>),
    BorderClipRight(Option<String>),
    BorderClipBottom(Option<String>),
    BorderClipLeft(Option<String>),
    BorderLimit(Option<String>),
}

impl Decl {
    pub(crate) fn apply(&self, s: &mut ComputedStyle) {
        match &self.kind {
            // Los keywords CSS-wide se resuelven en `compute_with_parent`
            // (vía `apply_decl`), donde el padre y el default están a mano.
            DeclKind::Wide { .. } => {}
            DeclKind::Color(c) => s.color = *c,
            DeclKind::Background(c) => s.background = Some(*c),
            DeclKind::Display(d) => s.display = *d,
            // Absoluto: fija el font-size y descarta cualquier relativo
            // pendiente de menor orden en la cascada.
            DeclKind::FontSize(v) => {
                s.font_size = *v;
                s.font_size_rel = None;
            }
            // Relativo: difiere la resolución (contra el heredado) al cierre.
            DeclKind::FontSizeRel(m) => s.font_size_rel = Some(*m),
            DeclKind::FontWeight(w) => s.font_weight = *w,
            DeclKind::FontStyle(fs) => s.font_style = *fs,
            DeclKind::FontFamily(ff) => s.font_family = Some(ff.clone()),
            // Un valor px explícito limpia el flag `auto` del mismo lado
            // (una regla posterior `margin-left: 10px` pisa a `auto`).
            DeclKind::Margin(v) => {
                s.margin = *v;
                s.margin_left_auto = false;
                s.margin_right_auto = false;
                s.margin_top_auto = false;
                s.margin_bottom_auto = false;
            }
            DeclKind::MarginTop(v) => {
                s.margin.top = *v;
                s.margin_top_auto = false;
            }
            DeclKind::MarginRight(v) => {
                s.margin.right = *v;
                s.margin_right_auto = false;
            }
            DeclKind::MarginBottom(v) => {
                s.margin.bottom = *v;
                s.margin_bottom_auto = false;
            }
            DeclKind::MarginLeft(v) => {
                s.margin.left = *v;
                s.margin_left_auto = false;
            }
            DeclKind::MarginLeftAuto(v) => s.margin_left_auto = *v,
            DeclKind::MarginRightAuto(v) => s.margin_right_auto = *v,
            DeclKind::MarginTopAuto(v) => s.margin_top_auto = *v,
            DeclKind::MarginBottomAuto(v) => s.margin_bottom_auto = *v,
            DeclKind::Padding(v) => s.padding = *v,
            DeclKind::PaddingTop(v) => s.padding.top = *v,
            DeclKind::PaddingRight(v) => s.padding.right = *v,
            DeclKind::PaddingBottom(v) => s.padding.bottom = *v,
            DeclKind::PaddingLeft(v) => s.padding.left = *v,
            DeclKind::Width(v) => s.width = *v,
            DeclKind::Height(v) => s.height = *v,
            DeclKind::MaxWidth(v) => s.max_width = *v,
            DeclKind::TextAlign(a) => s.text_align = *a,
            DeclKind::LineHeight(v) => s.line_height = Some(*v),
            DeclKind::LineHeightNormal => s.line_height = None,
            DeclKind::BorderWidth(v) => s.border_widths = Sides::all(*v),
            DeclKind::BorderColor(c) => s.border_colors = Sides::all(Some(*c)),
            DeclKind::BorderEnabled(on) => {
                if !*on {
                    s.border_colors = Sides::all(None);
                    s.border_widths = Sides::all(0.0);
                }
            }
            DeclKind::BorderStyleKind(st) => s.border_style = *st,
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
            DeclKind::BoxShadows(v) => s.box_shadows = v.clone(),
            DeclKind::TextDecoration(t) => s.text_decoration = *t,
            DeclKind::TextDecorationColor(c) => s.text_decoration_color = *c,
            DeclKind::TextDecorationStyle(st) => s.text_decoration_style = *st,
            DeclKind::TextDecorationThickness(t) => s.text_decoration_thickness = *t,
            DeclKind::TextUnderlineOffset(o) => s.text_underline_offset = *o,
            DeclKind::ListStyleType(t) => s.list_style_type = *t,
            DeclKind::FlexDirection(d) => s.flex_direction = *d,
            DeclKind::JustifyContent(j) => s.justify_content = *j,
            DeclKind::AlignItems(a) => s.align_items = *a,
            DeclKind::AlignContent(a) => s.align_content = *a,
            DeclKind::JustifyItems(a) => s.justify_items = Some(*a),
            DeclKind::JustifySelf(a) => s.justify_self = *a,
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
            DeclKind::AspectRatio(v) => s.aspect_ratio = *v,
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
            DeclKind::OutlineStylePattern(p) => s.outline.style = *p,
            DeclKind::OutlineOffset(v) => s.outline.offset = *v,
            DeclKind::BackgroundGradient(g) => s.background_gradient = Some(g.clone()),
            DeclKind::BackgroundGradientNone => {
                s.background_gradient = None;
                s.background_image_url = None;
                // `background-image: none` no deja ninguna imagen — limpia
                // también las capas extra de una lista previa.
                s.background_extra_layers.clear();
            }
            DeclKind::BackgroundImageUrl(u) => s.background_image_url = Some(u.clone()),
            DeclKind::BackgroundSize(sz) => s.background_size = *sz,
            DeclKind::BackgroundPosition(pos) => s.background_position = *pos,
            DeclKind::BackgroundRepeat(r) => s.background_repeat = *r,
            DeclKind::BackgroundExtraLayers(ls) => s.background_extra_layers = ls.clone(),
            DeclKind::BackgroundOrigin(o) => s.background_origin = *o,
            DeclKind::BackgroundClip(c) => s.background_clip = *c,
            DeclKind::Position(p) => s.position = *p,
            DeclKind::InsetTop(v) => s.inset_top = *v,
            DeclKind::InsetRight(v) => s.inset_right = *v,
            DeclKind::InsetBottom(v) => s.inset_bottom = *v,
            DeclKind::InsetLeft(v) => s.inset_left = *v,
            DeclKind::VerticalAlign(va) => s.vertical_align = *va,
            DeclKind::Visibility(v) => s.visibility = *v,
            DeclKind::PointerEvents(pe) => s.pointer_events = *pe,
            DeclKind::ObjectFit(f) => s.object_fit = Some(*f),
            DeclKind::ObjectPosition(p) => s.object_position = Some(*p),
            DeclKind::CaretColor(c) => s.caret_color = *c,
            DeclKind::AccentColor(c) => s.accent_color = *c,
            DeclKind::Cursor(c) => s.cursor = *c,
            DeclKind::TextOverflow(t) => s.text_overflow = *t,
            DeclKind::ScrollBehavior(b) => s.scroll_behavior = *b,
            DeclKind::TabSize(t) => s.tab_size = *t,
            DeclKind::UserSelect(u) => s.user_select = *u,
            DeclKind::OverflowWrap(o) => s.overflow_wrap = *o,
            DeclKind::WordBreak(w) => s.word_break = *w,
            DeclKind::Hyphens(h) => s.hyphens = *h,
            DeclKind::Resize(r) => s.resize = *r,
            DeclKind::WritingMode(w) => s.writing_mode = *w,
            DeclKind::Direction(d) => s.direction = *d,
            DeclKind::UnicodeBidi(b) => s.unicode_bidi = *b,
            DeclKind::FontStretch(m) => s.font_stretch = *m,
            DeclKind::ImageRendering(r) => s.image_rendering = *r,
            DeclKind::MixBlendMode(b) => s.mix_blend_mode = *b,
            DeclKind::BackgroundBlendMode(v) => s.background_blend_mode = v.clone(),
            DeclKind::Isolation(i) => s.isolation = *i,
            DeclKind::WillChange(v) => s.will_change = v.clone(),
            DeclKind::Appearance(a) => s.appearance = *a,
            DeclKind::FontKerning(k) => s.font_kerning = *k,
            DeclKind::FontFeatureSettings(v) => s.font_feature_settings = v.clone(),
            DeclKind::FontVariationSettings(v) => s.font_variation_settings = v.clone(),
            DeclKind::FontLanguageOverride(o) => s.font_language_override = o.clone(),
            DeclKind::TextRendering(r) => s.text_rendering = *r,
            DeclKind::Filter(v) => s.filter = v.clone(),
            DeclKind::BackdropFilter(v) => s.backdrop_filter = v.clone(),
            DeclKind::TextOrientation(t) => s.text_orientation = *t,
            DeclKind::OverscrollBehaviorX(b) => s.overscroll_behavior_x = *b,
            DeclKind::OverscrollBehaviorY(b) => s.overscroll_behavior_y = *b,
            DeclKind::ScrollSnapType(t) => s.scroll_snap_type = *t,
            DeclKind::ScrollSnapAlignBlock(a) => s.scroll_snap_align_block = *a,
            DeclKind::ScrollSnapAlignInline(a) => s.scroll_snap_align_inline = *a,
            DeclKind::ScrollSnapStop(st) => s.scroll_snap_stop = *st,
            DeclKind::ScrollPadding(sd) => s.scroll_padding = *sd,
            DeclKind::ScrollPaddingTop(v) => s.scroll_padding.top = *v,
            DeclKind::ScrollPaddingRight(v) => s.scroll_padding.right = *v,
            DeclKind::ScrollPaddingBottom(v) => s.scroll_padding.bottom = *v,
            DeclKind::ScrollPaddingLeft(v) => s.scroll_padding.left = *v,
            DeclKind::ScrollMargin(sd) => s.scroll_margin = *sd,
            DeclKind::ScrollMarginTop(v) => s.scroll_margin.top = *v,
            DeclKind::ScrollMarginRight(v) => s.scroll_margin.right = *v,
            DeclKind::ScrollMarginBottom(v) => s.scroll_margin.bottom = *v,
            DeclKind::ScrollMarginLeft(v) => s.scroll_margin.left = *v,
            DeclKind::TouchAction(t) => s.touch_action = *t,
            DeclKind::ClipPath(c) => s.clip_path = *c,
            DeclKind::MaskImage(m) => s.mask_image = m.clone(),
            DeclKind::ContentVisibility(v) => s.content_visibility = *v,
            DeclKind::Contain(c) => s.contain = *c,
            DeclKind::ColumnCount(n) => s.column_count = *n,
            DeclKind::ColumnWidth(v) => s.column_width = *v,
            DeclKind::ColumnRuleWidth(v) => s.column_rule_width = *v,
            DeclKind::ColumnRuleColor(c) => s.column_rule_color = *c,
            DeclKind::ColumnRuleStyleActive(on) => s.column_rule_style_active = *on,
            DeclKind::ColumnRuleStylePattern(p) => s.column_rule_style = *p,
            DeclKind::RowRuleWidth(v) => s.row_rule_width = *v,
            DeclKind::RowRuleColor(c) => s.row_rule_color = *c,
            DeclKind::RowRuleStyleActive(on) => s.row_rule_style_active = *on,
            DeclKind::RowRuleStylePattern(p) => s.row_rule_style = *p,
            DeclKind::ColumnFill(f) => s.column_fill = *f,
            DeclKind::ColumnSpan(sp) => s.column_span = *sp,
            DeclKind::BreakInside(b) => s.break_inside = *b,
            DeclKind::TableLayout(t) => s.table_layout = *t,
            DeclKind::BorderCollapse(c) => s.border_collapse = *c,
            DeclKind::BorderSpacing { h, v } => {
                s.border_spacing_h = *h;
                s.border_spacing_v = *v;
            }
            DeclKind::CaptionSide(c) => s.caption_side = *c,
            DeclKind::EmptyCells(e) => s.empty_cells = *e,
            DeclKind::BreakBefore(b) => s.break_before = *b,
            DeclKind::BreakAfter(b) => s.break_after = *b,
            DeclKind::Orphans(n) => s.orphans = *n,
            DeclKind::Widows(n) => s.widows = *n,
            DeclKind::ColorScheme(c) => s.color_scheme = *c,
            DeclKind::ListStylePosition(p) => s.list_style_position = *p,
            DeclKind::ListStyleImage(u) => s.list_style_image = u.clone(),
            DeclKind::CounterSet(v) => s.counter_set = v.clone(),
            DeclKind::Quotes(q) => s.quotes = q.clone(),
            DeclKind::TextUnderlinePosition(p) => s.text_underline_position = *p,
            DeclKind::TextJustify(j) => s.text_justify = *j,
            DeclKind::PrintColorAdjust(a) => s.print_color_adjust = *a,
            DeclKind::ForcedColorAdjust(a) => s.forced_color_adjust = *a,
            DeclKind::LineClamp(n) => s.line_clamp = *n,
            DeclKind::FontVariantCaps(c) => s.font_variant_caps = *c,
            DeclKind::FontVariantNumeric(n) => s.font_variant_numeric = *n,
            DeclKind::FontVariantLigatures(l) => s.font_variant_ligatures = *l,
            DeclKind::FontVariantEastAsian(e) => s.font_variant_east_asian = *e,
            DeclKind::FontVariantPosition(p) => s.font_variant_position = *p,
            DeclKind::TextEmphasisStyle(st) => s.text_emphasis_style = st.clone(),
            DeclKind::TextEmphasisColor(c) => s.text_emphasis_color = *c,
            DeclKind::TextEmphasisPosition(p) => s.text_emphasis_position = *p,
            DeclKind::RubyPosition(r) => s.ruby_position = *r,
            DeclKind::TransformOrigin(o) => s.transform_origin = *o,
            DeclKind::TransformStyle(t) => s.transform_style = *t,
            DeclKind::Perspective(p) => s.perspective = *p,
            DeclKind::PerspectiveOrigin(o) => s.perspective_origin = *o,
            DeclKind::BackfaceVisibility(v) => s.backface_visibility = *v,
            DeclKind::ScrollbarWidth(w) => s.scrollbar_width = *w,
            DeclKind::ScrollbarColor(c) => s.scrollbar_color = *c,
            DeclKind::ScrollbarGutter(g) => s.scrollbar_gutter = *g,
            DeclKind::OverflowAnchor(a) => s.overflow_anchor = *a,
            DeclKind::OverflowClipMargin(m) => s.overflow_clip_margin = *m,
            DeclKind::TextAlignLast(a) => s.text_align_last = *a,
            DeclKind::TextWrap(w) => s.text_wrap = *w,
            DeclKind::LineBreak(lb) => s.line_break = *lb,
            DeclKind::HangingPunctuation(hp) => s.hanging_punctuation = *hp,
            DeclKind::TextDecorationSkipInk(si) => s.text_decoration_skip_ink = *si,
            DeclKind::FontOpticalSizing(o) => s.font_optical_sizing = *o,
            DeclKind::FontSynthesisWeight(b) => s.font_synthesis.weight = *b,
            DeclKind::FontSynthesisStyle(b) => s.font_synthesis.style = *b,
            DeclKind::FontSynthesisSmallCaps(b) => s.font_synthesis.small_caps = *b,
            DeclKind::FontSynthesisAll(fs) => s.font_synthesis = *fs,
            DeclKind::FontSizeAdjust(a) => s.font_size_adjust = *a,
            DeclKind::ImageOrientation(o) => s.image_orientation = *o,
            DeclKind::AnimationTimeline(t) => s.animation_timeline = t.clone(),
            DeclKind::ScrollTimelineName(n) => s.scroll_timeline_name = n.clone(),
            DeclKind::ScrollTimelineAxis(a) => s.scroll_timeline_axis = *a,
            DeclKind::ViewTimelineName(n) => s.view_timeline_name = n.clone(),
            DeclKind::ViewTimelineAxis(a) => s.view_timeline_axis = *a,
            DeclKind::WhiteSpaceCollapse(c) => s.white_space_collapse = *c,
            DeclKind::TextWrapMode(m) => s.text_wrap_mode = *m,
            DeclKind::TextWrapStyle(st) => s.text_wrap_style = *st,
            DeclKind::WrapBefore(v) => s.wrap_before = *v,
            DeclKind::WrapAfter(v) => s.wrap_after = *v,
            DeclKind::WrapInside(v) => s.wrap_inside = *v,
            DeclKind::TextSpacingTrim(t) => s.text_spacing_trim = *t,
            DeclKind::TextBoxTrim(t) => s.text_box_trim = *t,
            DeclKind::MathStyle(m) => s.math_style = *m,
            DeclKind::MathDepth(m) => s.math_depth = *m,
            DeclKind::MathShift(m) => s.math_shift = *m,
            DeclKind::FieldSizing(f) => s.field_sizing = *f,
            DeclKind::Overlay(o) => s.overlay = *o,
            DeclKind::DynamicRangeLimit(d) => s.dynamic_range_limit = *d,
            DeclKind::TextBoxEdge(e) => s.text_box_edge = *e,
            DeclKind::AnchorName(n) => s.anchor_name = n.clone(),
            DeclKind::PositionAnchor(a) => s.position_anchor = a.clone(),
            DeclKind::AnchorScope(sc) => s.anchor_scope = sc.clone(),
            DeclKind::ViewTransitionName(n) => s.view_transition_name = n.clone(),
            DeclKind::ViewTransitionClass(c) => s.view_transition_class = c.clone(),
            DeclKind::FontPalette(p) => s.font_palette = p.clone(),
            DeclKind::FontVariantAlternates(a) => {
                s.font_variant_alternates = a.clone()
            }
            DeclKind::BackgroundAttachment(att) => {
                s.background_attachment =
                    if att.is_empty() { vec![BackgroundAttachment::Scroll] } else { att.clone() };
            }
            DeclKind::CaretShape(c) => s.caret_shape = *c,
            DeclKind::BaselineSource(b) => s.baseline_source = *b,
            DeclKind::AlignmentBaseline(a) => s.alignment_baseline = *a,
            DeclKind::DominantBaseline(d) => s.dominant_baseline = *d,
            DeclKind::PaintOrder(p) => s.paint_order = *p,
            DeclKind::MarkerSide(m) => s.marker_side = *m,
            DeclKind::Fill(p) => s.fill = p.clone(),
            DeclKind::Stroke(p) => s.stroke = p.clone(),
            DeclKind::FillOpacity(v) => s.fill_opacity = *v,
            DeclKind::StrokeOpacity(v) => s.stroke_opacity = *v,
            DeclKind::StrokeWidth(v) => s.stroke_width = *v,
            DeclKind::StrokeLinecap(c) => s.stroke_linecap = *c,
            DeclKind::StrokeLinejoin(j) => s.stroke_linejoin = *j,
            DeclKind::StrokeMiterlimit(m) => s.stroke_miterlimit = *m,
            DeclKind::StrokeDasharray(d) => s.stroke_dasharray = d.clone(),
            DeclKind::StrokeDashoffset(o) => s.stroke_dashoffset = *o,
            DeclKind::FillRule(r) => s.fill_rule = *r,
            DeclKind::ClipRule(r) => s.clip_rule = *r,
            DeclKind::ColorInterpolation(c) => s.color_interpolation = *c,
            DeclKind::ShapeRendering(r) => s.shape_rendering = *r,
            DeclKind::VectorEffect(e) => s.vector_effect = *e,
            DeclKind::FloodColor(c) => s.flood_color = *c,
            DeclKind::FloodOpacity(v) => s.flood_opacity = *v,
            DeclKind::LightingColor(c) => s.lighting_color = *c,
            DeclKind::StopColor(c) => s.stop_color = *c,
            DeclKind::StopOpacity(v) => s.stop_opacity = *v,
            DeclKind::TextAnchor(a) => s.text_anchor = *a,
            DeclKind::ColorRendering(r) => s.color_rendering = *r,
            DeclKind::ColorInterpolationFilters(c) => {
                s.color_interpolation_filters = *c
            }
            DeclKind::GlyphOrientationVertical(g) => {
                s.glyph_orientation_vertical = *g
            }
            DeclKind::TransformBox(b) => s.transform_box = *b,
            DeclKind::MarkerStart(r) => s.marker_start = r.clone(),
            DeclKind::MarkerMid(r) => s.marker_mid = r.clone(),
            DeclKind::MarkerEnd(r) => s.marker_end = r.clone(),
            DeclKind::MaskType(t) => s.mask_type = *t,
            DeclKind::MaskMode(m) => s.mask_mode = *m,
            DeclKind::MaskClip(c) => s.mask_clip = *c,
            DeclKind::MaskComposite(c) => s.mask_composite = *c,
            DeclKind::MaskOrigin(o) => s.mask_origin = *o,
            DeclKind::MaskRepeat(r) => s.mask_repeat = *r,
            DeclKind::MaskPosition(p) => s.mask_position = *p,
            DeclKind::MaskSize(sz) => s.mask_size = *sz,
            DeclKind::ContainerName(v) => s.container_name = v.clone(),
            DeclKind::ContainerType(t) => s.container_type = *t,
            DeclKind::OffsetPath(p) => s.offset_path = p.clone(),
            DeclKind::OffsetDistance(d) => s.offset_distance = *d,
            DeclKind::HyphenateCharacter(c) => s.hyphenate_character = c.clone(),
            DeclKind::HyphenateLimitChars(h) => s.hyphenate_limit_chars = *h,
            DeclKind::TextSizeAdjust(t) => s.text_size_adjust = *t,
            DeclKind::LineHeightStep(v) => s.line_height_step = *v,
            DeclKind::FontVariantEmoji(e) => s.font_variant_emoji = *e,
            DeclKind::ContainIntrinsicWidth(c) => s.contain_intrinsic_width = *c,
            DeclKind::ContainIntrinsicHeight(c) => s.contain_intrinsic_height = *c,
            DeclKind::BackgroundPositionX(v) => s.background_position.x = *v,
            DeclKind::BackgroundPositionY(v) => s.background_position.y = *v,
            DeclKind::GridAutoFlow(f) => s.grid_auto_flow = *f,
            DeclKind::GridAutoColumns(t) => s.grid_auto_columns = t.clone(),
            DeclKind::GridAutoRows(t) => s.grid_auto_rows = t.clone(),
            DeclKind::ShapeOutside(o) => s.shape_outside = o.clone(),
            DeclKind::ShapeMargin(m) => s.shape_margin = *m,
            DeclKind::ShapeImageThreshold(t) => s.shape_image_threshold = *t,
            DeclKind::TextCombineUpright(t) => s.text_combine_upright = *t,
            DeclKind::RubyAlign(r) => s.ruby_align = *r,
            DeclKind::OffsetRotate(r) => s.offset_rotate = *r,
            DeclKind::OffsetAnchor(a) => s.offset_anchor = *a,
            DeclKind::OffsetPosition(p) => s.offset_position = *p,
            DeclKind::ObjectViewBox(o) => s.object_view_box = o.clone(),
            DeclKind::RubyOverhang(r) => s.ruby_overhang = *r,
            DeclKind::BlockStepSize(b) => s.block_step_size = *b,
            DeclKind::BlockStepInsert(b) => s.block_step_insert = *b,
            DeclKind::BlockStepAlign(b) => s.block_step_align = *b,
            DeclKind::BlockStepRound(b) => s.block_step_round = *b,
            DeclKind::PositionVisibility(p) => s.position_visibility = *p,
            DeclKind::PositionTryOrder(o) => s.position_try_order = *o,
            DeclKind::PositionTryFallbacks(v) => s.position_try_fallbacks = v.clone(),
            DeclKind::PositionArea(a) => s.position_area = a.clone(),
            DeclKind::AnimationRangeStart(r) => s.animation_range_start = r.clone(),
            DeclKind::AnimationRangeEnd(r) => s.animation_range_end = r.clone(),
            DeclKind::TransitionBehavior(b) => s.transition_behavior = *b,
            DeclKind::InterpolateSize(i) => s.interpolate_size = *i,
            DeclKind::ViewTimelineInset(a, b) => {
                s.view_timeline_inset_start = *a;
                s.view_timeline_inset_end = *b;
            }
            DeclKind::FontSynthesisPosition(b) => s.font_synthesis.position = *b,
            DeclKind::Interactivity(i) => s.interactivity = *i,
            DeclKind::Cx(v) => s.cx = *v,
            DeclKind::Cy(v) => s.cy = *v,
            DeclKind::X(v) => s.x = *v,
            DeclKind::Y(v) => s.y = *v,
            DeclKind::BaselineShift(v) => s.baseline_shift = *v,
            DeclKind::SolidColor(v) => s.solid_color = *v,
            DeclKind::SolidOpacity(v) => s.solid_opacity = *v,
            DeclKind::R(v) => s.r = *v,
            DeclKind::Rx(v) => s.rx = *v,
            DeclKind::Ry(v) => s.ry = *v,
            DeclKind::Order(v) => s.order = *v,
            DeclKind::PathLength(v) => s.path_length = *v,
            DeclKind::AnimationComposition(v) => s.animation_composition = *v,
            DeclKind::TimelineScope(v) => s.timeline_scope = v.clone(),
            DeclKind::ReadingOrder(v) => s.reading_order = *v,
            DeclKind::ReadingFlow(v) => s.reading_flow = *v,
            DeclKind::ImageResolution(v) => s.image_resolution = *v,
            DeclKind::BookmarkLevel(v) => s.bookmark_level = *v,
            DeclKind::BookmarkState(v) => s.bookmark_state = *v,
            DeclKind::BookmarkLabel(v) => s.bookmark_label = v.clone(),
            DeclKind::StringSet(v) => s.string_set = v.clone(),
            DeclKind::FootnoteDisplay(v) => s.footnote_display = *v,
            DeclKind::FootnotePolicy(v) => s.footnote_policy = *v,
            DeclKind::MarkerKnockoutLeft(v) => s.marker_knockout_left = *v,
            DeclKind::MarkerKnockoutRight(v) => s.marker_knockout_right = *v,
            DeclKind::LeadingTrim(v) => s.leading_trim = *v,
            DeclKind::InitialLetterAlign(v) => s.initial_letter_align = *v,
            DeclKind::TextAutospace(v) => s.text_autospace = v.clone(),
            DeclKind::WhiteSpaceTrim(v) => s.white_space_trim = v.clone(),
            DeclKind::ViewTransitionGroup(v) => s.view_transition_group = v.clone(),
            DeclKind::InsetArea(v) => s.inset_area = v.clone(),
            DeclKind::ViewTransitionImagePair(v) => s.view_transition_image_pair = v.clone(),
            DeclKind::AnimationTrigger(v) => s.animation_trigger = v.clone(),
            DeclKind::BorderImageSource(v) => s.border_image_source = v.clone(),
            DeclKind::BorderImageRepeat(h, v) => {
                s.border_image_repeat_h = *h;
                s.border_image_repeat_v = *v;
            }
            DeclKind::BorderImageSlice(v) => s.border_image_slice = v.clone(),
            DeclKind::BorderImageWidth(v) => s.border_image_width = v.clone(),
            DeclKind::BorderImageOutset(v) => s.border_image_outset = v.clone(),
            DeclKind::BorderImage(v) => s.border_image = v.clone(),
            DeclKind::GridTemplateAreas(v) => s.grid_template_areas = v.clone(),
            DeclKind::GridRowStart(v) => s.grid_row_start = v.clone(),
            DeclKind::GridRowEnd(v) => s.grid_row_end = v.clone(),
            DeclKind::GridColumnStart(v) => s.grid_column_start = v.clone(),
            DeclKind::GridColumnEnd(v) => s.grid_column_end = v.clone(),
            DeclKind::TextEmphasisSkip(v) => s.text_emphasis_skip = *v,
            DeclKind::AnimationName(v) => match v {
                None => s.animation = None,
                Some(name) => {
                    let b = s.animation.get_or_insert_with(AnimationBinding::default);
                    b.name = name.clone();
                }
            },
            DeclKind::AnimationDuration(v) => {
                let b = s.animation.get_or_insert_with(AnimationBinding::default);
                b.duration_s = *v;
            }
            DeclKind::AnimationTimingFunction(v) => {
                let b = s.animation.get_or_insert_with(AnimationBinding::default);
                b.timing = *v;
            }
            DeclKind::AnimationIterationCount(v) => {
                let b = s.animation.get_or_insert_with(AnimationBinding::default);
                b.iterations = *v;
            }
            DeclKind::AnimationFillMode(v) => {
                let b = s.animation.get_or_insert_with(AnimationBinding::default);
                b.fill_mode = *v;
            }
            DeclKind::AnimationDirection(v) => {
                let b = s.animation.get_or_insert_with(AnimationBinding::default);
                b.direction = *v;
            }
            DeclKind::AnimationPlayState(v) => {
                let b = s.animation.get_or_insert_with(AnimationBinding::default);
                b.play_state = *v;
            }
            DeclKind::AnimationDelay(v) => {
                let b = s.animation.get_or_insert_with(AnimationBinding::default);
                b.delay_s = *v;
            }
            DeclKind::TransitionPropertyFirst(p) => match p {
                None => s.transitions.clear(),
                Some(name) => transition_first(&mut s.transitions).property = name.clone(),
            },
            DeclKind::TransitionDurationFirst(v) => {
                transition_first(&mut s.transitions).duration_s = *v;
            }
            DeclKind::TransitionTimingFirst(v) => {
                transition_first(&mut s.transitions).timing = *v;
            }
            DeclKind::TransitionDelayFirst(v) => {
                transition_first(&mut s.transitions).delay_s = *v;
            }
            DeclKind::Translate(t) => s.translate = *t,
            DeclKind::Rotate(t) => s.rotate = *t,
            DeclKind::Scale(t) => s.scale = *t,
            DeclKind::Float(v) => s.float = *v,
            DeclKind::Clear(v) => s.clear = *v,
            DeclKind::Page(v) => s.page = v.clone(),
            DeclKind::Clip(v) => s.clip = *v,
            DeclKind::D(v) => s.d = v.clone(),
            DeclKind::MasonryAutoFlow(v) => s.masonry_auto_flow = *v,
            DeclKind::JustifyTracks(v) => s.justify_tracks = v.clone(),
            DeclKind::AlignTracks(v) => s.align_tracks = v.clone(),
            DeclKind::GridTemplateColumnsSubgrid(v) => s.grid_template_columns_subgrid = *v,
            DeclKind::GridTemplateRowsSubgrid(v) => s.grid_template_rows_subgrid = *v,
            DeclKind::FloatDefer(v) => s.float_defer = *v,
            DeclKind::FloatReference(v) => s.float_reference = *v,
            DeclKind::FloatOffset(v) => s.float_offset = *v,
            DeclKind::BoxDecorationBreak(v) => s.box_decoration_break = *v,
            DeclKind::LineSnap(v) => s.line_snap = *v,
            DeclKind::LineGrid(v) => s.line_grid = *v,
            DeclKind::InitialLetter(v) => s.initial_letter = v.clone(),
            DeclKind::Highlight(v) => s.highlight = v.clone(),
            DeclKind::RubyMerge(v) => s.ruby_merge = *v,
            DeclKind::TextSpacing(v) => s.text_spacing = v.clone(),
            DeclKind::SpeakAs(v) => s.speak_as = *v,
            DeclKind::VoiceBalance(v) => s.voice_balance = *v,
            DeclKind::VoicePitch(v) => s.voice_pitch = v.clone(),
            DeclKind::VoiceRate(v) => s.voice_rate = v.clone(),
            DeclKind::VoiceVolume(v) => s.voice_volume = v.clone(),
            DeclKind::VoiceFamily(v) => s.voice_family = v.clone(),
            DeclKind::VoiceStress(v) => s.voice_stress = v.clone(),
            DeclKind::VoiceDuration(v) => s.voice_duration = v.clone(),
            DeclKind::PauseBefore(v) => s.pause_before = v.clone(),
            DeclKind::PauseAfter(v) => s.pause_after = v.clone(),
            DeclKind::RestBefore(v) => s.rest_before = v.clone(),
            DeclKind::RestAfter(v) => s.rest_after = v.clone(),
            DeclKind::CueFadeDuration(v) => s.cue_fade_duration = *v,
            DeclKind::CueBefore(v) => s.cue_before = v.clone(),
            DeclKind::CueAfter(v) => s.cue_after = v.clone(),
            DeclKind::Cue(v) => s.cue = v.clone(),
            DeclKind::NavigationUp(v) => s.navigation_up = v.clone(),
            DeclKind::GlyphOrientationHorizontal(v) => s.glyph_orientation_horizontal = *v,
            DeclKind::NavigationDown(v) => s.navigation_down = v.clone(),
            DeclKind::NavigationLeft(v) => s.navigation_left = v.clone(),
            DeclKind::NavigationRight(v) => s.navigation_right = v.clone(),
            DeclKind::CounterIncrementStyle(v) => s.counter_increment_style = v.clone(),
            DeclKind::OverflowClipBox(v) => s.overflow_clip_box = *v,
            DeclKind::MaskBorderSource(v) => s.mask_border_source = v.clone(),
            DeclKind::MaskBorderSlice(v) => s.mask_border_slice = v.clone(),
            DeclKind::MaskBorderWidth(v) => s.mask_border_width = v.clone(),
            DeclKind::MaskBorderOutset(v) => s.mask_border_outset = v.clone(),
            DeclKind::MaskBorderRepeat(v) => s.mask_border_repeat = *v,
            DeclKind::MaskBorderMode(v) => s.mask_border_mode = *v,
            DeclKind::MaskBorder(v) => s.mask_border = v.clone(),
            DeclKind::CaretAnimation(v) => s.caret_animation = *v,
            DeclKind::ScrollMarkerGroup(v) => s.scroll_marker_group = *v,
            DeclKind::ScrollInitialTarget(v) => s.scroll_initial_target = *v,
            DeclKind::CornerShape(v) => s.corner_shape = v.clone(),
            DeclKind::HyphenateLimitLines(v) => s.hyphenate_limit_lines = *v,
            DeclKind::HyphenateLimitLast(v) => s.hyphenate_limit_last = *v,
            DeclKind::HyphenateLimitZone(v) => s.hyphenate_limit_zone = v.clone(),
            DeclKind::InterestTarget(v) => s.interest_target = v.clone(),
            DeclKind::ScrollStart(v) => s.scroll_start = v.clone(),
            DeclKind::ScrollStartBlock(v) => s.scroll_start_block = v.clone(),
            DeclKind::ScrollStartInline(v) => s.scroll_start_inline = v.clone(),
            DeclKind::ScrollStartTarget(v) => s.scroll_start_target = v.clone(),
            DeclKind::ScrollStartTargetBlock(v) => s.scroll_start_target_block = v.clone(),
            DeclKind::ScrollStartTargetInline(v) => s.scroll_start_target_inline = v.clone(),
            DeclKind::InterestDelayStart(v) => s.interest_delay_start = v.clone(),
            DeclKind::InterestDelayEnd(v) => s.interest_delay_end = v.clone(),
            DeclKind::Azimuth(v) => s.azimuth = v.clone(),
            DeclKind::Elevation(v) => s.elevation = v.clone(),
            DeclKind::Richness(v) => s.richness = *v,
            DeclKind::SpeakHeader(v) => s.speak_header = v.clone(),
            DeclKind::PitchRange(v) => s.pitch_range = *v,
            DeclKind::MarginTrim(v) => s.margin_trim = v.clone(),
            DeclKind::MarginBreak(v) => s.margin_break = v.clone(),
            DeclKind::InputSecurity(v) => s.input_security = v.clone(),
            DeclKind::BorderBoundary(v) => s.border_boundary = v.clone(),
            DeclKind::ShapeInside(v) => s.shape_inside = v.clone(),
            DeclKind::SpeakPunctuation(v) => s.speak_punctuation = v.clone(),
            DeclKind::SpeakNumeral(v) => s.speak_numeral = v.clone(),
            DeclKind::Stress(v) => s.stress = *v,
            DeclKind::Pitch(v) => s.pitch = v.clone(),
            DeclKind::SpeechRate(v) => s.speech_rate = v.clone(),
            DeclKind::Volume(v) => s.volume = v.clone(),
            DeclKind::Speak(v) => s.speak = *v,
            DeclKind::PlayDuring(v) => s.play_during = v.clone(),
            DeclKind::TextDecorationSkip(v) => s.text_decoration_skip = v.clone(),
            DeclKind::TextDecorationSkipBox(v) => s.text_decoration_skip_box = *v,
            DeclKind::TextDecorationSkipSelf(v) => s.text_decoration_skip_self = v.clone(),
            DeclKind::TextDecorationSkipSpaces(v) => s.text_decoration_skip_spaces = v.clone(),
            DeclKind::TextDecorationSkipInset(v) => s.text_decoration_skip_inset = *v,
            DeclKind::WebkitTextStrokeWidth(v) => s.webkit_text_stroke_width = *v,
            DeclKind::WebkitTextStrokeColor(v) => s.webkit_text_stroke_color = v.clone(),
            DeclKind::WebkitTextFillColor(v) => s.webkit_text_fill_color = v.clone(),
            DeclKind::FontSmooth(v) => s.font_smooth = v.clone(),
            DeclKind::TextGroupAlign(v) => s.text_group_align = *v,
            DeclKind::Continue(v) => s.continue_ = *v,
            DeclKind::BlockEllipsis(v) => s.block_ellipsis = v.clone(),
            DeclKind::MaxLines(v) => s.max_lines = *v,
            DeclKind::RegionFragment(v) => s.region_fragment = *v,
            DeclKind::OverflowStyle(v) => s.overflow_style = v.clone(),
            DeclKind::MarqueeStyle(v) => s.marquee_style = *v,
            DeclKind::MarqueeDirection(v) => s.marquee_direction = *v,
            DeclKind::MarqueeSpeed(v) => s.marquee_speed = *v,
            DeclKind::MarqueeLoop(v) => s.marquee_loop = *v,
            DeclKind::MarqueeIncrement(v) => s.marquee_increment = v.clone(),
            DeclKind::NavIndex(v) => s.nav_index = v.clone(),
            DeclKind::NavUp(v) => s.nav_up = v.clone(),
            DeclKind::NavDown(v) => s.nav_down = v.clone(),
            DeclKind::NavLeft(v) => s.nav_left = v.clone(),
            DeclKind::NavRight(v) => s.nav_right = v.clone(),
            DeclKind::WebkitBoxOrient(v) => s.webkit_box_orient = v.clone(),
            DeclKind::WebkitBoxDirection(v) => s.webkit_box_direction = v.clone(),
            DeclKind::WebkitBoxAlign(v) => s.webkit_box_align = v.clone(),
            DeclKind::WebkitBoxPack(v) => s.webkit_box_pack = v.clone(),
            DeclKind::WebkitBoxFlex(v) => s.webkit_box_flex = *v,
            DeclKind::WebkitBoxOrdinalGroup(v) => s.webkit_box_ordinal_group = *v,
            DeclKind::WebkitFontSmoothing(v) => s.webkit_font_smoothing = v.clone(),
            DeclKind::MozOsxFontSmoothing(v) => s.moz_osx_font_smoothing = v.clone(),
            DeclKind::WebkitTapHighlightColor(v) => s.webkit_tap_highlight_color = v.clone(),
            DeclKind::Zoom(v) => s.zoom = v.clone(),
            DeclKind::ColumnBreakBefore(v) => s.column_break_before = v.clone(),
            DeclKind::ColumnBreakAfter(v) => s.column_break_after = v.clone(),
            DeclKind::ColumnBreakInside(v) => s.column_break_inside = v.clone(),
            DeclKind::UserModify(v) => s.user_modify = v.clone(),
            DeclKind::WebkitTouchCallout(v) => s.webkit_touch_callout = v.clone(),
            DeclKind::WebkitUserDrag(v) => s.webkit_user_drag = v.clone(),
            DeclKind::WebkitRtlOrdering(v) => s.webkit_rtl_ordering = v.clone(),
            DeclKind::WebkitTextSecurity(v) => s.webkit_text_security = v.clone(),
            DeclKind::WebkitNbspMode(v) => s.webkit_nbsp_mode = v.clone(),
            DeclKind::WebkitLocale(v) => s.webkit_locale = v.clone(),
            DeclKind::WebkitColumnAxis(v) => s.webkit_column_axis = v.clone(),
            DeclKind::WebkitColumnProgression(v) => s.webkit_column_progression = v.clone(),
            DeclKind::WebkitAppRegion(v) => s.webkit_app_region = v.clone(),
            DeclKind::WebkitHighlight(v) => s.webkit_highlight = v.clone(),
            DeclKind::WebkitBoxReflect(v) => s.webkit_box_reflect = v.clone(),
            DeclKind::WebkitMaskComposite(v) => s.webkit_mask_composite = v.clone(),
            DeclKind::WebkitMaskPositionX(v) => s.webkit_mask_position_x = v.clone(),
            DeclKind::WebkitMaskPositionY(v) => s.webkit_mask_position_y = v.clone(),
            DeclKind::WebkitMaskRepeatX(v) => s.webkit_mask_repeat_x = v.clone(),
            DeclKind::WebkitMaskRepeatY(v) => s.webkit_mask_repeat_y = v.clone(),
            DeclKind::WebkitMarginStart(v) => s.webkit_margin_start = v.clone(),
            DeclKind::WebkitMarginEnd(v) => s.webkit_margin_end = v.clone(),
            DeclKind::WebkitMarginBefore(v) => s.webkit_margin_before = v.clone(),
            DeclKind::WebkitMarginAfter(v) => s.webkit_margin_after = v.clone(),
            DeclKind::WebkitPaddingStart(v) => s.webkit_padding_start = v.clone(),
            DeclKind::WebkitPaddingEnd(v) => s.webkit_padding_end = v.clone(),
            DeclKind::WebkitPaddingBefore(v) => s.webkit_padding_before = v.clone(),
            DeclKind::WebkitPaddingAfter(v) => s.webkit_padding_after = v.clone(),
            DeclKind::WebkitLogicalWidth(v) => s.webkit_logical_width = v.clone(),
            DeclKind::WebkitLogicalHeight(v) => s.webkit_logical_height = v.clone(),
            DeclKind::WebkitTransformOriginX(v) => s.webkit_transform_origin_x = v.clone(),
            DeclKind::WebkitTransformOriginY(v) => s.webkit_transform_origin_y = v.clone(),
            DeclKind::WebkitTransformOriginZ(v) => s.webkit_transform_origin_z = v.clone(),
            DeclKind::WebkitPerspectiveOriginX(v) => s.webkit_perspective_origin_x = v.clone(),
            DeclKind::WebkitPerspectiveOriginY(v) => s.webkit_perspective_origin_y = v.clone(),
            DeclKind::WebkitMinLogicalWidth(v) => s.webkit_min_logical_width = v.clone(),
            DeclKind::WebkitMaxLogicalWidth(v) => s.webkit_max_logical_width = v.clone(),
            DeclKind::WebkitMinLogicalHeight(v) => s.webkit_min_logical_height = v.clone(),
            DeclKind::WebkitMaxLogicalHeight(v) => s.webkit_max_logical_height = v.clone(),
            DeclKind::WebkitBackgroundComposite(v) => s.webkit_background_composite = v.clone(),
            DeclKind::WebkitBorderBefore(v) => s.webkit_border_before = v.clone(),
            DeclKind::WebkitBorderAfter(v) => s.webkit_border_after = v.clone(),
            DeclKind::WebkitBorderStart(v) => s.webkit_border_start = v.clone(),
            DeclKind::WebkitBorderEnd(v) => s.webkit_border_end = v.clone(),
            DeclKind::WebkitBorderHorizontalSpacing(v) => s.webkit_border_horizontal_spacing = v.clone(),
            DeclKind::WebkitFlowInto(v) => s.webkit_flow_into = v.clone(),
            DeclKind::WebkitFlowFrom(v) => s.webkit_flow_from = v.clone(),
            DeclKind::WebkitRegionBreakBefore(v) => s.webkit_region_break_before = v.clone(),
            DeclKind::WebkitRegionBreakAfter(v) => s.webkit_region_break_after = v.clone(),
            DeclKind::WebkitRegionBreakInside(v) => s.webkit_region_break_inside = v.clone(),
            DeclKind::WebkitBorderBeforeColor(v) => s.webkit_border_before_color = v.clone(),
            DeclKind::WebkitBorderBeforeStyle(v) => s.webkit_border_before_style = v.clone(),
            DeclKind::WebkitBorderBeforeWidth(v) => s.webkit_border_before_width = v.clone(),
            DeclKind::WebkitBorderAfterColor(v) => s.webkit_border_after_color = v.clone(),
            DeclKind::WebkitBorderAfterStyle(v) => s.webkit_border_after_style = v.clone(),
            DeclKind::WebkitBorderAfterWidth(v) => s.webkit_border_after_width = v.clone(),
            DeclKind::WebkitBorderStartColor(v) => s.webkit_border_start_color = v.clone(),
            DeclKind::WebkitBorderStartStyle(v) => s.webkit_border_start_style = v.clone(),
            DeclKind::WebkitBorderStartWidth(v) => s.webkit_border_start_width = v.clone(),
            DeclKind::WebkitBorderEndColor(v) => s.webkit_border_end_color = v.clone(),
            DeclKind::WebkitBorderEndStyle(v) => s.webkit_border_end_style = v.clone(),
            DeclKind::WebkitBorderEndWidth(v) => s.webkit_border_end_width = v.clone(),
            DeclKind::WebkitMarginTopCollapse(v) => s.webkit_margin_top_collapse = v.clone(),
            DeclKind::WebkitMarginBottomCollapse(v) => s.webkit_margin_bottom_collapse = v.clone(),
            DeclKind::WebkitMarginCollapse(v) => s.webkit_margin_collapse = v.clone(),
            DeclKind::WebkitBorderVerticalSpacing(v) => s.webkit_border_vertical_spacing = v.clone(),
            DeclKind::WebkitMaskSourceType(v) => s.webkit_mask_source_type = v.clone(),
            DeclKind::WebkitMarqueeDirection(v) => s.webkit_marquee_direction = v.clone(),
            DeclKind::WebkitMarqueeIncrement(v) => s.webkit_marquee_increment = v.clone(),
            DeclKind::WebkitMarqueeRepetition(v) => s.webkit_marquee_repetition = v.clone(),
            DeclKind::WebkitMarqueeSpeed(v) => s.webkit_marquee_speed = v.clone(),
            DeclKind::WebkitMarqueeStyle(v) => s.webkit_marquee_style = v.clone(),
            DeclKind::WebkitOverflowScrolling(v) => s.webkit_overflow_scrolling = v.clone(),
            DeclKind::WebkitLineGrid(v) => s.webkit_line_grid = v.clone(),
            DeclKind::WebkitCursorVisibility(v) => s.webkit_cursor_visibility = v.clone(),
            DeclKind::WebkitBorderFit(v) => s.webkit_border_fit = v.clone(),
            DeclKind::WebkitColorCorrection(v) => s.webkit_color_correction = v.clone(),
            DeclKind::TextIndent(v) => s.text_indent = *v,
            DeclKind::WordSpacing(v) => s.word_spacing = *v,
            DeclKind::LetterSpacing(v) => s.letter_spacing = *v,
            DeclKind::TextShadows(shadows) => s.text_shadows = shadows.clone(),
            DeclKind::Transforms(tr) => s.transforms = tr.clone(),
            DeclKind::GridTemplateColumns(t) => {
                s.grid_template_columns = t.clone();
                s.grid_template_columns_subgrid = false;
            }
            DeclKind::GridTemplateRows(t) => {
                s.grid_template_rows = t.clone();
                s.grid_template_rows_subgrid = false;
            }
            DeclKind::Animation(a) => s.animation = a.clone(),
            DeclKind::Transitions(t) => s.transitions = t.clone(),
            // No resolvemos acá: el `color` final del elemento puede no
            // estar aplicado todavía (otra regla de la cascada). Acumulamos
            // el target y `compute_internal` lo resuelve al cierre.
            DeclKind::CurrentColor(target) => s.current_color.push(*target),
            // Fase 7.966-7.985 — plumb opaco.
            DeclKind::SpatialNavigationAction(v) => s.spatial_navigation_action = v.clone(),
            DeclKind::SpatialNavigationContain(v) => s.spatial_navigation_contain = v.clone(),
            DeclKind::SpatialNavigationFunction(v) => s.spatial_navigation_function = v.clone(),
            DeclKind::WrapFlow(v) => s.wrap_flow = v.clone(),
            DeclKind::WrapThrough(v) => s.wrap_through = v.clone(),
            DeclKind::FlowInto(v) => s.flow_into = v.clone(),
            DeclKind::FlowFrom(v) => s.flow_from = v.clone(),
            DeclKind::MarkBefore(v) => s.mark_before = v.clone(),
            DeclKind::MarkAfter(v) => s.mark_after = v.clone(),
            DeclKind::TextAlignAll(v) => s.text_align_all = v.clone(),
            DeclKind::MinZoom(v) => s.min_zoom = v.clone(),
            DeclKind::MaxZoom(v) => s.max_zoom = v.clone(),
            DeclKind::UserZoom(v) => s.user_zoom = v.clone(),
            DeclKind::ViewportFit(v) => s.viewport_fit = v.clone(),
            DeclKind::ImeMode(v) => s.ime_mode = v.clone(),
            DeclKind::Kerning(v) => s.kerning = v.clone(),
            DeclKind::EnableBackground(v) => s.enable_background = v.clone(),
            DeclKind::ColorProfile(v) => s.color_profile = v.clone(),
            DeclKind::VoiceRange(v) => s.voice_range = v.clone(),
            DeclKind::TextSecurity(v) => s.text_security = v.clone(),
            // Fase 7.986-7.1005 — plumb opaco.
            DeclKind::ShapePadding(v) => s.shape_padding = v.clone(),
            DeclKind::LineFitEdge(v) => s.line_fit_edge = v.clone(),
            DeclKind::InlineSizing(v) => s.inline_sizing = v.clone(),
            DeclKind::BoxSnap(v) => s.box_snap = v.clone(),
            DeclKind::CopyInto(v) => s.copy_into = v.clone(),
            DeclKind::LineStacking(v) => s.line_stacking = v.clone(),
            DeclKind::LineStackingRuby(v) => s.line_stacking_ruby = v.clone(),
            DeclKind::LineStackingShift(v) => s.line_stacking_shift = v.clone(),
            DeclKind::LineStackingStrategy(v) => s.line_stacking_strategy = v.clone(),
            DeclKind::InlineBoxAlign(v) => s.inline_box_align = v.clone(),
            DeclKind::AlignmentAdjust(v) => s.alignment_adjust = v.clone(),
            DeclKind::TextHeight(v) => s.text_height = v.clone(),
            DeclKind::DropInitialSize(v) => s.drop_initial_size = v.clone(),
            DeclKind::DropInitialValue(v) => s.drop_initial_value = v.clone(),
            DeclKind::DropInitialBeforeAlign(v) => s.drop_initial_before_align = v.clone(),
            DeclKind::DropInitialAfterAlign(v) => s.drop_initial_after_align = v.clone(),
            DeclKind::DropInitialBeforeAdjust(v) => s.drop_initial_before_adjust = v.clone(),
            DeclKind::DropInitialAfterAdjust(v) => s.drop_initial_after_adjust = v.clone(),
            DeclKind::BlockProgression(v) => s.block_progression = v.clone(),
            DeclKind::SnapHeight(v) => s.snap_height = v.clone(),
            // Fase 7.1031-7.1034 — CSS Scroll Snap v0, plumb opaco.
            DeclKind::ScrollSnapPointsX(v) => s.scroll_snap_points_x = v.clone(),
            DeclKind::ScrollSnapPointsY(v) => s.scroll_snap_points_y = v.clone(),
            DeclKind::ScrollSnapDestination(v) => s.scroll_snap_destination = v.clone(),
            DeclKind::ScrollSnapCoordinate(v) => s.scroll_snap_coordinate = v.clone(),
            // Fase 7.1035-7.1042 — Gecko -moz-, plumb opaco.
            DeclKind::MozOrient(v) => s.moz_orient = v.clone(),
            DeclKind::MozUserFocus(v) => s.moz_user_focus = v.clone(),
            DeclKind::MozUserInput(v) => s.moz_user_input = v.clone(),
            DeclKind::MozWindowDragging(v) => s.moz_window_dragging = v.clone(),
            DeclKind::MozFloatEdge(v) => s.moz_float_edge = v.clone(),
            DeclKind::MozForceBrokenImageIcon(v) => s.moz_force_broken_image_icon = v.clone(),
            DeclKind::MozImageRegion(v) => s.moz_image_region = v.clone(),
            DeclKind::MozBinding(v) => s.moz_binding = v.clone(),
            // Fase 7.1043-7.1047 — -moz-outline-radius, plumb opaco.
            DeclKind::MozOutlineRadius(v) => s.moz_outline_radius = v.clone(),
            DeclKind::MozOutlineRadiusTopleft(v) => s.moz_outline_radius_topleft = v.clone(),
            DeclKind::MozOutlineRadiusTopright(v) => s.moz_outline_radius_topright = v.clone(),
            DeclKind::MozOutlineRadiusBottomleft(v) => s.moz_outline_radius_bottomleft = v.clone(),
            DeclKind::MozOutlineRadiusBottomright(v) => s.moz_outline_radius_bottomright = v.clone(),
            // Fase 7.1048-7.1051 — SVG/masking/scroll-snap-type v0, plumb opaco.
            DeclKind::BufferedRendering(v) => s.buffered_rendering = v.clone(),
            DeclKind::MaskSourceType(v) => s.mask_source_type = v.clone(),
            DeclKind::ScrollSnapTypeX(v) => s.scroll_snap_type_x = v.clone(),
            DeclKind::ScrollSnapTypeY(v) => s.scroll_snap_type_y = v.clone(),
            // Fase 7.1052-7.1057 — IE -ms- legacy, plumb opaco.
            DeclKind::MsOverflowStyle(v) => s.ms_overflow_style = v.clone(),
            DeclKind::MsScrollChaining(v) => s.ms_scroll_chaining = v.clone(),
            DeclKind::MsContentZooming(v) => s.ms_content_zooming = v.clone(),
            DeclKind::MsScrollRails(v) => s.ms_scroll_rails = v.clone(),
            DeclKind::MsFlexAlign(v) => s.ms_flex_align = v.clone(),
            DeclKind::MsFlexPack(v) => s.ms_flex_pack = v.clone(),
            // Fase 7.1058-7.1062 — Gecko -moz- misc, plumb opaco.
            DeclKind::MozContextProperties(v) => s.moz_context_properties = v.clone(),
            DeclKind::MozStackSizing(v) => s.moz_stack_sizing = v.clone(),
            DeclKind::MozTextBlink(v) => s.moz_text_blink = v.clone(),
            DeclKind::MozDefaultAppearance(v) => s.moz_default_appearance = v.clone(),
            DeclKind::MozBoxFlexgroup(v) => s.moz_box_flexgroup = v.clone(),
            // Fase 7.1063-7.1072 — CSS Fill and Stroke 3 stroke-*, plumb opaco.
            DeclKind::StrokeAlign(v) => s.stroke_align = v.clone(),
            DeclKind::StrokeBreak(v) => s.stroke_break = v.clone(),
            DeclKind::StrokeColorCss(v) => s.stroke_color_css = v.clone(),
            DeclKind::StrokeImage(v) => s.stroke_image = v.clone(),
            DeclKind::StrokeOrigin(v) => s.stroke_origin = v.clone(),
            DeclKind::StrokePosition(v) => s.stroke_position = v.clone(),
            DeclKind::StrokeRepeat(v) => s.stroke_repeat = v.clone(),
            DeclKind::StrokeSize(v) => s.stroke_size = v.clone(),
            DeclKind::StrokeDashCorner(v) => s.stroke_dash_corner = v.clone(),
            DeclKind::StrokeDashJustify(v) => s.stroke_dash_justify = v.clone(),
            // Fase 7.1073-7.1079 — CSS Fill and Stroke 3 fill-*, plumb opaco.
            DeclKind::FillBreak(v) => s.fill_break = v.clone(),
            DeclKind::FillColorCss(v) => s.fill_color_css = v.clone(),
            DeclKind::FillImage(v) => s.fill_image = v.clone(),
            DeclKind::FillOrigin(v) => s.fill_origin = v.clone(),
            DeclKind::FillPosition(v) => s.fill_position = v.clone(),
            DeclKind::FillSize(v) => s.fill_size = v.clone(),
            DeclKind::FillRepeat(v) => s.fill_repeat = v.clone(),
            // Fase 7.1080-7.1087 — animation-trigger-* longhands, plumb opaco.
            DeclKind::AnimationTriggerBehavior(v) => s.animation_trigger_behavior = v.clone(),
            DeclKind::AnimationTriggerTimeline(v) => s.animation_trigger_timeline = v.clone(),
            DeclKind::AnimationTriggerRange(v) => s.animation_trigger_range = v.clone(),
            DeclKind::AnimationTriggerRangeStart(v) => s.animation_trigger_range_start = v.clone(),
            DeclKind::AnimationTriggerRangeEnd(v) => s.animation_trigger_range_end = v.clone(),
            DeclKind::AnimationTriggerExitRange(v) => s.animation_trigger_exit_range = v.clone(),
            DeclKind::AnimationTriggerExitRangeStart(v) => s.animation_trigger_exit_range_start = v.clone(),
            DeclKind::AnimationTriggerExitRangeEnd(v) => s.animation_trigger_exit_range_end = v.clone(),
            // Fase 7.1088-7.1089 — WebKit -webkit-box-* legacy, plumb opaco.
            DeclKind::WebkitBoxLines(v) => s.webkit_box_lines = v.clone(),
            DeclKind::WebkitBoxFlexGroup(v) => s.webkit_box_flex_group = v.clone(),
            // Fase 7.1093-7.1100 — IE scrollbar-*-color, plumb opaco.
            DeclKind::ScrollbarBaseColor(v) => s.scrollbar_base_color = v.clone(),
            DeclKind::ScrollbarFaceColor(v) => s.scrollbar_face_color = v.clone(),
            DeclKind::ScrollbarTrackColor(v) => s.scrollbar_track_color = v.clone(),
            DeclKind::ScrollbarArrowColor(v) => s.scrollbar_arrow_color = v.clone(),
            DeclKind::ScrollbarShadowColor(v) => s.scrollbar_shadow_color = v.clone(),
            DeclKind::ScrollbarHighlightColor(v) => s.scrollbar_highlight_color = v.clone(),
            DeclKind::Scrollbar3dlightColor(v) => s.scrollbar_3dlight_color = v.clone(),
            DeclKind::ScrollbarDarkshadowColor(v) => s.scrollbar_darkshadow_color = v.clone(),
            // Fase 7.1101-7.1108 — IE -ms-grid, plumb opaco.
            DeclKind::MsGridColumns(v) => s.ms_grid_columns = v.clone(),
            DeclKind::MsGridRows(v) => s.ms_grid_rows = v.clone(),
            DeclKind::MsGridColumn(v) => s.ms_grid_column = v.clone(),
            DeclKind::MsGridRow(v) => s.ms_grid_row = v.clone(),
            DeclKind::MsGridColumnSpan(v) => s.ms_grid_column_span = v.clone(),
            DeclKind::MsGridRowSpan(v) => s.ms_grid_row_span = v.clone(),
            DeclKind::MsGridColumnAlign(v) => s.ms_grid_column_align = v.clone(),
            DeclKind::MsGridRowAlign(v) => s.ms_grid_row_align = v.clone(),
            // Fase 7.1109-7.1116 — IE -ms- exclusions/regions/text, plumb opaco.
            DeclKind::MsTouchSelect(v) => s.ms_touch_select = v.clone(),
            DeclKind::MsTextAutospace(v) => s.ms_text_autospace = v.clone(),
            DeclKind::MsWrapFlow(v) => s.ms_wrap_flow = v.clone(),
            DeclKind::MsWrapMargin(v) => s.ms_wrap_margin = v.clone(),
            DeclKind::MsWrapThrough(v) => s.ms_wrap_through = v.clone(),
            DeclKind::MsFlowInto(v) => s.ms_flow_into = v.clone(),
            DeclKind::MsFlowFrom(v) => s.ms_flow_from = v.clone(),
            DeclKind::MsHyphenateLimitChars(v) => s.ms_hyphenate_limit_chars = v.clone(),
            // Fase 7.1121-7.1122 — WebKit misc, plumb opaco.
            DeclKind::WebkitMaskAttachment(v) => s.webkit_mask_attachment = v.clone(),
            DeclKind::WebkitTextDecorationsInEffect(v) => s.webkit_text_decorations_in_effect = v.clone(),
            // Fase 7.1123-7.1128 — CSS Borders 4, plumb opaco.
            DeclKind::BorderClip(v) => s.border_clip = v.clone(),
            DeclKind::BorderClipTop(v) => s.border_clip_top = v.clone(),
            DeclKind::BorderClipRight(v) => s.border_clip_right = v.clone(),
            DeclKind::BorderClipBottom(v) => s.border_clip_bottom = v.clone(),
            DeclKind::BorderClipLeft(v) => s.border_clip_left = v.clone(),
            DeclKind::BorderLimit(v) => s.border_limit = v.clone(),
        }
    }
}

/// Devuelve el 1er `TransitionBinding` de la lista, creándolo con defaults
/// (`property: all`) si está vacía. Los longhands `transition-*` (Fase
/// 7.822-7.825) editan este binding — espejo de cómo los longhands
/// `animation-*` usan `get_or_insert_with` sobre un único binding.
fn transition_first(t: &mut Vec<TransitionBinding>) -> &mut TransitionBinding {
    if t.is_empty() {
        t.push(TransitionBinding {
            property: "all".to_string(),
            duration_s: 0.0,
            timing: EasingFunction::default(),
            delay_s: 0.0,
        });
    }
    &mut t[0]
}
