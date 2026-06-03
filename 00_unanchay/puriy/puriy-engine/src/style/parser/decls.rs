//! Parsing de declaraciones: `parse_declarations` + el dispatch gigante
//! `decl_kind_from_pair`, y los value-parsers base (box-shadow, border, content,
//! counters, calc, list-style, line-height). Sub-módulo de `parser` (regla #1).
use super::*;

/// `true` si el value es el keyword `currentColor` (case-insensitive).
/// Se resuelve al `color` computado del elemento en la cascada (Fase 7.210),
/// no acá — el parser no conoce el color final todavía.
pub(crate) fn is_current_color(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("currentcolor")
}

pub(crate) fn parse_declarations(css: &str, vars: &HashMap<String, String>) -> Vec<Decl> {
    // Cada decl separada por `;`. Detectamos `!important` recortando
    // el sufijo del value antes de pasarlo al parser de tipo. La
    // shorthand `border:` se expande inline a 1..3 decls atómicas.
    let mut out = Vec::new();
    for chunk in css.split(';') {
        let Some((prop, value)) = chunk.split_once(':') else {
            continue;
        };
        let prop = prop.trim();
        // Las declaraciones de variables (`--name: value`) ya se
        // recogieron en la pasada de `extract_root_vars`. Acá las
        // saltamos para no intentar parsearlas como propiedades reales.
        if prop.starts_with("--") {
            continue;
        }
        let value = value.trim();
        let (value, important) = match strip_important(value) {
            Some(stripped) => (stripped, true),
            None => (value, false),
        };
        // Sustituye `var(--name)` antes de parsear. `substitute_vars` es
        // cheap si el value no contiene `var(` (early-out al primer find).
        let substituted = substitute_vars(value, vars);
        let value = substituted.as_str();
        if prop.eq_ignore_ascii_case("border") {
            out.extend(parse_border_shorthand(value, important));
            continue;
        }
        if let Some(decls) = parse_logical_border(prop, value, important) {
            out.extend(decls);
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "") {
            out.extend(parse_border_side_shorthand(edge, value, important));
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "-width") {
            if let Some(w) = parse_length_px(value) {
                out.push(Decl { kind: DeclKind::BorderSideWidth(edge, w), important });
            }
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "-color") {
            if is_current_color(value) {
                out.push(Decl {
                    kind: DeclKind::CurrentColor(ColorTarget::BorderSide(edge)),
                    important,
                });
            } else if let Some(c) = parse_color(value) {
                out.push(Decl { kind: DeclKind::BorderSideColor(edge, c), important });
            }
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "-style") {
            if let Some(s) = parse_border_style(value) {
                out.push(Decl { kind: DeclKind::BorderSideStyle(edge, s), important });
            }
            continue;
        }
        if let Some(corner) = match_border_corner_prop(prop) {
            if let Some(r) = parse_length_px(value) {
                out.push(Decl { kind: DeclKind::BorderCornerRadius(corner, r), important });
            }
            continue;
        }
        if prop.eq_ignore_ascii_case("flex") {
            out.extend(parse_flex_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("font") {
            out.extend(parse_font_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("inset") {
            out.extend(parse_inset_shorthand(value, important));
            continue;
        }
        if let Some(decls) = parse_logical_box(prop, value, important) {
            out.extend(decls);
            continue;
        }
        if prop.eq_ignore_ascii_case("flex-flow") {
            out.extend(parse_flex_flow_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("place-content") {
            out.extend(parse_place_content_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("place-items") {
            out.extend(parse_place_items_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("place-self") {
            out.extend(parse_place_self_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("outline") {
            out.extend(parse_outline_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("background") {
            out.extend(parse_background_shorthand(value, important));
            continue;
        }
        // `background-image: a, b` con varias capas → expandir a capa 0 +
        // BackgroundExtraLayers. Una sola capa cae al path normal de abajo.
        if prop.eq_ignore_ascii_case("background-image")
            && split_top_level_comma(value).len() > 1
        {
            out.extend(parse_background_image_list(value, important));
            continue;
        }
        if let Some(kind) = decl_kind_from_pair(prop, value) {
            out.push(Decl { kind, important });
        }
    }
    out
}

/// Si `value` termina en `!important` (con o sin espacios), devuelve la
/// porción antes del bang. Sino, `None`.
pub(crate) fn strip_important(value: &str) -> Option<&str> {
    let v = value.trim_end();
    if v.len() < "!important".len() {
        return None;
    }
    let tail = &v[v.len() - "!important".len()..];
    if tail.eq_ignore_ascii_case("!important") {
        Some(v[..v.len() - "!important".len()].trim_end())
    } else {
        None
    }
}

pub(crate) fn decl_kind_from_pair(prop: &str, value: &str) -> Option<DeclKind> {
    match prop.to_ascii_lowercase().as_str() {
        // `color: currentColor` = heredar el color (default), así que lo
        // dropeamos (None → el color heredado queda en pie).
        "color" if is_current_color(value) => None,
        "color" => parse_color(value).map(DeclKind::Color),
        // `background` (shorthand) se expande en `parse_declarations` antes
        // de llegar acá; sólo el longhand `background-color` toma color suelto.
        "background-color" if is_current_color(value) => {
            Some(DeclKind::CurrentColor(ColorTarget::Background))
        }
        "background-color" => parse_color(value).map(DeclKind::Background),
        "display" => parse_display(value).map(DeclKind::Display),
        "font-size" => parse_px_or_math(value).map(DeclKind::FontSize),
        "font-weight" => parse_weight(value).map(DeclKind::FontWeight),
        "font-style" => parse_font_style(value).map(DeclKind::FontStyle),
        "font-family" => Some(DeclKind::FontFamily(value.trim().to_string())),
        "margin" => parse_sides(value).map(DeclKind::Margin),
        "margin-top" => parse_length_px(value).map(DeclKind::MarginTop),
        "margin-right" => parse_length_px(value).map(DeclKind::MarginRight),
        "margin-bottom" => parse_length_px(value).map(DeclKind::MarginBottom),
        "margin-left" => parse_length_px(value).map(DeclKind::MarginLeft),
        "padding" => parse_sides(value).map(DeclKind::Padding),
        "padding-top" => parse_length_px(value).map(DeclKind::PaddingTop),
        "padding-right" => parse_length_px(value).map(DeclKind::PaddingRight),
        "padding-bottom" => parse_length_px(value).map(DeclKind::PaddingBottom),
        "padding-left" => parse_length_px(value).map(DeclKind::PaddingLeft),
        "width" => parse_length_or_pct(value).map(DeclKind::Width),
        "height" => parse_length_or_pct(value).map(DeclKind::Height),
        "max-width" => parse_length_or_pct(value).map(DeclKind::MaxWidth),
        "text-align" => parse_text_align(value).map(DeclKind::TextAlign),
        "line-height" => parse_line_height(value).map(DeclKind::LineHeight),
        "border-width" => parse_px_or_math(value).map(DeclKind::BorderWidth),
        "border-color" if is_current_color(value) => {
            Some(DeclKind::CurrentColor(ColorTarget::BorderAll))
        }
        "border-color" => parse_color(value).map(DeclKind::BorderColor),
        "border-style" => parse_border_style(value).map(DeclKind::BorderEnabled),
        "border-radius" => parse_length_px(value).map(DeclKind::BorderRadius),
        "z-index" => {
            // `auto` → 0; sino int. Negativos OK.
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ZIndex(0))
            } else {
                v.parse::<i32>().ok().map(DeclKind::ZIndex)
            }
        }
        "content" => Some(DeclKind::Content(parse_content_value(value))),
        "counter-reset" => Some(DeclKind::CounterReset(parse_counter_list(value, 0))),
        "counter-increment" => Some(DeclKind::CounterIncrement(parse_counter_list(value, 1))),
        "box-shadow" => Some(DeclKind::BoxShadow(parse_box_shadow(value))),
        "text-decoration" | "text-decoration-line" => {
            parse_text_decoration(value).map(DeclKind::TextDecoration)
        }
        "list-style-type" => parse_list_style_type(value).map(DeclKind::ListStyleType),
        // `list-style` shorthand reducido: sólo capturamos el `-type`.
        // Image y position los ignoramos — `none` desactiva el marker
        // entero (matchea el comportamiento del browser).
        "list-style" => parse_list_style_shorthand(value).map(DeclKind::ListStyleType),
        "flex-direction" => parse_flex_direction(value).map(DeclKind::FlexDirection),
        "flex-wrap" => parse_flex_wrap(value).map(DeclKind::FlexWrap),
        "justify-content" => parse_justify_content(value).map(DeclKind::JustifyContent),
        "align-items" => parse_align_items(value).map(DeclKind::AlignItems),
        "align-content" => parse_align_content(value).map(DeclKind::AlignContent),
        "justify-items" => parse_justify_items(value).map(DeclKind::JustifyItems),
        "justify-self" => parse_justify_self(value).map(DeclKind::JustifySelf),
        "gap" => parse_gap(value).map(|(r, c)| DeclKind::Gap { row: r, column: c }),
        "row-gap" => parse_length_px(value).map(DeclKind::RowGap),
        "column-gap" => parse_length_px(value).map(DeclKind::ColumnGap),
        "box-sizing" => parse_box_sizing(value).map(DeclKind::BoxSizing),
        "min-width" => parse_length_or_pct(value).map(DeclKind::MinWidth),
        "min-height" => parse_length_or_pct(value).map(DeclKind::MinHeight),
        "max-height" => parse_length_or_pct(value).map(DeclKind::MaxHeight),
        // `aspect-ratio: auto` resetea; `W / H` o un número crudo fijan la
        // relación. La forma `auto W/H` (auto + ratio) toma sólo el ratio.
        "aspect-ratio" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::AspectRatio(None))
            } else {
                // Descarta un prefijo `auto` opcional (`auto 16/9`).
                let v = v.strip_prefix("auto").map(str::trim).unwrap_or(v);
                parse_aspect_ratio(v).map(|r| DeclKind::AspectRatio(Some(r)))
            }
        }
        // Tamaños lógicos → físicos (LTR + escritura horizontal): inline ↔
        // width, block ↔ height. Fase 7.194.
        "inline-size" => parse_length_or_pct(value).map(DeclKind::Width),
        "block-size" => parse_length_or_pct(value).map(DeclKind::Height),
        "min-inline-size" => parse_length_or_pct(value).map(DeclKind::MinWidth),
        "min-block-size" => parse_length_or_pct(value).map(DeclKind::MinHeight),
        "max-inline-size" => parse_length_or_pct(value).map(DeclKind::MaxWidth),
        "max-block-size" => parse_length_or_pct(value).map(DeclKind::MaxHeight),
        "overflow" | "overflow-x" | "overflow-y" => {
            parse_overflow(value).map(DeclKind::Overflow)
        }
        "white-space" => parse_white_space(value).map(DeclKind::WhiteSpace),
        "text-transform" => parse_text_transform(value).map(DeclKind::TextTransform),
        "opacity" => parse_opacity(value).map(DeclKind::Opacity),
        "align-self" => parse_align_self(value).map(DeclKind::AlignSelf),
        "flex-grow" => value.trim().parse::<f32>().ok().map(DeclKind::FlexGrow),
        "flex-shrink" => value.trim().parse::<f32>().ok().map(DeclKind::FlexShrink),
        "flex-basis" => parse_length_or_pct(value).map(DeclKind::FlexBasis),
        // `flex` y `outline` son shorthands múltiples — se expanden en
        // `parse_declarations` antes de llegar acá.
        "flex" | "outline" => None,
        "outline-width" => parse_length_px(value).map(DeclKind::OutlineWidth),
        "outline-color" if is_current_color(value) => {
            Some(DeclKind::CurrentColor(ColorTarget::Outline))
        }
        "outline-color" => parse_color(value).map(DeclKind::OutlineColor),
        "outline-style" => parse_border_style(value).map(DeclKind::OutlineStyle),
        "outline-offset" => parse_length_px(value).map(DeclKind::OutlineOffset),
        "background-image" => parse_background_image(value),
        "background-size" => parse_background_size(value),
        "background-position" => parse_background_position(value),
        "background-repeat" => parse_background_repeat(value),
        "background-origin" => parse_background_origin(value),
        // `-webkit-background-clip: text` es el spelling dominante en la web
        // para texto con gradiente — lo tratamos igual que el sin-prefijo.
        "background-clip" | "-webkit-background-clip" => parse_background_clip(value),
        "position" => parse_position(value).map(DeclKind::Position),
        "top" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetTop),
        "right" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetRight),
        "bottom" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetBottom),
        "left" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetLeft),
        "vertical-align" => parse_vertical_align(value).map(DeclKind::VerticalAlign),
        "visibility" => parse_visibility(value).map(DeclKind::Visibility),
        "pointer-events" => parse_pointer_events(value).map(DeclKind::PointerEvents),
        "text-indent" => parse_px_or_math(value).map(DeclKind::TextIndent),
        "word-spacing" => parse_px_or_math(value).map(DeclKind::WordSpacing),
        "letter-spacing" => {
            // `normal` = sin tracking extra (0px).
            if value.trim().eq_ignore_ascii_case("normal") {
                Some(DeclKind::LetterSpacing(0.0))
            } else {
                parse_px_or_math(value).map(DeclKind::LetterSpacing)
            }
        }
        "text-shadow" => parse_text_shadows(value).map(DeclKind::TextShadows),
        "transform" => parse_transforms(value).map(DeclKind::Transforms),
        "grid-template-columns" => {
            parse_grid_template(value).map(DeclKind::GridTemplateColumns)
        }
        "grid-template-rows" => parse_grid_template(value).map(DeclKind::GridTemplateRows),
        "animation" => parse_animation(value),
        "transition" => parse_transition(value),
        // `grid-gap` (legacy) = `gap`.
        "grid-gap" => parse_gap(value).map(|(r, c)| DeclKind::Gap { row: r, column: c }),
        "grid-row-gap" => parse_length_px(value).map(DeclKind::RowGap),
        "grid-column-gap" => parse_length_px(value).map(DeclKind::ColumnGap),
        // `border: 1px solid #ccc` — shorthand. Devolvemos un único
        // DeclKind sintético: en realidad ya hay 3 sub-decls que el
        // caller debe emitir, así que delegamos a una ruta especial vía
        // parse_declarations (ver más arriba). Acá no podemos producir
        // varios, así que ignoramos — la entrada se rellena en
        // parse_declarations cuando ve `border`.
        "border" => None,
        _ => None,
    }
}

/// Parsea el argumento de `:nth-child(...)`. Soporta:
/// - palabras clave: `odd` (= `2n+1`), `even` (= `2n`)
/// - número entero: `3` → `(0, 3)` (sólo la 3a)
/// - `n` → `(1, 0)` (todos), `-n` → `(-1, 0)`
/// - `an` → `(a, 0)`; `an+b` y `an-b` → `(a, ±b)`
/// - `-n+b` → `(-1, b)`
///
/// Devuelve `Some((a, b))` o `None` si el formato no encaja.
pub(crate) fn parse_nth_arg(arg: &str) -> Option<(i32, i32)> {
    let s: String = arg.chars().filter(|c| !c.is_whitespace()).collect();
    let s = s.to_ascii_lowercase();
    if s == "odd" {
        return Some((2, 1));
    }
    if s == "even" {
        return Some((2, 0));
    }
    // Caso entero puro: "3" o "-3".
    if let Ok(n) = s.parse::<i32>() {
        return Some((0, n));
    }
    // Buscar la 'n' que separa coeficiente de constante.
    let n_pos = s.find('n')?;
    let coeff_str = &s[..n_pos];
    let rest = &s[n_pos + 1..];
    let a: i32 = match coeff_str {
        "" => 1,
        "-" => -1,
        "+" => 1,
        other => other.parse().ok()?,
    };
    let b: i32 = if rest.is_empty() { 0 } else { rest.parse().ok()? };
    Some((a, b))
}

/// Parsea `box-shadow: <offset-x> <offset-y> [blur] [spread] <color>`
/// o `box-shadow: none`. Devuelve `None` (= no-shadow) si:
/// - value es exactamente `none`, o
/// - falta el offset-x/offset-y, o
/// - no se reconoce el color.
///
/// `inset` y múltiples sombras separadas por coma no soportadas — el
/// resto del declaration se ignora silenciosamente.
pub(crate) fn parse_box_shadow(value: &str) -> Option<BoxShadow> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") || v.is_empty() {
        return None;
    }
    // Toma sólo la primera sombra (si hay coma).
    let first = v.split(',').next().unwrap_or(v).trim();
    let mut lengths: Vec<f32> = Vec::with_capacity(4);
    let mut color: Option<Color> = None;
    for tok in first.split_whitespace() {
        if tok.eq_ignore_ascii_case("inset") {
            // No soportado todavía — abortamos.
            return None;
        }
        if let Some(l) = parse_length_px(tok) {
            lengths.push(l);
            continue;
        }
        if let Some(c) = parse_color(tok) {
            color = Some(c);
            continue;
        }
    }
    if lengths.len() < 2 {
        return None;
    }
    Some(BoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_px: lengths.get(2).copied().unwrap_or(0.0),
        spread_px: lengths.get(3).copied().unwrap_or(0.0),
        color: color.unwrap_or(Color::rgb(0, 0, 0)),
    })
}

pub(crate) fn parse_border_style(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "solid" | "dashed" | "dotted" | "double" => Some(true),
        "none" | "hidden" => Some(false),
        _ => None,
    }
}

/// Parsea el shorthand `border: <width> <style> <color>` (componentes en
/// cualquier orden). Devuelve hasta 3 decls. Si falta el style, se asume
/// `solid`. Cualquier "none" en la posición de style desactiva el border.
pub(crate) fn parse_border_shorthand(value: &str, important: bool) -> Vec<Decl> {
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
    // Defaults razonables: si hay width+color sin style, asumimos solid.
    if style_on.is_none() && (width.is_some() || color.is_some() || current) {
        style_on = Some(true);
    }
    let mut out = Vec::new();
    if let Some(on) = style_on {
        out.push(Decl { kind: DeclKind::BorderEnabled(on), important });
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

/// Match propiedades `border-{top|bottom}-{left|right}-radius`.
pub(crate) fn match_border_corner_prop(prop: &str) -> Option<BorderCorner> {
    match prop.to_ascii_lowercase().as_str() {
        "border-top-left-radius" => Some(BorderCorner::TopLeft),
        "border-top-right-radius" => Some(BorderCorner::TopRight),
        "border-bottom-right-radius" => Some(BorderCorner::BottomRight),
        "border-bottom-left-radius" => Some(BorderCorner::BottomLeft),
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
    let mut out = Vec::new();
    for edge in edges {
        match sub {
            "" => out.extend(parse_border_side_shorthand(edge, value, important)),
            "-width" => {
                if let Some(w) = parse_length_px(value) {
                    out.push(Decl { kind: DeclKind::BorderSideWidth(edge, w), important });
                }
            }
            "-color" => {
                if let Some(c) = parse_color(value) {
                    out.push(Decl { kind: DeclKind::BorderSideColor(edge, c), important });
                }
            }
            "-style" => {
                if let Some(s) = parse_border_style(value) {
                    out.push(Decl { kind: DeclKind::BorderSideStyle(edge, s), important });
                }
            }
            _ => return None, // sufijo desconocido → no es una lógica de borde
        }
    }
    Some(out)
}

/// Parsea `text-decoration` o `text-decoration-line`. Acepta el shorthand
/// con varios tokens — busca el primer keyword reconocido como line y
/// devuelve eso. Estilos (`dotted`/`wavy`) y color se ignoran (sólo
/// pintamos línea sólida del color del texto).
pub(crate) fn parse_text_decoration(value: &str) -> Option<TextDecorationLine> {
    for tok in value.split_whitespace() {
        match tok.to_ascii_lowercase().as_str() {
            "none" => return Some(TextDecorationLine::None),
            "underline" => return Some(TextDecorationLine::Underline),
            "line-through" => return Some(TextDecorationLine::LineThrough),
            "overline" => return Some(TextDecorationLine::Overline),
            _ => {}
        }
    }
    None
}

/// Parsea `list-style-type: <keyword>`. Acepta los aliases comunes
/// (`lower-latin` = `lower-alpha`, `upper-latin` = `upper-alpha`).
/// Keywords no soportados (`georgian`, `hebrew`, …) caen a `None` y la
/// declaración se ignora — el caller mantiene el valor anterior.
pub(crate) fn parse_list_style_type(s: &str) -> Option<ListStyleType> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(ListStyleType::None),
        "disc" => Some(ListStyleType::Disc),
        "circle" => Some(ListStyleType::Circle),
        "square" => Some(ListStyleType::Square),
        "decimal" => Some(ListStyleType::Decimal),
        "lower-alpha" | "lower-latin" => Some(ListStyleType::LowerAlpha),
        "upper-alpha" | "upper-latin" => Some(ListStyleType::UpperAlpha),
        "lower-roman" => Some(ListStyleType::LowerRoman),
        "upper-roman" => Some(ListStyleType::UpperRoman),
        _ => None,
    }
}

/// Shorthand `list-style: [type] [position] [image]` muy reducido. Sólo
/// extraemos el primer token que matchee un `-type` keyword. `list-style:
/// none` desactiva el marker (matchea browsers — `none` ahí setea ambos
/// `-type` e `-image` a none, y como no tenemos `-image`, alcanza con
/// poner `-type` en `None`).
pub(crate) fn parse_list_style_shorthand(s: &str) -> Option<ListStyleType> {
    for tok in s.split_whitespace() {
        if let Some(t) = parse_list_style_type(tok) {
            return Some(t);
        }
    }
    None
}

pub(crate) fn parse_text_align(s: &str) -> Option<TextAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => Some(TextAlign::Left),
        "center" => Some(TextAlign::Center),
        "right" | "end" => Some(TextAlign::Right),
        "justify" => Some(TextAlign::Justify),
        _ => None,
    }
}

