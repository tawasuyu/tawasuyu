//! Parsers de efectos/texto: touch-action, clip-path, mask, columns, tablas, breaks, variantes de fuente, énfasis.
//! Value-parsers extraídos de `decls.rs` (regla #1). Lógica intacta.
use super::*;

/// `touch-action`: `auto | none | manipulation | [ pan-x|pan-left|pan-right ]
/// || [ pan-y|pan-up|pan-down ] || pinch-zoom`. Los `pan-left/right/up/down`
/// se aplastan al eje correspondiente (no modelamos dirección por simplicidad
/// — la spec admite la combinación, pero el chrome tampoco las distingue
/// todavía). Fase 7.273.
pub(crate) fn parse_touch_action(value: &str) -> Option<TouchAction> {
    let v = value.trim().to_ascii_lowercase();
    if v == "auto" {
        return Some(TouchAction::Auto);
    }
    if v == "none" {
        return Some(TouchAction::None);
    }
    if v == "manipulation" {
        return Some(TouchAction::Manipulation);
    }
    let mut pan_x = false;
    let mut pan_y = false;
    let mut pinch_zoom = false;
    for tok in v.split_whitespace() {
        match tok {
            "pan-x" | "pan-left" | "pan-right" => pan_x = true,
            "pan-y" | "pan-up" | "pan-down" => pan_y = true,
            "pinch-zoom" => pinch_zoom = true,
            _ => return None,
        }
    }
    if !pan_x && !pan_y && !pinch_zoom {
        return None;
    }
    Some(TouchAction::Pan { pan_x, pan_y, pinch_zoom })
}

/// `clip-path` (subset). Acepta `none` (→ `None`), `inset(...)`,
/// `circle(...)`, `ellipse(...)`. Otras shapes (polygon/path) y URLs a
/// SVG quedan fuera por ahora. Fase 7.274.
pub(crate) fn parse_clip_path(value: &str) -> Option<ClipPath> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("none") {
        return None;
    }
    // `fn(args)` — extraer nombre + args separadamente.
    let (name, args) = split_fn_call(v)?;
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "inset" => parse_inset_shape(args),
        "circle" => parse_circle_shape(args),
        "ellipse" => parse_ellipse_shape(args),
        "polygon" => parse_polygon_shape(args),
        _ => None,
    }
}

/// `polygon([<fill-rule>,]? <x> <y> [, <x> <y>]*)`. `fill-rule` opcional
/// (`nonzero`/`evenodd`, default nonzero) seguido de coma; cada vértice es un
/// par `<x> <y>` de `<length-percentage>`, vértices separados por coma.
/// Requiere ≥1 vértice y que cada vértice tenga exactamente 2 coords.
fn parse_polygon_shape(args: &str) -> Option<ClipPath> {
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    let (evenodd, vertex_parts) = match parts.first() {
        Some(&"evenodd") => (true, &parts[1..]),
        Some(&"nonzero") => (false, &parts[1..]),
        _ => (false, &parts[..]),
    };
    if vertex_parts.is_empty() {
        return None;
    }
    let mut points = Vec::with_capacity(vertex_parts.len());
    for v in vertex_parts {
        let mut coords = v.split_whitespace();
        let x = parse_length_or_pct(coords.next()?)?;
        let y = parse_length_or_pct(coords.next()?)?;
        if coords.next().is_some() {
            return None; // más de 2 coords por vértice → inválido
        }
        points.push((x, y));
    }
    Some(ClipPath::Polygon { evenodd, points })
}

/// Recorta `name(args)` → `(name, args)`. Devuelve `None` si no hay `(`
/// o no cierra.
fn split_fn_call(s: &str) -> Option<(&str, &str)> {
    let s = s.trim();
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    if close <= open {
        return None;
    }
    Some((s[..open].trim(), s[open + 1..close].trim()))
}

/// `inset(<top> [<right> [<bottom> [<left>]]] [round <r>])`.
fn parse_inset_shape(args: &str) -> Option<ClipPath> {
    // Separar `round <r>` si existe.
    let (offsets_str, radius) = match args.find(" round ") {
        Some(idx) => (
            args[..idx].trim(),
            parse_length_px(args[idx + " round ".len()..].trim())?,
        ),
        None => (args, 0.0),
    };
    let parts: Vec<f32> = offsets_str
        .split_whitespace()
        .map(parse_length_px)
        .collect::<Option<Vec<_>>>()?;
    let (top, right, bottom, left) = match parts.as_slice() {
        [a] => (*a, *a, *a, *a),
        [v, h] => (*v, *h, *v, *h),
        [t, h, b] => (*t, *h, *b, *h),
        [t, r, b, l] => (*t, *r, *b, *l),
        _ => return None,
    };
    Some(ClipPath::Inset { top, right, bottom, left, radius })
}

