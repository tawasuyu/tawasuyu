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
        // `border-style` (todos los lados): togglea enabled + fija el patrón.
        if prop.eq_ignore_ascii_case("border-style") {
            if let Some(on) = parse_border_style(value) {
                out.push(Decl { kind: DeclKind::BorderEnabled(on), important });
                if let Some(ls) = parse_border_line_style(value) {
                    out.push(Decl { kind: DeclKind::BorderStyleKind(ls), important });
                }
            }
            continue;
        }
        // `outline-style`: togglea style_active + fija el patrón visual.
        if prop.eq_ignore_ascii_case("outline-style") {
            if let Some(on) = parse_border_style(value) {
                out.push(Decl { kind: DeclKind::OutlineStyle(on), important });
                if let Some(ls) = parse_border_line_style(value) {
                    out.push(Decl { kind: DeclKind::OutlineStylePattern(ls), important });
                }
            }
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
        // `overscroll-behavior: <x> [<y>]` — desagrega en x e y.
        if prop.eq_ignore_ascii_case("overscroll-behavior") {
            let mut tokens = value.split_whitespace();
            let x = tokens.next().and_then(parse_overscroll_behavior);
            let y = tokens.next().and_then(parse_overscroll_behavior).or(x);
            if let Some(xv) = x {
                out.push(Decl { kind: DeclKind::OverscrollBehaviorX(xv), important });
            }
            if let Some(yv) = y {
                out.push(Decl { kind: DeclKind::OverscrollBehaviorY(yv), important });
            }
            continue;
        }
        // `scroll-margin-block: <start> [<end>]` (Fase 7.417). En LTR
        // horizontal: start=top, end=bottom. Con 1 valor se aplica a
        // ambos lados; con 2, primero start, después end. Si falla algún
        // token descartamos el shorthand entero (CSS spec).
        if prop.eq_ignore_ascii_case("scroll-margin-block") {
            let parts: Vec<&str> = value.split_whitespace().collect();
            let vals: Vec<f32> =
                parts.iter().filter_map(|p| parse_length_px(p)).collect();
            if !vals.is_empty() && vals.len() == parts.len() {
                let (s, e) =
                    if vals.len() == 1 { (vals[0], vals[0]) } else { (vals[0], vals[1]) };
                out.push(Decl { kind: DeclKind::ScrollMarginTop(s), important });
                out.push(Decl { kind: DeclKind::ScrollMarginBottom(e), important });
            }
            continue;
        }
        // `scroll-margin-inline: <start> [<end>]` (Fase 7.420). En LTR
        // horizontal: start=left, end=right. Misma semántica que el
        // `-block` (1→ambos, 2→start/end; rechazo total si algún token
        // falla).
        if prop.eq_ignore_ascii_case("scroll-margin-inline") {
            let parts: Vec<&str> = value.split_whitespace().collect();
            let vals: Vec<f32> =
                parts.iter().filter_map(|p| parse_length_px(p)).collect();
            if !vals.is_empty() && vals.len() == parts.len() {
                let (s, e) =
                    if vals.len() == 1 { (vals[0], vals[0]) } else { (vals[0], vals[1]) };
                out.push(Decl { kind: DeclKind::ScrollMarginLeft(s), important });
                out.push(Decl { kind: DeclKind::ScrollMarginRight(e), important });
            }
            continue;
        }
        // `scroll-padding-block: <start> [<end>]` (Fase 7.423). Misma
        // semántica que `scroll-margin-block` pero los longhands usan
        // `LengthVal` (length/%, no f32 puro).
        if prop.eq_ignore_ascii_case("scroll-padding-block") {
            let parts: Vec<&str> = value.split_whitespace().collect();
            let vals: Vec<LengthVal> =
                parts.iter().filter_map(|p| parse_length_or_pct(p)).collect();
            if !vals.is_empty() && vals.len() == parts.len() {
                let (s, e) =
                    if vals.len() == 1 { (vals[0], vals[0]) } else { (vals[0], vals[1]) };
                out.push(Decl { kind: DeclKind::ScrollPaddingTop(s), important });
                out.push(Decl { kind: DeclKind::ScrollPaddingBottom(e), important });
            }
            continue;
        }
        // `scroll-padding-inline: <start> [<end>]` (Fase 7.426). En LTR
        // horizontal: start=left, end=right. Misma semántica que
        // `scroll-padding-block`.
        if prop.eq_ignore_ascii_case("scroll-padding-inline") {
            let parts: Vec<&str> = value.split_whitespace().collect();
            let vals: Vec<LengthVal> =
                parts.iter().filter_map(|p| parse_length_or_pct(p)).collect();
            if !vals.is_empty() && vals.len() == parts.len() {
                let (s, e) =
                    if vals.len() == 1 { (vals[0], vals[0]) } else { (vals[0], vals[1]) };
                out.push(Decl { kind: DeclKind::ScrollPaddingLeft(s), important });
                out.push(Decl { kind: DeclKind::ScrollPaddingRight(e), important });
            }
            continue;
        }
        // `contain-intrinsic-size: <w> [<h>]` shorthand (Fase 7.438). Cada
        // mitad acepta `none | <length> | auto none | auto <length>`. Con 1
        // valor se aplica a width y height; con 2 (separados por la primera
        // "frontera" `<length-or-none>` que no esté precedida por `auto`), el
        // primero va a width, el segundo a height. Si algún token falla,
        // rechazamos el shorthand entero (no parcial).
        if prop.eq_ignore_ascii_case("contain-intrinsic-size") {
            let toks: Vec<&str> = value.split_whitespace().collect();
            let halves = split_contain_intrinsic_halves(&toks);
            if let Some((w_raw, h_raw)) = halves {
                let w = parse_contain_intrinsic_size(&w_raw);
                let h = match h_raw.as_deref() {
                    Some(s) => parse_contain_intrinsic_size(s),
                    None => w,
                };
                if let (Some(w), Some(h)) = (w, h) {
                    out.push(Decl {
                        kind: DeclKind::ContainIntrinsicWidth(w),
                        important,
                    });
                    out.push(Decl {
                        kind: DeclKind::ContainIntrinsicHeight(h),
                        important,
                    });
                }
            }
            continue;
        }
        // `scroll-snap-align: <block> [<inline>]` — con 1 valor se aplica a
        // ambos ejes. Fase 7.269.
        if prop.eq_ignore_ascii_case("scroll-snap-align") {
            let mut tokens = value.split_whitespace();
            let block = tokens.next().and_then(parse_scroll_snap_align);
            let inline = tokens.next().and_then(parse_scroll_snap_align).or(block);
            if let Some(b) = block {
                out.push(Decl { kind: DeclKind::ScrollSnapAlignBlock(b), important });
            }
            if let Some(i) = inline {
                out.push(Decl { kind: DeclKind::ScrollSnapAlignInline(i), important });
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
        // `margin` shorthand: ruteado acá (no por decl_kind_from_pair) para
        // soportar `auto` por lado (`margin: 0 auto` = centrado horizontal).
        if prop.eq_ignore_ascii_case("margin") {
            out.extend(parse_margin_shorthand(value, important));
            continue;
        }
        // Longhands de margen con `auto`. Horizontal → flag de centrado;
        // vertical → 0 (no centra en block flow).
        if prop.eq_ignore_ascii_case("margin-left") && value.eq_ignore_ascii_case("auto") {
            out.push(Decl { kind: DeclKind::MarginLeft(0.0), important });
            out.push(Decl { kind: DeclKind::MarginLeftAuto(true), important });
            continue;
        }
        if prop.eq_ignore_ascii_case("margin-right") && value.eq_ignore_ascii_case("auto") {
            out.push(Decl { kind: DeclKind::MarginRight(0.0), important });
            out.push(Decl { kind: DeclKind::MarginRightAuto(true), important });
            continue;
        }
        if prop.eq_ignore_ascii_case("margin-top") && value.eq_ignore_ascii_case("auto") {
            out.push(Decl { kind: DeclKind::MarginTop(0.0), important });
            continue;
        }
        if prop.eq_ignore_ascii_case("margin-bottom") && value.eq_ignore_ascii_case("auto") {
            out.push(Decl { kind: DeclKind::MarginBottom(0.0), important });
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
        // `container` shorthand (Fase 7.408): `<container-name> [ / <container-type> ]?`.
        // Sin `/` → sólo name; con `/` → name antes, type después.
        if prop.eq_ignore_ascii_case("container") {
            let (name_part, type_part) = match value.split_once('/') {
                Some((n, t)) => (n.trim(), Some(t.trim())),
                None => (value.trim(), None),
            };
            if let Some(names) = parse_ident_list_or_none(name_part) {
                out.push(Decl {
                    kind: DeclKind::ContainerName(names),
                    important,
                });
                let ct = match type_part {
                    Some(s) => parse_container_type(s).unwrap_or(ContainerType::Normal),
                    None => ContainerType::Normal,
                };
                out.push(Decl {
                    kind: DeclKind::ContainerType(ct),
                    important,
                });
            }
            continue;
        }
        // `marker` shorthand (Fase 7.397): `none | <funcIRI>` — setea
        // los 3 longhands (`marker-start/-mid/-end`) al mismo valor.
        if prop.eq_ignore_ascii_case("marker") {
            if let Some(r) = parse_marker_ref(value) {
                out.push(Decl {
                    kind: DeclKind::MarkerStart(r.clone()),
                    important,
                });
                out.push(Decl {
                    kind: DeclKind::MarkerMid(r.clone()),
                    important,
                });
                out.push(Decl {
                    kind: DeclKind::MarkerEnd(r),
                    important,
                });
            }
            continue;
        }
        // `list-style` shorthand (Fase 7.296): `<type> || <position> || <image>`.
        // `none` apaga `type` y `image`; el resto cae en su longhand.
        if prop.eq_ignore_ascii_case("list-style") {
            out.extend(parse_list_style_shorthand_full(value, important));
            continue;
        }
        // `text-emphasis` shorthand (Fase 7.312): `<style> || <color>`. `none`
        // setea style=None y NO toca el color. Orden libre.
        if prop.eq_ignore_ascii_case("text-emphasis") {
            out.extend(parse_text_emphasis_shorthand(value, important));
            continue;
        }
        // `font-synthesis` shorthand (Fase 7.333):
        // `none | [weight || style || small-caps]`. Orden libre, sin
        // duplicados, sin tokens desconocidos. Emite `FontSynthesisAll`
        // con los 3 axes resueltos.
        if prop.eq_ignore_ascii_case("font-synthesis") {
            if let Some(fs) = parse_font_synthesis_shorthand(value) {
                out.push(Decl { kind: DeclKind::FontSynthesisAll(fs), important });
            }
            continue;
        }
        // `columns` shorthand (Fase 7.361):
        // `auto | <column-width> || <column-count>`. Emite los 2
        // longhands `ColumnWidth` y `ColumnCount`. `auto` único setea
        // ambos a auto.
        if prop.eq_ignore_ascii_case("columns") {
            if let Some((w, n)) = parse_columns_shorthand(value) {
                out.push(Decl { kind: DeclKind::ColumnWidth(w), important });
                out.push(Decl { kind: DeclKind::ColumnCount(n), important });
            }
            continue;
        }
        // `place-items` shorthand (Fase 7.336): `<align-items>
        // [<justify-items>]?`. Si falta el 2º, vale para ambos ejes
        // (regla CSS-Align 3). Emite los 2 longhands.
        if prop.eq_ignore_ascii_case("place-items") {
            if let Some((al, ji)) = parse_place_items(value) {
                out.push(Decl { kind: DeclKind::AlignItems(al), important });
                out.push(Decl { kind: DeclKind::JustifyItems(ji), important });
            }
            continue;
        }
        // `place-content` shorthand (Fase 7.337): `<align-content>
        // [<justify-content>]?`.
        if prop.eq_ignore_ascii_case("place-content") {
            if let Some((ac, jc)) = parse_place_content(value) {
                out.push(Decl { kind: DeclKind::AlignContent(ac), important });
                out.push(Decl { kind: DeclKind::JustifyContent(jc), important });
            }
            continue;
        }
        // `place-self` shorthand (Fase 7.338): `<align-self>
        // [<justify-self>]?`.
        if prop.eq_ignore_ascii_case("place-self") {
            if let Some((al, jl)) = parse_place_self(value) {
                out.push(Decl { kind: DeclKind::AlignSelf(al), important });
                out.push(Decl { kind: DeclKind::JustifySelf(jl), important });
            }
            continue;
        }
        // `column-rule` shorthand (Fase 7.280): mismo shape que `outline`.
        if prop.eq_ignore_ascii_case("column-rule") {
            out.extend(parse_column_rule_shorthand(value, important));
            continue;
        }
        // `column-rule-style: dotted` → activa + fija el patrón.
        if prop.eq_ignore_ascii_case("column-rule-style") {
            if let Some(on) = parse_border_style(value) {
                out.push(Decl { kind: DeclKind::ColumnRuleStyleActive(on), important });
                if let Some(ls) = parse_border_line_style(value) {
                    out.push(Decl { kind: DeclKind::ColumnRuleStylePattern(ls), important });
                }
            }
            continue;
        }
        if prop.eq_ignore_ascii_case("text-decoration") {
            out.extend(parse_text_decoration_shorthand(value, important));
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

/// Keyword CSS-wide (`inherit`/`initial`/`unset`/`revert`). `revert` se
/// aproxima como `unset`. Fase 7.225.
fn wide_keyword(value: &str) -> Option<WideKw> {
    match value.trim().to_ascii_lowercase().as_str() {
        "inherit" => Some(WideKw::Inherit),
        "initial" => Some(WideKw::Initial),
        "unset" => Some(WideKw::Unset),
        "revert" | "revert-layer" => Some(WideKw::Unset),
        _ => None,
    }
}

/// Mapea una propiedad longhand al `WideProp` del subset curado. `None` para
/// las no soportadas (su keyword wide se dropea). Fase 7.225.
fn wide_prop(prop: &str) -> Option<WideProp> {
    Some(match prop.to_ascii_lowercase().as_str() {
        "color" => WideProp::Color,
        "background-color" => WideProp::Background,
        "font-size" => WideProp::FontSize,
        "font-weight" => WideProp::FontWeight,
        "font-style" => WideProp::FontStyle,
        "font-family" => WideProp::FontFamily,
        "line-height" => WideProp::LineHeight,
        "text-align" => WideProp::TextAlign,
        "text-decoration" | "text-decoration-line" => WideProp::TextDecoration,
        "visibility" => WideProp::Visibility,
        "display" => WideProp::Display,
        "box-sizing" => WideProp::BoxSizing,
        "border-color" => WideProp::BorderColor,
        _ => return None,
    })
}

pub(crate) fn decl_kind_from_pair(prop: &str, value: &str) -> Option<DeclKind> {
    // Keywords CSS-wide (inherit/initial/unset/revert) sobre el subset
    // curado de propiedades — se resuelven luego contra padre/default.
    if let Some(kw) = wide_keyword(value) {
        return wide_prop(prop).map(|prop| DeclKind::Wide { prop, kw });
    }
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
        "font-size" => parse_font_size(value),
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
        "box-shadow" => Some(DeclKind::BoxShadows(parse_box_shadows(value))),
        // `text-decoration` (shorthand) se expande en `parse_declarations`.
        "text-decoration-line" => {
            parse_text_decoration(value).map(DeclKind::TextDecoration)
        }
        "text-decoration-color" if is_current_color(value) => {
            Some(DeclKind::TextDecorationColor(None))
        }
        "text-decoration-color" => parse_color(value).map(|c| DeclKind::TextDecorationColor(Some(c))),
        "text-decoration-style" => {
            parse_text_decoration_style(value).map(DeclKind::TextDecorationStyle)
        }
        // `auto`/`from-font` → None (grosor derivado); longitud → px.
        "text-decoration-thickness" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "from-font" => Some(DeclKind::TextDecorationThickness(None)),
            _ => parse_length_px(value).map(|p| DeclKind::TextDecorationThickness(Some(p))),
        },
        "text-underline-offset" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::TextUnderlineOffset(None)),
            _ => parse_length_px(value).map(|p| DeclKind::TextUnderlineOffset(Some(p))),
        },
        "list-style-type" => parse_list_style_type(value).map(DeclKind::ListStyleType),
        "list-style-position" => {
            parse_list_style_position(value).map(DeclKind::ListStylePosition)
        }
        "list-style-image" => Some(DeclKind::ListStyleImage(parse_list_style_image(value))),
        // `list-style` shorthand: ruteado por `parse_declarations` para
        // emitir varias longhands en orden libre. Acá NO se dispatcha.
        "list-style" => None,
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
        "object-fit" => parse_object_fit(value).map(DeclKind::ObjectFit),
        "object-position" => match parse_background_position(value) {
            // Reusa el parser de background-position; sólo cambia el destino.
            Some(DeclKind::BackgroundPosition(p)) => Some(DeclKind::ObjectPosition(p)),
            _ => None,
        },
        // `caret-color: auto | currentColor | <color>`. `currentColor` queda
        // como `None` (sigue al color heredado en el chrome eventual).
        "caret-color" => Some(DeclKind::CaretColor(parse_caret_color(value))),
        // `accent-color: auto | <color>`. Sin `currentColor` por espec.
        "accent-color" => Some(DeclKind::AccentColor(parse_auto_or_color(value))),
        "cursor" => parse_cursor(value).map(DeclKind::Cursor),
        "text-overflow" => parse_text_overflow(value).map(DeclKind::TextOverflow),
        "scroll-behavior" => parse_scroll_behavior(value).map(DeclKind::ScrollBehavior),
        "tab-size" | "-moz-tab-size" => parse_tab_size(value).map(DeclKind::TabSize),
        // CSS UI 4 — `user-select` con sus prefijos legacy.
        "user-select" | "-webkit-user-select" | "-moz-user-select" | "-ms-user-select" => {
            parse_user_select(value).map(DeclKind::UserSelect)
        }
        // `word-wrap` es alias legacy IE; CSS Text 3 los unificó.
        "overflow-wrap" | "word-wrap" => {
            parse_overflow_wrap(value).map(DeclKind::OverflowWrap)
        }
        "word-break" => parse_word_break(value).map(DeclKind::WordBreak),
        "hyphens" | "-webkit-hyphens" | "-moz-hyphens" | "-ms-hyphens" => {
            parse_hyphens(value).map(DeclKind::Hyphens)
        }
        "resize" => parse_resize(value).map(DeclKind::Resize),
        "writing-mode" => parse_writing_mode(value).map(DeclKind::WritingMode),
        "direction" => parse_direction(value).map(DeclKind::Direction),
        "unicode-bidi" => parse_unicode_bidi(value).map(DeclKind::UnicodeBidi),
        "font-stretch" => parse_font_stretch(value).map(DeclKind::FontStretch),
        "image-rendering" => parse_image_rendering(value).map(DeclKind::ImageRendering),
        "mix-blend-mode" => parse_blend_mode(value).map(DeclKind::MixBlendMode),
        "background-blend-mode" => {
            Some(DeclKind::BackgroundBlendMode(parse_blend_mode_list(value)))
        }
        "isolation" => parse_isolation(value).map(DeclKind::Isolation),
        "will-change" => Some(DeclKind::WillChange(parse_will_change(value))),
        // Aliases legacy: `-webkit-appearance` y `-moz-appearance`.
        "appearance" | "-webkit-appearance" | "-moz-appearance" => {
            parse_appearance(value).map(DeclKind::Appearance)
        }
        "font-kerning" => parse_font_kerning(value).map(DeclKind::FontKerning),
        "font-feature-settings" => {
            Some(DeclKind::FontFeatureSettings(parse_font_feature_settings(value)))
        }
        "font-variation-settings" => {
            Some(DeclKind::FontVariationSettings(parse_font_variation_settings(value)))
        }
        "font-language-override" => {
            Some(DeclKind::FontLanguageOverride(parse_font_language_override(value)))
        }
        "text-rendering" => parse_text_rendering(value).map(DeclKind::TextRendering),
        "filter" => Some(DeclKind::Filter(parse_filter_list(value))),
        "backdrop-filter" | "-webkit-backdrop-filter" => {
            Some(DeclKind::BackdropFilter(parse_filter_list(value)))
        }
        "text-orientation" => parse_text_orientation(value).map(DeclKind::TextOrientation),
        "overscroll-behavior-x" => {
            parse_overscroll_behavior(value).map(DeclKind::OverscrollBehaviorX)
        }
        "overscroll-behavior-y" => {
            parse_overscroll_behavior(value).map(DeclKind::OverscrollBehaviorY)
        }
        // Fase 7.413 — `overscroll-behavior-block`. En LTR horizontal el
        // eje `block` es el vertical → mapea al longhand `-y`.
        "overscroll-behavior-block" => {
            parse_overscroll_behavior(value).map(DeclKind::OverscrollBehaviorY)
        }
        // Fase 7.414 — `overscroll-behavior-inline`. En LTR horizontal el
        // eje `inline` es el horizontal → mapea al longhand `-x`.
        "overscroll-behavior-inline" => {
            parse_overscroll_behavior(value).map(DeclKind::OverscrollBehaviorX)
        }
        // `overscroll-behavior` shorthand: ver `parse_declarations`.
        "scroll-snap-type" => parse_scroll_snap_type(value).map(DeclKind::ScrollSnapType),
        // `scroll-snap-align` shorthand: ver `parse_declarations`.
        "scroll-snap-align-block" => {
            parse_scroll_snap_align(value).map(DeclKind::ScrollSnapAlignBlock)
        }
        "scroll-snap-align-inline" => {
            parse_scroll_snap_align(value).map(DeclKind::ScrollSnapAlignInline)
        }
        "scroll-snap-stop" => parse_scroll_snap_stop(value).map(DeclKind::ScrollSnapStop),
        "scroll-padding" => parse_sides_lp(value).map(DeclKind::ScrollPadding),
        "scroll-padding-top" => parse_length_or_pct(value).map(DeclKind::ScrollPaddingTop),
        "scroll-padding-right" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingRight)
        }
        "scroll-padding-bottom" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingBottom)
        }
        "scroll-padding-left" => parse_length_or_pct(value).map(DeclKind::ScrollPaddingLeft),
        "scroll-margin" => parse_sides(value).map(DeclKind::ScrollMargin),
        "scroll-margin-top" => parse_length_px(value).map(DeclKind::ScrollMarginTop),
        "scroll-margin-right" => parse_length_px(value).map(DeclKind::ScrollMarginRight),
        "scroll-margin-bottom" => parse_length_px(value).map(DeclKind::ScrollMarginBottom),
        "scroll-margin-left" => parse_length_px(value).map(DeclKind::ScrollMarginLeft),
        // Fase 7.415 — `scroll-margin-block-start` = top en LTR horizontal.
        "scroll-margin-block-start" => {
            parse_length_px(value).map(DeclKind::ScrollMarginTop)
        }
        // Fase 7.416 — `scroll-margin-block-end` = bottom en LTR horizontal.
        "scroll-margin-block-end" => {
            parse_length_px(value).map(DeclKind::ScrollMarginBottom)
        }
        // Fase 7.418 — `scroll-margin-inline-start` = left en LTR horizontal.
        "scroll-margin-inline-start" => {
            parse_length_px(value).map(DeclKind::ScrollMarginLeft)
        }
        // Fase 7.419 — `scroll-margin-inline-end` = right en LTR horizontal.
        "scroll-margin-inline-end" => {
            parse_length_px(value).map(DeclKind::ScrollMarginRight)
        }
        // Fase 7.421 — `scroll-padding-block-start` = top en LTR horizontal.
        "scroll-padding-block-start" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingTop)
        }
        // Fase 7.422 — `scroll-padding-block-end` = bottom en LTR horizontal.
        "scroll-padding-block-end" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingBottom)
        }
        // Fase 7.424 — `scroll-padding-inline-start` = left en LTR horizontal.
        "scroll-padding-inline-start" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingLeft)
        }
        // Fase 7.425 — `scroll-padding-inline-end` = right en LTR horizontal.
        "scroll-padding-inline-end" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingRight)
        }
        // Fase 7.427 — `offset-path` (CSS Motion Path 1). Plumb: guarda el
        // valor crudo (sin parsear `path(...)` / `ray(...)` / `<url>`).
        // `none` o vacío → `None`. NO hereda.
        "offset-path" => {
            let raw = value.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
                Some(DeclKind::OffsetPath(None))
            } else {
                Some(DeclKind::OffsetPath(Some(raw.to_string())))
            }
        }
        // Fase 7.428 — `offset-distance` (CSS Motion Path 1). Acepta
        // length o porcentaje (no `auto`). NO hereda.
        "offset-distance" => {
            parse_length_or_pct(value).map(DeclKind::OffsetDistance)
        }
        // Fase 7.429 — `hyphenate-character` (CSS Text 4). `auto` o un
        // string entre comillas. HEREDA. Plumb (no rompemos palabras).
        "hyphenate-character" => Some(DeclKind::HyphenateCharacter(
            parse_hyphenate_character(value),
        )),
        // Fase 7.430 — `hyphenate-limit-chars`. `auto | <int>{1,3}`. HEREDA.
        "hyphenate-limit-chars" => {
            parse_hyphenate_limit_chars(value).map(DeclKind::HyphenateLimitChars)
        }
        // Fase 7.431 — `text-size-adjust` (CSS Text Inline 3). HEREDA. Plumb.
        "text-size-adjust" | "-webkit-text-size-adjust" => {
            parse_text_size_adjust(value).map(DeclKind::TextSizeAdjust)
        }
        // Fase 7.432 — `line-height-step` (CSS Text 4 draft). HEREDA. Plumb.
        "line-height-step" => {
            parse_length_px(value).map(DeclKind::LineHeightStep)
        }
        // Fase 7.433 — `font-variant-emoji` (CSS Fonts 4). HEREDA. Plumb.
        "font-variant-emoji" => {
            parse_font_variant_emoji(value).map(DeclKind::FontVariantEmoji)
        }
        // Fase 7.434 — `contain-intrinsic-width` (CSS Containment 3). NO hereda. Plumb.
        "contain-intrinsic-width" => {
            parse_contain_intrinsic_size(value).map(DeclKind::ContainIntrinsicWidth)
        }
        // Fase 7.435 — `contain-intrinsic-height` (CSS Containment 3). NO hereda. Plumb.
        "contain-intrinsic-height" => {
            parse_contain_intrinsic_size(value).map(DeclKind::ContainIntrinsicHeight)
        }
        // Fase 7.436 — `contain-intrinsic-block-size` = height en horizontal LTR.
        "contain-intrinsic-block-size" => {
            parse_contain_intrinsic_size(value).map(DeclKind::ContainIntrinsicHeight)
        }
        // Fase 7.437 — `contain-intrinsic-inline-size` = width en horizontal LTR.
        "contain-intrinsic-inline-size" => {
            parse_contain_intrinsic_size(value).map(DeclKind::ContainIntrinsicWidth)
        }
        // Fase 7.438 — `contain-intrinsic-size` shorthand: ver `parse_declarations`.
        // Fase 7.439 — `background-position-x` (CSS Backgrounds 3). NO hereda.
        "background-position-x" => {
            parse_background_position_x(value).map(DeclKind::BackgroundPositionX)
        }
        // Fase 7.440 — `background-position-y` (CSS Backgrounds 3). NO hereda.
        "background-position-y" => {
            parse_background_position_y(value).map(DeclKind::BackgroundPositionY)
        }
        // Fase 7.441 — `grid-auto-flow` (CSS Grid 1). NO hereda.
        "grid-auto-flow" => parse_grid_auto_flow(value).map(DeclKind::GridAutoFlow),
        // Fase 7.442 — `grid-auto-columns` (CSS Grid 1). NO hereda.
        "grid-auto-columns" => parse_grid_template(value).map(DeclKind::GridAutoColumns),
        // Fase 7.443 — `grid-auto-rows` (CSS Grid 1). NO hereda.
        "grid-auto-rows" => parse_grid_template(value).map(DeclKind::GridAutoRows),
        // Fase 7.444 — `shape-outside` (CSS Shapes 1). Parse opaco (igual que
        // offset-path) — guardamos el valor crudo. NO hereda.
        "shape-outside" => {
            let raw = value.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
                Some(DeclKind::ShapeOutside(None))
            } else {
                Some(DeclKind::ShapeOutside(Some(raw.to_string())))
            }
        }
        // Fase 7.445 — `shape-margin` (CSS Shapes 1). `<length-or-pct>`
        // no-negativo. NO hereda.
        "shape-margin" => {
            let v = parse_length_or_pct(value)?;
            match v {
                LengthVal::Px(n) if n < 0.0 => None,
                LengthVal::Pct(n) if n < 0.0 => None,
                _ => Some(DeclKind::ShapeMargin(v)),
            }
        }
        // Fase 7.446 — `shape-image-threshold` (CSS Shapes 1). Alpha [0..1].
        // Acepta también porcentaje (50% → 0.5). NO hereda.
        "shape-image-threshold" => {
            parse_alpha_value(value).map(DeclKind::ShapeImageThreshold)
        }
        // Fase 7.447 — `text-combine-upright` (CSS Writing Modes 3). NO hereda.
        "text-combine-upright" => {
            parse_text_combine_upright(value).map(DeclKind::TextCombineUpright)
        }
        // Fase 7.448 — `ruby-align` (CSS Ruby 1). HEREDA.
        "ruby-align" => parse_ruby_align(value).map(DeclKind::RubyAlign),
        // `scroll-margin-block` (Fase 7.417), `scroll-margin-inline` (Fase
        // 7.420), `scroll-padding-block` (Fase 7.423), `scroll-padding-inline`
        // (Fase 7.426) shorthands: ver `parse_declarations`.
        "touch-action" => parse_touch_action(value).map(DeclKind::TouchAction),
        "clip-path" | "-webkit-clip-path" => Some(DeclKind::ClipPath(parse_clip_path(value))),
        "mask-image" => Some(DeclKind::MaskImage(parse_mask_image(value))),
        // `mask` shorthand: hoy sólo el subset image (igual que mask-image).
        "mask" | "-webkit-mask" | "-webkit-mask-image" => {
            Some(DeclKind::MaskImage(parse_mask_image(value)))
        }
        "content-visibility" => {
            parse_content_visibility(value).map(DeclKind::ContentVisibility)
        }
        "contain" => parse_contain(value).map(DeclKind::Contain),
        "column-count" => Some(DeclKind::ColumnCount(parse_column_count(value))),
        "column-width" => parse_length_or_pct(value).map(DeclKind::ColumnWidth),
        "column-rule-width" => parse_length_px(value).map(DeclKind::ColumnRuleWidth),
        "column-rule-color" => {
            if is_current_color(value) {
                Some(DeclKind::ColumnRuleColor(None))
            } else {
                parse_color(value).map(|c| DeclKind::ColumnRuleColor(Some(c)))
            }
        }
        // `column-rule-style` y `column-rule` van por `parse_declarations`.
        "column-fill" => parse_column_fill(value).map(DeclKind::ColumnFill),
        "column-span" => parse_column_span(value).map(DeclKind::ColumnSpan),
        // `page-break-inside` (legacy CSS 2.1) = `break-inside` (subset).
        "break-inside" | "page-break-inside" => {
            parse_break_inside(value).map(DeclKind::BreakInside)
        }
        "table-layout" => parse_table_layout(value).map(DeclKind::TableLayout),
        "border-collapse" => parse_border_collapse(value).map(DeclKind::BorderCollapse),
        "border-spacing" => parse_border_spacing(value).map(|(h, v)| DeclKind::BorderSpacing { h, v }),
        "caption-side" => parse_caption_side(value).map(DeclKind::CaptionSide),
        "empty-cells" => parse_empty_cells(value).map(DeclKind::EmptyCells),
        // `break-before` / `break-after` (CSS Fragmentation 3) + alias
        // legacy `page-break-before` / `page-break-after` (CSS 2.1, subset
        // auto/avoid/always/left/right).
        "break-before" | "page-break-before" => {
            parse_break_between(value).map(DeclKind::BreakBefore)
        }
        "break-after" | "page-break-after" => {
            parse_break_between(value).map(DeclKind::BreakAfter)
        }
        "orphans" => parse_positive_int(value).map(DeclKind::Orphans),
        "widows" => parse_positive_int(value).map(DeclKind::Widows),
        "color-scheme" => parse_color_scheme(value).map(DeclKind::ColorScheme),
        "counter-set" => Some(DeclKind::CounterSet(parse_counter_list(value, 0))),
        "quotes" => Some(DeclKind::Quotes(parse_quotes(value))),
        "text-underline-position" => {
            parse_text_underline_position(value).map(DeclKind::TextUnderlinePosition)
        }
        "text-justify" => parse_text_justify(value).map(DeclKind::TextJustify),
        // `color-adjust` es alias legacy de `print-color-adjust`.
        "print-color-adjust" | "color-adjust" => {
            parse_print_color_adjust(value).map(DeclKind::PrintColorAdjust)
        }
        "forced-color-adjust" => {
            parse_forced_color_adjust(value).map(DeclKind::ForcedColorAdjust)
        }
        // `-webkit-line-clamp` (de facto estándar) y `line-clamp` (CSS Overflow 4).
        "line-clamp" | "-webkit-line-clamp" => Some(DeclKind::LineClamp(parse_line_clamp(value))),
        "font-variant-caps" => {
            parse_font_variant_caps(value).map(DeclKind::FontVariantCaps)
        }
        "font-variant-numeric" => {
            parse_font_variant_numeric(value).map(DeclKind::FontVariantNumeric)
        }
        "font-variant-ligatures" => {
            parse_font_variant_ligatures(value).map(DeclKind::FontVariantLigatures)
        }
        "font-variant-east-asian" => {
            parse_font_variant_east_asian(value).map(DeclKind::FontVariantEastAsian)
        }
        "font-variant-position" => {
            parse_font_variant_position(value).map(DeclKind::FontVariantPosition)
        }
        "text-emphasis-style" => {
            parse_text_emphasis_style(value).map(DeclKind::TextEmphasisStyle)
        }
        "text-emphasis-color" => {
            if is_current_color(value) {
                Some(DeclKind::TextEmphasisColor(None))
            } else {
                parse_color(value).map(|c| DeclKind::TextEmphasisColor(Some(c)))
            }
        }
        "text-emphasis-position" => {
            parse_text_emphasis_position(value).map(DeclKind::TextEmphasisPosition)
        }
        // `text-emphasis` shorthand: ver `parse_declarations`.
        "ruby-position" => parse_ruby_position(value).map(DeclKind::RubyPosition),
        "transform-origin" => {
            parse_transform_origin(value).map(DeclKind::TransformOrigin)
        }
        "transform-style" => {
            parse_transform_style(value).map(DeclKind::TransformStyle)
        }
        "perspective" => parse_perspective(value).map(DeclKind::Perspective),
        "perspective-origin" => {
            parse_perspective_origin(value).map(DeclKind::PerspectiveOrigin)
        }
        "backface-visibility" => {
            parse_backface_visibility(value).map(DeclKind::BackfaceVisibility)
        }
        "scrollbar-width" => {
            parse_scrollbar_width(value).map(DeclKind::ScrollbarWidth)
        }
        "scrollbar-color" => {
            parse_scrollbar_color(value).map(DeclKind::ScrollbarColor)
        }
        "scrollbar-gutter" => {
            parse_scrollbar_gutter(value).map(DeclKind::ScrollbarGutter)
        }
        "overflow-anchor" => {
            parse_overflow_anchor(value).map(DeclKind::OverflowAnchor)
        }
        "overflow-clip-margin" => {
            parse_overflow_clip_margin(value).map(DeclKind::OverflowClipMargin)
        }
        "text-align-last" => {
            parse_text_align_last(value).map(DeclKind::TextAlignLast)
        }
        "text-wrap" => parse_text_wrap(value).map(DeclKind::TextWrap),
        "line-break" => parse_line_break(value).map(DeclKind::LineBreak),
        "hanging-punctuation" => {
            parse_hanging_punctuation(value).map(DeclKind::HangingPunctuation)
        }
        "text-decoration-skip-ink" => {
            parse_text_decoration_skip_ink(value)
                .map(DeclKind::TextDecorationSkipInk)
        }
        "font-optical-sizing" => {
            parse_font_optical_sizing(value).map(DeclKind::FontOpticalSizing)
        }
        "font-synthesis-weight" => {
            parse_auto_or_none(value).map(DeclKind::FontSynthesisWeight)
        }
        "font-synthesis-style" => {
            parse_auto_or_none(value).map(DeclKind::FontSynthesisStyle)
        }
        "font-synthesis-small-caps" => {
            parse_auto_or_none(value).map(DeclKind::FontSynthesisSmallCaps)
        }
        // `font-synthesis` shorthand: ver `parse_declarations`.
        "font-size-adjust" => {
            parse_font_size_adjust(value).map(DeclKind::FontSizeAdjust)
        }
        "image-orientation" => {
            parse_image_orientation(value).map(DeclKind::ImageOrientation)
        }
        "animation-timeline" => {
            parse_timeline_ref(value).map(DeclKind::AnimationTimeline)
        }
        "scroll-timeline-name" => {
            parse_dashed_ident_or_none(value).map(DeclKind::ScrollTimelineName)
        }
        "scroll-timeline-axis" => {
            parse_timeline_axis(value).map(DeclKind::ScrollTimelineAxis)
        }
        "view-timeline-name" => {
            parse_dashed_ident_or_none(value).map(DeclKind::ViewTimelineName)
        }
        "view-timeline-axis" => {
            parse_timeline_axis(value).map(DeclKind::ViewTimelineAxis)
        }
        "white-space-collapse" => {
            parse_white_space_collapse(value).map(DeclKind::WhiteSpaceCollapse)
        }
        "text-wrap-mode" => {
            parse_text_wrap_mode(value).map(DeclKind::TextWrapMode)
        }
        "text-wrap-style" => {
            parse_text_wrap_style(value).map(DeclKind::TextWrapStyle)
        }
        "text-spacing-trim" => {
            parse_text_spacing_trim(value).map(DeclKind::TextSpacingTrim)
        }
        "text-box-trim" => {
            parse_text_box_trim(value).map(DeclKind::TextBoxTrim)
        }
        "math-style" => parse_math_style(value).map(DeclKind::MathStyle),
        "math-depth" => parse_math_depth(value).map(DeclKind::MathDepth),
        "math-shift" => parse_math_shift(value).map(DeclKind::MathShift),
        "field-sizing" => {
            parse_field_sizing(value).map(DeclKind::FieldSizing)
        }
        "text-box-edge" => {
            parse_text_box_edge(value).map(DeclKind::TextBoxEdge)
        }
        "anchor-name" => parse_ident_list_or_none(value).map(DeclKind::AnchorName),
        "position-anchor" => {
            parse_ident_or_auto(value).map(DeclKind::PositionAnchor)
        }
        "anchor-scope" => {
            parse_anchor_scope(value).map(DeclKind::AnchorScope)
        }
        "view-transition-name" => {
            parse_dashed_ident_or_none(value).map(DeclKind::ViewTransitionName)
        }
        "view-transition-class" => {
            parse_ident_list_or_none(value).map(DeclKind::ViewTransitionClass)
        }
        "font-palette" => parse_font_palette(value).map(DeclKind::FontPalette),
        "font-variant-alternates" => parse_font_variant_alternates(value)
            .map(DeclKind::FontVariantAlternates),
        "background-attachment" => {
            parse_background_attachment(value).map(DeclKind::BackgroundAttachment)
        }
        "caret-shape" => parse_caret_shape(value).map(DeclKind::CaretShape),
        "baseline-source" => {
            parse_baseline_source(value).map(DeclKind::BaselineSource)
        }
        "alignment-baseline" => {
            parse_alignment_baseline(value).map(DeclKind::AlignmentBaseline)
        }
        "dominant-baseline" => {
            parse_dominant_baseline(value).map(DeclKind::DominantBaseline)
        }
        "paint-order" => parse_paint_order(value).map(DeclKind::PaintOrder),
        "marker-side" => parse_marker_side(value).map(DeclKind::MarkerSide),
        "fill" => parse_svg_paint(value).map(DeclKind::Fill),
        "stroke" => parse_svg_paint(value).map(DeclKind::Stroke),
        "fill-opacity" => parse_svg_opacity(value).map(DeclKind::FillOpacity),
        "stroke-opacity" => {
            parse_svg_opacity(value).map(DeclKind::StrokeOpacity)
        }
        "stroke-width" => {
            parse_length_or_pct(value).map(DeclKind::StrokeWidth)
        }
        "stroke-linecap" => {
            parse_stroke_linecap(value).map(DeclKind::StrokeLinecap)
        }
        "stroke-linejoin" => {
            parse_stroke_linejoin(value).map(DeclKind::StrokeLinejoin)
        }
        "stroke-miterlimit" => {
            parse_stroke_miterlimit(value).map(DeclKind::StrokeMiterlimit)
        }
        "stroke-dasharray" => {
            parse_stroke_dasharray(value).map(DeclKind::StrokeDasharray)
        }
        "stroke-dashoffset" => {
            parse_length_or_pct(value).map(DeclKind::StrokeDashoffset)
        }
        "fill-rule" => parse_fill_rule(value).map(DeclKind::FillRule),
        "clip-rule" => parse_fill_rule(value).map(DeclKind::ClipRule),
        "color-interpolation" => {
            parse_color_interpolation(value).map(DeclKind::ColorInterpolation)
        }
        "shape-rendering" => {
            parse_shape_rendering(value).map(DeclKind::ShapeRendering)
        }
        "vector-effect" => {
            parse_vector_effect(value).map(DeclKind::VectorEffect)
        }
        "text-anchor" => parse_text_anchor(value).map(DeclKind::TextAnchor),
        "color-rendering" => {
            parse_color_rendering(value).map(DeclKind::ColorRendering)
        }
        "color-interpolation-filters" => parse_color_interpolation_filters(value)
            .map(DeclKind::ColorInterpolationFilters),
        "glyph-orientation-vertical" => parse_glyph_orientation_vertical(value)
            .map(DeclKind::GlyphOrientationVertical),
        "transform-box" => parse_transform_box(value).map(DeclKind::TransformBox),
        "marker-start" => {
            parse_marker_ref(value).map(DeclKind::MarkerStart)
        }
        "marker-mid" => parse_marker_ref(value).map(DeclKind::MarkerMid),
        "marker-end" => parse_marker_ref(value).map(DeclKind::MarkerEnd),
        "mask-type" => parse_mask_type(value).map(DeclKind::MaskType),
        "mask-mode" => parse_mask_mode(value).map(DeclKind::MaskMode),
        "mask-clip" => parse_mask_clip(value).map(DeclKind::MaskClip),
        "mask-composite" => {
            parse_mask_composite(value).map(DeclKind::MaskComposite)
        }
        "mask-origin" => parse_mask_origin(value).map(DeclKind::MaskOrigin),
        "mask-repeat" => {
            // Reusa `parse_background_repeat` (devuelve `DeclKind::BackgroundRepeat`);
            // extraemos el valor y lo reemitimos como `MaskRepeat`.
            match parse_background_repeat(value) {
                Some(DeclKind::BackgroundRepeat(r)) => {
                    Some(DeclKind::MaskRepeat(r))
                }
                _ => None,
            }
        }
        "mask-position" => match parse_background_position(value) {
            Some(DeclKind::BackgroundPosition(p)) => {
                Some(DeclKind::MaskPosition(p))
            }
            _ => None,
        },
        "mask-size" => match parse_background_size(value) {
            Some(DeclKind::BackgroundSize(sz)) => {
                Some(DeclKind::MaskSize(sz))
            }
            _ => None,
        },
        "container-name" => {
            parse_ident_list_or_none(value).map(DeclKind::ContainerName)
        }
        "container-type" => {
            parse_container_type(value).map(DeclKind::ContainerType)
        }
        "flood-color" => {
            parse_color_or_current(value).map(DeclKind::FloodColor)
        }
        "flood-opacity" => parse_svg_opacity(value).map(DeclKind::FloodOpacity),
        "lighting-color" => {
            parse_color_or_current(value).map(DeclKind::LightingColor)
        }
        "stop-color" => {
            parse_color_or_current(value).map(DeclKind::StopColor)
        }
        "stop-opacity" => parse_svg_opacity(value).map(DeclKind::StopOpacity),
        // `columns` shorthand: ver `parse_declarations`.
        // `place-items`, `place-content`, `place-self`: ver `parse_declarations`.
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

