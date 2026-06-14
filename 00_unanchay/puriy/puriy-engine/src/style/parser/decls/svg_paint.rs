//! Parsers SVG/paint, baseline, máscaras, container-type, hyphenate, offset, grid-flow, font settings.
//! Value-parsers extraídos de `decls.rs` (regla #1). Lógica intacta.
use super::super::*;
use super::*;

/// `columns` shorthand: `auto | <length> || <integer> || auto`. Si una
/// pieza falta, queda en su default (`LengthVal::Auto` para width,
/// `None` para count). `auto` solo setea ambos a auto. Fase 7.361.
pub(crate) fn parse_columns_shorthand(
    value: &str,
) -> Option<(LengthVal, Option<u32>)> {
    let toks: Vec<&str> = value.trim().split_whitespace().collect();
    if toks.is_empty() {
        return None;
    }
    let mut width: Option<LengthVal> = None;
    let mut count: Option<Option<u32>> = None;
    for t in &toks {
        if t.eq_ignore_ascii_case("auto") {
            // `auto` toca el primer slot vacío (orden libre).
            if width.is_none() {
                width = Some(LengthVal::Auto);
            } else if count.is_none() {
                count = Some(None);
            } else {
                return None;
            }
            continue;
        }
        if let Ok(n) = t.parse::<u32>() {
            if count.is_some() {
                return None;
            }
            if n == 0 {
                return None;
            }
            count = Some(Some(n));
            continue;
        }
        if let Some(l) = parse_length_or_pct(t) {
            if width.is_some() {
                return None;
            }
            width = Some(l);
            continue;
        }
        return None;
    }
    Some((width.unwrap_or(LengthVal::Auto), count.unwrap_or(None)))
}

/// `background-attachment`: lista separada por coma de
/// `scroll | fixed | local`. Fase 7.362.
pub(crate) fn parse_background_attachment(
    value: &str,
) -> Option<Vec<BackgroundAttachment>> {
    let mut out = Vec::new();
    for layer in value.split(',') {
        let v = layer.trim().to_ascii_lowercase();
        let att = match v.as_str() {
            "scroll" => BackgroundAttachment::Scroll,
            "fixed" => BackgroundAttachment::Fixed,
            "local" => BackgroundAttachment::Local,
            _ => return None,
        };
        out.push(att);
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// `caret-shape`: `auto | bar | block | underscore`. Fase 7.363.
pub(crate) fn parse_caret_shape(value: &str) -> Option<CaretShape> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(CaretShape::Auto),
        "bar" => Some(CaretShape::Bar),
        "block" => Some(CaretShape::Block),
        "underscore" => Some(CaretShape::Underscore),
        _ => None,
    }
}

/// `baseline-source`: `auto | first | last`. Fase 7.364.
pub(crate) fn parse_baseline_source(value: &str) -> Option<BaselineSource> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(BaselineSource::Auto),
        "first" => Some(BaselineSource::First),
        "last" => Some(BaselineSource::Last),
        _ => None,
    }
}

/// `alignment-baseline` (SVG 2):
/// `baseline | text-bottom | alphabetic | ideographic | middle |
/// central | mathematical | text-top | bottom | center | top`.
/// Fase 7.365.
pub(crate) fn parse_alignment_baseline(value: &str) -> Option<AlignmentBaseline> {
    match value.trim().to_ascii_lowercase().as_str() {
        "baseline" => Some(AlignmentBaseline::Baseline),
        "text-bottom" => Some(AlignmentBaseline::TextBottom),
        "alphabetic" => Some(AlignmentBaseline::Alphabetic),
        "ideographic" => Some(AlignmentBaseline::Ideographic),
        "middle" => Some(AlignmentBaseline::Middle),
        "central" => Some(AlignmentBaseline::Central),
        "mathematical" => Some(AlignmentBaseline::Mathematical),
        "text-top" => Some(AlignmentBaseline::TextTop),
        "bottom" => Some(AlignmentBaseline::Bottom),
        "center" => Some(AlignmentBaseline::Center),
        "top" => Some(AlignmentBaseline::Top),
        _ => None,
    }
}

/// `dominant-baseline` (SVG 2):
/// `auto | text-bottom | alphabetic | ideographic | middle | central |
/// mathematical | hanging | text-top`. Fase 7.366.
pub(crate) fn parse_dominant_baseline(value: &str) -> Option<DominantBaseline> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(DominantBaseline::Auto),
        "text-bottom" => Some(DominantBaseline::TextBottom),
        "alphabetic" => Some(DominantBaseline::Alphabetic),
        "ideographic" => Some(DominantBaseline::Ideographic),
        "middle" => Some(DominantBaseline::Middle),
        "central" => Some(DominantBaseline::Central),
        "mathematical" => Some(DominantBaseline::Mathematical),
        "hanging" => Some(DominantBaseline::Hanging),
        "text-top" => Some(DominantBaseline::TextTop),
        _ => None,
    }
}

/// `paint-order` (SVG 2): `normal | [fill | stroke | markers]+`.
/// Si se especifican < 3 fragments, los faltantes se completan en el
/// orden canónico `fill stroke markers` (descartando duplicados).
/// Fase 7.367.
pub(crate) fn parse_paint_order(value: &str) -> Option<PaintOrder> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("normal") {
        return Some(PaintOrder::default());
    }
    fn frag(t: &str) -> Option<PaintFragment> {
        match t.to_ascii_lowercase().as_str() {
            "fill" => Some(PaintFragment::Fill),
            "stroke" => Some(PaintFragment::Stroke),
            "markers" => Some(PaintFragment::Markers),
            _ => None,
        }
    }
    let mut given: Vec<PaintFragment> = Vec::new();
    for tok in v.split_whitespace() {
        let f = frag(tok)?;
        if given.contains(&f) {
            return None;
        }
        given.push(f);
    }
    if given.is_empty() || given.len() > 3 {
        return None;
    }
    // Completar con los faltantes en orden canónico.
    for f in [PaintFragment::Fill, PaintFragment::Stroke, PaintFragment::Markers] {
        if !given.contains(&f) {
            given.push(f);
        }
    }
    Some(PaintOrder {
        one: given[0],
        two: given[1],
        three: given[2],
    })
}