/// Un radio de basic-shape: keyword de lado o `<length-percentage>`. Fase 7.1222.
fn parse_clip_radius(s: &str) -> Option<ClipRadius> {
    match s.trim().to_ascii_lowercase().as_str() {
        "closest-side" => Some(ClipRadius::ClosestSide),
        "farthest-side" => Some(ClipRadius::FarthestSide),
        other => parse_length_or_pct(other).map(ClipRadius::Len),
    }
}

/// `circle(<radius> [at <x> <y>])`. `radius` es `<length-percentage>` (un
/// `%` resuelve contra la diagonal de la caja, en el compositor) o
/// `closest-side`/`farthest-side`; centro default 50%/50%. Vacío →
/// `closest-side` (el default de la spec).
fn parse_circle_shape(args: &str) -> Option<ClipPath> {
    let (radius_str, center) = match args.find(" at ") {
        Some(idx) => (args[..idx].trim(), args[idx + " at ".len()..].trim()),
        None => (args, ""),
    };
    let radius = if radius_str.is_empty() {
        ClipRadius::ClosestSide
    } else {
        parse_clip_radius(radius_str)?
    };
    let (cx, cy) = parse_center(center);
    Some(ClipPath::Circle { radius, cx, cy })
}

/// `ellipse(<rx> <ry> [at <x> <y>])`. `rx`/`ry` son `<length-percentage>`
/// (`%`→ancho/alto) o keywords de lado. Sin radios → ambos `closest-side`.
fn parse_ellipse_shape(args: &str) -> Option<ClipPath> {
    let (radii_str, center) = match args.find(" at ") {
        Some(idx) => (args[..idx].trim(), args[idx + " at ".len()..].trim()),
        None => (args, ""),
    };
    let (rx, ry) = if radii_str.is_empty() {
        (ClipRadius::ClosestSide, ClipRadius::ClosestSide)
    } else {
        let mut tokens = radii_str.split_whitespace();
        let rx = parse_clip_radius(tokens.next()?)?;
        let ry = parse_clip_radius(tokens.next()?)?;
        (rx, ry)
    };
    let (cx, cy) = parse_center(center);
    Some(ClipPath::Ellipse { rx, ry, cx, cy })
}

/// `<x> <y>` para el centro de `circle()`/`ellipse()`. Default
/// `50% 50%` (sólo `LengthVal`; sin keywords por ahora).
fn parse_center(s: &str) -> (LengthVal, LengthVal) {
    let mut tokens = s.split_whitespace();
    let cx = tokens
        .next()
        .and_then(parse_length_or_pct)
        .unwrap_or(LengthVal::Pct(50.0));
    let cy = tokens
        .next()
        .and_then(parse_length_or_pct)
        .unwrap_or(LengthVal::Pct(50.0));
    (cx, cy)
}

/// `mask-image` (subset). Sólo `url(...)`. Fase 7.275.
pub(crate) fn parse_mask_image(value: &str) -> Option<MaskImage> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("none") {
        return None;
    }
    let lower = v.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("url(") {
        // Recorta el `)` final del value ORIGINAL para preservar case
        // del URL (puede ser case-sensitive).
        let close = v.rfind(')')?;
        let inner = v[lower.len() - rest.len()..close].trim();
        // Quitar comillas (simples o dobles) si las hay.
        let inner = inner
            .trim_start_matches(['"', '\''])
            .trim_end_matches(['"', '\'']);
        if inner.is_empty() {
            return None;
        }
        return Some(MaskImage::Url(inner.to_string()));
    }
    None
}

/// `content-visibility`: `visible | auto | hidden`. Fase 7.276.
pub(crate) fn parse_content_visibility(value: &str) -> Option<ContentVisibility> {
    match value.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(ContentVisibility::Visible),
        "auto" => Some(ContentVisibility::Auto),
        "hidden" => Some(ContentVisibility::Hidden),
        _ => None,
    }
}