/// Parsea `box-shadow: <s1>[, <s2>...]` o `box-shadow: none`. Cada
/// sombra: `[inset] <offset-x> <offset-y> [blur] [spread] <color>`,
/// tokens en cualquier orden. Sombras inválidas se descartan en
/// silencio; si la lista queda vacía devuelve un vec vacío (= `none`).
pub(crate) fn parse_box_shadows(value: &str) -> Vec<BoxShadow> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") || v.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for sh in v.split(',') {
        if let Some(s) = parse_one_box_shadow(sh) {
            out.push(s);
        }
    }
    out
}

fn parse_one_box_shadow(s: &str) -> Option<BoxShadow> {
    let mut lengths: Vec<f32> = Vec::with_capacity(4);
    let mut color: Option<Color> = None;
    let mut inset = false;
    for tok in s.split_whitespace() {
        if tok.eq_ignore_ascii_case("inset") {
            inset = true;
            continue;
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
        inset,
    })
}

pub(crate) fn parse_border_style(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" => {
            Some(true)
        }
        "none" | "hidden" => Some(false),
        _ => None,
    }
}

/// Mapea un keyword de `border-style` al patrón visual. `none`/`hidden` →
/// `None` (no togglea estilo, sólo el enabled). `groove`/`ridge`/
/// `inset`/`outset` reciben render 3D real desde la Fase 7.237.
pub(crate) fn parse_border_line_style(s: &str) -> Option<BorderLineStyle> {
    match s.trim().to_ascii_lowercase().as_str() {
        "solid" => Some(BorderLineStyle::Solid),
        "dashed" => Some(BorderLineStyle::Dashed),
        "dotted" => Some(BorderLineStyle::Dotted),
        "double" => Some(BorderLineStyle::Double),
        "groove" => Some(BorderLineStyle::Groove),
        "ridge" => Some(BorderLineStyle::Ridge),
        "inset" => Some(BorderLineStyle::Inset),
        "outset" => Some(BorderLineStyle::Outset),
        _ => None,
    }
}

