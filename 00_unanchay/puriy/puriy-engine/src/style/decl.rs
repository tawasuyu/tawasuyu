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
    TextSpacingTrim(TextSpacingTrim),
    TextBoxTrim(TextBoxTrim),
    MathStyle(MathStyle),
    MathDepth(MathDepth),
    MathShift(MathShift),
    FieldSizing(FieldSizing),
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
            }
            DeclKind::MarginTop(v) => s.margin.top = *v,
            DeclKind::MarginRight(v) => {
                s.margin.right = *v;
                s.margin_right_auto = false;
            }
            DeclKind::MarginBottom(v) => s.margin.bottom = *v,
            DeclKind::MarginLeft(v) => {
                s.margin.left = *v;
                s.margin_left_auto = false;
            }
            DeclKind::MarginLeftAuto(v) => s.margin_left_auto = *v,
            DeclKind::MarginRightAuto(v) => s.margin_right_auto = *v,
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
            DeclKind::TextSpacingTrim(t) => s.text_spacing_trim = *t,
            DeclKind::TextBoxTrim(t) => s.text_box_trim = *t,
            DeclKind::MathStyle(m) => s.math_style = *m,
            DeclKind::MathDepth(m) => s.math_depth = *m,
            DeclKind::MathShift(m) => s.math_shift = *m,
            DeclKind::FieldSizing(f) => s.field_sizing = *f,
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
            DeclKind::CaretAnimation(v) => s.caret_animation = *v,
            DeclKind::ScrollMarkerGroup(v) => s.scroll_marker_group = *v,
            DeclKind::ScrollInitialTarget(v) => s.scroll_initial_target = *v,
            DeclKind::CornerShape(v) => s.corner_shape = v.clone(),
            DeclKind::HyphenateLimitLines(v) => s.hyphenate_limit_lines = *v,
            DeclKind::HyphenateLimitLast(v) => s.hyphenate_limit_last = *v,
            DeclKind::HyphenateLimitZone(v) => s.hyphenate_limit_zone = v.clone(),
            DeclKind::InterestTarget(v) => s.interest_target = v.clone(),
            DeclKind::InterestDelayStart(v) => s.interest_delay_start = v.clone(),
            DeclKind::InterestDelayEnd(v) => s.interest_delay_end = v.clone(),
            DeclKind::Azimuth(v) => s.azimuth = v.clone(),
            DeclKind::Elevation(v) => s.elevation = v.clone(),
            DeclKind::Richness(v) => s.richness = *v,
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
            DeclKind::TextIndent(v) => s.text_indent = *v,
            DeclKind::WordSpacing(v) => s.word_spacing = *v,
            DeclKind::LetterSpacing(v) => s.letter_spacing = *v,
            DeclKind::TextShadows(shadows) => s.text_shadows = shadows.clone(),
            DeclKind::Transforms(tr) => s.transforms = tr.clone(),
            DeclKind::GridTemplateColumns(t) => s.grid_template_columns = t.clone(),
            DeclKind::GridTemplateRows(t) => s.grid_template_rows = t.clone(),
            DeclKind::Animation(a) => s.animation = a.clone(),
            DeclKind::Transitions(t) => s.transitions = t.clone(),
            // No resolvemos acá: el `color` final del elemento puede no
            // estar aplicado todavía (otra regla de la cascada). Acumulamos
            // el target y `compute_internal` lo resuelve al cierre.
            DeclKind::CurrentColor(target) => s.current_color.push(*target),
        }
    }
}