/// `contain`: `none | strict | content | [size||layout||style||paint||inline-size]`.
/// Fase 7.277.
pub(crate) fn parse_contain(value: &str) -> Option<ContainFlags> {
    let v = value.trim().to_ascii_lowercase();
    if v == "none" {
        return Some(ContainFlags::default());
    }
    if v == "strict" {
        return Some(ContainFlags::STRICT);
    }
    if v == "content" {
        return Some(ContainFlags::CONTENT);
    }
    let mut flags = ContainFlags::default();
    for tok in v.split_whitespace() {
        match tok {
            "size" => flags.size = true,
            "inline-size" => flags.inline_size = true,
            "layout" => flags.layout = true,
            "style" => flags.style = true,
            "paint" => flags.paint = true,
            _ => return None,
        }
    }
    if flags.is_none() {
        return None;
    }
    Some(flags)
}

/// `column-count`: `auto` → `None`; entero positivo → `Some(n)`. Fase 7.278.
pub(crate) fn parse_column_count(value: &str) -> Option<u32> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") {
        return None;
    }
    v.parse::<u32>().ok().filter(|n| *n > 0)
}

/// Eje de una regla de hueco (CSS Gap Decorations 1). `Both` cubre el
/// shorthand `rule`, que fija filas y columnas a la vez. Fase 7.920.
#[derive(Clone, Copy)]
pub(crate) enum RuleAxis {
    Column,
    Row,
}

fn rule_width_decl(axis: RuleAxis, w: f32) -> DeclKind {
    match axis {
        RuleAxis::Column => DeclKind::ColumnRuleWidth(w),
        RuleAxis::Row => DeclKind::RowRuleWidth(w),
    }
}
fn rule_color_decl(axis: RuleAxis, c: Option<Color>) -> DeclKind {
    match axis {
        RuleAxis::Column => DeclKind::ColumnRuleColor(c),
        RuleAxis::Row => DeclKind::RowRuleColor(c),
    }
}
pub(crate) fn rule_style_active_decl(axis: RuleAxis, on: bool) -> DeclKind {
    match axis {
        RuleAxis::Column => DeclKind::ColumnRuleStyleActive(on),
        RuleAxis::Row => DeclKind::RowRuleStyleActive(on),
    }
}
pub(crate) fn rule_style_pattern_decl(axis: RuleAxis, ls: BorderLineStyle) -> DeclKind {
    match axis {
        RuleAxis::Column => DeclKind::ColumnRuleStylePattern(ls),
        RuleAxis::Row => DeclKind::RowRuleStylePattern(ls),
    }
}

/// `column-rule` shorthand: `<width> <style> <color>` (orden libre,
/// igual que `outline`). Fase 7.280.
pub(crate) fn parse_column_rule_shorthand(value: &str, important: bool) -> Vec<Decl> {
    parse_rule_shorthand(value, important, &[RuleAxis::Column])
}

/// Shorthand de regla de hueco genérico, emitido sobre uno o varios ejes.
/// `column-rule` → `[Column]`, `row-rule` → `[Row]`, `rule` → ambos. Misma
/// gramática `<width> || <style> || <color>` (orden libre). Fase 7.920.
pub(crate) fn parse_rule_shorthand(value: &str, important: bool, axes: &[RuleAxis]) -> Vec<Decl> {
    let mut width: Option<f32> = None;
    let mut color: Option<Color> = None;
    let mut current: bool = false;
    let mut style_active: Option<bool> = None;
    let mut line_style: Option<BorderLineStyle> = None;
    for tok in value.split_whitespace() {
        if !current && color.is_none() && is_current_color(tok) {
            current = true;
            continue;
        }
        if width.is_none() {
            if let Some(w) = parse_length_px(tok) {
                width = Some(w);
                continue;
            }
        }
        if style_active.is_none() {
            if let Some(active) = parse_border_style(tok) {
                style_active = Some(active);
                line_style = parse_border_line_style(tok);
                continue;
            }
        }
        if color.is_none() {
            if let Some(c) = parse_color(tok) {
                color = Some(c);
                continue;
            }
        }
    }
    let mut out = Vec::new();
    let active = style_active.unwrap_or(true);
    if !active {
        for &ax in axes {
            out.push(Decl { kind: rule_style_active_decl(ax, false), important });
        }
        return out;
    }
    for &ax in axes {
        if let Some(w) = width {
            out.push(Decl { kind: rule_width_decl(ax, w), important });
        }
        if current {
            out.push(Decl { kind: rule_color_decl(ax, None), important });
        } else if let Some(c) = color {
            out.push(Decl { kind: rule_color_decl(ax, Some(c)), important });
        }
        if style_active.is_some() {
            out.push(Decl { kind: rule_style_active_decl(ax, true), important });
        }
        if let Some(ls) = line_style {
            out.push(Decl { kind: rule_style_pattern_decl(ax, ls), important });
        }
    }
    out
}