/// Acepta `auto`, `Npx`, `Nrem`/`Nem` (→ px), `N%`. Sin unidad y
/// distinto de `0` → falla (a diferencia de `parse_length_px`, que
/// asume px).
pub(crate) fn parse_length_or_pct(s: &str) -> Option<LengthVal> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") {
        return Some(LengthVal::Auto);
    }
    if let Some(num) = s.strip_suffix('%') {
        return num.trim().parse::<f32>().ok().map(LengthVal::Pct);
    }
    // Funciones matemáticas: `calc()`/`min()`/`max()`/`clamp()` (anidables,
    // con precedencia `*`/`/` sobre `+`/`-` y paréntesis).
    if is_math_fn(s) {
        return eval_calc(s).and_then(calcval_to_length);
    }
    parse_length_px(s).map(LengthVal::Px)
}

/// Parsea el value de `content:` para pseudo-elements. Soporta una
/// secuencia de items separados por whitespace: strings quoted,
/// `counter(name)` y `attr(name)`. Devuelve `None` para `none`/`normal`
/// (que suprime el pseudo-element) o si encuentra algo no reconocible.
pub(crate) fn parse_content_value(value: &str) -> Option<Vec<ContentItem>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("normal") {
        return None;
    }
    let mut items = Vec::new();
    let mut chars = v.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '"' || c == '\'' {
            let item = parse_string_literal(&mut chars)?;
            items.push(ContentItem::Text(item));
            continue;
        }
        // Identificador: `counter(...)` o `attr(...)` (case-insensitive).
        let ident = read_ident(&mut chars);
        if ident.is_empty() {
            return None;
        }
        let lower = ident.to_ascii_lowercase();
        // Comer paréntesis de apertura.
        if chars.next() != Some('(') {
            return None;
        }
        let arg = read_until(&mut chars, ')')?;
        let arg = arg.trim();
        // `counter(name[, list-style])`: nos quedamos con el name; el
        // list-style queda para más adelante.
        let name = arg.split(',').next().unwrap_or("").trim();
        if name.is_empty() {
            return None;
        }
        match lower.as_str() {
            "counter" => items.push(ContentItem::Counter(name.to_string())),
            "attr" => items.push(ContentItem::Attr(name.to_string())),
            "url" => {
                // El arg de url() puede venir entre comillas o sin.
                // arg ya fue trimmeado del paréntesis exterior; acá
                // strippeamos comillas si las hay y devolvemos el resto
                // sin trim adicional (las URLs pueden tener espacios
                // encodeados pero no whitespace literal interno).
                let raw = arg.trim();
                let clean = raw
                    .trim_start_matches(['"', '\''].as_ref())
                    .trim_end_matches(['"', '\''].as_ref())
                    .trim()
                    .to_string();
                if clean.is_empty() {
                    return None;
                }
                items.push(ContentItem::Url(clean));
            }
            _ => return None, // `counters(...)` no soportado aún.
        }
    }
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

