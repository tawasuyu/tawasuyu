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
