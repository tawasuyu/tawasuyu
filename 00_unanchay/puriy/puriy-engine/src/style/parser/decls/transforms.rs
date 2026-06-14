//! Parsers de transform/3D, scrollbar, overflow, wrap de texto, place-*, timelines.
//! Value-parsers extraídos de `decls.rs` (regla #1). Lógica intacta.
use super::super::*;
use super::*;

/// Prop individual `translate` (CSS Transforms 2): `none | <len-pct>{1,3}`
/// separado por espacios (no coma, a diferencia de la función `translate()`).
/// El 3er valor (Z) se ignora en el modelo 2D. `none` → `Some(None)`.
/// Devuelve `None` (rechazo) si algún token no parsea. Fase 7.826.
pub(crate) fn parse_translate_prop(value: &str) -> Option<Option<Transform>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    // Eje `<length-percentage>` → (px, pct): el `%` no es longitud, lo
    // capturamos aparte. Igual que en la función `translate()`.
    fn axis(s: &str) -> Option<(f32, f32)> {
        let s = s.trim();
        if let Some(p) = s.strip_suffix('%') {
            return p.trim().parse::<f32>().ok().map(|n| (0.0, n));
        }
        parse_length_px(s).map(|px| (px, 0.0))
    }
    // La prop individual `translate` guarda UN solo Transform (se prepende a
    // la cadena en compute). Soportamos los casos puros (todo px → Translate,
    // todo % → TranslatePct); mezclar px y % en un mismo `translate:` es raro
    // y queda fuera (devuelve None → drop).
    fn pick((xpx, xpct): (f32, f32), (ypx, ypct): (f32, f32)) -> Option<Option<Transform>> {
        if xpct == 0.0 && ypct == 0.0 {
            Some(Some(Transform::Translate(xpx, ypx)))
        } else if xpx == 0.0 && ypx == 0.0 {
            Some(Some(Transform::TranslatePct(xpct, ypct)))
        } else {
            None // mezcla px/% en la prop individual: fuera de alcance
        }
    }
    let toks: Vec<&str> = v.split_whitespace().collect();
    match toks.as_slice() {
        [x] => pick(axis(x)?, (0.0, 0.0)),
        [x, y] | [x, y, _] => pick(axis(x)?, axis(y)?),
        _ => None,
    }
}

/// Prop individual `scale` (CSS Transforms 2): `none | <number-or-pct>{1,3}`.
/// 1 valor → uniforme; el 3º (Z) se ignora en 2D. `50%` = `0.5`. Fase 7.828.
pub(crate) fn parse_scale_prop(value: &str) -> Option<Option<Transform>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    fn num_or_pct(s: &str) -> Option<f32> {
        if let Some(p) = s.strip_suffix('%') {
            p.trim().parse::<f32>().ok().map(|n| n / 100.0)
        } else {
            s.parse::<f32>().ok()
        }
    }
    let toks: Vec<&str> = v.split_whitespace().collect();
    match toks.as_slice() {
        [s] => {
            let n = num_or_pct(s)?;
            Some(Some(Transform::Scale(n, n)))
        }
        [sx, sy] | [sx, sy, _] => {
            Some(Some(Transform::Scale(num_or_pct(sx)?, num_or_pct(sy)?)))
        }
        _ => None,
    }
}

/// Prop individual `rotate` (CSS Transforms 2): `none | <angle> |
/// [ x | y | z | <number>{3} ] && <angle>`. En el modelo 2D sólo la
/// rotación alrededor de Z gira en pantalla; un eje `x`/`y` explícito da
/// `Rotate(0)` (sin efecto plano). Fase 7.827.
pub(crate) fn parse_rotate_prop(value: &str) -> Option<Option<Transform>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    // Fase 7.875 — tokeniza respetando paréntesis para no partir `calc(…)`
    // (que lleva espacios internos) en el eje del ángulo.
    let toks = split_top_level_ws(v);
    // El ángulo es el token que parsea como tal; el resto es eje/vector.
    let angle_tok = toks.iter().find(|t| parse_angle_degrees(t.as_str()).is_some())?;
    let deg = parse_angle_degrees(angle_tok.as_str())?;
    // Eje explícito x/y (no-Z) → sin rotación en el plano 2D.
    let non_z_axis = toks
        .iter()
        .any(|t| t.eq_ignore_ascii_case("x") || t.eq_ignore_ascii_case("y"));
    Some(Some(Transform::Rotate(if non_z_axis { 0.0 } else { deg })))
}