/// Lee una string literal (incluyendo las comillas) de `chars` —
/// consume hasta encontrar la comilla de cierre matching. Soporta
/// escape `\X` que vuelca X tal cual. Devuelve None si la string queda
/// sin cerrar.
pub(crate) fn parse_string_literal(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<String> {
    let quote = chars.next()?;
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(esc) = chars.next() {
                out.push(esc);
                continue;
            }
            return None;
        }
        if c == quote {
            return Some(out);
        }
        out.push(c);
    }
    None
}

/// Lee chars mientras sean alfanuméricos, `-` o `_`. Devuelve el ident
/// como String (vacío si el siguiente char no era válido).
pub(crate) fn read_ident(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut out = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
            chars.next();
        } else {
            break;
        }
    }
    out
}

/// Lee chars hasta el delimitador `end` (exclusivo) — lo consume. Devuelve
/// el contenido. None si no encuentra el delim.
pub(crate) fn read_until(chars: &mut std::iter::Peekable<std::str::Chars>, end: char) -> Option<String> {
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == end {
            return Some(out);
        }
        out.push(c);
    }
    None
}

/// Parsea `counter-reset` o `counter-increment`. Devuelve pares
/// `(name, value)` — para reset el default es `0`, para increment es
/// `1`. Si el value es `none`, devuelve vec vacío.
pub(crate) fn parse_counter_list(value: &str, default: i32) -> Vec<(String, i32)> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    let mut out: Vec<(String, i32)> = Vec::new();
    let toks: Vec<&str> = v.split_whitespace().collect();
    let mut i = 0;
    while i < toks.len() {
        let name = toks[i];
        if !is_valid_counter_name(name) {
            // Token no nombre — skip (parser tolerante).
            i += 1;
            continue;
        }
        let value = toks
            .get(i + 1)
            .and_then(|t| t.parse::<i32>().ok());
        if let Some(v) = value {
            out.push((name.to_string(), v));
            i += 2;
        } else {
            out.push((name.to_string(), default));
            i += 1;
        }
    }
    out
}

