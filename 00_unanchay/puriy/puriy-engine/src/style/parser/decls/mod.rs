//! Parsing de declaraciones: punto de entrada `parse_declarations` + el dispatch
//! gigante `decl_kind_from_pair` (en `dispatch*`), y los value-parsers base
//! repartidos por familia en submódulos hermanos. Sub-módulo de `parser` (regla #1).
use super::*;

// Submódulos: dispatch principal (partido en 4 grupos) + value-parsers por familia.
mod dispatch;
pub(crate) use dispatch::*;
mod dispatch_a;
pub(crate) use dispatch_a::*;
mod dispatch_b;
pub(crate) use dispatch_b::*;
mod dispatch_c;
pub(crate) use dispatch_c::*;
mod dispatch_d;
pub(crate) use dispatch_d::*;
mod misc;
pub(crate) use misc::*;
mod effects;
pub(crate) use effects::*;
mod transforms;
pub(crate) use transforms::*;
mod svg_paint;
pub(crate) use svg_paint::*;
mod border;
pub(crate) use border::*;
mod tail;
pub(crate) use tail::*;

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
        // Fase 7.869 — resuelve `env(...)` (áreas seguras → 0px / fallback)
        // tras `var()`. Early-out si no hay `env(`.
        let substituted = substitute_env(&substituted);
        let value = substituted.as_str();
        // Fase 7.851 — shorthand CSS-wide `all: inherit|initial|unset|revert`.
        // Expande a un `Wide{prop, kw}` por cada propiedad del subset curado
        // (`WideProp::ALL`). Sólo acepta keywords wide; `all: <otro>` se dropea.
        if prop.eq_ignore_ascii_case("all") {
            if let Some(kw) = wide_keyword(value) {
                for prop in WideProp::ALL {
                    out.push(Decl { kind: DeclKind::Wide { prop, kw }, important });
                }
            }
            continue;
        }
        if prop.eq_ignore_ascii_case("border") {
            out.extend(parse_border_shorthand(value, important));
            continue;
        }
        // Fase 7.858 — `border-radius` (+ alias vendor) acepta 1-4 valores y
        // la forma `/` (horiz / vert). Se expande a las 4 esquinas; el caso
        // de 1 valor sin `/` lo sigue tomando el dispatch (vía `BorderRadius`).
        if (prop.eq_ignore_ascii_case("border-radius")
            || prop.eq_ignore_ascii_case("-webkit-border-radius")
            || prop.eq_ignore_ascii_case("-moz-border-radius"))
            // Fase 7.877 — cuenta tokens de nivel superior (un `calc()` con
            // espacios internos NO es multivalor; cae al dispatch single).
            && (value.contains('/') || split_top_level_ws(value).len() > 1)
        {
            out.extend(parse_border_radius_shorthand(value, important));
            continue;
        }
        // Fase 7.837 — `border-width: <1-4>` (TRBL) con keywords thin/medium/
        // thick. >1 token → per-side; 1 token → global (ahora también acepta
        // los keywords, que antes se descartaban).
        if prop.eq_ignore_ascii_case("border-width") {
            // Fase 7.877 — tokeniza respetando paréntesis (calc con espacios).
            let toks_owned = split_top_level_ws(value);
            let toks: Vec<&str> = toks_owned.iter().map(String::as_str).collect();
            if toks.len() >= 2 {
                if let Some(sides) = expand_trbl_f32(&toks, parse_border_width_token) {
                    for (edge, w) in sides {
                        out.push(Decl { kind: DeclKind::BorderSideWidth(edge, w), important });
                    }
                }
                continue;
            }
            if let Some(w) = parse_border_width_token(toks.first().copied().unwrap_or("")) {
                out.push(Decl { kind: DeclKind::BorderWidth(w), important });
            }
            continue;
        }
        // Fase 7.838 — `border-color: <1-4>` (TRBL), con `currentColor` por
        // lado. 1 token cae al path global (dispatch_a). Rechazo total si algún
        // token no es color válido.
        if prop.eq_ignore_ascii_case("border-color") {
            let toks: Vec<&str> = value.split_whitespace().collect();
            if toks.len() >= 2 {
                let idx: [usize; 4] = match toks.len() {
                    2 => [0, 1, 0, 1],
                    3 => [0, 1, 2, 1],
                    4 => [0, 1, 2, 3],
                    _ => continue, // >4 inválido
                };
                if toks.iter().any(|t| !is_current_color(t) && parse_color(t).is_none()) {
                    continue;
                }
                let edges =
                    [BorderEdge::Top, BorderEdge::Right, BorderEdge::Bottom, BorderEdge::Left];
                for (e, &i) in edges.iter().zip(idx.iter()) {
                    let tok = toks[i];
                    if is_current_color(tok) {
                        out.push(Decl {
                            kind: DeclKind::CurrentColor(ColorTarget::BorderSide(*e)),
                            important,
                        });
                    } else if let Some(c) = parse_color(tok) {
                        out.push(Decl { kind: DeclKind::BorderSideColor(*e, c), important });
                    }
                }
                continue;
            }
        }
        // `border-style` (todos los lados): togglea enabled + fija el patrón.
        // Fase 7.874 — acepta multi-valor (`solid dotted`, regla TRBL per-side);
        // como el modelo de patrón es uniforme, tomamos el 1er token.
        if prop.eq_ignore_ascii_case("border-style") {
            let first = value.split_whitespace().next().unwrap_or(value);
            if let Some(on) = parse_border_style(first) {
                out.push(Decl { kind: DeclKind::BorderEnabled(on), important });
                if let Some(ls) = parse_border_line_style(first) {
                    out.push(Decl { kind: DeclKind::BorderStyleKind(ls), important });
                }
            }
            continue;
        }
        // `outline-style`: togglea style_active + fija el patrón visual.
        if prop.eq_ignore_ascii_case("outline-style") {
            // Fase 7.836 — `auto` (anillo de foco por defecto del navegador):
            // outline visible con patrón sólido (aproximación; no dibujamos el
            // anillo nativo del SO).
            if value.trim().eq_ignore_ascii_case("auto") {
                out.push(Decl { kind: DeclKind::OutlineStyle(true), important });
                if let Some(ls) = parse_border_line_style("solid") {
                    out.push(Decl { kind: DeclKind::OutlineStylePattern(ls), important });
                }
                continue;
            }
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
        // `scroll-timeline: [<name> || <axis>]` shorthand (Fase 7.471). CSS
        // Scroll-Driven Animations 1. Tokens en cualquier orden: el primero
        // que matchea axis (`block`/`inline`/`x`/`y`) va a axis, el resto
        // (un `<dashed-ident>` o `none`) va a name. Faltantes → default.
        // Vacío rechaza entero.
        if prop.eq_ignore_ascii_case("scroll-timeline") {
            if let Some((name, axis)) = parse_scroll_view_timeline_short(value) {
                out.push(Decl {
                    kind: DeclKind::ScrollTimelineName(name),
                    important,
                });
                out.push(Decl {
                    kind: DeclKind::ScrollTimelineAxis(axis),
                    important,
                });
            }
            continue;
        }
        // `view-timeline: [<name> || <axis> || <inset>]` shorthand (Fase
        // 7.472). Mismo dispatcher por token: name (`<dashed-ident>`/`none`)
        // + axis + inset (1 ó 2 `<length-or-pct>`/`auto`). Faltantes →
        // default. Vacío rechaza entero.
        if prop.eq_ignore_ascii_case("view-timeline") {
            if let Some((name, axis, inset_a, inset_b)) =
                parse_view_timeline_short(value)
            {
                out.push(Decl {
                    kind: DeclKind::ViewTimelineName(name),
                    important,
                });
                out.push(Decl {
                    kind: DeclKind::ViewTimelineAxis(axis),
                    important,
                });
                out.push(Decl {
                    kind: DeclKind::ViewTimelineInset(inset_a, inset_b),
                    important,
                });
            }
            continue;
        }
        // `animation-range: <start> [<end>]` shorthand (Fase 7.466). El valor
        // de cada lado puede ser 1 o 2 tokens (`cover`, `cover 20%`, `100px`).
        // El divisor es el primer token que NO continúa el lado actual: si el
        // lado actual ya consumió un keyword + offset, o un solo token que NO
        // es una fase nombrada, el lado se cierra. Para simplificar y mantener
        // la regla de "shorthand inválido = no emite", probamos las divisiones
        // posibles (i=1,2,3) y nos quedamos con la primera donde ambos lados
        // parsean. Si el valor tiene un solo lado válido, end ≡ start.
        if prop.eq_ignore_ascii_case("animation-range") {
            let toks: Vec<&str> = value.split_whitespace().collect();
            if toks.is_empty() {
                continue;
            }
            let mut start: Option<AnimationRange> = None;
            let mut end: Option<AnimationRange> = None;
            // Probar una sola pieza para start, end ≡ start.
            if let Some(s) = parse_animation_range(&toks.join(" ")) {
                start = Some(s.clone());
                end = Some(s);
            }
            // Probar todas las divisiones, quedarse con la primera donde
            // start y end son válidos.
            for i in 1..toks.len() {
                let left = toks[..i].join(" ");
                let right = toks[i..].join(" ");
                if let (Some(s), Some(e)) = (
                    parse_animation_range(&left),
                    parse_animation_range(&right),
                ) {
                    start = Some(s);
                    end = Some(e);
                    break;
                }
            }
            if let (Some(s), Some(e)) = (start, end) {
                out.push(Decl {
                    kind: DeclKind::AnimationRangeStart(s),
                    important,
                });
                out.push(Decl {
                    kind: DeclKind::AnimationRangeEnd(e),
                    important,
                });
            }
            continue;
        }
        // `position-try: [<order>]? <fallbacks>` shorthand (Fase 7.462).
        // `<order>` puede aparecer 0 o 1 vez al inicio; el resto se interpreta
        // como `position-try-fallbacks` (lista separada por coma). Si el
        // primer token es un `<order>` keyword conocido, lo consumimos y el
        // resto va al fallbacks; si no, todo el valor va al fallbacks.
        // Faltantes se emiten con default explícito (reset).
        if prop.eq_ignore_ascii_case("position-try") {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            let (order, rest) = match trimmed.split_once(char::is_whitespace) {
                Some((head, tail)) => match parse_position_try_order(head) {
                    Some(o) => (o, tail.trim()),
                    None => (PositionTryOrder::Normal, trimmed),
                },
                None => match parse_position_try_order(trimmed) {
                    Some(o) => (o, ""),
                    None => (PositionTryOrder::Normal, trimmed),
                },
            };
            let fallbacks = if rest.is_empty() {
                Some(Vec::new())
            } else {
                parse_position_try_fallbacks(rest)
            };
            if let Some(fb) = fallbacks {
                out.push(Decl {
                    kind: DeclKind::PositionTryOrder(order),
                    important,
                });
                out.push(Decl {
                    kind: DeclKind::PositionTryFallbacks(fb),
                    important,
                });
            }
            continue;
        }
        // `block-step: <size>? <insert>? <align>? <round>?` shorthand
        // (Fase 7.458). Hasta 4 tokens, en cualquier orden. Cada longhand
        // se emite a lo sumo una vez (token redundante → rechazo). Faltantes
        // se emiten con default explícito (reset del campo).
        if prop.eq_ignore_ascii_case("block-step") {
            let mut size: Option<BlockStepSize> = None;
            let mut insert: Option<BlockStepInsert> = None;
            let mut align: Option<BlockStepAlign> = None;
            let mut round: Option<BlockStepRound> = None;
            let mut ok = true;
            for tok in value.split_whitespace() {
                match parse_block_step_piece(tok) {
                    Some(DeclKind::BlockStepSize(v)) => {
                        if size.is_some() {
                            ok = false;
                            break;
                        }
                        size = Some(v);
                    }
                    Some(DeclKind::BlockStepInsert(v)) => {
                        if insert.is_some() {
                            ok = false;
                            break;
                        }
                        insert = Some(v);
                    }
                    Some(DeclKind::BlockStepAlign(v)) => {
                        if align.is_some() {
                            ok = false;
                            break;
                        }
                        align = Some(v);
                    }
                    Some(DeclKind::BlockStepRound(v)) => {
                        if round.is_some() {
                            ok = false;
                            break;
                        }
                        round = Some(v);
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                out.push(Decl {
                    kind: DeclKind::BlockStepSize(size.unwrap_or(BlockStepSize::None)),
                    important,
                });
                out.push(Decl {
                    kind: DeclKind::BlockStepInsert(
                        insert.unwrap_or(BlockStepInsert::MarginBox),
                    ),
                    important,
                });
                out.push(Decl {
                    kind: DeclKind::BlockStepAlign(
                        align.unwrap_or(BlockStepAlign::Auto),
                    ),
                    important,
                });
                out.push(Decl {
                    kind: DeclKind::BlockStepRound(
                        round.unwrap_or(BlockStepRound::Up),
                    ),
                    important,
                });
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
        // Fase 7.720 — `-webkit-flex` alias vendor del shorthand `flex`.
        if prop.eq_ignore_ascii_case("flex")
            || prop.eq_ignore_ascii_case("-webkit-flex")
        {
            out.extend(parse_flex_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("font") {
            out.extend(parse_font_shorthand(value, important));
            continue;
        }
        // Fase 7.829 — shorthand `font-variant` (CSS Fonts 4). `normal`/`none`
        // resetean los longhands; sino se reparten los tokens por grupo
        // (caps/position/numeric/ligatures/east-asian) — los conjuntos de
        // keywords no se solapan, así que clasificamos cada token probando el
        // sub-parser de cada longhand. Un token desconocido descarta el
        // shorthand entero. No cubre stylistic()/swash()/etc. (raros).
        if prop.eq_ignore_ascii_case("font-variant") {
            let v = value.trim();
            if v.eq_ignore_ascii_case("normal") || v.eq_ignore_ascii_case("none") {
                let lig = if v.eq_ignore_ascii_case("none") { "none" } else { "normal" };
                if let Some(c) = parse_font_variant_caps("normal") {
                    out.push(Decl { kind: DeclKind::FontVariantCaps(c), important });
                }
                if let Some(n) = parse_font_variant_numeric("normal") {
                    out.push(Decl { kind: DeclKind::FontVariantNumeric(n), important });
                }
                if let Some(l) = parse_font_variant_ligatures(lig) {
                    out.push(Decl { kind: DeclKind::FontVariantLigatures(l), important });
                }
                if let Some(e) = parse_font_variant_east_asian("normal") {
                    out.push(Decl { kind: DeclKind::FontVariantEastAsian(e), important });
                }
                if let Some(p) = parse_font_variant_position("normal") {
                    out.push(Decl { kind: DeclKind::FontVariantPosition(p), important });
                }
                continue;
            }
            let (mut caps, mut numeric, mut lig, mut ea, mut pos) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut ok = true;
            for tok in v.split_whitespace() {
                if parse_font_variant_caps(tok).is_some() {
                    caps.push(tok);
                } else if parse_font_variant_position(tok).is_some() {
                    pos.push(tok);
                } else if parse_font_variant_numeric(tok).is_some() {
                    numeric.push(tok);
                } else if parse_font_variant_ligatures(tok).is_some() {
                    lig.push(tok);
                } else if parse_font_variant_east_asian(tok).is_some() {
                    ea.push(tok);
                } else {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }
            if let Some(c) = caps.first().and_then(|t| parse_font_variant_caps(t)) {
                out.push(Decl { kind: DeclKind::FontVariantCaps(c), important });
            }
            if let Some(p) = pos.first().and_then(|t| parse_font_variant_position(t)) {
                out.push(Decl { kind: DeclKind::FontVariantPosition(p), important });
            }
            if !numeric.is_empty() {
                if let Some(n) = parse_font_variant_numeric(&numeric.join(" ")) {
                    out.push(Decl { kind: DeclKind::FontVariantNumeric(n), important });
                }
            }
            if !lig.is_empty() {
                if let Some(l) = parse_font_variant_ligatures(&lig.join(" ")) {
                    out.push(Decl { kind: DeclKind::FontVariantLigatures(l), important });
                }
            }
            if !ea.is_empty() {
                if let Some(e) = parse_font_variant_east_asian(&ea.join(" ")) {
                    out.push(Decl { kind: DeclKind::FontVariantEastAsian(e), important });
                }
            }
            continue;
        }
        // Fase 7.819-7.820 — shorthands `grid-row` / `grid-column`
        // (CSS Grid §8.3): `<start> [ / <end> ]?`. Reparte en los longhands
        // `grid-{row,column}-{start,end}` ya existentes (parse opaco). Al
        // omitir el end, un `<custom-ident>` de start se replica; un
        // `<integer>`/`span` deja `auto`.
        if prop.eq_ignore_ascii_case("grid-row") || prop.eq_ignore_ascii_case("grid-column") {
            let is_col = prop.eq_ignore_ascii_case("grid-column");
            let mut it = value.splitn(2, '/');
            let start_raw = it.next().unwrap_or("").trim();
            if start_raw.is_empty() {
                continue;
            }
            let end_raw = it.next().map(str::trim);
            let start = grid_line_opt(start_raw);
            let end = match end_raw {
                Some(e) => grid_line_opt(e),
                None if grid_line_is_custom_ident(start_raw) => start.clone(),
                None => None,
            };
            if is_col {
                out.push(Decl { kind: DeclKind::GridColumnStart(start), important });
                out.push(Decl { kind: DeclKind::GridColumnEnd(end), important });
            } else {
                out.push(Decl { kind: DeclKind::GridRowStart(start), important });
                out.push(Decl { kind: DeclKind::GridRowEnd(end), important });
            }
            continue;
        }
        // Fase 7.821 — shorthand `grid-area` (CSS Grid §8.4):
        // `<row-start> [ / <col-start> [ / <row-end> [ / <col-end> ]?]?]?`.
        // Reglas de omisión: al faltar col-start, si row-start es custom-ident
        // los cuatro toman ese valor; al faltar row-end/col-end, si el start
        // del mismo eje es custom-ident se replica, sino `auto`.
        if prop.eq_ignore_ascii_case("grid-area") {
            let parts: Vec<&str> = value.split('/').map(str::trim).collect();
            let rs_raw = parts.first().copied().unwrap_or("");
            if rs_raw.is_empty() {
                continue;
            }
            let rs_ident = grid_line_is_custom_ident(rs_raw);
            let cs_raw = parts
                .get(1)
                .copied()
                .unwrap_or(if rs_ident { rs_raw } else { "auto" });
            let re_raw = parts
                .get(2)
                .copied()
                .unwrap_or(if rs_ident { rs_raw } else { "auto" });
            let ce_raw = parts.get(3).copied().unwrap_or(
                if grid_line_is_custom_ident(cs_raw) { cs_raw } else { "auto" },
            );
            out.push(Decl { kind: DeclKind::GridRowStart(grid_line_opt(rs_raw)), important });
            out.push(Decl { kind: DeclKind::GridColumnStart(grid_line_opt(cs_raw)), important });
            out.push(Decl { kind: DeclKind::GridRowEnd(grid_line_opt(re_raw)), important });
            out.push(Decl { kind: DeclKind::GridColumnEnd(grid_line_opt(ce_raw)), important });
            continue;
        }
        // Fase 7.848 — shorthands `grid-template` y `grid` (CSS Grid §7.4/§7.8).
        // Subset soportado: `none` (resetea explicit grid) y la forma
        // `<rows> / <columns>` (track-lists). La sintaxis con strings de áreas
        // y la forma con `auto-flow` (que sólo aplica a `grid`) no se expanden.
        if prop.eq_ignore_ascii_case("grid-template") || prop.eq_ignore_ascii_case("grid") {
            if value.eq_ignore_ascii_case("none") {
                out.push(Decl { kind: DeclKind::GridTemplateRows(Vec::new()), important });
                out.push(Decl { kind: DeclKind::GridTemplateColumns(Vec::new()), important });
                out.push(Decl { kind: DeclKind::GridTemplateAreas(None), important });
                continue;
            }
            // `auto-flow` (sólo `grid`) y strings de áreas: fuera de alcance.
            if !value.contains('"') && value.to_ascii_lowercase().contains("auto-flow") {
                continue;
            }
            if let Some((rows_raw, cols_raw)) = value.split_once('/') {
                if !rows_raw.contains('"') {
                    if let (Some(rows), Some(cols)) = (
                        parse_grid_template(rows_raw.trim()),
                        parse_grid_template(cols_raw.trim()),
                    ) {
                        out.push(Decl { kind: DeclKind::GridTemplateRows(rows), important });
                        out.push(Decl { kind: DeclKind::GridTemplateColumns(cols), important });
                    }
                }
            }
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
        // Fase 7.721 — `-webkit-flex-flow` alias vendor del shorthand.
        if prop.eq_ignore_ascii_case("flex-flow")
            || prop.eq_ignore_ascii_case("-webkit-flex-flow")
        {
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
        // Fase 7.744 — alias `-webkit-text-emphasis` → estándar (shorthand).
        if prop.eq_ignore_ascii_case("text-emphasis")
            || prop.eq_ignore_ascii_case("-webkit-text-emphasis")
        {
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
        // Fase 7.690 — `-webkit-columns` alias vendor del shorthand `columns`.
        // Fase 7.795 — `-moz-columns` alias vendor del shorthand `columns`.
        if prop.eq_ignore_ascii_case("columns")
            || prop.eq_ignore_ascii_case("-webkit-columns")
            || prop.eq_ignore_ascii_case("-moz-columns")
        {
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
        // Fase 7.691 — `-webkit-column-rule` alias vendor del shorthand.
        // Fase 7.783 — `-moz-column-rule` alias vendor del shorthand.
        if prop.eq_ignore_ascii_case("column-rule")
            || prop.eq_ignore_ascii_case("-webkit-column-rule")
            || prop.eq_ignore_ascii_case("-moz-column-rule")
        {
            out.extend(parse_column_rule_shorthand(value, important));
            continue;
        }
        // `column-rule-style: dotted` → activa + fija el patrón.
        // Fase 7.692 — `-webkit-column-rule-style` / Fase 7.782 — `-moz-column-rule-style` alias vendor.
        if prop.eq_ignore_ascii_case("column-rule-style")
            || prop.eq_ignore_ascii_case("-webkit-column-rule-style")
            || prop.eq_ignore_ascii_case("-moz-column-rule-style")
        {
            if let Some(on) = parse_border_style(value) {
                out.push(Decl { kind: DeclKind::ColumnRuleStyleActive(on), important });
                if let Some(ls) = parse_border_line_style(value) {
                    out.push(Decl { kind: DeclKind::ColumnRuleStylePattern(ls), important });
                }
            }
            continue;
        }
        // Fase 7.800 — shorthand `-webkit-text-stroke`: `<width> || <color>`, orden
        // libre. Reparte en los longhands `-webkit-text-stroke-width/color` (Fase
        // 7.579-7.580). El primer token reconocible como ancho fija el ancho; el
        // resto (reensamblado con espacios) es el color, así `2px rgb(0, 0, 0)`
        // no se rompe al partir por espacios.
        if prop.eq_ignore_ascii_case("-webkit-text-stroke") {
            let mut width: Option<f32> = None;
            let mut color_parts: Vec<&str> = Vec::new();
            for tok in value.split_whitespace() {
                let low = tok.to_ascii_lowercase();
                let as_width = match low.as_str() {
                    "thin" => Some(1.0),
                    "medium" => Some(3.0),
                    "thick" => Some(5.0),
                    _ => low.strip_suffix("px").unwrap_or(&low).parse::<f32>().ok(),
                };
                if width.is_none() && as_width.is_some() {
                    width = as_width;
                } else {
                    color_parts.push(tok);
                }
            }
            if let Some(w) = width {
                out.push(Decl { kind: DeclKind::WebkitTextStrokeWidth(w), important });
            }
            if !color_parts.is_empty() {
                let c = color_parts.join(" ");
                if c.eq_ignore_ascii_case("currentcolor") {
                    out.push(Decl { kind: DeclKind::WebkitTextStrokeColor(None), important });
                } else {
                    out.push(Decl { kind: DeclKind::WebkitTextStrokeColor(Some(c)), important });
                }
            }
            continue;
        }
        // Fase 7.760 — alias `-webkit-text-decoration` → estándar (shorthand legacy).
        if prop.eq_ignore_ascii_case("text-decoration")
            || prop.eq_ignore_ascii_case("-webkit-text-decoration")
        {
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

/// Normaliza un `<grid-line>` a `Option<String>`: `auto`/vacío → `None`
/// (el resolver de grid lo trata como colocación automática), el resto se
/// guarda opaco (`3`, `span 2`, `header`, `span header`...).
fn grid_line_opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(t.to_string())
    }
}

/// `true` si el `<grid-line>` es un `<custom-ident>` puro — el caso en que,
/// al omitir el lado opuesto del shorthand, el ident se replica (CSS Grid
/// §8.3). No lo es `auto`, un `<integer>` (con signo) ni nada que empiece
/// por `span`.
fn grid_line_is_custom_ident(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("auto") {
        return false;
    }
    let first = t.split_whitespace().next().unwrap_or("");
    if first.eq_ignore_ascii_case("span") {
        return false;
    }
    let head = first.trim_start_matches(['+', '-']);
    !head.chars().next().is_none_or(|c| c.is_ascii_digit())
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