/// `caret-color`: `auto`/`currentColor` → `None` (= seguir al color
/// heredado); de lo contrario, color CSS. Si nada matchea, `None`
/// (default seguro = auto, no se dropea la regla).
pub(crate) fn parse_caret_color(value: &str) -> Option<Color> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") || is_current_color(v) {
        return None;
    }
    parse_color(v)
}

/// `accent-color`: `auto` → `None`; de lo contrario, color CSS.
/// Sin `currentColor` por espec (CSS UI 4).
pub(crate) fn parse_auto_or_color(value: &str) -> Option<Color> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") {
        return None;
    }
    parse_color(v)
}

/// `cursor`: subset reconocido (los más comunes en web). Valores no
/// listados (incluyendo el fallback `url(...) x y`) caen a `Auto`
/// para no dropear la regla. Case-insensitive.
pub(crate) fn parse_cursor(value: &str) -> Option<Cursor> {
    let v = value.trim().to_ascii_lowercase();
    // `cursor` puede traer una lista `url(...), pointer` — tomamos el
    // último token reconocido (= el fallback CSS), no el primer url.
    let last = v.split(',').last()?.trim();
    Some(match last {
        "auto" => Cursor::Auto,
        "default" => Cursor::Default,
        "pointer" => Cursor::Pointer,
        "text" => Cursor::Text,
        "wait" => Cursor::Wait,
        "help" => Cursor::Help,
        "crosshair" => Cursor::Crosshair,
        "move" => Cursor::Move,
        "not-allowed" => Cursor::NotAllowed,
        "grab" => Cursor::Grab,
        "grabbing" => Cursor::Grabbing,
        "zoom-in" => Cursor::ZoomIn,
        "zoom-out" => Cursor::ZoomOut,
        "e-resize" => Cursor::EResize,
        "n-resize" => Cursor::NResize,
        "s-resize" => Cursor::SResize,
        "w-resize" => Cursor::WResize,
        "ns-resize" => Cursor::NsResize,
        "ew-resize" => Cursor::EwResize,
        "nesw-resize" => Cursor::NeswResize,
        "nwse-resize" => Cursor::NwseResize,
        "row-resize" => Cursor::RowResize,
        "col-resize" => Cursor::ColResize,
        _ => Cursor::Auto,
    })
}