/// `column-fill`: `auto | balance | balance-all`. Fase 7.281.
pub(crate) fn parse_column_fill(value: &str) -> Option<ColumnFill> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ColumnFill::Auto),
        "balance" => Some(ColumnFill::Balance),
        "balance-all" => Some(ColumnFill::BalanceAll),
        _ => None,
    }
}

/// `column-span`: `none | all`. Fase 7.282.
pub(crate) fn parse_column_span(value: &str) -> Option<ColumnSpan> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Some(ColumnSpan::None),
        "all" => Some(ColumnSpan::All),
        _ => None,
    }
}

/// `table-layout`: `auto | fixed`. Fase 7.284.
pub(crate) fn parse_table_layout(value: &str) -> Option<TableLayout> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(TableLayout::Auto),
        "fixed" => Some(TableLayout::Fixed),
        _ => None,
    }
}

/// `border-collapse`: `separate | collapse`. Fase 7.285.
pub(crate) fn parse_border_collapse(value: &str) -> Option<BorderCollapse> {
    match value.trim().to_ascii_lowercase().as_str() {
        "separate" => Some(BorderCollapse::Separate),
        "collapse" => Some(BorderCollapse::Collapse),
        _ => None,
    }
}

/// `border-spacing`: `<h-length> [<v-length>]`. Sin v explícito, v=h.
/// Fase 7.286.
pub(crate) fn parse_border_spacing(value: &str) -> Option<(f32, f32)> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let parsed: Vec<f32> = parts
        .iter()
        .map(|t| parse_length_px(t))
        .collect::<Option<Vec<_>>>()?;
    match parsed.as_slice() {
        [h] => Some((*h, *h)),
        [h, v] => Some((*h, *v)),
        _ => None,
    }
}

/// `caption-side`: `top | bottom | inline-start | inline-end`. Logicals
/// se aplastan a Top/Bottom (LTR/RTL no diferencia para captions en
/// tablas horizontales). Fase 7.287.
pub(crate) fn parse_caption_side(value: &str) -> Option<CaptionSide> {
    match value.trim().to_ascii_lowercase().as_str() {
        "top" | "block-start" | "inline-start" => Some(CaptionSide::Top),
        "bottom" | "block-end" | "inline-end" => Some(CaptionSide::Bottom),
        _ => None,
    }
}

/// `empty-cells`: `show | hide`. Fase 7.288.
pub(crate) fn parse_empty_cells(value: &str) -> Option<EmptyCells> {
    match value.trim().to_ascii_lowercase().as_str() {
        "show" => Some(EmptyCells::Show),
        "hide" => Some(EmptyCells::Hide),
        _ => None,
    }
}

/// `break-before` / `break-after`: superset que cubre el legacy
/// `page-break-*` (auto/avoid/always/left/right) y los valores nuevos
/// (page/recto/verso/column/region + variantes avoid-*). Fase 7.289 / 7.290.
pub(crate) fn parse_break_between(value: &str) -> Option<BreakBetween> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(BreakBetween::Auto),
        "avoid" => Some(BreakBetween::Avoid),
        "always" => Some(BreakBetween::Always),
        "avoid-page" => Some(BreakBetween::AvoidPage),
        "page" => Some(BreakBetween::Page),
        "left" => Some(BreakBetween::Left),
        "right" => Some(BreakBetween::Right),
        "recto" => Some(BreakBetween::Recto),
        "verso" => Some(BreakBetween::Verso),
        "avoid-column" => Some(BreakBetween::AvoidColumn),
        "column" => Some(BreakBetween::Column),
        "avoid-region" => Some(BreakBetween::AvoidRegion),
        "region" => Some(BreakBetween::Region),
        _ => None,
    }
}

/// Entero positivo (>= 1). Para `orphans` y `widows`. Fase 7.291 / 7.292.
pub(crate) fn parse_positive_int(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok().filter(|n| *n >= 1)
}

/// `color-scheme: normal | [light||dark] [only]?`. Tokens duplicados o
/// desconocidos descartan la declaración. Fase 7.293.
pub(crate) fn parse_color_scheme(value: &str) -> Option<ColorScheme> {
    let v = value.trim().to_ascii_lowercase();
    if v == "normal" {
        return Some(ColorScheme::NORMAL);
    }
    let mut cs = ColorScheme { light: false, dark: false, only: false };
    for tok in v.split_whitespace() {
        match tok {
            "light" => {
                if cs.light {
                    return None;
                }
                cs.light = true;
            }
            "dark" => {
                if cs.dark {
                    return None;
                }
                cs.dark = true;
            }
            "only" => {
                if cs.only {
                    return None;
                }
                cs.only = true;
            }
            _ => return None,
        }
    }
    // `only` por sí solo no aporta nada; necesita al menos un esquema.
    if cs.only && !cs.light && !cs.dark {
        return None;
    }
    if !cs.light && !cs.dark && !cs.only {
        return None;
    }
    Some(cs)
}