/// `transform-origin` (CSS Transforms 1). Acepta 1, 2 ó 3 tokens; el
/// 3º es siempre Z en px (sin `%`). Para el eje X/Y reusamos la misma
/// lógica de keywords/lengths que `background-position`:
///   - 1 token: si es vertical (`top`/`bottom`) fija Y; si es horizontal
///     o ambiguo (length/%/`center`) fija X. El otro eje queda en 50%.
///   - 2 tokens: si los keywords explicitan ejes invertidos
///     (`top left`, `center right`), se reordenan.
/// Fase 7.314.
pub(crate) fn parse_transform_origin(value: &str) -> Option<TransformOrigin> {
    fn axis_token(t: &str) -> Option<(LengthVal, Option<bool>)> {
        match t.to_ascii_lowercase().as_str() {
            "left" => Some((LengthVal::Pct(0.0), Some(true))),
            "right" => Some((LengthVal::Pct(100.0), Some(true))),
            "top" => Some((LengthVal::Pct(0.0), Some(false))),
            "bottom" => Some((LengthVal::Pct(100.0), Some(false))),
            "center" => Some((LengthVal::Pct(50.0), None)),
            other => parse_length_or_pct(other).map(|l| (l, None)),
        }
    }
    // Fase 7.877 — tokeniza respetando paréntesis (calc en X/Y).
    let toks_owned = split_top_level_ws(value.trim());
    let toks: Vec<&str> = toks_owned.iter().map(String::as_str).collect();
    let (x, y, z_tok) = match toks.as_slice() {
        [a] => {
            let (la, axis) = axis_token(a)?;
            if axis == Some(false) {
                (LengthVal::Pct(50.0), la, None)
            } else {
                (la, LengthVal::Pct(50.0), None)
            }
        }
        [a, b] => {
            let (la, aa) = axis_token(a)?;
            let (lb, ab) = axis_token(b)?;
            if aa == Some(false) || ab == Some(true) {
                (lb, la, None)
            } else {
                (la, lb, None)
            }
        }
        [a, b, c] => {
            let (la, aa) = axis_token(a)?;
            let (lb, ab) = axis_token(b)?;
            let (x, y) = if aa == Some(false) || ab == Some(true) {
                (lb, la)
            } else {
                (la, lb)
            };
            (x, y, Some(*c))
        }
        _ => return None,
    };
    let z = if let Some(t) = z_tok {
        // El eje Z no admite `%`. Aceptamos sólo length-en-px.
        parse_length_px(t)?
    } else {
        0.0
    };
    Some(TransformOrigin { x, y, z })
}

/// `transform-style`: `flat | preserve-3d`. Fase 7.315.
pub(crate) fn parse_transform_style(value: &str) -> Option<TransformStyle> {
    match value.trim().to_ascii_lowercase().as_str() {
        "flat" => Some(TransformStyle::Flat),
        "preserve-3d" => Some(TransformStyle::Preserve3d),
        _ => None,
    }
}

/// `perspective`: `none | <length>` (no negativo). Fase 7.316.
pub(crate) fn parse_perspective(value: &str) -> Option<Option<f32>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    let px = parse_length_px(v)?;
    if px < 0.0 {
        return None;
    }
    Some(Some(px))
}

/// `perspective-origin` (CSS Transforms 2). 1 ó 2 tokens, sólo
/// dimensión 2D — mismo set de keywords/lengths que `transform-origin`
/// (sin eje Z). Fase 7.317.
pub(crate) fn parse_perspective_origin(value: &str) -> Option<PerspectiveOrigin> {
    fn axis_token(t: &str) -> Option<(LengthVal, Option<bool>)> {
        match t.to_ascii_lowercase().as_str() {
            "left" => Some((LengthVal::Pct(0.0), Some(true))),
            "right" => Some((LengthVal::Pct(100.0), Some(true))),
            "top" => Some((LengthVal::Pct(0.0), Some(false))),
            "bottom" => Some((LengthVal::Pct(100.0), Some(false))),
            "center" => Some((LengthVal::Pct(50.0), None)),
            other => parse_length_or_pct(other).map(|l| (l, None)),
        }
    }
    let toks: Vec<&str> = value.trim().split_whitespace().collect();
    match toks.as_slice() {
        [a] => {
            let (la, axis) = axis_token(a)?;
            Some(if axis == Some(false) {
                PerspectiveOrigin { x: LengthVal::Pct(50.0), y: la }
            } else {
                PerspectiveOrigin { x: la, y: LengthVal::Pct(50.0) }
            })
        }
        [a, b] => {
            let (la, aa) = axis_token(a)?;
            let (lb, ab) = axis_token(b)?;
            Some(if aa == Some(false) || ab == Some(true) {
                PerspectiveOrigin { x: lb, y: la }
            } else {
                PerspectiveOrigin { x: la, y: lb }
            })
        }
        _ => None,
    }
}