pub(crate) fn is_valid_counter_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Valor intermedio de la evaluación de `calc()`/`min`/`max`/`clamp`: un
/// número adimensional, o una longitud con componente absoluto (`px`) +
/// componente porcentual (`pct`). px/em/rem/vw/vh/vmin/vmax se resuelven a
/// px en parse-time; sólo `%` queda como componente `pct`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CalcVal {
    Number(f32),
    Length { px: f32, pct: f32 },
}

/// `true` si `s` arranca con una función matemática CSS (`calc`/`min`/
/// `max`/`clamp`) seguida de `(`.
pub(crate) fn is_math_fn(s: &str) -> bool {
    let l = s.trim_start().to_ascii_lowercase();
    ["calc(", "min(", "max(", "clamp("].iter().any(|p| l.starts_with(p))
}

/// Convierte un `CalcVal` final a `LengthVal`. Un número crudo sólo es
/// válido si es 0 (un número no es una longitud). Mezcla px+pct degrada a
/// `Pct` (se pierde el offset px — sin container width, igual que el calc
/// histórico). Ver [`parse_length_or_pct`].
pub(crate) fn calcval_to_length(v: CalcVal) -> Option<LengthVal> {
    match v {
        CalcVal::Number(n) if n == 0.0 => Some(LengthVal::Px(0.0)),
        CalcVal::Number(_) => None,
        CalcVal::Length { px, pct } => {
            if pct == 0.0 {
                Some(LengthVal::Px(px))
            } else {
                // pct puro o mezcla → Pct (mezcla pierde el offset px).
                Some(LengthVal::Pct(pct))
            }
        }
    }
}