/// `list-style-position`: `inside | outside`. Fase 7.294.
pub(crate) fn parse_list_style_position(value: &str) -> Option<ListStylePosition> {
    match value.trim().to_ascii_lowercase().as_str() {
        "outside" => Some(ListStylePosition::Outside),
        "inside" => Some(ListStylePosition::Inside),
        _ => None,
    }
}

/// `list-style-image`: `none | url(...)` (subset). Comparte el shape con
/// `mask-image`; el resto de generated images (linear-gradient, etc.)
/// quedan fuera. Fase 7.295.
pub(crate) fn parse_list_style_image(value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("none") {
        return None;
    }
    let lower = v.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("url(") {
        let close = v.rfind(')')?;
        let inner = v[lower.len() - rest.len()..close].trim();
        let inner = inner
            .trim_start_matches(['"', '\''])
            .trim_end_matches(['"', '\'']);
        if inner.is_empty() {
            return None;
        }
        return Some(inner.to_string());
    }
    None
}

/// `list-style` shorthand (Fase 7.296): orden libre de `<type>`,
/// `<position>`, `<image>`. `none` (la primera ocurrencia) marca type=None
/// + image=None.
pub(crate) fn parse_list_style_shorthand_full(value: &str, important: bool) -> Vec<Decl> {
    let v = value.trim();
    if v.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut found_type = false;
    let mut found_position = false;
    let mut found_image = false;
    let mut none_count = 0usize;
    for tok in v.split_whitespace() {
        let tok_lower = tok.to_ascii_lowercase();
        if tok_lower == "none" {
            none_count += 1;
            continue;
        }
        if !found_position {
            if let Some(p) = parse_list_style_position(tok) {
                out.push(Decl { kind: DeclKind::ListStylePosition(p), important });
                found_position = true;
                continue;
            }
        }
        if !found_image && tok_lower.starts_with("url(") {
            if let Some(u) = parse_list_style_image(tok) {
                out.push(Decl { kind: DeclKind::ListStyleImage(Some(u)), important });
                found_image = true;
                continue;
            }
        }
        if !found_type {
            if let Some(t) = parse_list_style_type(tok) {
                out.push(Decl { kind: DeclKind::ListStyleType(t), important });
                found_type = true;
                continue;
            }
        }
    }
    // `none` aplica a type+image (un solo `none` apaga ambos; dos `none`
    // también pero el efecto es el mismo).
    if none_count >= 1 {
        if !found_type {
            out.push(Decl { kind: DeclKind::ListStyleType(ListStyleType::None), important });
        }
        if !found_image {
            out.push(Decl { kind: DeclKind::ListStyleImage(None), important });
        }
    }
    out
}

/// `quotes`: `auto | none | <pair>+` donde cada par es `<string> <string>`.
/// Fase 7.298.
pub(crate) fn parse_quotes(value: &str) -> Quotes {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") || v.is_empty() {
        return Quotes::Auto;
    }
    if v.eq_ignore_ascii_case("none") {
        return Quotes::None;
    }
    // Recortar pares de strings sucesivos: "«" "»" "‹" "›".
    let mut strings = Vec::new();
    let bytes = v.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i] as char;
        if quote != '"' && quote != '\'' {
            // Token no-string: descartar todo (fallback a Auto).
            return Quotes::Auto;
        }
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] as char != quote {
            i += 1;
        }
        if i >= bytes.len() {
            return Quotes::Auto;
        }
        strings.push(v[start..i].to_string());
        i += 1;
    }
    if strings.is_empty() || strings.len() % 2 != 0 {
        return Quotes::Auto;
    }
    let mut pairs = Vec::with_capacity(strings.len() / 2);
    let mut it = strings.into_iter();
    while let (Some(open), Some(close)) = (it.next(), it.next()) {
        pairs.push((open, close));
    }
    Quotes::Pairs(pairs)
}