/// `marker-side`: `match-self | match-parent`. Fase 7.368.
pub(crate) fn parse_marker_side(value: &str) -> Option<MarkerSide> {
    match value.trim().to_ascii_lowercase().as_str() {
        "match-self" => Some(MarkerSide::MatchSelf),
        "match-parent" => Some(MarkerSide::MatchParent),
        _ => None,
    }
}

/// SVG `<paint>` (SVG 2): `none | currentColor | <color> | url(<id>)`.
/// La sintaxis completa permite además un fallback `url(...) <paint>`
/// — el fallback se descarta (acepta el url pelado o el fallback solo).
/// Fases 7.369–7.370.
pub(crate) fn parse_svg_paint(value: &str) -> Option<SvgPaint> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(SvgPaint::None);
    }
    if v.eq_ignore_ascii_case("currentcolor") {
        return Some(SvgPaint::CurrentColor);
    }
    // Fase 7.904 — `context-fill`/`context-stroke` (SVG 2): heredan el paint
    // del elemento que referencia. Sin pipeline de contexto, degradan a
    // `currentColor`. Divergencia documentada.
    if v.eq_ignore_ascii_case("context-fill") || v.eq_ignore_ascii_case("context-stroke") {
        return Some(SvgPaint::CurrentColor);
    }
    // `url(...)` — opcional fallback ignorado.
    let lower = v.to_ascii_lowercase();
    if let Some(open) = lower.strip_prefix("url(") {
        if let Some(close) = open.find(')') {
            // Tomamos el id entre paréntesis tal cual del original
            // (preservando case).
            let url_inner = &v[4..4 + close];
            return Some(SvgPaint::Url(url_inner.trim().to_string()));
        }
        return None;
    }
    parse_color(v).map(SvgPaint::Color)
}

/// `stroke-linecap`: `butt | round | square`. Fase 7.374.
pub(crate) fn parse_stroke_linecap(value: &str) -> Option<StrokeLinecap> {
    match value.trim().to_ascii_lowercase().as_str() {
        "butt" => Some(StrokeLinecap::Butt),
        "round" => Some(StrokeLinecap::Round),
        "square" => Some(StrokeLinecap::Square),
        _ => None,
    }
}

/// `stroke-linejoin`: `miter | round | bevel | arcs | miter-clip`.
/// Fase 7.375.
pub(crate) fn parse_stroke_linejoin(value: &str) -> Option<StrokeLinejoin> {
    match value.trim().to_ascii_lowercase().as_str() {
        "miter" => Some(StrokeLinejoin::Miter),
        "round" => Some(StrokeLinejoin::Round),
        "bevel" => Some(StrokeLinejoin::Bevel),
        "arcs" => Some(StrokeLinejoin::Arcs),
        "miter-clip" => Some(StrokeLinejoin::MiterClip),
        _ => None,
    }
}

/// `stroke-miterlimit`: número >= 1. Fase 7.376.
pub(crate) fn parse_stroke_miterlimit(value: &str) -> Option<f32> {
    let n: f32 = value.trim().parse().ok()?;
    if !n.is_finite() || n < 1.0 {
        return None;
    }
    Some(n)
}

/// `<color> | currentColor`: para los SVG paint-color (`flood-color`,
/// `lighting-color`, `stop-color`). `None` = se difiere al `color`
/// del elemento. Fases 7.384, 7.386, 7.387.
pub(crate) fn parse_color_or_current(value: &str) -> Option<Option<Color>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("currentcolor") {
        return Some(None);
    }
    parse_color(v).map(Some)
}

/// `fill-rule` / `clip-rule`: `nonzero | evenodd`. Fases 7.379, 7.380.
pub(crate) fn parse_fill_rule(value: &str) -> Option<FillRule> {
    match value.trim().to_ascii_lowercase().as_str() {
        "nonzero" => Some(FillRule::Nonzero),
        "evenodd" => Some(FillRule::Evenodd),
        _ => None,
    }
}

/// `color-interpolation`: `auto | sRGB | linearRGB`. Fase 7.381.
pub(crate) fn parse_color_interpolation(
    value: &str,
) -> Option<ColorInterpolation> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ColorInterpolation::Auto),
        "srgb" => Some(ColorInterpolation::SRgb),
        "linearrgb" => Some(ColorInterpolation::LinearRgb),
        _ => None,
    }
}

/// `shape-rendering`: `auto | optimizeSpeed | crispEdges |
/// geometricPrecision`. Fase 7.382.
pub(crate) fn parse_shape_rendering(value: &str) -> Option<ShapeRendering> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ShapeRendering::Auto),
        "optimizespeed" => Some(ShapeRendering::OptimizeSpeed),
        "crispedges" => Some(ShapeRendering::CrispEdges),
        "geometricprecision" => Some(ShapeRendering::GeometricPrecision),
        _ => None,
    }
}

/// `vector-effect`: `none | non-scaling-stroke | non-scaling-size |
/// non-rotation | fixed-position`. Fase 7.383.
pub(crate) fn parse_vector_effect(value: &str) -> Option<VectorEffect> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Some(VectorEffect::None),
        "non-scaling-stroke" => Some(VectorEffect::NonScalingStroke),
        "non-scaling-size" => Some(VectorEffect::NonScalingSize),
        "non-rotation" => Some(VectorEffect::NonRotation),
        "fixed-position" => Some(VectorEffect::FixedPosition),
        _ => None,
    }
}