/// `text-overflow`: `clip` (default, recorta a la línea) | `ellipsis`
/// (muestra `…`). Strings custom de CSS3 (`text-overflow: "—"`) y `fade`
/// quedan fuera. Case-insensitive.
pub(crate) fn parse_text_overflow(value: &str) -> Option<TextOverflow> {
    match value.trim().to_ascii_lowercase().as_str() {
        "clip" => Some(TextOverflow::Clip),
        "ellipsis" => Some(TextOverflow::Ellipsis),
        _ => None,
    }
}

/// `scroll-behavior`: `auto` (instant) | `smooth` (animado).
pub(crate) fn parse_scroll_behavior(value: &str) -> Option<ScrollBehavior> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ScrollBehavior::Auto),
        "smooth" => Some(ScrollBehavior::Smooth),
        _ => None,
    }
}

/// `user-select`: subset CSS UI 4. Case-insensitive. `none` desactiva
/// la selección por mouse; `text` la fuerza incluso en elementos donde
/// el UA la suprime; `all` selecciona el subárbol entero al click;
/// `contain` aísla la selección al elemento (sin propagar al padre).
pub(crate) fn parse_user_select(value: &str) -> Option<UserSelect> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(UserSelect::Auto),
        "none" => Some(UserSelect::None),
        "text" => Some(UserSelect::Text),
        "all" => Some(UserSelect::All),
        "contain" => Some(UserSelect::Contain),
        _ => None,
    }
}