/// `text-underline-position`: `auto | from-font | under | left | right`.
/// Fase 7.299.
pub(crate) fn parse_text_underline_position(value: &str) -> Option<TextUnderlinePosition> {
    let lc = value.trim().to_ascii_lowercase();
    match lc.as_str() {
        "auto" => return Some(TextUnderlinePosition::Auto),
        "from-font" => return Some(TextUnderlinePosition::FromFont),
        "under" => return Some(TextUnderlinePosition::Under),
        "left" => return Some(TextUnderlinePosition::Left),
        "right" => return Some(TextUnderlinePosition::Right),
        _ => {}
    }
    // Fase 7.909 — forma de dos valores `under || [ left | right ]` (CSS Text
    // 4, p.ej. texto vertical CJK). `left`/`right` son mutuamente excluyentes
    // (`left right` es inválido → drop). El modelo es enum de un valor:
    // priorizamos el eje left/right cuando está presente (pierde el `under`).
    let toks: Vec<&str> = lc.split_whitespace().collect();
    if toks.len() == 2 {
        let has_under = toks.contains(&"under");
        let left = toks.contains(&"left");
        let right = toks.contains(&"right");
        if has_under && left {
            return Some(TextUnderlinePosition::Left);
        }
        if has_under && right {
            return Some(TextUnderlinePosition::Right);
        }
    }
    None
}

/// `text-justify`: `auto | none | inter-word | inter-character | distribute`.
/// `distribute` (legacy) se mantiene como variante separada por compat.
/// Fase 7.300.
pub(crate) fn parse_text_justify(value: &str) -> Option<TextJustify> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(TextJustify::Auto),
        "none" => Some(TextJustify::None),
        "inter-word" => Some(TextJustify::InterWord),
        "inter-character" => Some(TextJustify::InterCharacter),
        "distribute" => Some(TextJustify::Distribute),
        _ => None,
    }
}

/// `print-color-adjust` / `color-adjust`: `economy | exact`. Fase 7.301.
pub(crate) fn parse_print_color_adjust(value: &str) -> Option<PrintColorAdjust> {
    match value.trim().to_ascii_lowercase().as_str() {
        "economy" => Some(PrintColorAdjust::Economy),
        "exact" => Some(PrintColorAdjust::Exact),
        _ => None,
    }
}

/// `forced-color-adjust`: `auto | none | preserve-parent-color` (subset).
/// Fase 7.302.
pub(crate) fn parse_forced_color_adjust(value: &str) -> Option<ForcedColorAdjust> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ForcedColorAdjust::Auto),
        "none" => Some(ForcedColorAdjust::None),
        "preserve" | "preserve-parent-color" => Some(ForcedColorAdjust::Preserve),
        _ => None,
    }
}

/// `line-clamp` / `-webkit-line-clamp`: `none | <integer>` positivo.
/// Fase 7.303.
pub(crate) fn parse_line_clamp(value: &str) -> Option<u32> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return None;
    }
    v.parse::<u32>().ok().filter(|n| *n >= 1)
}

/// `font-variant-caps`: 7 valores enum. Fase 7.304.
pub(crate) fn parse_font_variant_caps(value: &str) -> Option<FontVariantCaps> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(FontVariantCaps::Normal),
        "small-caps" => Some(FontVariantCaps::SmallCaps),
        "all-small-caps" => Some(FontVariantCaps::AllSmallCaps),
        "petite-caps" => Some(FontVariantCaps::PetiteCaps),
        "all-petite-caps" => Some(FontVariantCaps::AllPetiteCaps),
        "unicase" => Some(FontVariantCaps::Unicase),
        "titling-caps" => Some(FontVariantCaps::TitlingCaps),
        _ => None,
    }
}

/// `font-variant-numeric`: `normal | <bitset>`. Token desconocido o
/// combinación inválida (proportional+tabular, lining+oldstyle,
/// diagonal+stacked) descarta la regla. Fase 7.305.
pub(crate) fn parse_font_variant_numeric(value: &str) -> Option<FontVariantNumeric> {
    let v = value.trim().to_ascii_lowercase();
    if v == "normal" {
        return Some(FontVariantNumeric::default());
    }
    let mut n = FontVariantNumeric::default();
    for tok in v.split_whitespace() {
        match tok {
            "lining-nums" => n.lining_nums = true,
            "oldstyle-nums" => n.oldstyle_nums = true,
            "proportional-nums" => n.proportional_nums = true,
            "tabular-nums" => n.tabular_nums = true,
            "diagonal-fractions" => n.diagonal_fractions = true,
            "stacked-fractions" => n.stacked_fractions = true,
            "ordinal" => n.ordinal = true,
            "slashed-zero" => n.slashed_zero = true,
            _ => return None,
        }
    }
    // Mutuamente excluyentes — la spec lo dice y los browsers descartan.
    if n.lining_nums && n.oldstyle_nums {
        return None;
    }
    if n.proportional_nums && n.tabular_nums {
        return None;
    }
    if n.diagonal_fractions && n.stacked_fractions {
        return None;
    }
    Some(n)
}