/// Evalúa una expresión matemática CSS (`calc`/`min`/`max`/`clamp`, con
/// anidamiento, precedencia `*`/`/` sobre `+`/`-` y paréntesis) a un
/// `CalcVal`. `None` si la sintaxis es inválida.
pub(crate) fn eval_calc(s: &str) -> Option<CalcVal> {
    let mut p = CalcCtx { b: s.as_bytes(), i: 0, src: s };
    let v = p.expr()?;
    p.ws();
    if p.i != p.b.len() {
        return None;
    }
    Some(v)
}

/// Parser recursivo-descendente sobre los bytes de la expresión.
struct CalcCtx<'a> {
    b: &'a [u8],
    i: usize,
    src: &'a str,
}

impl CalcCtx<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    /// `expr := term ((' + ' | ' - ') term)*` — `+`/`-` exigen whitespace.
    fn expr(&mut self) -> Option<CalcVal> {
        let mut acc = self.term()?;
        loop {
            self.ws();
            let Some(c) = self.peek() else { break };
            if c == b'+' || c == b'-' {
                // CSS exige whitespace alrededor de `+`/`-` (antes ya hubo
                // por `ws()`; exigimos también después para no confundir con
                // un signo de número).
                let after_ws = self
                    .b
                    .get(self.i + 1)
                    .is_some_and(|x| (*x as char).is_ascii_whitespace());
                if !after_ws {
                    break;
                }
                self.i += 1;
                let rhs = self.term()?;
                acc = calc_add(acc, rhs, if c == b'+' { 1.0 } else { -1.0 })?;
            } else {
                break;
            }
        }
        Some(acc)
    }

    /// `term := factor (('*' | '/') factor)*` — `*`/`/` sin whitespace req.
    fn term(&mut self) -> Option<CalcVal> {
        let mut acc = self.factor()?;
        loop {
            self.ws();
            let Some(c) = self.peek() else { break };
            if c == b'*' || c == b'/' {
                self.i += 1;
                let rhs = self.factor()?;
                acc = if c == b'*' { calc_mul(acc, rhs)? } else { calc_div(acc, rhs)? };
            } else {
                break;
            }
        }
        Some(acc)
    }

    /// `factor := '(' expr ')' | func '(' args ')' | número`.
    fn factor(&mut self) -> Option<CalcVal> {
        self.ws();
        let c = self.peek()?;
        if c == b'(' {
            self.i += 1;
            let v = self.expr()?;
            self.ws();
            if self.peek()? != b')' {
                return None;
            }
            self.i += 1;
            return Some(v);
        }
        if c.is_ascii_alphabetic() {
            let start = self.i;
            while self.i < self.b.len() && self.b[self.i].is_ascii_alphabetic() {
                self.i += 1;
            }
            let name = self.src[start..self.i].to_ascii_lowercase();
            // CSS no permite whitespace entre el nombre y `(`.
            if self.peek() != Some(b'(') {
                return None;
            }
            self.i += 1;
            let args = self.args()?;
            return apply_math_fn(&name, &args);
        }
        self.number()
    }

    /// Lista de expresiones separadas por coma hasta el `)`.
    fn args(&mut self) -> Option<Vec<CalcVal>> {
        let mut out = Vec::new();
        loop {
            out.push(self.expr()?);
            self.ws();
            match self.peek()? {
                b',' => self.i += 1,
                b')' => {
                    self.i += 1;
                    return Some(out);
                }
                _ => return None,
            }
        }
    }

    /// Número con unidad opcional o signo líder.
    fn number(&mut self) -> Option<CalcVal> {
        self.ws();
        let start = self.i;
        if matches!(self.peek(), Some(b'+') | Some(b'-')) {
            self.i += 1;
        }
        let mut saw_digit = false;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c.is_ascii_digit() {
                saw_digit = true;
                self.i += 1;
            } else if c == b'.' || c.is_ascii_alphabetic() || c == b'%' {
                self.i += 1;
            } else {
                break;
            }
        }
        if !saw_digit {
            return None;
        }
        classify_calc_num(self.src[start..self.i].trim())
    }
}