/// `overflow-wrap`: `normal` (quiebres del idioma), `break-word`
/// (cualquier punto si no entra), `anywhere` (idem `break-word` pero
/// además contribuye al `min-content`). Alias `word-wrap`.
pub(crate) fn parse_overflow_wrap(value: &str) -> Option<OverflowWrap> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(OverflowWrap::Normal),
        "break-word" => Some(OverflowWrap::BreakWord),
        "anywhere" => Some(OverflowWrap::Anywhere),
        _ => None,
    }
}

/// `word-break`: `normal`, `break-all` (cualquier carácter, típico CJK),
/// `keep-all` (sólo separadores reales). `break-word` legacy se mapea a
/// `Normal` por compat (CSS spec dice computar a `normal` y setear
/// `overflow-wrap: anywhere` — acá no lo cruzamos para no acoplar).
pub(crate) fn parse_word_break(value: &str) -> Option<WordBreak> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(WordBreak::Normal),
        "break-all" => Some(WordBreak::BreakAll),
        "keep-all" => Some(WordBreak::KeepAll),
        "break-word" => Some(WordBreak::Normal),
        _ => None,
    }
}

/// `hyphens`: control de hyphenation. `auto` requeriría diccionarios
/// por idioma — lo aceptamos como valor pero el shaper no lo aplica.
pub(crate) fn parse_hyphens(value: &str) -> Option<Hyphens> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Some(Hyphens::None),
        "manual" => Some(Hyphens::Manual),
        "auto" => Some(Hyphens::Auto),
        _ => None,
    }
}