/// `backface-visibility`: `visible | hidden`. Fase 7.318.
pub(crate) fn parse_backface_visibility(value: &str) -> Option<BackfaceVisibility> {
    match value.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(BackfaceVisibility::Visible),
        "hidden" => Some(BackfaceVisibility::Hidden),
        _ => None,
    }
}

/// `scrollbar-width`: `auto | thin | none`. Fase 7.319.
pub(crate) fn parse_scrollbar_width(value: &str) -> Option<ScrollbarWidth> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ScrollbarWidth::Auto),
        "thin" => Some(ScrollbarWidth::Thin),
        "none" => Some(ScrollbarWidth::None),
        _ => None,
    }
}

/// `scrollbar-color`: `auto | <thumb> <track>` (2 colores obligatorios).
/// Fase 7.320.
pub(crate) fn parse_scrollbar_color(
    value: &str,
) -> Option<Option<ScrollbarColorPair>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") {
        return Some(None);
    }
    // Dos colores. Como un color puede contener espacios (rgb(...)),
    // tokenizamos respetando paréntesis.
    let toks = split_top_level_ws(v);
    if toks.len() != 2 {
        return None;
    }
    let thumb = parse_color(&toks[0])?;
    let track = parse_color(&toks[1])?;
    Some(Some(ScrollbarColorPair { thumb, track }))
}

/// `scrollbar-gutter`: `auto | stable [both-edges]?`. Fase 7.321.
pub(crate) fn parse_scrollbar_gutter(value: &str) -> Option<ScrollbarGutter> {
    let toks: Vec<String> = value
        .trim()
        .split_whitespace()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    match toks.as_slice() {
        [a] if a == "auto" => Some(ScrollbarGutter::AUTO),
        [a] if a == "stable" => Some(ScrollbarGutter::STABLE),
        [a, b] if a == "stable" && b == "both-edges" => {
            Some(ScrollbarGutter::STABLE_BOTH)
        }
        // `both-edges stable` también es válido por orden libre (la spec
        // no manda orden); aceptamos ambos.
        [a, b] if a == "both-edges" && b == "stable" => {
            Some(ScrollbarGutter::STABLE_BOTH)
        }
        _ => None,
    }
}

/// `overflow-anchor`: `auto | none`. Fase 7.322.
pub(crate) fn parse_overflow_anchor(value: &str) -> Option<OverflowAnchor> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(OverflowAnchor::Auto),
        "none" => Some(OverflowAnchor::None),
        _ => None,
    }
}

/// `overflow-clip-margin`: `<visual-box> || <length>` (al menos uno;
/// length >= 0). Si falta visual-box, default `padding-box`; si falta
/// length, default `0px`. `0px` solo (sin visual-box) emite `None`
/// (sin extensión). Fase 7.323.
pub(crate) fn parse_overflow_clip_margin(
    value: &str,
) -> Option<Option<OverflowClipMargin>> {
    let mut visual_box: Option<VisualBox> = None;
    let mut length: Option<f32> = None;
    for tok in value.trim().split_whitespace() {
        match tok.to_ascii_lowercase().as_str() {
            "content-box" => {
                if visual_box.is_some() {
                    return None;
                }
                visual_box = Some(VisualBox::ContentBox);
            }
            "padding-box" => {
                if visual_box.is_some() {
                    return None;
                }
                visual_box = Some(VisualBox::PaddingBox);
            }
            "border-box" => {
                if visual_box.is_some() {
                    return None;
                }
                visual_box = Some(VisualBox::BorderBox);
            }
            other => {
                if length.is_some() {
                    return None;
                }
                let n = parse_length_px(other)?;
                if n < 0.0 {
                    return None;
                }
                length = Some(n);
            }
        }
    }
    if visual_box.is_none() && length.is_none() {
        return None;
    }
    let len = length.unwrap_or(0.0);
    let vb = visual_box.unwrap_or(VisualBox::PaddingBox);
    // length=0 + visual_box=default → semánticamente equivalente a
    // “sin extensión”. Mantenemos `Some(...)` igualmente para preservar
    // la intención del autor; sólo emitimos `None` cuando el valor
    // explícito es justamente `0px` (sin visual-box) — eso lo deja
    // como un reset suave del shorthand.
    if visual_box.is_none() && len == 0.0 {
        return Some(None);
    }
    Some(Some(OverflowClipMargin { visual_box: vb, length: len }))
}