/// `text-anchor`: `start | middle | end`. Fase 7.389.
pub(crate) fn parse_text_anchor(value: &str) -> Option<TextAnchor> {
    match value.trim().to_ascii_lowercase().as_str() {
        "start" => Some(TextAnchor::Start),
        "middle" => Some(TextAnchor::Middle),
        "end" => Some(TextAnchor::End),
        _ => None,
    }
}

/// `color-rendering`: `auto | optimizeSpeed | optimizeQuality`. Fase 7.390.
pub(crate) fn parse_color_rendering(value: &str) -> Option<ColorRendering> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ColorRendering::Auto),
        "optimizespeed" => Some(ColorRendering::OptimizeSpeed),
        "optimizequality" => Some(ColorRendering::OptimizeQuality),
        _ => None,
    }
}

/// `color-interpolation-filters`: `auto | sRGB | linearRGB`. Fase 7.391.
pub(crate) fn parse_color_interpolation_filters(
    value: &str,
) -> Option<ColorInterpolationFilters> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ColorInterpolationFilters::Auto),
        "srgb" => Some(ColorInterpolationFilters::SRgb),
        "linearrgb" => Some(ColorInterpolationFilters::LinearRgb),
        _ => None,
    }
}

/// `glyph-orientation-vertical`: `auto | 0 | 90 | 180 | 270` (con o sin
/// `deg`). Fase 7.392 (SVG 1.1 deprecated, parseo defensivo).
pub(crate) fn parse_glyph_orientation_vertical(
    value: &str,
) -> Option<GlyphOrientationVertical> {
    let v = value.trim().to_ascii_lowercase();
    if v == "auto" {
        return Some(GlyphOrientationVertical::Auto);
    }
    let num = v.strip_suffix("deg").unwrap_or(&v).trim();
    match num {
        "0" => Some(GlyphOrientationVertical::Deg0),
        "90" => Some(GlyphOrientationVertical::Deg90),
        "180" => Some(GlyphOrientationVertical::Deg180),
        "270" => Some(GlyphOrientationVertical::Deg270),
        _ => None,
    }
}

/// `transform-box`: `content-box | border-box | fill-box | stroke-box |
/// view-box`. Fase 7.393.
pub(crate) fn parse_transform_box(value: &str) -> Option<TransformBox> {
    match value.trim().to_ascii_lowercase().as_str() {
        "content-box" => Some(TransformBox::ContentBox),
        "border-box" => Some(TransformBox::BorderBox),
        "fill-box" => Some(TransformBox::FillBox),
        "stroke-box" => Some(TransformBox::StrokeBox),
        "view-box" => Some(TransformBox::ViewBox),
        _ => None,
    }
}

/// `marker-{start,mid,end}` / `marker`: `none | <funcIRI>`. Fases
/// 7.394–7.397. Guardamos el IRI tal cual (`url(#mid)`) o `None` para
/// `none`. El IRI debe empezar con `url(` y cerrar con `)`.
pub(crate) fn parse_marker_ref(value: &str) -> Option<MarkerRef> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    if v.len() < 5 || !v.to_ascii_lowercase().starts_with("url(") || !v.ends_with(')') {
        return None;
    }
    Some(Some(v.to_string()))
}

/// `mask-type`: `luminance | alpha`. Fase 7.398.
pub(crate) fn parse_mask_type(value: &str) -> Option<MaskType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "luminance" => Some(MaskType::Luminance),
        "alpha" => Some(MaskType::Alpha),
        _ => None,
    }
}

/// `mask-mode`: `alpha | luminance | match-source`. Fase 7.399.
pub(crate) fn parse_mask_mode(value: &str) -> Option<MaskMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "alpha" => Some(MaskMode::Alpha),
        "luminance" => Some(MaskMode::Luminance),
        "match-source" => Some(MaskMode::MatchSource),
        _ => None,
    }
}

/// `mask-clip`: `<geometry-box> | no-clip`. Fase 7.400.
pub(crate) fn parse_mask_clip(value: &str) -> Option<MaskClip> {
    match value.trim().to_ascii_lowercase().as_str() {
        "border-box" => Some(MaskClip::BorderBox),
        "padding-box" => Some(MaskClip::PaddingBox),
        "content-box" => Some(MaskClip::ContentBox),
        "fill-box" => Some(MaskClip::FillBox),
        "stroke-box" => Some(MaskClip::StrokeBox),
        "view-box" => Some(MaskClip::ViewBox),
        "no-clip" => Some(MaskClip::NoClip),
        _ => None,
    }
}

/// `mask-composite`: `add | subtract | intersect | exclude`. Fase 7.401.
pub(crate) fn parse_mask_composite(value: &str) -> Option<MaskComposite> {
    match value.trim().to_ascii_lowercase().as_str() {
        "add" => Some(MaskComposite::Add),
        "subtract" => Some(MaskComposite::Subtract),
        "intersect" => Some(MaskComposite::Intersect),
        "exclude" => Some(MaskComposite::Exclude),
        _ => None,
    }
}

/// `container-type`: `normal | size | inline-size`. Fase 7.407.
pub(crate) fn parse_container_type(value: &str) -> Option<ContainerType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(ContainerType::Normal),
        "size" => Some(ContainerType::Size),
        "inline-size" => Some(ContainerType::InlineSize),
        _ => None,
    }
}

/// `hyphenate-character`: `auto | <string>`. Devuelve `None` para `auto`
/// (motor elige el carácter del idioma); para un string entre comillas,
/// desempareja las comillas y devuelve el contenido. Fase 7.429.
pub(crate) fn parse_hyphenate_character(value: &str) -> Option<String> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") {
        return None;
    }
    let bytes = v.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'') && first == last {
            return Some(v[1..v.len() - 1].to_string());
        }
    }
    // Sin comillas (no-spec, pero generoso) — tomamos el primer token.
    Some(v.to_string())
}