/// `resize`: el usuario arrastra el borde para redimensionar.
/// `block`/`inline` mapean a vertical/horizontal en `writing-mode`
/// horizontal-tb (el único que soportamos). Sólo aplica si el elemento
/// tiene `overflow != visible` por spec; ese chequeo queda al consumidor.
pub(crate) fn parse_resize(value: &str) -> Option<Resize> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Some(Resize::None),
        "both" => Some(Resize::Both),
        "horizontal" => Some(Resize::Horizontal),
        "vertical" => Some(Resize::Vertical),
        "block" => Some(Resize::Block),
        "inline" => Some(Resize::Inline),
        _ => None,
    }
}

/// `writing-mode`: orientación de bloque. Soporta los 5 valores
/// modernos. Case-insensitive. Inválido = `None` (dropea la regla).
pub(crate) fn parse_writing_mode(value: &str) -> Option<WritingMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "horizontal-tb" => Some(WritingMode::HorizontalTb),
        "vertical-rl" => Some(WritingMode::VerticalRl),
        "vertical-lr" => Some(WritingMode::VerticalLr),
        "sideways-rl" => Some(WritingMode::SidewaysRl),
        "sideways-lr" => Some(WritingMode::SidewaysLr),
        // Aliases legacy (`lr-tb`, `tb-rl`, `tb-lr`) y `tb` quedan fuera.
        _ => None,
    }
}

/// `direction`: dirección base. Case-insensitive.
pub(crate) fn parse_direction(value: &str) -> Option<Direction> {
    match value.trim().to_ascii_lowercase().as_str() {
        "ltr" => Some(Direction::Ltr),
        "rtl" => Some(Direction::Rtl),
        _ => None,
    }
}

/// `unicode-bidi`: 6 valores. Case-insensitive.
pub(crate) fn parse_unicode_bidi(value: &str) -> Option<UnicodeBidi> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(UnicodeBidi::Normal),
        "embed" => Some(UnicodeBidi::Embed),
        "isolate" => Some(UnicodeBidi::Isolate),
        "bidi-override" => Some(UnicodeBidi::BidiOverride),
        "isolate-override" => Some(UnicodeBidi::IsolateOverride),
        "plaintext" => Some(UnicodeBidi::Plaintext),
        _ => None,
    }
}

/// `font-stretch`: keyword o porcentaje 50%..200%. Devuelve el
/// multiplicador (1.0 = normal). Valores fuera del rango se clampan
/// — coherente con CSS Fonts 4 (`font-stretch: 1%` y `200%` se aceptan,
/// pero acá conservamos el rango oficial de keywords).
pub(crate) fn parse_font_stretch(value: &str) -> Option<f32> {
    let v = value.trim().to_ascii_lowercase();
    let kw = match v.as_str() {
        "ultra-condensed" => Some(0.50),
        "extra-condensed" => Some(0.625),
        "condensed" => Some(0.75),
        "semi-condensed" => Some(0.875),
        "normal" => Some(1.0),
        "semi-expanded" => Some(1.125),
        "expanded" => Some(1.25),
        "extra-expanded" => Some(1.50),
        "ultra-expanded" => Some(2.00),
        _ => None,
    };
    if let Some(k) = kw {
        return Some(k);
    }
    // `Npc%`.
    if let Some(pct) = v.strip_suffix('%') {
        if let Ok(p) = pct.trim().parse::<f32>() {
            if p >= 0.0 {
                return Some((p / 100.0).clamp(0.5, 2.0));
            }
        }
    }
    None
}

/// `image-rendering`: 4 keywords. Case-insensitive. `optimizeSpeed` /
/// `optimizeQuality` (CSS2 legacy) caen a `Auto` por compat.
pub(crate) fn parse_image_rendering(value: &str) -> Option<ImageRendering> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ImageRendering::Auto),
        "smooth" => Some(ImageRendering::Smooth),
        "crisp-edges" => Some(ImageRendering::CrispEdges),
        "pixelated" => Some(ImageRendering::Pixelated),
        "optimizespeed" | "optimizequality" => Some(ImageRendering::Auto),
        _ => None,
    }
}

/// `mix-blend-mode` / cada item de `background-blend-mode`. Subset
/// W3C Compositing 1. `plus-lighter` aceptado por compat.
pub(crate) fn parse_blend_mode(value: &str) -> Option<BlendMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(BlendMode::Normal),
        "multiply" => Some(BlendMode::Multiply),
        "screen" => Some(BlendMode::Screen),
        "overlay" => Some(BlendMode::Overlay),
        "darken" => Some(BlendMode::Darken),
        "lighten" => Some(BlendMode::Lighten),
        "color-dodge" => Some(BlendMode::ColorDodge),
        "color-burn" => Some(BlendMode::ColorBurn),
        "hard-light" => Some(BlendMode::HardLight),
        "soft-light" => Some(BlendMode::SoftLight),
        "difference" => Some(BlendMode::Difference),
        "exclusion" => Some(BlendMode::Exclusion),
        "hue" => Some(BlendMode::Hue),
        "saturation" => Some(BlendMode::Saturation),
        "color" => Some(BlendMode::Color),
        "luminosity" => Some(BlendMode::Luminosity),
        "plus-lighter" => Some(BlendMode::PlusLighter),
        _ => None,
    }
}