/// `text-align-last` (CSS Text 4):
/// `auto | start | end | left | right | center | justify`. Fase 7.324.
pub(crate) fn parse_text_align_last(value: &str) -> Option<TextAlignLast> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(TextAlignLast::Auto),
        "start" => Some(TextAlignLast::Start),
        "end" => Some(TextAlignLast::End),
        "left" => Some(TextAlignLast::Left),
        "right" => Some(TextAlignLast::Right),
        "center" => Some(TextAlignLast::Center),
        "justify" => Some(TextAlignLast::Justify),
        _ => None,
    }
}

/// `text-wrap` (CSS Text 4):
/// `wrap | nowrap | balance | pretty | stable`. Fase 7.325.
pub(crate) fn parse_text_wrap(value: &str) -> Option<TextWrap> {
    match value.trim().to_ascii_lowercase().as_str() {
        "wrap" => Some(TextWrap::Wrap),
        "nowrap" => Some(TextWrap::Nowrap),
        "balance" => Some(TextWrap::Balance),
        "pretty" => Some(TextWrap::Pretty),
        "stable" => Some(TextWrap::Stable),
        _ => None,
    }
}

/// `line-break` (CSS Text 3):
/// `auto | loose | normal | strict | anywhere`. Fase 7.326.
pub(crate) fn parse_line_break(value: &str) -> Option<LineBreak> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(LineBreak::Auto),
        "loose" => Some(LineBreak::Loose),
        "normal" => Some(LineBreak::Normal),
        "strict" => Some(LineBreak::Strict),
        "anywhere" => Some(LineBreak::Anywhere),
        _ => None,
    }
}

/// `hanging-punctuation` (CSS Text 4):
/// `none | [first || [force-end | allow-end] || last]`. Fase 7.327.
pub(crate) fn parse_hanging_punctuation(
    value: &str,
) -> Option<HangingPunctuation> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(HangingPunctuation::default());
    }
    let mut out = HangingPunctuation::default();
    for tok in v.split_whitespace() {
        match tok.to_ascii_lowercase().as_str() {
            "first" => {
                if out.first {
                    return None;
                }
                out.first = true;
            }
            "force-end" => {
                // `force-end` y `allow-end` se excluyen mutuamente.
                if out.force_end || out.allow_end {
                    return None;
                }
                out.force_end = true;
            }
            "allow-end" => {
                if out.force_end || out.allow_end {
                    return None;
                }
                out.allow_end = true;
            }
            "last" => {
                if out.last {
                    return None;
                }
                out.last = true;
            }
            _ => return None,
        }
    }
    if out.is_none() {
        return None;
    }
    Some(out)
}

/// `text-decoration-skip-ink` (CSS Text Decoration 4):
/// `auto | none | all`. Fase 7.328.
pub(crate) fn parse_text_decoration_skip_ink(
    value: &str,
) -> Option<TextDecorationSkipInk> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(TextDecorationSkipInk::Auto),
        "none" => Some(TextDecorationSkipInk::None),
        "all" => Some(TextDecorationSkipInk::All),
        _ => None,
    }
}

/// `font-optical-sizing`: `auto | none`. Fase 7.329.
pub(crate) fn parse_font_optical_sizing(
    value: &str,
) -> Option<FontOpticalSizing> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(FontOpticalSizing::Auto),
        "none" => Some(FontOpticalSizing::None),
        _ => None,
    }
}

/// `font-synthesis-{weight,style,small-caps}`: `auto | none`. Devuelve
/// `true` para `auto` (síntesis habilitada, default) y `false` para
/// `none`. Fases 7.330–7.332.
pub(crate) fn parse_auto_or_none(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(true),
        "none" => Some(false),
        _ => None,
    }
}