/// `hyphenate-limit-chars: auto | <integer>{1,3}`. Cada entero puede ser
/// `auto` por separado. Spec CSS Text 4. Fase 7.430.
pub(crate) fn parse_hyphenate_limit_chars(value: &str) -> Option<HyphenateLimitChars> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") {
        return Some(HyphenateLimitChars::default());
    }
    let mut tokens = v.split_whitespace();
    let total = parse_int_or_auto(tokens.next()?)?;
    let start = match tokens.next() {
        Some(t) => parse_int_or_auto(t)?,
        None => None,
    };
    let end = match tokens.next() {
        Some(t) => parse_int_or_auto(t)?,
        None => None,
    };
    if tokens.next().is_some() {
        return None;
    }
    Some(HyphenateLimitChars { total, start, end })
}

/// `auto` → `Some(None)`; un entero positivo → `Some(Some(n))`; cualquier
/// otra cosa rechaza el shorthand entero (`None`).
fn parse_int_or_auto(tok: &str) -> Option<Option<u32>> {
    if tok.eq_ignore_ascii_case("auto") {
        return Some(None);
    }
    tok.parse::<u32>().ok().map(Some)
}

/// `text-size-adjust: auto | none | <pct>`. CSS Text Inline 3. Fase 7.431.
pub(crate) fn parse_text_size_adjust(value: &str) -> Option<TextSizeAdjust> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") {
        return Some(TextSizeAdjust::Auto);
    }
    if v.eq_ignore_ascii_case("none") {
        return Some(TextSizeAdjust::None);
    }
    if let Some(num) = v.strip_suffix('%') {
        return num.trim().parse::<f32>().ok().map(TextSizeAdjust::Pct);
    }
    None
}

/// `block-step-size: none | <length>`. CSS Inline Layout 3. Fase 7.454.
pub(crate) fn parse_block_step_size(value: &str) -> Option<BlockStepSize> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(BlockStepSize::None);
    }
    parse_length_px(v).map(BlockStepSize::Length)
}

/// `scroll-timeline: [<name> || <axis>]`. Fase 7.471. Devuelve `(name, axis)`
/// con defaults rellenos (name=None, axis=Block). Tokens en orden libre. Cada
/// rol se acepta a lo sumo una vez (token redundante → rechazo). Vacío
/// rechaza entero.
pub(crate) fn parse_scroll_view_timeline_short(
    value: &str,
) -> Option<(Option<String>, TimelineAxis)> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    let mut name: Option<Option<String>> = None;
    let mut axis: Option<TimelineAxis> = None;
    for tok in v.split_whitespace() {
        if let Some(a) = parse_timeline_axis(tok) {
            if axis.is_some() {
                return None;
            }
            axis = Some(a);
            continue;
        }
        if let Some(n) = parse_dashed_ident_or_none(tok) {
            if name.is_some() {
                return None;
            }
            name = Some(n);
            continue;
        }
        return None;
    }
    Some((name.unwrap_or(None), axis.unwrap_or(TimelineAxis::Block)))
}

/// `view-timeline: [<name> || <axis> || <inset>{1,2}]`. Fase 7.472. Devuelve
/// `(name, axis, inset_start, inset_end)`. Mismo dispatcher: axis y name como
/// en `scroll-timeline`; el resto se interpreta como inset (cada inset es
/// `auto`/`<length-or-pct>`, hasta 2). Vacío rechaza entero.
pub(crate) fn parse_view_timeline_short(
    value: &str,
) -> Option<(Option<String>, TimelineAxis, LengthVal, LengthVal)> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    let mut name: Option<Option<String>> = None;
    let mut axis: Option<TimelineAxis> = None;
    let mut insets: Vec<LengthVal> = Vec::new();
    for tok in v.split_whitespace() {
        if let Some(a) = parse_timeline_axis(tok) {
            if axis.is_some() {
                return None;
            }
            axis = Some(a);
            continue;
        }
        // Inset tiene precedencia sobre name para tokens como `auto` y
        // `<length>` — `auto` y `none` son la única ambigüedad: `none`
        // siempre va a name (el inset no tiene `none`); `auto` siempre
        // va a inset (el name acepta `<dashed-ident>` pero no `auto`).
        if tok.eq_ignore_ascii_case("auto") {
            if insets.len() >= 2 {
                return None;
            }
            insets.push(LengthVal::Px(0.0));
            continue;
        }
        if let Some(l) = parse_length_or_pct(tok) {
            if insets.len() >= 2 {
                return None;
            }
            insets.push(l);
            continue;
        }
        if let Some(n) = parse_dashed_ident_or_none(tok) {
            if name.is_some() {
                return None;
            }
            name = Some(n);
            continue;
        }
        return None;
    }
    let inset_a = insets.first().copied().unwrap_or(LengthVal::Px(0.0));
    let inset_b = insets.get(1).copied().unwrap_or(inset_a);
    Some((
        name.unwrap_or(None),
        axis.unwrap_or(TimelineAxis::Block),
        inset_a,
        inset_b,
    ))
}

/// `animation-range-{start,end}: normal | <length-percentage> | <name>
/// <length-percentage>?`. CSS Animations 2. Fase 7.464/465.
///
/// - `normal` → `Normal`.
/// - 1 token `<length-or-pct>` → `Length(LengthVal)`.
/// - 1 token `<phase>` (`cover`/`contain`/`entry`/`exit`/`entry-crossing`/
///   `exit-crossing`) → `Named { phase, offset: None }`.
/// - 2 tokens `<phase> <length-or-pct>` → `Named { phase, offset: Some(%) }`.
///
/// Cualquier otra forma → `None`. El offset se acepta como length pero el
/// modelo lo guarda como porcentaje crudo (el chrome no implementa scroll/
/// view-timelines aún — sólo plumb).
pub(crate) fn parse_animation_range(value: &str) -> Option<AnimationRange> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if v.eq_ignore_ascii_case("normal") {
        return Some(AnimationRange::Normal);
    }
    let toks: Vec<&str> = v.split_whitespace().collect();
    if toks.len() == 1 {
        if let Some(phase) = parse_animation_range_phase(toks[0]) {
            return Some(AnimationRange::Named { phase, offset_pct: None });
        }
        if let Some(len) = parse_length_or_pct(toks[0]) {
            return Some(AnimationRange::Length(len));
        }
        return None;
    }
    if toks.len() == 2 {
        let phase = parse_animation_range_phase(toks[0])?;
        let off = parse_pct_value(toks[1])?;
        return Some(AnimationRange::Named { phase, offset_pct: Some(off) });
    }
    None
}