/// `background-blend-mode: m1, m2, ...`. Tokens inválidos caen a
/// `Normal` para no desalinear la lista con las capas de background.
pub(crate) fn parse_blend_mode_list(value: &str) -> Vec<BlendMode> {
    value
        .split(',')
        .map(|item| parse_blend_mode(item.trim()).unwrap_or(BlendMode::Normal))
        .collect()
}

/// `isolation`: 2 valores.
pub(crate) fn parse_isolation(value: &str) -> Option<Isolation> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(Isolation::Auto),
        "isolate" => Some(Isolation::Isolate),
        _ => None,
    }
}

/// `will-change: auto | <feature>[, <feature>...]`. CSS spec exige que
/// `auto` aparezca solo; aceptamos cualquier tokenizado y filtramos
/// `auto`/strings vacíos. Las features no reconocidas se guardan como
/// `Property(token)` (en lowercase). Devuelve `Vec` vacío para `auto`
/// o lista vacía.
pub(crate) fn parse_will_change(value: &str) -> Vec<WillChangeHint> {
    let mut out = Vec::new();
    for item in value.split(',') {
        let token = item.trim().to_ascii_lowercase();
        if token.is_empty() || token == "auto" {
            continue;
        }
        out.push(match token.as_str() {
            "scroll-position" => WillChangeHint::ScrollPosition,
            "contents" => WillChangeHint::Contents,
            _ => WillChangeHint::Property(token),
        });
    }
    out
}

/// `appearance` (CSS UI 4): subset de keywords. Cualquier otro
/// keyword conocido legacy (`searchfield`, `slider-horizontal`, etc.)
/// cae a `Auto` para no dropear la regla. Inválido total = `None`.
pub(crate) fn parse_appearance(value: &str) -> Option<Appearance> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Some(Appearance::None),
        "auto" => Some(Appearance::Auto),
        "textfield" => Some(Appearance::Textfield),
        "menulist-button" => Some(Appearance::MenulistButton),
        "button" => Some(Appearance::Button),
        "checkbox" => Some(Appearance::Checkbox),
        "radio" => Some(Appearance::Radio),
        // Compats legacy → `Auto` (no rechazo).
        "searchfield"
        | "slider-horizontal"
        | "menulist"
        | "listbox"
        | "meter"
        | "progress-bar"
        | "push-button"
        | "square-button"
        | "textarea" => Some(Appearance::Auto),
        _ => None,
    }
}

/// `filter` / `backdrop-filter`: lista de funciones. `none` = lista
/// vacía. Funciones desconocidas se descartan; las conocidas con
/// argumento malformado también. Reusa `parse_box_shadows` para el caso
/// `drop-shadow(...)`.
pub(crate) fn parse_filter_list(value: &str) -> Vec<FilterFn> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    // Tokenizar respetando paréntesis: `blur(2px) drop-shadow(1px 1px red)`.
    let mut out = Vec::new();
    let mut chars = v.char_indices().peekable();
    while let Some(&(start, _)) = chars.peek() {
        // Skip whitespace.
        while let Some(&(_, c)) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }
        let Some(&(name_start, _)) = chars.peek() else {
            break;
        };
        // Avanzar hasta `(`.
        let mut name_end = name_start;
        let mut found_paren = false;
        while let Some(&(idx, c)) = chars.peek() {
            if c == '(' {
                name_end = idx;
                found_paren = true;
                chars.next();
                break;
            }
            chars.next();
            name_end = idx + c.len_utf8();
        }
        let _ = start;
        if !found_paren {
            break;
        }
        // Buscar el `)` que cierra (sin nesting — drop-shadow no anida).
        let mut depth: i32 = 1;
        let mut arg_end = name_end + 1;
        while let Some((idx, c)) = chars.next() {
            arg_end = idx + c.len_utf8();
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    arg_end = idx;
                    break;
                }
            }
        }
        let name = v[name_start..name_end].trim().to_ascii_lowercase();
        let arg = v[name_end + 1..arg_end].trim();
        if let Some(f) = parse_filter_fn(&name, arg) {
            out.push(f);
        }
    }
    out
}

fn parse_filter_fn(name: &str, arg: &str) -> Option<FilterFn> {
    match name {
        "blur" => parse_length_px(arg).map(FilterFn::Blur),
        "brightness" => parse_number_or_pct(arg).map(FilterFn::Brightness),
        "contrast" => parse_number_or_pct(arg).map(FilterFn::Contrast),
        "grayscale" => parse_number_or_pct(arg).map(FilterFn::Grayscale),
        "hue-rotate" => parse_angle_deg(arg).map(FilterFn::HueRotate),
        "invert" => parse_number_or_pct(arg).map(FilterFn::Invert),
        "opacity" => parse_number_or_pct(arg).map(FilterFn::Opacity),
        "saturate" => parse_number_or_pct(arg).map(FilterFn::Saturate),
        "sepia" => parse_number_or_pct(arg).map(FilterFn::Sepia),
        "drop-shadow" => {
            // Reusa el shape de box-shadow pero sólo 1.
            parse_box_shadows(arg).into_iter().next().map(FilterFn::DropShadow)
        }
        _ => None,
    }
}

/// Parsea `<number>` o `<percentage>` devolviendo un f32 (50% → 0.5).
/// Negativo se conserva (algunos filtros lo aceptan).
fn parse_number_or_pct(s: &str) -> Option<f32> {
    let v = s.trim();
    if let Some(pct) = v.strip_suffix('%') {
        return pct.trim().parse::<f32>().ok().map(|n| n / 100.0);
    }
    v.parse::<f32>().ok()
}

/// Parsea `<angle>` (deg | rad | turn | grad) → grados.
fn parse_angle_deg(s: &str) -> Option<f32> {
    let v = s.trim();
    if let Some(n) = v.strip_suffix("deg") {
        return n.trim().parse::<f32>().ok();
    }
    if let Some(n) = v.strip_suffix("rad") {
        return n.trim().parse::<f32>().ok().map(|r| r.to_degrees());
    }
    if let Some(n) = v.strip_suffix("turn") {
        return n.trim().parse::<f32>().ok().map(|t| t * 360.0);
    }
    if let Some(n) = v.strip_suffix("grad") {
        return n.trim().parse::<f32>().ok().map(|g| g * 0.9);
    }
    // Unitless = 0deg sólo para `0`.
    if v == "0" {
        return Some(0.0);
    }
    None
}

/// `text-orientation` (CSS Writing Modes 3).
pub(crate) fn parse_text_orientation(value: &str) -> Option<TextOrientation> {
    match value.trim().to_ascii_lowercase().as_str() {
        "mixed" => Some(TextOrientation::Mixed),
        "upright" => Some(TextOrientation::Upright),
        "sideways" => Some(TextOrientation::Sideways),
        "sideways-right" => Some(TextOrientation::SidewaysRight),
        _ => None,
    }
}

/// `overscroll-behavior-x/y` (un solo valor).
pub(crate) fn parse_overscroll_behavior(value: &str) -> Option<OverscrollBehavior> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(OverscrollBehavior::Auto),
        "contain" => Some(OverscrollBehavior::Contain),
        "none" => Some(OverscrollBehavior::None),
        _ => None,
    }
}

/// `scroll-snap-type: none | <axis> <strictness>?`. Strictness default
/// `proximity`. Acepta sólo lo que matchea — `xy` no es válido en CSS.
pub(crate) fn parse_scroll_snap_type(value: &str) -> Option<ScrollSnapType> {
    let v = value.trim().to_ascii_lowercase();
    if v == "none" {
        return Some(ScrollSnapType(None));
    }
    let mut tokens = v.split_whitespace();
    let axis = match tokens.next()? {
        "x" => ScrollSnapAxis::X,
        "y" => ScrollSnapAxis::Y,
        "block" => ScrollSnapAxis::Block,
        "inline" => ScrollSnapAxis::Inline,
        "both" => ScrollSnapAxis::Both,
        _ => return None,
    };
    let strict = match tokens.next() {
        Some("mandatory") => ScrollSnapStrictness::Mandatory,
        Some("proximity") => ScrollSnapStrictness::Proximity,
        Some(_) => return None,
        None => ScrollSnapStrictness::Proximity,
    };
    Some(ScrollSnapType(Some((axis, strict))))
}

/// `scroll-snap-align` (un solo valor por eje). Fase 7.269.
pub(crate) fn parse_scroll_snap_align(value: &str) -> Option<ScrollSnapAlign> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Some(ScrollSnapAlign::None),
        "start" => Some(ScrollSnapAlign::Start),
        "end" => Some(ScrollSnapAlign::End),
        "center" => Some(ScrollSnapAlign::Center),
        _ => None,
    }
}

/// `scroll-snap-stop`: `normal | always`. Fase 7.270.
pub(crate) fn parse_scroll_snap_stop(value: &str) -> Option<ScrollSnapStop> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(ScrollSnapStop::Normal),
        "always" => Some(ScrollSnapStop::Always),
        _ => None,
    }
}