/// `font-size-adjust` (CSS Fonts 5):
/// `none | <number> | from-font | <metric> [<number>|from-font]`.
/// Si viene `<metric> <num>`, se modela como `Value(metric, num)`;
/// `<metric> from-font` ⇒ `FromFont(metric)`. `<num>` solo ⇒
/// `Value(ExHeight, num)`. Fase 7.334.
pub(crate) fn parse_font_size_adjust(value: &str) -> Option<FontSizeAdjust> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(FontSizeAdjust::None);
    }
    if v.eq_ignore_ascii_case("from-font") {
        return Some(FontSizeAdjust::FromFont(FontMetric::default()));
    }
    if let Ok(n) = v.parse::<f32>() {
        if n < 0.0 || !n.is_finite() {
            return None;
        }
        return Some(FontSizeAdjust::Value(FontMetric::default(), n));
    }
    let toks: Vec<&str> = v.split_whitespace().collect();
    if toks.len() != 2 {
        return None;
    }
    let metric = match toks[0].to_ascii_lowercase().as_str() {
        "ex-height" => FontMetric::ExHeight,
        "cap-height" => FontMetric::CapHeight,
        "ch-width" => FontMetric::ChWidth,
        "ic-width" => FontMetric::IcWidth,
        "ic-height" => FontMetric::IcHeight,
        _ => return None,
    };
    if toks[1].eq_ignore_ascii_case("from-font") {
        return Some(FontSizeAdjust::FromFont(metric));
    }
    let n = toks[1].parse::<f32>().ok()?;
    if n < 0.0 || !n.is_finite() {
        return None;
    }
    Some(FontSizeAdjust::Value(metric, n))
}

/// `image-orientation` (CSS Images 3):
/// `from-image | none | flip | <angle> [flip]?`. Acepta deg, rad,
/// grad, turn (la unidad se convierte a grados). Fase 7.335.
pub(crate) fn parse_image_orientation(value: &str) -> Option<ImageOrientation> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("from-image") {
        return Some(ImageOrientation::FromImage);
    }
    if v.eq_ignore_ascii_case("none") {
        return Some(ImageOrientation::None);
    }
    if v.eq_ignore_ascii_case("flip") {
        return Some(ImageOrientation::Angle { degrees: 0.0, flip: true });
    }
    let toks: Vec<&str> = v.split_whitespace().collect();
    let (angle_str, flip) = match toks.as_slice() {
        [a] => (*a, false),
        [a, b] if b.eq_ignore_ascii_case("flip") => (*a, true),
        // `flip <angle>` orden invertido también es válido.
        [a, b] if a.eq_ignore_ascii_case("flip") => (*b, true),
        _ => return None,
    };
    let degrees = parse_angle_degrees(angle_str)?;
    Some(ImageOrientation::Angle { degrees, flip })
}

/// `<angle>` → grados. Soporta `deg`, `rad`, `grad`, `turn`. Sin
/// unidad descarta (CSS exige unidad excepto cuando el valor es 0).
///
/// Fase 7.875 — acepta también `calc()`/min/max/clamp sobre ángulos: el
/// evaluador trata cada `<angle>` como `Number(grados)` (ver
/// `classify_calc_num`), así que un resultado `Number` ES los grados.
pub(crate) fn parse_angle_degrees(s: &str) -> Option<f32> {
    let t = s.trim();
    if is_math_fn(t) {
        return match eval_calc(t)? {
            CalcVal::Number(n) if n.is_finite() => Some(n),
            _ => None,
        };
    }
    if t == "0" {
        return Some(0.0);
    }
    let (num, unit) = if let Some(rest) = t.strip_suffix("deg") {
        (rest, "deg")
    } else if let Some(rest) = t.strip_suffix("rad") {
        (rest, "rad")
    } else if let Some(rest) = t.strip_suffix("grad") {
        (rest, "grad")
    } else if let Some(rest) = t.strip_suffix("turn") {
        (rest, "turn")
    } else {
        return None;
    };
    let n: f32 = num.parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    Some(match unit {
        "deg" => n,
        "rad" => n.to_degrees(),
        "grad" => n * 360.0 / 400.0,
        "turn" => n * 360.0,
        _ => unreachable!(),
    })
}