fn parse_animation_range_phase(tok: &str) -> Option<AnimationRangePhase> {
    match tok.to_ascii_lowercase().as_str() {
        "cover" => Some(AnimationRangePhase::Cover),
        "contain" => Some(AnimationRangePhase::Contain),
        "entry" => Some(AnimationRangePhase::Entry),
        "exit" => Some(AnimationRangePhase::Exit),
        "entry-crossing" => Some(AnimationRangePhase::EntryCrossing),
        "exit-crossing" => Some(AnimationRangePhase::ExitCrossing),
        _ => None,
    }
}

fn parse_pct_value(tok: &str) -> Option<f32> {
    let t = tok.trim();
    if let Some(num) = t.strip_suffix('%') {
        return num.trim().parse::<f32>().ok();
    }
    None
}

/// `position-try-order` keyword. Fase 7.460.
pub(crate) fn parse_position_try_order(value: &str) -> Option<PositionTryOrder> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(PositionTryOrder::Normal),
        "most-width" => Some(PositionTryOrder::MostWidth),
        "most-height" => Some(PositionTryOrder::MostHeight),
        "most-block-size" => Some(PositionTryOrder::MostBlockSize),
        "most-inline-size" => Some(PositionTryOrder::MostInlineSize),
        _ => None,
    }
}

/// `position-try-fallbacks: none | <try-tactic-list>`. CSS Anchor Positioning
/// 1. Lista separada por COMA — cada entrada se guarda como string crudo
/// (`<dashed-ident>` o try-tactic compuesta `flip-block flip-inline`). `none`
/// → Vec vacío. Vacío rechaza (no se emite). Fase 7.461.
pub(crate) fn parse_position_try_fallbacks(value: &str) -> Option<Vec<String>> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out: Vec<String> = Vec::new();
    for part in v.split(',') {
        let p = part.trim();
        if p.is_empty() {
            return None;
        }
        out.push(p.to_string());
    }
    Some(out)
}

/// Pieza individual del shorthand `block-step`. Devuelve un `DeclKind` con
/// el longhand correspondiente, o `None` si el token no pertenece a NINGÚN
/// longhand. Fase 7.458.
pub(crate) fn parse_block_step_piece(tok: &str) -> Option<DeclKind> {
    let low = tok.to_ascii_lowercase();
    match low.as_str() {
        "none" => Some(DeclKind::BlockStepSize(BlockStepSize::None)),
        "margin-box" => Some(DeclKind::BlockStepInsert(BlockStepInsert::MarginBox)),
        "padding-box" => Some(DeclKind::BlockStepInsert(BlockStepInsert::PaddingBox)),
        "auto" => Some(DeclKind::BlockStepAlign(BlockStepAlign::Auto)),
        "center" => Some(DeclKind::BlockStepAlign(BlockStepAlign::Center)),
        "start" => Some(DeclKind::BlockStepAlign(BlockStepAlign::Start)),
        "end" => Some(DeclKind::BlockStepAlign(BlockStepAlign::End)),
        "up" => Some(DeclKind::BlockStepRound(BlockStepRound::Up)),
        "down" => Some(DeclKind::BlockStepRound(BlockStepRound::Down)),
        "nearest" => Some(DeclKind::BlockStepRound(BlockStepRound::Nearest)),
        _ => parse_length_px(tok).map(|n| DeclKind::BlockStepSize(BlockStepSize::Length(n))),
    }
}

/// `offset-rotate: [ auto | reverse ] || <angle>`. CSS Motion Path 1.
/// Sin tokens explícitos → default `auto`. Fase 7.449.
pub(crate) fn parse_offset_rotate(value: &str) -> Option<OffsetRotate> {
    let mut out = OffsetRotate { auto: false, reverse: false, angle_deg: 0.0 };
    let mut saw_angle = false;
    let mut saw_kw = false;
    for tok in value.trim().split_whitespace() {
        let low = tok.to_ascii_lowercase();
        match low.as_str() {
            "auto" => {
                if saw_kw {
                    return None;
                }
                out.auto = true;
                saw_kw = true;
            }
            "reverse" => {
                if saw_kw {
                    return None;
                }
                out.reverse = true;
                saw_kw = true;
            }
            _ => {
                if saw_angle {
                    return None;
                }
                out.angle_deg = parse_angle_deg(tok)?;
                saw_angle = true;
            }
        }
    }
    if !saw_kw && !saw_angle {
        return None;
    }
    // Sin keyword explícito + sólo ángulo → fixed (no auto).
    if !saw_kw {
        out.auto = false;
    }
    Some(out)
}

/// `<alpha-value>`: `<number>` clamp [0..1] o `<pct>` (50% → 0.5). CSS Color 4.
/// Fase 7.446.
pub(crate) fn parse_alpha_value(value: &str) -> Option<f32> {
    let v = value.trim();
    if let Some(num) = v.strip_suffix('%') {
        let n: f32 = num.trim().parse().ok()?;
        return Some((n / 100.0).clamp(0.0, 1.0));
    }
    let n: f32 = v.parse().ok()?;
    Some(n.clamp(0.0, 1.0))
}

