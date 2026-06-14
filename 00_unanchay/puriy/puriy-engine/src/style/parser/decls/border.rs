//! Shorthands y helpers de border (incluye match_border_side_prop / parse_logical_border que despachan ~80 props).
//! Value-parsers extraídos de `decls.rs` (regla #1). Lógica intacta.
use super::*;

/// Parsea el shorthand `border: <width> <style> <color>` (componentes en
/// cualquier orden). Devuelve hasta 3 decls. Si falta el style, se asume
/// `solid`. Cualquier "none" en la posición de style desactiva el border.
pub(crate) fn parse_border_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut width: Option<f32> = None;
    let mut color: Option<Color> = None;
    let mut current: bool = false;
    let mut style_on: Option<bool> = None;
    let mut line_style: Option<BorderLineStyle> = None;
    for tok in value.split_whitespace() {
        if let Some(w) = parse_length_px(tok) {
            width = Some(w);
            continue;
        }
        if is_current_color(tok) {
            current = true;
            continue;
        }
        if let Some(s) = parse_border_style(tok) {
            style_on = Some(s);
            // El patrón visual sólo aplica si el border queda activo.
            line_style = parse_border_line_style(tok);
            continue;
        }
        if let Some(c) = parse_color(tok) {
            color = Some(c);
            continue;
        }
    }
    // Defaults razonables: si hay width+color sin style, asumimos solid.
    if style_on.is_none() && (width.is_some() || color.is_some() || current) {
        style_on = Some(true);
    }
    let mut out = Vec::new();
    if let Some(on) = style_on {
        out.push(Decl { kind: DeclKind::BorderEnabled(on), important });
    }
    if let Some(ls) = line_style {
        out.push(Decl { kind: DeclKind::BorderStyleKind(ls), important });
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::BorderWidth(w), important });
    }
    if current {
        out.push(Decl { kind: DeclKind::CurrentColor(ColorTarget::BorderAll), important });
    } else if let Some(c) = color {
        out.push(Decl { kind: DeclKind::BorderColor(c), important });
    }
    out
}

/// `border-width` token: keywords `thin`/`medium`/`thick` (1/3/5px, los
/// valores de referencia de los browsers) o cualquier length/calc. Fase 7.837.
pub(crate) fn parse_border_width_token(s: &str) -> Option<f32> {
    match s.trim().to_ascii_lowercase().as_str() {
        "thin" => Some(1.0),
        "medium" => Some(3.0),
        "thick" => Some(5.0),
        other => parse_px_or_math(other),
    }
}

/// Expande 1-4 valores a los 4 lados con la regla TRBL de CSS (1→todos,
/// 2→vert/horiz, 3→top/horiz/bottom, 4→top/right/bottom/left). `f` parsea
/// cada token; si alguno falla → `None` (shorthand inválido, no parcial).
/// Fase 7.837.
pub(crate) fn expand_trbl_f32(
    toks: &[&str],
    f: impl Fn(&str) -> Option<f32>,
) -> Option<[(BorderEdge, f32); 4]> {
    let vals: Vec<f32> = toks.iter().filter_map(|t| f(t)).collect();
    if vals.is_empty() || vals.len() != toks.len() {
        return None;
    }
    let (t, r, b, l) = match vals.as_slice() {
        [a] => (*a, *a, *a, *a),
        [a, b] => (*a, *b, *a, *b),
        [a, b, c] => (*a, *b, *c, *b),
        [a, b, c, d] => (*a, *b, *c, *d),
        _ => return None,
    };
    Some([
        (BorderEdge::Top, t),
        (BorderEdge::Right, r),
        (BorderEdge::Bottom, b),
        (BorderEdge::Left, l),
    ])
}