/// Clasifica un token numérico: `%` → componente pct; número crudo →
/// `Number`; con unidad (px/em/rem/vw/…) → componente px resuelto.
fn classify_calc_num(t: &str) -> Option<CalcVal> {
    let t = t.trim();
    if let Some(p) = t.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().map(|v| CalcVal::Length { px: 0.0, pct: v });
    }
    if let Ok(n) = t.parse::<f32>() {
        return Some(CalcVal::Number(n));
    }
    parse_length_px(t).map(|px| CalcVal::Length { px, pct: 0.0 })
}

/// Longitud px de un solo valor, aceptando funciones matemáticas que
/// resuelvan a **px puro** (`calc`/`min`/`max`/`clamp`). El caso estrella es
/// la tipografía fluida `font-size: clamp(1rem, 2.5vw, 3rem)`. Un resultado
/// `%` o número crudo (no resoluble sin contexto) → `None`. Ver Fase 7.216.
pub(crate) fn parse_px_or_math(s: &str) -> Option<f32> {
    let s = s.trim();
    if is_math_fn(s) {
        return match eval_calc(s)? {
            CalcVal::Length { px, pct } if pct == 0.0 => Some(px),
            _ => None,
        };
    }
    parse_length_px(s)
}

fn calc_add(a: CalcVal, b: CalcVal, sign: f32) -> Option<CalcVal> {
    match (a, b) {
        (CalcVal::Number(x), CalcVal::Number(y)) => Some(CalcVal::Number(x + sign * y)),
        (CalcVal::Length { px: p1, pct: q1 }, CalcVal::Length { px: p2, pct: q2 }) => {
            Some(CalcVal::Length { px: p1 + sign * p2, pct: q1 + sign * q2 })
        }
        // Sumar número + longitud es inválido en CSS.
        _ => None,
    }
}