/// `text-combine-upright: none | all | digits <integer>?`. CSS Writing Modes 3.
/// `digits` sin entero usa 2 (default del spec). Fase 7.447.
pub(crate) fn parse_text_combine_upright(value: &str) -> Option<TextCombineUpright> {
    let toks: Vec<String> = value
        .trim()
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();
    let refs: Vec<&str> = toks.iter().map(|s| s.as_str()).collect();
    match refs.as_slice() {
        ["none"] => Some(TextCombineUpright::None),
        ["all"] => Some(TextCombineUpright::All),
        ["digits"] => Some(TextCombineUpright::Digits(2)),
        ["digits", n] => n.parse().ok().map(TextCombineUpright::Digits),
        _ => None,
    }
}

/// `ruby-align: start | center | space-between | space-around`. CSS Ruby 1.
/// Fase 7.448.
pub(crate) fn parse_ruby_align(value: &str) -> Option<RubyAlign> {
    match value.trim().to_ascii_lowercase().as_str() {
        "start" => Some(RubyAlign::Start),
        "center" => Some(RubyAlign::Center),
        "space-between" => Some(RubyAlign::SpaceBetween),
        "space-around" => Some(RubyAlign::SpaceAround),
        _ => None,
    }
}

/// `background-position-x: left | center | right | <length-or-pct>`.
/// Fase 7.904 — admite además `<edge> <offset>` (`right 10px`, `left 20%`).
pub(crate) fn parse_background_position_x(value: &str) -> Option<LengthVal> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" => Some(LengthVal::Pct(0.0)),
        "center" => Some(LengthVal::Pct(50.0)),
        "right" => Some(LengthVal::Pct(100.0)),
        other => parse_pos_edge_offset(other, "left", "right")
            .or_else(|| parse_length_or_pct(other)),
    }
}

/// `background-position-y: top | center | bottom | <length-or-pct>`.
/// Fase 7.904 — admite además `<edge> <offset>` (`bottom 20%`, `top 5px`).
pub(crate) fn parse_background_position_y(value: &str) -> Option<LengthVal> {
    match value.trim().to_ascii_lowercase().as_str() {
        "top" => Some(LengthVal::Pct(0.0)),
        "center" => Some(LengthVal::Pct(50.0)),
        "bottom" => Some(LengthVal::Pct(100.0)),
        other => parse_pos_edge_offset(other, "top", "bottom")
            .or_else(|| parse_length_or_pct(other)),
    }
}

/// `<edge> <offset>` de `background-position-{x,y}`: `near`=origen (`left`/
/// `top`), `far`=borde opuesto (`right`/`bottom`). Desde el borde cercano el
/// offset es directo; desde el lejano, `100% − <offset>` si es porcentaje (un
/// offset en px contra el borde lejano no se modela → degrada al borde 100%).
/// Fase 7.904.
fn parse_pos_edge_offset(value: &str, near: &str, far: &str) -> Option<LengthVal> {
    let mut it = value.split_whitespace();
    let edge = it.next()?;
    let offset = it.next()?;
    if it.next().is_some() {
        return None;
    }
    let off = parse_length_or_pct(offset)?;
    if edge.eq_ignore_ascii_case(near) {
        Some(off)
    } else if edge.eq_ignore_ascii_case(far) {
        match off {
            LengthVal::Pct(p) => Some(LengthVal::Pct(100.0 - p)),
            _ => Some(LengthVal::Pct(100.0)),
        }
    } else {
        None
    }
}

/// `grid-auto-flow: row | column | row dense | column dense | dense`. CSS
/// Grid 1. El `dense` solo equivale a `row dense`. Fase 7.441.
pub(crate) fn parse_grid_auto_flow(value: &str) -> Option<GridAutoFlow> {
    let toks: Vec<String> = value
        .trim()
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();
    let refs: Vec<&str> = toks.iter().map(|s| s.as_str()).collect();
    match refs.as_slice() {
        ["row"] => Some(GridAutoFlow::Row),
        ["column"] => Some(GridAutoFlow::Column),
        ["dense"] => Some(GridAutoFlow::RowDense),
        ["row", "dense"] | ["dense", "row"] => Some(GridAutoFlow::RowDense),
        ["column", "dense"] | ["dense", "column"] => Some(GridAutoFlow::ColumnDense),
        _ => None,
    }
}

/// Divide los tokens del shorthand `contain-intrinsic-size` en width y
/// height (cada uno acepta `auto?` seguido de `none | <length>`). Devuelve
/// `(raw_w, raw_h)` listos para `parse_contain_intrinsic_size`. Si hay un
/// solo "lado", `h` queda en `None` (el caller copia w → h).
pub(crate) fn split_contain_intrinsic_halves<'a>(
    toks: &[&'a str],
) -> Option<(String, Option<String>)> {
    if toks.is_empty() {
        return None;
    }
    let mut sides: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        if t.eq_ignore_ascii_case("auto") {
            // `auto` arranca un lado (y consume el siguiente token como su
            // valor `none | <length>`). Si ya empezamos un lado sin cerrar,
            // este `auto` pertenece al próximo → cerramos.
            if !cur.is_empty() {
                sides.push(std::mem::take(&mut cur));
            }
            cur.push(t);
            if let Some(&next) = toks.get(i + 1) {
                cur.push(next);
                i += 2;
            } else {
                return None;
            }
            sides.push(std::mem::take(&mut cur));
        } else {
            // `none | <length>` cierra un lado por sí solo.
            if !cur.is_empty() {
                sides.push(std::mem::take(&mut cur));
            }
            cur.push(t);
            sides.push(std::mem::take(&mut cur));
            i += 1;
        }
    }
    match sides.len() {
        1 => Some((sides[0].join(" "), None)),
        2 => Some((sides[0].join(" "), Some(sides[1].join(" ")))),
        _ => None,
    }
}