/// `place-items` shorthand. 1 token ⇒ aplica a los dos ejes; 2 tokens
/// ⇒ align luego justify. Fase 7.336.
pub(crate) fn parse_place_items(
    value: &str,
) -> Option<(AlignItems, AlignItems)> {
    let toks: Vec<&str> = value.trim().split_whitespace().collect();
    match toks.as_slice() {
        [a] => {
            let v = parse_align_items(a)?;
            Some((v, v))
        }
        [a, b] => Some((parse_align_items(a)?, parse_justify_items(b)?)),
        _ => None,
    }
}

/// `place-content` shorthand. Fase 7.337.
pub(crate) fn parse_place_content(
    value: &str,
) -> Option<(AlignContent, JustifyContent)> {
    let toks: Vec<&str> = value.trim().split_whitespace().collect();
    match toks.as_slice() {
        [a] => {
            // El 1er valor sirve para los dos ejes — pero AlignContent y
            // JustifyContent son enums distintos. Reusamos los parsers
            // de cada eje sobre el mismo token.
            Some((parse_align_content(a)?, parse_justify_content(a)?))
        }
        [a, b] => Some((parse_align_content(a)?, parse_justify_content(b)?)),
        _ => None,
    }
}

/// `place-self` shorthand. Fase 7.338.
pub(crate) fn parse_place_self(
    value: &str,
) -> Option<(AlignSelf, AlignSelf)> {
    let toks: Vec<&str> = value.trim().split_whitespace().collect();
    match toks.as_slice() {
        [a] => {
            let v = parse_align_self(a)?;
            Some((v, v))
        }
        [a, b] => Some((parse_align_self(a)?, parse_justify_self(b)?)),
        _ => None,
    }
}

/// `animation-timeline`: `auto | none | <dashed-ident> | scroll(...) |
/// view(...)`. Fase 7.339 + notaciones funcionales anónimas.
pub(crate) fn parse_timeline_ref(value: &str) -> Option<TimelineRef> {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    match lower.as_str() {
        "auto" => return Some(TimelineRef::Auto),
        "none" => return Some(TimelineRef::None),
        _ => {}
    }
    // `scroll([<scroller>] || [<axis>])` — orden libre, hasta 2 tokens.
    if let Some(inner) = fn_inner(&lower, "scroll") {
        let mut scroller = ScrollScroller::Nearest;
        let mut axis = TimelineAxis::Block;
        for tok in inner.split_whitespace() {
            match tok {
                "nearest" => scroller = ScrollScroller::Nearest,
                "root" => scroller = ScrollScroller::Root,
                "self" => scroller = ScrollScroller::SelfElement,
                _ => match parse_timeline_axis(tok) {
                    Some(a) => axis = a,
                    None => return None,
                },
            }
        }
        return Some(TimelineRef::Scroll { scroller, axis });
    }
    // `view([<axis>] || [<inset>])` — el primer token-axis fija el eje;
    // el resto (longitudes/`auto`) se guarda opaco como inset.
    if let Some(inner) = fn_inner(&lower, "view") {
        let mut axis = TimelineAxis::Block;
        let mut inset_toks: Vec<&str> = Vec::new();
        for tok in inner.split_whitespace() {
            match parse_timeline_axis(tok) {
                Some(a) => axis = a,
                None => inset_toks.push(tok),
            }
        }
        let inset = if inset_toks.is_empty() {
            None
        } else {
            Some(inset_toks.join(" "))
        };
        return Some(TimelineRef::View { axis, inset });
    }
    // `<custom-ident>` (validamos no-vacío y sin espacios internos —
    // el lexer ya separó por whitespace, pero filtramos por las dudas).
    if v.is_empty() || v.contains(char::is_whitespace) {
        return None;
    }
    Some(TimelineRef::Named(v.to_string()))
}

/// Si `v` es `name(<inner>)`, devuelve el `<inner>` recortado (puede ser
/// vacío para `view()`); si no, `None`.
fn fn_inner<'a>(v: &'a str, name: &str) -> Option<&'a str> {
    let rest = v.strip_prefix(name)?;
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.trim())
}

/// `scroll-timeline-name` / `view-timeline-name`: `none | <dashed-ident>`.
/// Fases 7.340, 7.342.
pub(crate) fn parse_dashed_ident_or_none(value: &str) -> Option<Option<String>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    if v.is_empty() || v.contains(char::is_whitespace) {
        return None;
    }
    Some(Some(v.to_string()))
}