/// `font-variant-ligatures`: `normal | none | <bitset>`. Fase 7.306.
pub(crate) fn parse_font_variant_ligatures(value: &str) -> Option<FontVariantLigatures> {
    let v = value.trim().to_ascii_lowercase();
    if v == "normal" {
        return Some(FontVariantLigatures::Normal);
    }
    if v == "none" {
        return Some(FontVariantLigatures::None);
    }
    let mut l = LigatureSet::default();
    for tok in v.split_whitespace() {
        match tok {
            "common-ligatures" => l.common_ligatures = true,
            "no-common-ligatures" => l.no_common_ligatures = true,
            "discretionary-ligatures" => l.discretionary_ligatures = true,
            "no-discretionary-ligatures" => l.no_discretionary_ligatures = true,
            "historical-ligatures" => l.historical_ligatures = true,
            "no-historical-ligatures" => l.no_historical_ligatures = true,
            "contextual" => l.contextual = true,
            "no-contextual" => l.no_contextual = true,
            _ => return None,
        }
    }
    // Cada par on/off es mutuamente excluyente.
    if l.common_ligatures && l.no_common_ligatures {
        return None;
    }
    if l.discretionary_ligatures && l.no_discretionary_ligatures {
        return None;
    }
    if l.historical_ligatures && l.no_historical_ligatures {
        return None;
    }
    if l.contextual && l.no_contextual {
        return None;
    }
    Some(FontVariantLigatures::Custom(l))
}

/// `font-variant-east-asian`: `normal | <bitset>` con grupos
/// mutuamente excluyentes. Fase 7.307.
pub(crate) fn parse_font_variant_east_asian(value: &str) -> Option<FontVariantEastAsian> {
    let v = value.trim().to_ascii_lowercase();
    if v == "normal" {
        return Some(FontVariantEastAsian::default());
    }
    let mut e = FontVariantEastAsian::default();
    for tok in v.split_whitespace() {
        match tok {
            "jis78" => e.jis78 = true,
            "jis83" => e.jis83 = true,
            "jis90" => e.jis90 = true,
            "jis04" => e.jis04 = true,
            "simplified" => e.simplified = true,
            "traditional" => e.traditional = true,
            "full-width" => e.full_width = true,
            "proportional-width" => e.proportional_width = true,
            "ruby" => e.ruby = true,
            _ => return None,
        }
    }
    // JIS78/83/90/04/simplified/traditional mutuamente excluyentes.
    let jis_count = (e.jis78 as u32) + (e.jis83 as u32) + (e.jis90 as u32) + (e.jis04 as u32)
        + (e.simplified as u32) + (e.traditional as u32);
    if jis_count > 1 {
        return None;
    }
    // full-width vs proportional-width también.
    if e.full_width && e.proportional_width {
        return None;
    }
    Some(e)
}

/// `font-variant-position`: `normal | sub | super`. Fase 7.308.
pub(crate) fn parse_font_variant_position(value: &str) -> Option<FontVariantPosition> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(FontVariantPosition::Normal),
        "sub" => Some(FontVariantPosition::Sub),
        "super" => Some(FontVariantPosition::Super),
        _ => None,
    }
}