/// `contain-intrinsic-*: none | <length> | auto none | auto <length>`.
/// CSS Containment 3. Fase 7.434.
pub(crate) fn parse_contain_intrinsic_size(value: &str) -> Option<ContainIntrinsicSize> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(ContainIntrinsicSize::None);
    }
    let mut tokens = v.split_whitespace();
    let first = tokens.next()?;
    if first.eq_ignore_ascii_case("auto") {
        let second = tokens.next()?;
        if tokens.next().is_some() {
            return None;
        }
        if second.eq_ignore_ascii_case("none") {
            return Some(ContainIntrinsicSize::AutoNone);
        }
        return parse_length_px(second).map(ContainIntrinsicSize::AutoLength);
    }
    if tokens.next().is_some() {
        return None;
    }
    parse_length_px(first).map(ContainIntrinsicSize::Length)
}

/// `font-variant-emoji: normal | text | emoji | unicode`. CSS Fonts 4.
/// Fase 7.433.
pub(crate) fn parse_font_variant_emoji(value: &str) -> Option<FontVariantEmoji> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(FontVariantEmoji::Normal),
        "text" => Some(FontVariantEmoji::Text),
        "emoji" => Some(FontVariantEmoji::Emoji),
        "unicode" => Some(FontVariantEmoji::Unicode),
        _ => None,
    }
}

/// `mask-origin`: `<geometry-box>`. Fase 7.402.
pub(crate) fn parse_mask_origin(value: &str) -> Option<MaskOrigin> {
    match value.trim().to_ascii_lowercase().as_str() {
        "border-box" => Some(MaskOrigin::BorderBox),
        "padding-box" => Some(MaskOrigin::PaddingBox),
        "content-box" => Some(MaskOrigin::ContentBox),
        "fill-box" => Some(MaskOrigin::FillBox),
        "stroke-box" => Some(MaskOrigin::StrokeBox),
        "view-box" => Some(MaskOrigin::ViewBox),
        _ => None,
    }
}

/// `stroke-dasharray`: `none | <length-percentage>+` separados por
/// espacios o comas. Fase 7.377.
pub(crate) fn parse_stroke_dasharray(value: &str) -> Option<Vec<LengthVal>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for tok in v.split(|c: char| c == ',' || c.is_whitespace()) {
        if tok.is_empty() {
            continue;
        }
        let l = parse_length_or_pct(tok)?;
        out.push(l);
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// SVG `<opacity-value>`: número `0..=1` o porcentaje `0%..=100%`.
/// Valores fuera de rango se clampean. Fases 7.371–7.372.
pub(crate) fn parse_svg_opacity(value: &str) -> Option<f32> {
    let v = value.trim();
    if let Some(num) = v.strip_suffix('%') {
        let n: f32 = num.trim().parse().ok()?;
        if !n.is_finite() {
            return None;
        }
        return Some((n / 100.0).clamp(0.0, 1.0));
    }
    let n: f32 = v.parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    Some(n.clamp(0.0, 1.0))
}

/// Lista de `<custom-ident>` separados por espacios, con `none`
/// → vector vacío. Reusada por `anchor-name`, `view-transition-class`,
/// etc. (Fases 7.354, 7.358).
pub(crate) fn parse_ident_list_or_none(value: &str) -> Option<Vec<String>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    if v.is_empty() {
        return None;
    }
    let toks: Vec<String> = v.split_whitespace().map(String::from).collect();
    if toks.is_empty() {
        return None;
    }
    Some(toks)
}

/// `position-anchor`: `auto | <dashed-ident>`. Fase 7.355.
pub(crate) fn parse_ident_or_auto(value: &str) -> Option<Option<String>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") {
        return Some(None);
    }
    if v.is_empty() || v.contains(char::is_whitespace) {
        return None;
    }
    Some(Some(v.to_string()))
}

/// `anchor-scope`: `none | all | <dashed-ident>+`. Fase 7.356.
pub(crate) fn parse_anchor_scope(value: &str) -> Option<AnchorScope> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(AnchorScope::None);
    }
    if v.eq_ignore_ascii_case("all") {
        return Some(AnchorScope::All);
    }
    if v.is_empty() {
        return None;
    }
    let toks: Vec<String> = v.split_whitespace().map(String::from).collect();
    if toks.is_empty() {
        return None;
    }
    Some(AnchorScope::Names(toks))
}

/// `text-box-edge`: `auto | <text-edge> [<text-edge>]?`.
/// `<text-edge>` ∈ `text | cap | ex | ideographic | ideographic-ink |
/// alphabetic`. Si hay 1 keyword, aplica a ambos lados. Fase 7.353.
pub(crate) fn parse_text_box_edge(value: &str) -> Option<TextBoxEdge> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") {
        return Some(TextBoxEdge::Auto);
    }
    fn edge(t: &str) -> Option<TextEdge> {
        match t.to_ascii_lowercase().as_str() {
            "text" => Some(TextEdge::Text),
            "cap" => Some(TextEdge::Cap),
            "ex" => Some(TextEdge::Ex),
            "ideographic" => Some(TextEdge::Ideographic),
            "ideographic-ink" => Some(TextEdge::IdeographicInk),
            "alphabetic" => Some(TextEdge::Alphabetic),
            _ => None,
        }
    }
    let toks: Vec<&str> = v.split_whitespace().collect();
    match toks.as_slice() {
        [a] => {
            let e = edge(a)?;
            Some(TextBoxEdge::Edge { over: e, under: e })
        }
        [a, b] => Some(TextBoxEdge::Edge { over: edge(a)?, under: edge(b)? }),
        _ => None,
    }
}