/// Match propiedades `border-{top|right|bottom|left}{suffix}`. `suffix`
/// puede ser "" (shorthand), "-width", "-color", o "-style". Devuelve
/// el `BorderEdge` matcheado, o `None` si no aplica.
pub(crate) fn match_border_side_prop(prop: &str, suffix: &str) -> Option<BorderEdge> {
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

/// `border-radius: <1-4 horiz> [ / <1-4 vert> ]` (CSS Backgrounds 3).
/// Distribución de esquinas estilo spec: 1→todas; 2→TL+BR / TR+BL; 3→TL /
/// TR+BL / BR; 4→TL/TR/BR/BL. Devuelve las 4 decls `BorderCornerRadius`.
///
/// Divergencia: el modelo es de radio **circular** (un `f32` por esquina),
/// así que el componente vertical tras `/` se ignora (sólo se usa el
/// horizontal). Lista vacía si algún token horizontal no parsea. Fase 7.858.
pub(crate) fn parse_border_radius_shorthand(value: &str, important: bool) -> Vec<Decl> {
    // El eje vertical (tras `/`) no se modela: nos quedamos con el horizontal.
    let horiz = value.split('/').next().unwrap_or(value).trim();
    // Fase 7.877 — tokeniza respetando paréntesis y acepta calc por esquina.
    let vals: Vec<f32> = split_top_level_ws(horiz)
        .iter()
        .map(|t| parse_px_or_math(t))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    let (tl, tr, br, bl) = match vals.as_slice() {
        [a] => (*a, *a, *a, *a),
        [a, b] => (*a, *b, *a, *b),
        [a, b, c] => (*a, *b, *c, *b),
        [a, b, c, d] => (*a, *b, *c, *d),
        _ => return Vec::new(),
    };
    use BorderCorner::{BottomLeft, BottomRight, TopLeft, TopRight};
    [(TopLeft, tl), (TopRight, tr), (BottomRight, br), (BottomLeft, bl)]
        .into_iter()
        .map(|(corner, r)| Decl { kind: DeclKind::BorderCornerRadius(corner, r), important })
        .collect()
}

/// Match propiedades `border-{top|bottom}-{left|right}-radius` y sus
/// equivalentes lógicos `border-{start|end}-{start|end}-radius` (Fase
/// 7.409-7.412). En LTR horizontal: `block-start = top`, `block-end =
/// bottom`, `inline-start = left`, `inline-end = right`. El primer eje
/// es el block; el segundo, el inline (spec CSS Backgrounds 4).
pub(crate) fn match_border_corner_prop(prop: &str) -> Option<BorderCorner> {
    match prop.to_ascii_lowercase().as_str() {
        // Fase 7.812-7.815 — nombres viejos de esquina de Gecko `-moz-border-radius-<corner>`
        // (sin guiones entre palabras del corner) → mismas esquinas estándar.
        "border-top-left-radius" | "-moz-border-radius-topleft" => Some(BorderCorner::TopLeft),
        "border-top-right-radius" | "-moz-border-radius-topright" => Some(BorderCorner::TopRight),
        "border-bottom-right-radius" | "-moz-border-radius-bottomright" => Some(BorderCorner::BottomRight),
        "border-bottom-left-radius" | "-moz-border-radius-bottomleft" => Some(BorderCorner::BottomLeft),
        // Fase 7.409 — `border-start-start-radius` = block-start + inline-start = top-left.
        "border-start-start-radius" => Some(BorderCorner::TopLeft),
        // Fase 7.410 — `border-start-end-radius` = block-start + inline-end = top-right.
        "border-start-end-radius" => Some(BorderCorner::TopRight),
        // Fase 7.411 — `border-end-start-radius` = block-end + inline-start = bottom-left.
        "border-end-start-radius" => Some(BorderCorner::BottomLeft),
        // Fase 7.412 — `border-end-end-radius` = block-end + inline-end = bottom-right.
        "border-end-end-radius" => Some(BorderCorner::BottomRight),
        _ => None,
    }
}

/// Shorthand `border-top: <width> <style> <color>` (componentes en
/// cualquier orden, sólo afecta a un lado). Mismo formato que `border:`
/// pero las decls resultantes son las variantes per-side.
pub(crate) fn parse_border_side_shorthand(edge: BorderEdge, value: &str, important: bool) -> Vec<Decl> {
    let mut width: Option<f32> = None;
    let mut color: Option<Color> = None;
    let mut current: bool = false;
    let mut style_on: Option<bool> = None;
    for tok in value.split_whitespace() {
        if let Some(w) = parse_length_px(tok) {
            width = Some(w);
            continue;
        }
        if is_current_color(tok) {
            current = true;
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
    if style_on.is_none() && (width.is_some() || color.is_some() || current) {
        style_on = Some(true);
    }
    let mut out = Vec::new();
    if let Some(on) = style_on {
        out.push(Decl { kind: DeclKind::BorderSideStyle(edge, on), important });
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::BorderSideWidth(edge, w), important });
    }
    if current {
        out.push(Decl { kind: DeclKind::CurrentColor(ColorTarget::BorderSide(edge)), important });
    } else if let Some(c) = color {
        out.push(Decl { kind: DeclKind::BorderSideColor(edge, c), important });
    }
    out
}

/// Propiedades lógicas de borde → físicas (LTR + escritura horizontal):
/// `border-inline*` ↔ left/right, `border-block*` ↔ top/bottom; `-start` =
/// left/top, `-end` = right/bottom. Cubre el shorthand (`border-inline:`),
/// los de ambos lados por propiedad (`border-inline-width/-color/-style`),
/// los de un lado (`border-inline-start:`) y los longhands de un lado
/// (`border-inline-start-width`, etc.). Fase 7.193.
pub(crate) fn parse_logical_border(prop: &str, value: &str, important: bool) -> Option<Vec<Decl>> {
    let lc = prop.to_ascii_lowercase();
    let rest = lc.strip_prefix("border-")?;
    // (start, end) según el eje.
    let (axis, after) = if let Some(a) = rest.strip_prefix("inline") {
        ((BorderEdge::Left, BorderEdge::Right), a)
    } else if let Some(a) = rest.strip_prefix("block") {
        ((BorderEdge::Top, BorderEdge::Bottom), a)
    } else {
        return None;
    };
    // `after` aísla lado (`-start`/`-end`/ambos) y sub-propiedad.
    let (edges, sub): (Vec<BorderEdge>, &str) = if let Some(s) = after.strip_prefix("-start") {
        (vec![axis.0], s)
    } else if let Some(s) = after.strip_prefix("-end") {
        (vec![axis.1], s)
    } else {
        (vec![axis.0, axis.1], after)
    };
    // Fase 7.908 — los sub-shorthands `-width`/`-style`/`-color` de ambos
    // lados aceptan DOS valores (`border-inline-color: red blue` = start, end).
    // Repartimos token[i]→edge[i] cuando hay 2 tokens y 2 lados; si no, el
    // value entero va a cada lado. El shorthand pleno (`sub == ""`) NO se
    // parte (es `<width> || <style> || <color>`, no por-lado).
    let two_vals = if sub.is_empty() {
        None
    } else {
        let toks = split_top_level_ws(value.trim());
        (edges.len() == 2 && toks.len() == 2).then_some(toks)
    };
    let mut out = Vec::new();
    for (i, edge) in edges.iter().copied().enumerate() {
        let v = two_vals.as_ref().map_or(value, |t| t[i].as_str());
        match sub {
            "" => out.extend(parse_border_side_shorthand(edge, value, important)),
            "-width" => {
                if let Some(w) = parse_length_px(v) {
                    out.push(Decl { kind: DeclKind::BorderSideWidth(edge, w), important });
                }
            }
            "-color" => {
                if let Some(c) = parse_color(v) {
                    out.push(Decl { kind: DeclKind::BorderSideColor(edge, c), important });
                }
            }
            "-style" => {
                if let Some(s) = parse_border_style(v) {
                    out.push(Decl { kind: DeclKind::BorderSideStyle(edge, s), important });
                }
            }
            _ => return None, // sufijo desconocido → no es una lógica de borde
        }
    }
    Some(out)
}

