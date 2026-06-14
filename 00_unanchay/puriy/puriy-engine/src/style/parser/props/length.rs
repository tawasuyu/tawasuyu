use super::*;

/// Acepta `12px`, `1.5rem` (tratada como em*16), `0`. Sin unidad → px.
/// `Nvw`/`Nvh`/`Nvmin`/`Nvmax` resuelven contra el viewport activo
/// ([`resolve_viewport`]): el real bajo un `ViewportScope` (carga normal),
/// `DEFAULT_VIEWPORT` fuera de él (parsers sueltos en tests).
pub(crate) fn parse_length_px(s: &str) -> Option<f32> {
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
pub(crate) fn parse_length_or_pct_or_auto(s: &str) -> Option<LengthVal> {
    parse_length_or_pct(s.trim())
}

pub(crate) fn parse_position(s: &str) -> Option<Position> {
    match s.trim().to_ascii_lowercase().as_str() {
        "static" => Some(Position::Static),
        "relative" => Some(Position::Relative),
        "absolute" => Some(Position::Absolute),
        "fixed" => Some(Position::Fixed),
        // Fase 7.832 — `-webkit-sticky` alias vendor legacy (Safari) de `sticky`.
        "sticky" | "-webkit-sticky" => Some(Position::Sticky),
        _ => None,
    }
}

pub(crate) fn parse_vertical_align(s: &str) -> Option<VerticalAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        "baseline" => Some(VerticalAlign::Baseline),
        "top" | "text-top" => Some(VerticalAlign::Top),
        // Fase 7.842 — `-webkit-baseline-middle` (alias legacy WebKit) ≈ middle.
        "middle" | "-webkit-baseline-middle" => Some(VerticalAlign::Middle),
        "bottom" | "text-bottom" => Some(VerticalAlign::Bottom),
        "super" => Some(VerticalAlign::Super),
        "sub" => Some(VerticalAlign::Sub),
        _ => None,
    }
}

pub(crate) fn parse_visibility(s: &str) -> Option<Visibility> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Visibility::Visible),
        // `collapse` lo tratamos igual que hidden (sólo aplica a
        // tablas/flex en CSS spec, aproximación segura).
        "hidden" | "collapse" => Some(Visibility::Hidden),
        _ => None,
    }
}

pub(crate) fn parse_pointer_events(s: &str) -> Option<PointerEvents> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(PointerEvents::None),
        // Fase 7.841 — los valores SVG (`visiblePainted`/`painted`/`fill`/
        // `stroke`/`all`/`visible`…) significan todos "recibe eventos de
        // puntero"; en nuestro modelo binario (Auto|None) colapsan a Auto.
        "auto" | "all" | "visible" | "visiblepainted" | "visiblefill"
        | "visiblestroke" | "painted" | "fill" | "stroke" => Some(PointerEvents::Auto),
        _ => None,
    }
}

/// `object-fit: fill | contain | cover | none | scale-down`. Fase 7.230.
pub(crate) fn parse_object_fit(s: &str) -> Option<ObjectFit> {
    match s.trim().to_ascii_lowercase().as_str() {
        "fill" => Some(ObjectFit::Fill),
        "contain" => Some(ObjectFit::Contain),
        "cover" => Some(ObjectFit::Cover),
        "none" => Some(ObjectFit::None),
        "scale-down" => Some(ObjectFit::ScaleDown),
        _ => None,
    }
}

/// Indica que `cssparser` está enlazado aunque el subset actual no use
/// la API completa — la presencia del crate evita que `cargo` lo
/// pruebe y deja el camino abierto para Fase 3.
#[doc(hidden)]
pub fn _cssparser_anchor() {
    let _ = cssparser::ParserInput::new("");
}