/// `font-synthesis` shorthand (CSS Fonts 4):
/// `none | [weight || style || small-caps]`. Fase 7.333.
pub(crate) fn parse_font_synthesis_shorthand(
    value: &str,
) -> Option<FontSynthesis> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(FontSynthesis::NONE);
    }
    let mut fs = FontSynthesis::NONE;
    for tok in v.split_whitespace() {
        match tok.to_ascii_lowercase().as_str() {
            "weight" => {
                if fs.weight {
                    return None;
                }
                fs.weight = true;
            }
            "style" => {
                if fs.style {
                    return None;
                }
                fs.style = true;
            }
            "small-caps" => {
                if fs.small_caps {
                    return None;
                }
                fs.small_caps = true;
            }
            // Fase 7.470 — CSS Fonts 4 extiende el shorthand al 4º axis
            // `position`.
            "position" => {
                if fs.position {
                    return None;
                }
                fs.position = true;
            }
            _ => return None,
        }
    }
    if fs == FontSynthesis::NONE {
        return None;
    }
    Some(fs)
}

/// `break-inside`: `auto | avoid | avoid-page | avoid-column | avoid-region`.
/// Acepta también el legacy `page-break-inside` (CSS 2.1) que sólo conoce
/// `auto | avoid` — los valores avoid-* se aceptan en el callsite legacy,
/// el engine los preserva si vienen escritos. Fase 7.283.
pub(crate) fn parse_break_inside(value: &str) -> Option<BreakInside> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(BreakInside::Auto),
        "avoid" => Some(BreakInside::Avoid),
        "avoid-page" => Some(BreakInside::AvoidPage),
        "avoid-column" => Some(BreakInside::AvoidColumn),
        "avoid-region" => Some(BreakInside::AvoidRegion),
        _ => None,
    }
}

/// `font-kerning`: `auto | normal | none`.
pub(crate) fn parse_font_kerning(value: &str) -> Option<FontKerning> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(FontKerning::Auto),
        "normal" => Some(FontKerning::Normal),
        "none" => Some(FontKerning::None),
        _ => None,
    }
}

/// `font-feature-settings`: `normal` o lista `"tag" [on|off|N], ...`.
/// Tag debe ser 4 ASCII chars entre comillas (simples o dobles). El
/// valor opcional default es 1 (on). `on`/`off` se convierten a 1/0.
pub(crate) fn parse_font_feature_settings(value: &str) -> Vec<FontFeatureSetting> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("normal") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for item in v.split(',') {
        let item = item.trim();
        let (tag_str, rest) = match strip_quoted_tag(item) {
            Some(p) => p,
            None => continue,
        };
        if tag_str.len() != 4 || !tag_str.is_ascii() {
            continue;
        }
        let mut tag = [0u8; 4];
        tag.copy_from_slice(tag_str.as_bytes());
        let val_str = rest.trim();
        let value = if val_str.is_empty() {
            1
        } else if val_str.eq_ignore_ascii_case("on") {
            1
        } else if val_str.eq_ignore_ascii_case("off") {
            0
        } else if let Ok(n) = val_str.parse::<i32>() {
            n
        } else {
            continue;
        };
        out.push(FontFeatureSetting { tag, value });
    }
    out
}

/// `font-variation-settings`: `normal` o `"tag" <number>`.
pub(crate) fn parse_font_variation_settings(value: &str) -> Vec<FontVariationSetting> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("normal") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for item in v.split(',') {
        let item = item.trim();
        let (tag_str, rest) = match strip_quoted_tag(item) {
            Some(p) => p,
            None => continue,
        };
        if tag_str.len() != 4 || !tag_str.is_ascii() {
            continue;
        }
        let mut tag = [0u8; 4];
        tag.copy_from_slice(tag_str.as_bytes());
        let val_str = rest.trim();
        let Ok(value) = val_str.parse::<f32>() else {
            continue;
        };
        out.push(FontVariationSetting { tag, value });
    }
    out
}

/// `font-language-override`: `normal` o `"tag"` (3-4 chars OpenType).
/// El tag se devuelve sin comillas, conservando el case.
pub(crate) fn parse_font_language_override(value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("normal") {
        return None;
    }
    let bytes = v.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let first = bytes[0];
    let last = bytes[bytes.len() - 1];
    if (first != b'"' && first != b'\'') || first != last {
        return None;
    }
    let inner = &v[1..v.len() - 1];
    if !inner.is_ascii() || inner.is_empty() {
        return None;
    }
    Some(inner.to_string())
}

/// `text-rendering`: 4 keywords. Case-insensitive.
pub(crate) fn parse_text_rendering(value: &str) -> Option<TextRendering> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(TextRendering::Auto),
        "optimizespeed" => Some(TextRendering::OptimizeSpeed),
        "optimizelegibility" => Some(TextRendering::OptimizeLegibility),
        "geometricprecision" => Some(TextRendering::GeometricPrecision),
        _ => None,
    }
}

/// Helper: dado `"tag" rest`, devuelve `(tag, rest)` sin las comillas.
/// Soporta tanto `"…"` como `'…'`. Devuelve `None` si no encuentra
/// comillas de cierre.
fn strip_quoted_tag(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let quote = bytes[0];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    // Buscar la próxima comilla del mismo tipo.
    let rest = &s[1..];
    let close = rest.find(quote as char)?;
    Some((&rest[..close], &rest[close + 1..]))
}

/// `tab-size`: integer (= ancho en caracteres del space) o length
/// (con unidad). `0` queda permitido (anula el tab). Valor negativo
/// dropea la regla. CSS distingue por unidad — un `4` unitless es
/// integer; un `4px` es length. Probamos integer-puro PRIMERO porque
/// `parse_length_px` acepta unitless como px y se comería el caso.
pub(crate) fn parse_tab_size(value: &str) -> Option<TabSize> {
    let v = value.trim();
    if let Ok(n) = v.parse::<i32>() {
        if n < 0 {
            return None;
        }
        return Some(TabSize::Chars(n as u16));
    }
    let px = parse_length_px(v)?;
    if px < 0.0 {
        return None;
    }
    Some(TabSize::Px(px))
}