fn calc_mul(a: CalcVal, b: CalcVal) -> Option<CalcVal> {
    match (a, b) {
        (CalcVal::Number(x), CalcVal::Number(y)) => Some(CalcVal::Number(x * y)),
        (CalcVal::Number(x), CalcVal::Length { px, pct })
        | (CalcVal::Length { px, pct }, CalcVal::Number(x)) => {
            Some(CalcVal::Length { px: px * x, pct: pct * x })
        }
        // longitud * longitud es inválido.
        _ => None,
    }
}

fn calc_div(a: CalcVal, b: CalcVal) -> Option<CalcVal> {
    match (a, b) {
        (CalcVal::Number(x), CalcVal::Number(y)) if y != 0.0 => Some(CalcVal::Number(x / y)),
        (CalcVal::Length { px, pct }, CalcVal::Number(y)) if y != 0.0 => {
            Some(CalcVal::Length { px: px / y, pct: pct / y })
        }
        _ => None,
    }
}

fn apply_math_fn(name: &str, args: &[CalcVal]) -> Option<CalcVal> {
    match name {
        "calc" => (args.len() == 1).then(|| args[0]),
        "min" => reduce_minmax(args, true),
        "max" => reduce_minmax(args, false),
        "clamp" if args.len() == 3 => clamp_calc(args[0], args[1], args[2]),
        _ => None,
    }
}