/// Shorthand de 1–4 valores con `LengthVal` (acepta `auto`/px/%) para
/// `scroll-padding`. Fase 7.271.
pub(crate) fn parse_sides_lp(value: &str) -> Option<Sides<LengthVal>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let parsed: Vec<LengthVal> = parts
        .iter()
        .map(|t| parse_length_or_pct(t))
        .collect::<Option<Vec<_>>>()?;
    Some(match parsed.as_slice() {
        [a] => Sides { top: *a, right: *a, bottom: *a, left: *a },
        [v, h] => Sides { top: *v, right: *h, bottom: *v, left: *h },
        [t, h, b] => Sides { top: *t, right: *h, bottom: *b, left: *h },
        [t, r, b, l] => Sides { top: *t, right: *r, bottom: *b, left: *l },
        _ => return None,
    })
}

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
        _ => None,
    }
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

/// `circle(<radius> [at <x> <y>])`. Radio en px; centro default 50%/50%.
fn parse_circle_shape(args: &str) -> Option<ClipPath> {
    let (radius_str, center) = match args.find(" at ") {
        Some(idx) => (args[..idx].trim(), args[idx + " at ".len()..].trim()),
        None => (args, ""),
    };
    let radius = if radius_str.is_empty() {
        0.0
    } else {
        parse_length_px(radius_str)?
    };
    let (cx, cy) = parse_center(center);
    Some(ClipPath::Circle { radius, cx, cy })
}

/// `ellipse(<rx> <ry> [at <x> <y>])`.
fn parse_ellipse_shape(args: &str) -> Option<ClipPath> {
    let (radii_str, center) = match args.find(" at ") {
        Some(idx) => (args[..idx].trim(), args[idx + " at ".len()..].trim()),
        None => (args, ""),
    };
    let mut tokens = radii_str.split_whitespace();
    let rx = parse_length_px(tokens.next()?)?;
    let ry = parse_length_px(tokens.next()?)?;
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

/// `column-rule` shorthand: `<width> <style> <color>` (orden libre,
/// igual que `outline`). Tokens en cualquier orden — `currentColor`
/// emite `ColumnRuleColor(None)`. Fase 7.280.
pub(crate) fn parse_column_rule_shorthand(value: &str, important: bool) -> Vec<Decl> {
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
        out.push(Decl { kind: DeclKind::ColumnRuleStyleActive(false), important });
        return out;
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::ColumnRuleWidth(w), important });
    }
    if current {
        out.push(Decl { kind: DeclKind::ColumnRuleColor(None), important });
    } else if let Some(c) = color {
        out.push(Decl { kind: DeclKind::ColumnRuleColor(Some(c)), important });
    }
    if style_active.is_some() {
        out.push(Decl { kind: DeclKind::ColumnRuleStyleActive(true), important });
    }
    if let Some(ls) = line_style {
        out.push(Decl { kind: DeclKind::ColumnRuleStylePattern(ls), important });
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
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(TextUnderlinePosition::Auto),
        "from-font" => Some(TextUnderlinePosition::FromFont),
        "under" => Some(TextUnderlinePosition::Under),
        "left" => Some(TextUnderlinePosition::Left),
        "right" => Some(TextUnderlinePosition::Right),
        _ => None,
    }
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
    let toks: Vec<&str> = value.trim().split_whitespace().collect();
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
fn parse_angle_degrees(s: &str) -> Option<f32> {
    let t = s.trim();
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

/// `animation-timeline`: `auto | none | <dashed-ident>`. Fase 7.339.
pub(crate) fn parse_timeline_ref(value: &str) -> Option<TimelineRef> {
    let v = value.trim();
    match v.to_ascii_lowercase().as_str() {
        "auto" => Some(TimelineRef::Auto),
        "none" => Some(TimelineRef::None),
        _ => {
            // Aceptamos cualquier `<custom-ident>` (validamos solo
            // que no esté vacío y no tenga espacios internos — el
            // lexer ya separó por whitespace, pero filtramos por
            // las dudas).
            if v.is_empty() || v.contains(char::is_whitespace) {
                return None;
            }
            Some(TimelineRef::Named(v.to_string()))
        }
    }
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

/// `background-position-x: left | center | right | <length-or-pct>`. Sólo
/// eje X — los offsets con keyword (`right 10%`) no se soportan. Fase 7.439.
pub(crate) fn parse_background_position_x(value: &str) -> Option<LengthVal> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" => Some(LengthVal::Pct(0.0)),
        "center" => Some(LengthVal::Pct(50.0)),
        "right" => Some(LengthVal::Pct(100.0)),
        other => parse_length_or_pct(other),
    }
}

/// `background-position-y: top | center | bottom | <length-or-pct>`. Sólo
/// eje Y — los offsets con keyword (`bottom 10%`) no se soportan. Fase 7.440.
pub(crate) fn parse_background_position_y(value: &str) -> Option<LengthVal> {
    match value.trim().to_ascii_lowercase().as_str() {
        "top" => Some(LengthVal::Pct(0.0)),
        "center" => Some(LengthVal::Pct(50.0)),
        "bottom" => Some(LengthVal::Pct(100.0)),
        other => parse_length_or_pct(other),
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
fn split_contain_intrinsic_halves<'a>(
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

/// Match propiedades `border-{top|bottom}-{left|right}-radius` y sus
/// equivalentes lógicos `border-{start|end}-{start|end}-radius` (Fase
/// 7.409-7.412). En LTR horizontal: `block-start = top`, `block-end =
/// bottom`, `inline-start = left`, `inline-end = right`. El primer eje
/// es el block; el segundo, el inline (spec CSS Backgrounds 4).
pub(crate) fn match_border_corner_prop(prop: &str) -> Option<BorderCorner> {
    match prop.to_ascii_lowercase().as_str() {
        "border-top-left-radius" => Some(BorderCorner::TopLeft),
        "border-top-right-radius" => Some(BorderCorner::TopRight),
        "border-bottom-right-radius" => Some(BorderCorner::BottomRight),
        "border-bottom-left-radius" => Some(BorderCorner::BottomLeft),
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

/// `text-decoration-style: solid | double | dotted | dashed | wavy`.
pub(crate) fn parse_text_decoration_style(value: &str) -> Option<TextDecorationStyle> {
    match value.trim().to_ascii_lowercase().as_str() {
        "solid" => Some(TextDecorationStyle::Solid),
        "double" => Some(TextDecorationStyle::Double),
        "dotted" => Some(TextDecorationStyle::Dotted),
        "dashed" => Some(TextDecorationStyle::Dashed),
        "wavy" => Some(TextDecorationStyle::Wavy),
        _ => None,
    }
}

/// Expande el shorthand `text-decoration: <line> || <style> || <color>`
/// (orden libre) a sus longhands. Cada token se prueba como line, luego
/// como style, luego como color; los no reconocidos se ignoran. Emite
/// siempre la línea (default `None` si no hubo keyword de línea) para que
/// el shorthand resetee; color/style sólo si aparecieron explícitos.
fn parse_text_decoration_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut line: Option<TextDecorationLine> = None;
    for tok in value.split_whitespace() {
        let low = tok.to_ascii_lowercase();
        match low.as_str() {
            "none" => line = Some(TextDecorationLine::None),
            "underline" => line = Some(TextDecorationLine::Underline),
            "line-through" => line = Some(TextDecorationLine::LineThrough),
            "overline" => line = Some(TextDecorationLine::Overline),
            "blink" => {} // CSS legacy, sin efecto
            _ => {
                if let Some(st) = parse_text_decoration_style(tok) {
                    out.push(Decl { kind: DeclKind::TextDecorationStyle(st), important });
                } else if is_current_color(tok) {
                    out.push(Decl { kind: DeclKind::TextDecorationColor(None), important });
                } else if let Some(c) = parse_color(tok) {
                    out.push(Decl { kind: DeclKind::TextDecorationColor(Some(c)), important });
                }
            }
        }
    }
    out.push(Decl {
        kind: DeclKind::TextDecoration(line.unwrap_or(TextDecorationLine::None)),
        important,
    });
    out
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

/// `font-size`: distingue valores absolutos (px/rem/vw/`calc`/`clamp` y los
/// keywords absolutos `medium`/`large`/…) de los relativos al font-size
/// HEREDADO (`em`, `%`, `larger`/`smaller`), que se difieren a la resolución
/// en `compute_with_parent`. `rem` queda absoluto (root = 16px). Fase 7.223.
pub(crate) fn parse_font_size(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    match v.to_ascii_lowercase().as_str() {
        // Keywords relativos al heredado.
        "larger" => return Some(DeclKind::FontSizeRel(1.2)),
        "smaller" => return Some(DeclKind::FontSizeRel(1.0 / 1.2)),
        // Keywords absolutos (escala estándar CSS, medium = 16px).
        "xx-small" => return Some(DeclKind::FontSize(9.0)),
        "x-small" => return Some(DeclKind::FontSize(10.0)),
        "small" => return Some(DeclKind::FontSize(13.0)),
        "medium" => return Some(DeclKind::FontSize(16.0)),
        "large" => return Some(DeclKind::FontSize(18.0)),
        "x-large" => return Some(DeclKind::FontSize(24.0)),
        "xx-large" => return Some(DeclKind::FontSize(32.0)),
        "xxx-large" => return Some(DeclKind::FontSize(48.0)),
        _ => {}
    }
    // `%` → multiplicador relativo al heredado.
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().map(|n| DeclKind::FontSizeRel(n / 100.0));
    }
    // `em` (no `rem`) → relativo al font-size del padre.
    if let Some(num) = v.strip_suffix("em") {
        if !num.ends_with('r') {
            if let Ok(n) = num.trim().parse::<f32>() {
                return Some(DeclKind::FontSizeRel(n));
            }
        }
    }
    // Absoluto: px / rem / vw / calc / clamp / …
    parse_px_or_math(v).map(DeclKind::FontSize)
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