/// `scroll-timeline-axis` / `view-timeline-axis`:
/// `block | inline | x | y`. Fases 7.341, 7.343.
pub(crate) fn parse_timeline_axis(value: &str) -> Option<TimelineAxis> {
    match value.trim().to_ascii_lowercase().as_str() {
        "block" => Some(TimelineAxis::Block),
        "inline" => Some(TimelineAxis::Inline),
        "x" => Some(TimelineAxis::X),
        "y" => Some(TimelineAxis::Y),
        _ => None,
    }
}

/// `masonry-auto-flow` (CSS Grid 3 draft): `[ pack | next ] ||
/// [ definite-first | ordered ]`. Orden libre, 1-2 tokens, sin repetir
/// componente.
pub(crate) fn parse_masonry_auto_flow(value: &str) -> Option<MasonryAutoFlow> {
    let mut placement: Option<MasonryPlacement> = None;
    let mut order: Option<MasonryOrder> = None;
    for tok in value.trim().to_ascii_lowercase().split_whitespace() {
        match tok {
            "pack" if placement.is_none() => placement = Some(MasonryPlacement::Pack),
            "next" if placement.is_none() => placement = Some(MasonryPlacement::Next),
            "definite-first" if order.is_none() => order = Some(MasonryOrder::DefiniteFirst),
            "ordered" if order.is_none() => order = Some(MasonryOrder::Ordered),
            _ => return None,
        }
    }
    if placement.is_none() && order.is_none() {
        return None; // valor vacío
    }
    Some(MasonryAutoFlow {
        placement: placement.unwrap_or_default(),
        order: order.unwrap_or_default(),
    })
}

/// `justify-tracks` (CSS Grid 3 draft): lista por coma de
/// `<content-distribution> | <content-position>`, uno por track de masonry.
/// Reusa `parse_justify_content`. Lista vacía → `None` (drop).
pub(crate) fn parse_justify_tracks(value: &str) -> Option<Vec<JustifyContent>> {
    let mut out = Vec::new();
    for item in value.split(',') {
        out.push(parse_justify_content(item.trim())?);
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// `align-tracks` (CSS Grid 3 draft): igual que `justify-tracks` pero sobre
/// el eje block. Reusa `parse_align_content`.
pub(crate) fn parse_align_tracks(value: &str) -> Option<Vec<AlignContent>> {
    let mut out = Vec::new();
    for item in value.split(',') {
        out.push(parse_align_content(item.trim())?);
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// `white-space-collapse`: `collapse | preserve | preserve-breaks |
/// break-spaces`. Fase 7.344.
pub(crate) fn parse_white_space_collapse(
    value: &str,
) -> Option<WhiteSpaceCollapse> {
    match value.trim().to_ascii_lowercase().as_str() {
        "collapse" => Some(WhiteSpaceCollapse::Collapse),
        "preserve" => Some(WhiteSpaceCollapse::Preserve),
        "preserve-breaks" => Some(WhiteSpaceCollapse::PreserveBreaks),
        "break-spaces" => Some(WhiteSpaceCollapse::BreakSpaces),
        _ => None,
    }
}

/// `text-wrap-mode`: `wrap | nowrap`. Fase 7.345.
pub(crate) fn parse_text_wrap_mode(value: &str) -> Option<TextWrapMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "wrap" => Some(TextWrapMode::Wrap),
        "nowrap" => Some(TextWrapMode::Nowrap),
        _ => None,
    }
}

/// `text-wrap-style`: `auto | balance | pretty | stable`. Fase 7.346.
pub(crate) fn parse_text_wrap_style(value: &str) -> Option<TextWrapStyle> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(TextWrapStyle::Auto),
        "balance" => Some(TextWrapStyle::Balance),
        "pretty" => Some(TextWrapStyle::Pretty),
        "stable" => Some(TextWrapStyle::Stable),
        _ => None,
    }
}

/// `text-spacing-trim`: `normal | space-all | space-first | trim-start`.
/// Fase 7.347.
pub(crate) fn parse_text_spacing_trim(value: &str) -> Option<TextSpacingTrim> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(TextSpacingTrim::Normal),
        "space-all" => Some(TextSpacingTrim::SpaceAll),
        "space-first" => Some(TextSpacingTrim::SpaceFirst),
        "trim-start" => Some(TextSpacingTrim::TrimStart),
        _ => None,
    }
}