/// `text-emphasis-style` (CSS Text Decoration 4). Acepta `none`, un
/// string quoted (`"x"`), o la combinación `[filled|open] && [dot|...]`.
/// Si sólo se da el fill o sólo la shape, los otros caen al default.
/// Fase 7.309.
pub(crate) fn parse_text_emphasis_style(value: &str) -> Option<TextEmphasisStyle> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(TextEmphasisStyle::None);
    }
    // String literal.
    if let Some(rest) = v.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(TextEmphasisStyle::Custom(rest[..end].to_string()));
    }
    if let Some(rest) = v.strip_prefix('\'') {
        let end = rest.find('\'')?;
        return Some(TextEmphasisStyle::Custom(rest[..end].to_string()));
    }
    let lower = v.to_ascii_lowercase();
    let mut fill: Option<TextEmphasisFill> = None;
    let mut shape: Option<TextEmphasisShape> = None;
    for tok in lower.split_whitespace() {
        match tok {
            "filled" => {
                if fill.is_some() {
                    return None;
                }
                fill = Some(TextEmphasisFill::Filled);
            }
            "open" => {
                if fill.is_some() {
                    return None;
                }
                fill = Some(TextEmphasisFill::Open);
            }
            "dot" => {
                if shape.is_some() {
                    return None;
                }
                shape = Some(TextEmphasisShape::Dot);
            }
            "circle" => {
                if shape.is_some() {
                    return None;
                }
                shape = Some(TextEmphasisShape::Circle);
            }
            "double-circle" => {
                if shape.is_some() {
                    return None;
                }
                shape = Some(TextEmphasisShape::DoubleCircle);
            }
            "triangle" => {
                if shape.is_some() {
                    return None;
                }
                shape = Some(TextEmphasisShape::Triangle);
            }
            "sesame" => {
                if shape.is_some() {
                    return None;
                }
                shape = Some(TextEmphasisShape::Sesame);
            }
            _ => return None,
        }
    }
    if fill.is_none() && shape.is_none() {
        return None;
    }
    Some(TextEmphasisStyle::Mark {
        fill: fill.unwrap_or_default(),
        shape: shape.unwrap_or_default(),
    })
}

/// `text-emphasis-position`: `[over|under] && [right|left]`. Si falta
/// el lado, default `right`; si falta el eje, default `over`. Fase 7.311.
pub(crate) fn parse_text_emphasis_position(value: &str) -> Option<TextEmphasisPosition> {
    let v = value.trim().to_ascii_lowercase();
    // Fase 7.931 — `auto` (CSS Text Decor 4): la UA decide; lo aproximamos al
    // default `over right`.
    if v == "auto" {
        return Some(TextEmphasisPosition { over: true, right: true });
    }
    let mut over: Option<bool> = None;
    let mut right: Option<bool> = None;
    for tok in v.split_whitespace() {
        match tok {
            "over" => {
                if over.is_some() {
                    return None;
                }
                over = Some(true);
            }
            "under" => {
                if over.is_some() {
                    return None;
                }
                over = Some(false);
            }
            "right" => {
                if right.is_some() {
                    return None;
                }
                right = Some(true);
            }
            "left" => {
                if right.is_some() {
                    return None;
                }
                right = Some(false);
            }
            _ => return None,
        }
    }
    if over.is_none() && right.is_none() {
        return None;
    }
    Some(TextEmphasisPosition {
        over: over.unwrap_or(true),
        right: right.unwrap_or(true),
    })
}

/// `text-emphasis` shorthand: `<style> || <color>`. Tokens en orden libre.
/// Fase 7.312.
pub(crate) fn parse_text_emphasis_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let v = value.trim();
    let mut out = Vec::new();
    if v.eq_ignore_ascii_case("none") {
        out.push(Decl {
            kind: DeclKind::TextEmphasisStyle(TextEmphasisStyle::None),
            important,
        });
        return out;
    }
    // Separar el primer color (si lo hay) y dejar el resto para style.
    // `text-emphasis: filled red` o `text-emphasis: "x" blue`. Buscamos
    // un color al final por simplicidad.
    let tokens: Vec<&str> = v.split_whitespace().collect();
    if tokens.is_empty() {
        return out;
    }
    // Probar último token como color.
    let mut style_str = v.to_string();
    let mut color_set = false;
    if let Some(last) = tokens.last() {
        if is_current_color(last) {
            out.push(Decl { kind: DeclKind::TextEmphasisColor(None), important });
            style_str = tokens[..tokens.len() - 1].join(" ");
            color_set = true;
        } else if let Some(c) = parse_color(last) {
            out.push(Decl {
                kind: DeclKind::TextEmphasisColor(Some(c)),
                important,
            });
            style_str = tokens[..tokens.len() - 1].join(" ");
            color_set = true;
        }
    }
    let _ = color_set;
    let style_str = style_str.trim();
    if !style_str.is_empty() {
        if let Some(st) = parse_text_emphasis_style(style_str) {
            out.push(Decl { kind: DeclKind::TextEmphasisStyle(st), important });
        }
    }
    out
}

/// `ruby-position`: `over | under | inter-character | alternate`. Fase 7.313.
pub(crate) fn parse_ruby_position(value: &str) -> Option<RubyPosition> {
    match value.trim().to_ascii_lowercase().as_str() {
        "over" => Some(RubyPosition::Over),
        "under" => Some(RubyPosition::Under),
        "inter-character" => Some(RubyPosition::InterCharacter),
        "alternate" => Some(RubyPosition::Alternate),
        _ => None,
    }
}