/// `true` si todos los valores son comparables (misma dimensión): todos
/// número, todos px puro, o todos pct puro.
fn all_comparable(vs: &[CalcVal]) -> bool {
    vs.iter().all(|v| matches!(v, CalcVal::Number(_)))
        || vs.iter().all(|v| matches!(v, CalcVal::Length { pct, .. } if *pct == 0.0))
        || vs.iter().all(|v| matches!(v, CalcVal::Length { px, .. } if *px == 0.0))
}

/// `min()`/`max()`. Si los args son comparables resuelve exacto; si hay
/// mezcla incomparable (px vs %) degrada al primer arg (sin container).
fn reduce_minmax(args: &[CalcVal], is_min: bool) -> Option<CalcVal> {
    let first = *args.first()?;
    let pick = |a: f32, b: f32| if is_min { a.min(b) } else { a.max(b) };
    if !all_comparable(args) {
        return Some(first); // incomparable → degradar
    }
    let scalar = |v: &CalcVal| match v {
        CalcVal::Number(n) => *n,
        CalcVal::Length { px, pct } => px + pct, // uno es 0 (all_comparable)
    };
    let best = args.iter().map(scalar).reduce(pick)?;
    Some(match first {
        CalcVal::Number(_) => CalcVal::Number(best),
        CalcVal::Length { pct, .. } if pct == 0.0 => CalcVal::Length { px: best, pct: 0.0 },
        CalcVal::Length { .. } => CalcVal::Length { px: 0.0, pct: best },
    })
}

/// `clamp(lo, val, hi)` = `max(lo, min(val, hi))`. Si los tres no son
/// comparables, degrada al valor central (`val`, el preferido).
fn clamp_calc(lo: CalcVal, val: CalcVal, hi: CalcVal) -> Option<CalcVal> {
    if all_comparable(&[lo, val, hi]) {
        let upper = reduce_minmax(&[val, hi], true)?;
        return reduce_minmax(&[lo, upper], false);
    }
    Some(val)
}

/// Acepta multiplicador adimensional (`1.5`, `1.6`), `Npx`, `Nem`/`Nrem`.
/// Devuelve siempre un multiplicador (px se divide por 16; `em`/`rem`
/// salen como ya están). Imperfecto pero alcanza para Fase 4.
pub(crate) fn parse_line_height(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix("px") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v / 16.0);
    }
    if let Some(num) = s.strip_suffix("rem") {
        return num.trim().parse().ok();
    }
    if let Some(num) = s.strip_suffix("em") {
        return num.trim().parse().ok();
    }
    s.parse::<f32>().ok()
}

/// Versión pública para que `boxes` parsee colors de attrs SVG.
pub(crate) fn parse_color_named_or_hex(s: &str) -> Option<Color> {
    parse_color(s)
}