/// `text-box-trim`: `none | trim-start | trim-end | trim-both`.
/// Fase 7.348.
pub(crate) fn parse_text_box_trim(value: &str) -> Option<TextBoxTrim> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Some(TextBoxTrim::None),
        "trim-start" => Some(TextBoxTrim::TrimStart),
        "trim-end" => Some(TextBoxTrim::TrimEnd),
        "trim-both" => Some(TextBoxTrim::TrimBoth),
        _ => None,
    }
}

/// `math-style`: `normal | compact`. Fase 7.349.
pub(crate) fn parse_math_style(value: &str) -> Option<MathStyle> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(MathStyle::Normal),
        "compact" => Some(MathStyle::Compact),
        _ => None,
    }
}

/// `math-depth`: `auto-add | add(<integer>) | <integer>`. Fase 7.350.
/// NOTA: `Add(n)` se preserva en el ComputedStyle sin resolverse contra
/// el heredado (la spec lo pide al cierre — TODO cuando haya layout
/// MathML real).
pub(crate) fn parse_math_depth(value: &str) -> Option<MathDepth> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto-add") {
        return Some(MathDepth::Auto);
    }
    if let Ok(n) = v.parse::<i32>() {
        return Some(MathDepth::Value(n));
    }
    let lower = v.to_ascii_lowercase();
    if let Some(inner) = lower.strip_prefix("add(").and_then(|s| s.strip_suffix(')')) {
        let n: i32 = inner.trim().parse().ok()?;
        return Some(MathDepth::Add(n));
    }
    None
}

/// `math-shift`: `normal | compact`. Fase 7.351.
pub(crate) fn parse_math_shift(value: &str) -> Option<MathShift> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(MathShift::Normal),
        "compact" => Some(MathShift::Compact),
        _ => None,
    }
}

/// `field-sizing`: `fixed | content`. Fase 7.352.
pub(crate) fn parse_field_sizing(value: &str) -> Option<FieldSizing> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fixed" => Some(FieldSizing::Fixed),
        "content" => Some(FieldSizing::Content),
        _ => None,
    }
}

/// `font-palette`: `normal | light | dark | <dashed-ident>`. Fase 7.359.
pub(crate) fn parse_font_palette(value: &str) -> Option<FontPalette> {
    let v = value.trim();
    match v.to_ascii_lowercase().as_str() {
        "normal" => Some(FontPalette::Normal),
        "light" => Some(FontPalette::Light),
        "dark" => Some(FontPalette::Dark),
        _ => {
            if v.is_empty() || v.contains(char::is_whitespace) {
                return None;
            }
            Some(FontPalette::Named(v.to_string()))
        }
    }
}

/// `font-variant-alternates` (subset MVP): `normal | historical-forms
/// || <funcname>(<ident>)+`. Soportamos `stylistic(--x)`, `styleset(...)`,
/// `character-variant(...)`, `swash(...)`, `ornaments(...)`,
/// `annotation(...)`. Fase 7.360.
pub(crate) fn parse_font_variant_alternates(
    value: &str,
) -> Option<FontVariantAlternates> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("normal") {
        return Some(FontVariantAlternates::default());
    }
    let mut out = FontVariantAlternates::default();
    for tok in split_top_level_ws(v) {
        let lower = tok.to_ascii_lowercase();
        if lower == "historical-forms" {
            if out.historical_forms {
                return None;
            }
            out.historical_forms = true;
            continue;
        }
        // `funcname(ident)` — el split top-level ws ya respeta paréntesis.
        if let Some(open) = tok.find('(') {
            if !tok.ends_with(')') {
                return None;
            }
            let name = tok[..open].to_ascii_lowercase();
            match name.as_str() {
                "stylistic" | "styleset" | "character-variant" | "swash"
                | "ornaments" | "annotation" => {}
                _ => return None,
            }
            let inner = &tok[open + 1..tok.len() - 1];
            let inner = inner.trim();
            if inner.is_empty() {
                return None;
            }
            out.functional.push((name, inner.to_string()));
            continue;
        }
        return None;
    }
    if out.is_normal() {
        return None;
    }
    Some(out)
}

