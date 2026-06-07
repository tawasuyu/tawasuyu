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
        if prop.eq_ignore_ascii_case("columns")
            || prop.eq_ignore_ascii_case("-webkit-columns")
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
        if prop.eq_ignore_ascii_case("column-rule")
            || prop.eq_ignore_ascii_case("-webkit-column-rule")
        {
            out.extend(parse_column_rule_shorthand(value, important));
            continue;
        }
        // `column-rule-style: dotted` → activa + fija el patrón.
        // Fase 7.692 — `-webkit-column-rule-style` alias vendor.
        if prop.eq_ignore_ascii_case("column-rule-style")
            || prop.eq_ignore_ascii_case("-webkit-column-rule-style")
        {
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
        // Fase 7.727 — `-webkit-border-radius` alias vendor de `border-radius`.
        "border-radius" | "-webkit-border-radius" => {
            parse_length_px(value).map(DeclKind::BorderRadius)
        }
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
        // Fase 7.726 — `-webkit-box-shadow` alias vendor de `box-shadow`.
        "box-shadow" | "-webkit-box-shadow" => {
            Some(DeclKind::BoxShadows(parse_box_shadows(value)))
        }
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
        // Fase 7.710-7.711 — la familia `-webkit-flex-*` es el alias vendor
        // (de facto, era prefijado en la era Flexbox 2012) de `flex-*`.
        "flex-direction" | "-webkit-flex-direction" => {
            parse_flex_direction(value).map(DeclKind::FlexDirection)
        }
        "flex-wrap" | "-webkit-flex-wrap" => parse_flex_wrap(value).map(DeclKind::FlexWrap),
        // Fase 7.716-7.718 — alias vendor Flexbox 2012 de la familia de
        // alineación (-webkit-justify-content / -align-items / -align-content).
        "justify-content" | "-webkit-justify-content" => {
            parse_justify_content(value).map(DeclKind::JustifyContent)
        }
        "align-items" | "-webkit-align-items" => {
            parse_align_items(value).map(DeclKind::AlignItems)
        }
        "align-content" | "-webkit-align-content" => {
            parse_align_content(value).map(DeclKind::AlignContent)
        }
        "justify-items" => parse_justify_items(value).map(DeclKind::JustifyItems),
        "justify-self" => parse_justify_self(value).map(DeclKind::JustifySelf),
        "gap" => parse_gap(value).map(|(r, c)| DeclKind::Gap { row: r, column: c }),
        "row-gap" => parse_length_px(value).map(DeclKind::RowGap),
        // Fase 7.689 — `-webkit-column-gap` alias vendor de `column-gap`.
        "column-gap" | "-webkit-column-gap" => {
            parse_length_px(value).map(DeclKind::ColumnGap)
        }
        // Fase 7.728 — `-webkit-box-sizing` alias vendor de `box-sizing`.
        "box-sizing" | "-webkit-box-sizing" => {
            parse_box_sizing(value).map(DeclKind::BoxSizing)
        }
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
        // Fase 7.729 — `-webkit-opacity` alias vendor legacy de `opacity`.
        "opacity" | "-webkit-opacity" => parse_opacity(value).map(DeclKind::Opacity),
        // Fase 7.719 — `-webkit-align-self` alias vendor de `align-self`.
        "align-self" | "-webkit-align-self" => {
            parse_align_self(value).map(DeclKind::AlignSelf)
        }
        "flex-grow" | "-webkit-flex-grow" => {
            value.trim().parse::<f32>().ok().map(DeclKind::FlexGrow)
        }
        "flex-shrink" | "-webkit-flex-shrink" => {
            value.trim().parse::<f32>().ok().map(DeclKind::FlexShrink)
        }
        "flex-basis" | "-webkit-flex-basis" => {
            parse_length_or_pct(value).map(DeclKind::FlexBasis)
        }
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
        // Fase 7.639 — `-epub-word-break` (perfil EPUB) alias de `word-break`.
        "word-break" | "-epub-word-break" => {
            parse_word_break(value).map(DeclKind::WordBreak)
        }
        "hyphens" | "-webkit-hyphens" | "-moz-hyphens" | "-ms-hyphens" => {
            parse_hyphens(value).map(DeclKind::Hyphens)
        }
        "resize" => parse_resize(value).map(DeclKind::Resize),
        // Fase 7.629 — `-webkit-writing-mode` es el alias vendor (de facto)
        // de `writing-mode`: enruta al mismo parser/almacén.
        // Fase 7.637 — `-epub-writing-mode` (perfil EPUB) al mismo destino.
        "writing-mode" | "-webkit-writing-mode" | "-epub-writing-mode" => {
            parse_writing_mode(value).map(DeclKind::WritingMode)
        }
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
        // Fase 7.745 — alias `-webkit-font-kerning` → estándar.
        "font-kerning" | "-webkit-font-kerning" => parse_font_kerning(value).map(DeclKind::FontKerning),
        // Fase 7.746 — alias `-webkit-font-feature-settings` → estándar.
        "font-feature-settings" | "-webkit-font-feature-settings" => {
            Some(DeclKind::FontFeatureSettings(parse_font_feature_settings(value)))
        }
        "font-variation-settings" => {
            Some(DeclKind::FontVariationSettings(parse_font_variation_settings(value)))
        }
        "font-language-override" => {
            Some(DeclKind::FontLanguageOverride(parse_font_language_override(value)))
        }
        "text-rendering" => parse_text_rendering(value).map(DeclKind::TextRendering),
        // Fase 7.725 — `-webkit-filter` alias vendor de `filter`.
        "filter" | "-webkit-filter" => Some(DeclKind::Filter(parse_filter_list(value))),
        "backdrop-filter" | "-webkit-backdrop-filter" => {
            Some(DeclKind::BackdropFilter(parse_filter_list(value)))
        }
        // Fase 7.630 — `-webkit-text-orientation` alias vendor de
        // `text-orientation`. Fase 7.638 — `-epub-text-orientation` (EPUB).
        "text-orientation" | "-webkit-text-orientation" | "-epub-text-orientation" => {
            parse_text_orientation(value).map(DeclKind::TextOrientation)
        }
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
        // Fase 7.632 — `-webkit-hyphenate-character` alias vendor.
        "hyphenate-character" | "-webkit-hyphenate-character" => {
            Some(DeclKind::HyphenateCharacter(parse_hyphenate_character(value)))
        }
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
        // Fase 7.659 — `-webkit-shape-outside` alias vendor de `shape-outside`.
        "shape-outside" | "-webkit-shape-outside" => {
            let raw = value.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
                Some(DeclKind::ShapeOutside(None))
            } else {
                Some(DeclKind::ShapeOutside(Some(raw.to_string())))
            }
        }
        // Fase 7.445 — `shape-margin` (CSS Shapes 1). `<length-or-pct>`
        // no-negativo. NO hereda. Fase 7.660 — `-webkit-shape-margin` alias.
        "shape-margin" | "-webkit-shape-margin" => {
            let v = parse_length_or_pct(value)?;
            match v {
                LengthVal::Px(n) if n < 0.0 => None,
                LengthVal::Pct(n) if n < 0.0 => None,
                _ => Some(DeclKind::ShapeMargin(v)),
            }
        }
        // Fase 7.446 — `shape-image-threshold` (CSS Shapes 1). Alpha [0..1].
        // Acepta también porcentaje (50% → 0.5). NO hereda.
        // Fase 7.661 — `-webkit-shape-image-threshold` alias vendor.
        "shape-image-threshold" | "-webkit-shape-image-threshold" => {
            parse_alpha_value(value).map(DeclKind::ShapeImageThreshold)
        }
        // Fase 7.447 — `text-combine-upright` (CSS Writing Modes 3). NO hereda.
        // Fase 7.633 — `-webkit-text-combine` es el nombre legacy WebKit.
        // Fase 7.641 — `-epub-text-combine` (EPUB) al mismo destino.
        "text-combine-upright" | "-webkit-text-combine" | "-epub-text-combine" => {
            parse_text_combine_upright(value).map(DeclKind::TextCombineUpright)
        }
        // Fase 7.448 — `ruby-align` (CSS Ruby 1). HEREDA.
        "ruby-align" => parse_ruby_align(value).map(DeclKind::RubyAlign),
        // Fase 7.449 — `offset-rotate` (CSS Motion Path 1). NO hereda.
        "offset-rotate" => parse_offset_rotate(value).map(DeclKind::OffsetRotate),
        // Fase 7.450 — `offset-anchor` (CSS Motion Path 1). `auto` o
        // `<position>`. Reusa `parse_background_position`.
        "offset-anchor" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::OffsetAnchor(None))
            } else {
                match parse_background_position(v) {
                    Some(DeclKind::BackgroundPosition(p)) => {
                        Some(DeclKind::OffsetAnchor(Some(p)))
                    }
                    _ => None,
                }
            }
        }
        // Fase 7.451 — `offset-position` (CSS Motion Path 1). `auto`/`normal`
        // o `<position>`.
        "offset-position" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") || v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::OffsetPosition(None))
            } else {
                match parse_background_position(v) {
                    Some(DeclKind::BackgroundPosition(p)) => {
                        Some(DeclKind::OffsetPosition(Some(p)))
                    }
                    _ => None,
                }
            }
        }
        // Fase 7.452 — `object-view-box` (CSS Images 5). Parse opaco.
        "object-view-box" => {
            let raw = value.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
                Some(DeclKind::ObjectViewBox(None))
            } else {
                Some(DeclKind::ObjectViewBox(Some(raw.to_string())))
            }
        }
        // Fase 7.453 — `ruby-overhang` (CSS Ruby 1). HEREDA.
        "ruby-overhang" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::RubyOverhang(RubyOverhang::Auto)),
            "none" => Some(DeclKind::RubyOverhang(RubyOverhang::None)),
            _ => None,
        },
        // Fase 7.454 — `block-step-size` (CSS Inline Layout 3). `none | <length>`.
        "block-step-size" => parse_block_step_size(value).map(DeclKind::BlockStepSize),
        // Fase 7.455 — `block-step-insert` (CSS Inline Layout 3).
        "block-step-insert" => match value.trim().to_ascii_lowercase().as_str() {
            "margin-box" => Some(DeclKind::BlockStepInsert(BlockStepInsert::MarginBox)),
            "padding-box" => Some(DeclKind::BlockStepInsert(BlockStepInsert::PaddingBox)),
            _ => None,
        },
        // Fase 7.456 — `block-step-align` (CSS Inline Layout 3).
        "block-step-align" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::BlockStepAlign(BlockStepAlign::Auto)),
            "center" => Some(DeclKind::BlockStepAlign(BlockStepAlign::Center)),
            "start" => Some(DeclKind::BlockStepAlign(BlockStepAlign::Start)),
            "end" => Some(DeclKind::BlockStepAlign(BlockStepAlign::End)),
            _ => None,
        },
        // Fase 7.457 — `block-step-round` (CSS Inline Layout 3).
        "block-step-round" => match value.trim().to_ascii_lowercase().as_str() {
            "up" => Some(DeclKind::BlockStepRound(BlockStepRound::Up)),
            "down" => Some(DeclKind::BlockStepRound(BlockStepRound::Down)),
            "nearest" => Some(DeclKind::BlockStepRound(BlockStepRound::Nearest)),
            _ => None,
        },
        // Fase 7.458 — `block-step` shorthand: ver `parse_declarations`.
        // Fase 7.459 — `position-visibility` (CSS Anchor Positioning 1).
        "position-visibility" => match value.trim().to_ascii_lowercase().as_str() {
            "always" => {
                Some(DeclKind::PositionVisibility(PositionVisibility::Always))
            }
            "anchors-visible" => Some(DeclKind::PositionVisibility(
                PositionVisibility::AnchorsVisible,
            )),
            "no-overflow" => Some(DeclKind::PositionVisibility(
                PositionVisibility::NoOverflow,
            )),
            _ => None,
        },
        // Fase 7.460 — `position-try-order` (CSS Anchor Positioning 1).
        "position-try-order" => parse_position_try_order(value)
            .map(DeclKind::PositionTryOrder),
        // Fase 7.461 — `position-try-fallbacks` (CSS Anchor Positioning 1).
        // `none` o lista de `<dashed-ident>` separados por coma. Tokens
        // distintos a un dashed-ident (ej. `flip-block`) los aceptamos como
        // string opaco — el chrome no implementa try yet.
        "position-try-fallbacks" => {
            parse_position_try_fallbacks(value).map(DeclKind::PositionTryFallbacks)
        }
        // Fase 7.462 — `position-try` shorthand: ver `parse_declarations`.
        // Fase 7.463 — `position-area` (CSS Anchor Positioning 1). Parse opaco.
        "position-area" => {
            let raw = value.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
                Some(DeclKind::PositionArea(None))
            } else {
                Some(DeclKind::PositionArea(Some(raw.to_string())))
            }
        }
        // Fase 7.464 — `animation-range-start` (CSS Animations 2).
        "animation-range-start" => {
            parse_animation_range(value).map(DeclKind::AnimationRangeStart)
        }
        // Fase 7.465 — `animation-range-end` (CSS Animations 2).
        "animation-range-end" => {
            parse_animation_range(value).map(DeclKind::AnimationRangeEnd)
        }
        // Fase 7.466 — `animation-range` shorthand: ver `parse_declarations`.
        // Fase 7.467 — `transition-behavior` (CSS Transitions 2).
        "transition-behavior" => match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::TransitionBehavior(TransitionBehavior::Normal)),
            "allow-discrete" => Some(DeclKind::TransitionBehavior(
                TransitionBehavior::AllowDiscrete,
            )),
            _ => None,
        },
        // Fase 7.468 — `interpolate-size` (CSS Values 5). HEREDA.
        "interpolate-size" => match value.trim().to_ascii_lowercase().as_str() {
            "numeric-only" => {
                Some(DeclKind::InterpolateSize(InterpolateSize::NumericOnly))
            }
            "allow-keywords" => {
                Some(DeclKind::InterpolateSize(InterpolateSize::AllowKeywords))
            }
            _ => None,
        },
        // Fase 7.469 — `view-timeline-inset: <inset>{1,2}` (CSS Scroll-Driven
        // Animations 1). Cada inset acepta `auto | <length-percentage>`.
        // `auto` se trata como `0px` (plumb sin runtime de timeline). Con 1
        // valor se aplica a ambos lados.
        "view-timeline-inset" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.is_empty() || parts.len() > 2 {
                None
            } else {
                let parse = |p: &str| -> Option<LengthVal> {
                    if p.eq_ignore_ascii_case("auto") {
                        Some(LengthVal::Px(0.0))
                    } else {
                        parse_length_or_pct(p)
                    }
                };
                let a = parse(parts[0])?;
                let b = if parts.len() == 2 {
                    parse(parts[1])?
                } else {
                    a
                };
                Some(DeclKind::ViewTimelineInset(a, b))
            }
        }
        // Fase 7.470 — `font-synthesis-position` (CSS Fonts 4). HEREDA.
        "font-synthesis-position" => {
            parse_auto_or_none(value).map(DeclKind::FontSynthesisPosition)
        }
        // Fase 7.471 — `scroll-timeline` shorthand: ver `parse_declarations`.
        // Fase 7.472 — `view-timeline` shorthand: ver `parse_declarations`.
        // Fase 7.473 — `interactivity` (CSS UI 4). HEREDA.
        "interactivity" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::Interactivity(Interactivity::Auto)),
            "inert" => Some(DeclKind::Interactivity(Interactivity::Inert)),
            _ => None,
        },
        // Fase 7.474-7.478 — geometría SVG (`cx`, `cy`, `r`, `rx`, `ry`).
        // SVG2 promueve estos atributos a propiedades CSS — `<length-percentage>`
        // para los 5; `auto` válido sólo en `rx`/`ry` (sentinel `LengthVal::Auto`).
        // NO heredan.
        "cx" => parse_length_or_pct(value).map(DeclKind::Cx),
        "cy" => parse_length_or_pct(value).map(DeclKind::Cy),
        "r" => parse_length_or_pct(value).map(DeclKind::R),
        "rx" => parse_length_or_pct(value).map(DeclKind::Rx),
        "ry" => parse_length_or_pct(value).map(DeclKind::Ry),
        // Fase 7.479 — `order` (CSS Flexbox/Grid). `<integer>`. Default 0.
        // Fase 7.715 — `-webkit-order` alias vendor de `order`.
        "order" | "-webkit-order" => {
            value.trim().parse::<i32>().ok().map(DeclKind::Order)
        }
        // Fase 7.480 — `path-length` (SVG2). `none | <number>`. NO hereda.
        "path-length" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::PathLength(None))
            } else {
                v.parse::<f32>()
                    .ok()
                    .filter(|n| *n >= 0.0)
                    .map(|n| DeclKind::PathLength(Some(n)))
            }
        }
        // Fase 7.481 — `animation-composition` (CSS Animations 2).
        "animation-composition" => match value.trim().to_ascii_lowercase().as_str() {
            "replace" => Some(DeclKind::AnimationComposition(AnimationComposition::Replace)),
            "add" => Some(DeclKind::AnimationComposition(AnimationComposition::Add)),
            "accumulate" => Some(DeclKind::AnimationComposition(
                AnimationComposition::Accumulate,
            )),
            _ => None,
        },
        // Fase 7.482 — `timeline-scope` (CSS Scroll-Driven Animations).
        // `none | <dashed-ident>#`.
        "timeline-scope" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("none") || v.is_empty() {
                Some(DeclKind::TimelineScope(Vec::new()))
            } else {
                let names: Vec<String> = v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if names.is_empty() {
                    None
                } else {
                    Some(DeclKind::TimelineScope(names))
                }
            }
        }
        // Fase 7.483 — `reading-order` (CSS Inline 3). `<integer>`. NO hereda.
        "reading-order" => value
            .trim()
            .parse::<i32>()
            .ok()
            .map(DeclKind::ReadingOrder),
        // Fase 7.484 — `reading-flow` (CSS Display 4). Enum focus-order.
        "reading-flow" => match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::ReadingFlow(ReadingFlow::Normal)),
            "flex-visual" => Some(DeclKind::ReadingFlow(ReadingFlow::FlexVisual)),
            "flex-flow" => Some(DeclKind::ReadingFlow(ReadingFlow::FlexFlow)),
            "grid-rows" => Some(DeclKind::ReadingFlow(ReadingFlow::GridRows)),
            "grid-columns" => Some(DeclKind::ReadingFlow(ReadingFlow::GridColumns)),
            "grid-order" => Some(DeclKind::ReadingFlow(ReadingFlow::GridOrder)),
            _ => None,
        },
        // Fase 7.485 — `image-resolution` (CSS Images 4).
        // `[ from-image || <resolution> ] && snap?`. HEREDA.
        "image-resolution" => parse_image_resolution(value).map(DeclKind::ImageResolution),
        // Fase 7.486 — `bookmark-level` (CSS GCPM). `none | <integer>`.
        "bookmark-level" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::BookmarkLevel(None))
            } else {
                v.parse::<u32>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .map(|n| DeclKind::BookmarkLevel(Some(n)))
            }
        }
        // Fase 7.487 — `bookmark-state` (CSS GCPM). `open | closed`.
        "bookmark-state" => match value.trim().to_ascii_lowercase().as_str() {
            "open" => Some(DeclKind::BookmarkState(BookmarkState::Open)),
            "closed" => Some(DeclKind::BookmarkState(BookmarkState::Closed)),
            _ => None,
        },
        // Fase 7.488 — `bookmark-label` (CSS GCPM). `none | <content-list>`.
        // Parse opaco: guardamos el value crudo para que un renderer GCPM
        // lo evalúe; `none` reservado.
        "bookmark-label" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::BookmarkLabel(None))
            } else {
                Some(DeclKind::BookmarkLabel(Some(v.to_string())))
            }
        }
        // Fase 7.489 — `string-set` (CSS GCPM). `none | [<custom-ident>
        // <content-list>]#`. Parse opaco para que un renderer GCPM lo
        // evalúe.
        "string-set" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::StringSet(None))
            } else {
                Some(DeclKind::StringSet(Some(v.to_string())))
            }
        }
        // Fase 7.490 — `footnote-display` (CSS GCPM 4).
        "footnote-display" => match value.trim().to_ascii_lowercase().as_str() {
            "block" => Some(DeclKind::FootnoteDisplay(FootnoteDisplay::Block)),
            "inline" => Some(DeclKind::FootnoteDisplay(FootnoteDisplay::Inline)),
            "compact" => Some(DeclKind::FootnoteDisplay(FootnoteDisplay::Compact)),
            _ => None,
        },
        // Fase 7.491 — `footnote-policy` (CSS GCPM 4).
        "footnote-policy" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::FootnotePolicy(FootnotePolicy::Auto)),
            "line" => Some(DeclKind::FootnotePolicy(FootnotePolicy::Line)),
            "block" => Some(DeclKind::FootnotePolicy(FootnotePolicy::Block)),
            _ => None,
        },
        // Fase 7.492 — `marker-knockout-left` (CSS GCPM 4).
        "marker-knockout-left" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::MarkerKnockoutLeft(MarkerKnockout::Auto)),
            "none" => Some(DeclKind::MarkerKnockoutLeft(MarkerKnockout::None)),
            _ => None,
        },
        // Fase 7.493 — `marker-knockout-right` (CSS GCPM 4).
        "marker-knockout-right" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::MarkerKnockoutRight(MarkerKnockout::Auto)),
            "none" => Some(DeclKind::MarkerKnockoutRight(MarkerKnockout::None)),
            _ => None,
        },
        // Fase 7.494 — `leading-trim` (CSS Inline 3). HEREDA.
        "leading-trim" => match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::LeadingTrim(LeadingTrim::Normal)),
            "start" => Some(DeclKind::LeadingTrim(LeadingTrim::Start)),
            "end" => Some(DeclKind::LeadingTrim(LeadingTrim::End)),
            "both" => Some(DeclKind::LeadingTrim(LeadingTrim::Both)),
            _ => None,
        },
        // Fase 7.495 — `initial-letter-align` (CSS Inline 3). HEREDA.
        "initial-letter-align" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::InitialLetterAlign(InitialLetterAlign::Auto)),
            "alphabetic" => Some(DeclKind::InitialLetterAlign(InitialLetterAlign::Alphabetic)),
            "hanging" => Some(DeclKind::InitialLetterAlign(InitialLetterAlign::Hanging)),
            "ideographic" => Some(DeclKind::InitialLetterAlign(InitialLetterAlign::Ideographic)),
            "border-box" => Some(DeclKind::InitialLetterAlign(InitialLetterAlign::BorderBox)),
            _ => None,
        },
        // Fase 7.496 — `text-autospace` (CSS Text 4). Parse opaco.
        // `normal` reservado → None.
        "text-autospace" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::TextAutospace(None))
            } else {
                Some(DeclKind::TextAutospace(Some(v.to_string())))
            }
        }
        // Fase 7.497 — `white-space-trim` (CSS Text 4). Parse opaco.
        // `none` reservado → None.
        "white-space-trim" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WhiteSpaceTrim(None))
            } else {
                Some(DeclKind::WhiteSpaceTrim(Some(v.to_string())))
            }
        }
        // Fase 7.498 — `view-transition-group` (CSS View Transitions 2).
        // `normal | contain | nearest | <custom-ident>`. Parse opaco
        // — `normal` reservado a None.
        "view-transition-group" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::ViewTransitionGroup(None))
            } else {
                Some(DeclKind::ViewTransitionGroup(Some(v.to_string())))
            }
        }
        // Fase 7.499 — `inset-area` (CSS Anchor Positioning 1, alias
        // legacy de `position-area`). Parse opaco.
        "inset-area" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::InsetArea(None))
            } else {
                Some(DeclKind::InsetArea(Some(v.to_string())))
            }
        }
        // Fase 7.500 — `view-transition-image-pair` (CSS View Transitions 2).
        "view-transition-image-pair" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ViewTransitionImagePair(None))
            } else {
                Some(DeclKind::ViewTransitionImagePair(Some(v.to_string())))
            }
        }
        // Fase 7.501 — `animation-trigger` (CSS Animations 2, scroll-
        // driven triggers). Shorthand opaco.
        "animation-trigger" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::AnimationTrigger(None))
            } else {
                Some(DeclKind::AnimationTrigger(Some(v.to_string())))
            }
        }
        // Fase 7.502 — `border-image-source` (CSS Backgrounds 3).
        // `none | <image>`. Parse opaco para `<image>` (url/gradient).
        "border-image-source" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::BorderImageSource(None))
            } else {
                Some(DeclKind::BorderImageSource(Some(v.to_string())))
            }
        }
        // Fase 7.503 — `border-image-repeat`. `[stretch|repeat|round|space]{1,2}`.
        "border-image-repeat" => {
            fn kw(s: &str) -> Option<BorderImageRepeat> {
                match s {
                    "stretch" => Some(BorderImageRepeat::Stretch),
                    "repeat" => Some(BorderImageRepeat::Repeat),
                    "round" => Some(BorderImageRepeat::Round),
                    "space" => Some(BorderImageRepeat::Space),
                    _ => None,
                }
            }
            let lower = value.trim().to_ascii_lowercase();
            let parts: Vec<&str> = lower.split_whitespace().collect();
            match parts.len() {
                1 => kw(parts[0]).map(|h| DeclKind::BorderImageRepeat(h, h)),
                2 => match (kw(parts[0]), kw(parts[1])) {
                    (Some(h), Some(v)) => Some(DeclKind::BorderImageRepeat(h, v)),
                    _ => None,
                },
                _ => None,
            }
        }
        // Fase 7.504 — `border-image-slice`. Parse opaco (`<n-p>{1,4} && fill?`).
        "border-image-slice" => {
            let v = value.trim();
            if v.is_empty() { None }
            else { Some(DeclKind::BorderImageSlice(Some(v.to_string()))) }
        }
        // Fase 7.505 — `border-image-width`. Parse opaco.
        "border-image-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else { Some(DeclKind::BorderImageWidth(Some(v.to_string()))) }
        }
        // Fase 7.506 — `border-image-outset`. Parse opaco.
        "border-image-outset" => {
            let v = value.trim();
            if v.is_empty() { None }
            else { Some(DeclKind::BorderImageOutset(Some(v.to_string()))) }
        }
        // Fase 7.507 — `border-image` shorthand. Parse opaco.
        "border-image" | "-webkit-border-image" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::BorderImage(None))
            } else {
                Some(DeclKind::BorderImage(Some(v.to_string())))
            }
        }
        // Fase 7.508 — `grid-template-areas`. Parse opaco (lista de strings
        // quoted que un resolver de grid consume).
        "grid-template-areas" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::GridTemplateAreas(None))
            } else {
                Some(DeclKind::GridTemplateAreas(Some(v.to_string())))
            }
        }
        // Fase 7.509-7.512 — `grid-{row,column}-{start,end}`. Parse opaco
        // de `<grid-line>` (gramática completa `auto | <ident> | <int> |
        // span ...`). El resolver de grid lo evalúa cuando coloca ítems.
        "grid-row-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") { Some(DeclKind::GridRowStart(None)) }
            else { Some(DeclKind::GridRowStart(Some(v.to_string()))) }
        }
        "grid-row-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") { Some(DeclKind::GridRowEnd(None)) }
            else { Some(DeclKind::GridRowEnd(Some(v.to_string()))) }
        }
        "grid-column-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") { Some(DeclKind::GridColumnStart(None)) }
            else { Some(DeclKind::GridColumnStart(Some(v.to_string()))) }
        }
        "grid-column-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") { Some(DeclKind::GridColumnEnd(None)) }
            else { Some(DeclKind::GridColumnEnd(Some(v.to_string()))) }
        }
        // Fase 7.513 — `text-emphasis-skip` (CSS Text Decoration 4). HEREDA.
        "text-emphasis-skip" => match value.trim().to_ascii_lowercase().as_str() {
            "spaces" => Some(DeclKind::TextEmphasisSkip(TextEmphasisSkip::Spaces)),
            "punctuation" => Some(DeclKind::TextEmphasisSkip(TextEmphasisSkip::Punctuation)),
            "symbols" => Some(DeclKind::TextEmphasisSkip(TextEmphasisSkip::Symbols)),
            "narrow" => Some(DeclKind::TextEmphasisSkip(TextEmphasisSkip::Narrow)),
            _ => None,
        },
        // Fase 7.514-7.518 — `animation-*` longhands. Mutación parcial de
        // `s.animation` (Option<AnimationBinding>) — el primer longhand
        // crea la binding con defaults, los siguientes ajustan campos.
        // De una lista separada por coma sólo tomamos el primer item, igual
        // que el shorthand `animation:` ya hace en parser/sheet.rs.
        // Fase 7.735 — alias `-webkit-animation-name` → estándar.
        "animation-name" | "-webkit-animation-name" => {
            let v = first_comma(value.trim());
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::AnimationName(None))
            } else {
                Some(DeclKind::AnimationName(Some(v.to_string())))
            }
        }
        // Fase 7.736 — alias `-webkit-animation-duration` → estándar.
        "animation-duration" | "-webkit-animation-duration" => parse_time_seconds(first_comma(value.trim()))
            .map(DeclKind::AnimationDuration),
        // Fase 7.737 — alias `-webkit-animation-timing-function` → estándar.
        "animation-timing-function" | "-webkit-animation-timing-function" => parse_easing_keyword(first_comma(value.trim()))
            .map(DeclKind::AnimationTimingFunction),
        // Fase 7.738 — alias `-webkit-animation-iteration-count` → estándar.
        "animation-iteration-count" | "-webkit-animation-iteration-count" => {
            let t = first_comma(value.trim());
            if t.eq_ignore_ascii_case("infinite") {
                Some(DeclKind::AnimationIterationCount(AnimationIterations::Infinite))
            } else {
                t.parse::<f32>()
                    .ok()
                    .filter(|n| *n >= 0.0)
                    .map(|n| DeclKind::AnimationIterationCount(AnimationIterations::Count(n)))
            }
        }
        // Fase 7.739 — alias `-webkit-animation-fill-mode` → estándar.
        "animation-fill-mode" | "-webkit-animation-fill-mode" => match first_comma(value.trim()).to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::AnimationFillMode(AnimationFillMode::None)),
            "forwards" => Some(DeclKind::AnimationFillMode(AnimationFillMode::Forwards)),
            "backwards" => Some(DeclKind::AnimationFillMode(AnimationFillMode::Backwards)),
            "both" => Some(DeclKind::AnimationFillMode(AnimationFillMode::Both)),
            _ => None,
        },
        // Fase 7.519 — `float-defer` (CSS Page Floats 3). `none|last|<int>`.
        "float-defer" => {
            let v = value.trim().to_ascii_lowercase();
            match v.as_str() {
                "none" => Some(DeclKind::FloatDefer(FloatDefer::None)),
                "last" => Some(DeclKind::FloatDefer(FloatDefer::Last)),
                _ => v.parse::<i32>().ok().map(|n| DeclKind::FloatDefer(FloatDefer::By(n))),
            }
        }
        // Fase 7.520 — `float-reference` (CSS Page Floats 3).
        "float-reference" => match value.trim().to_ascii_lowercase().as_str() {
            "inline" => Some(DeclKind::FloatReference(FloatReference::Inline)),
            "column" => Some(DeclKind::FloatReference(FloatReference::Column)),
            "region" => Some(DeclKind::FloatReference(FloatReference::Region)),
            "page" => Some(DeclKind::FloatReference(FloatReference::Page)),
            _ => None,
        },
        // Fase 7.521 — `float-offset` (CSS Page Floats 3). `<length-percentage>`.
        "float-offset" => parse_length_px(value).map(DeclKind::FloatOffset),
        // Fase 7.522 — `box-decoration-break` (CSS Fragmentation 4).
        "box-decoration-break" | "-webkit-box-decoration-break" => match value
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "slice" => Some(DeclKind::BoxDecorationBreak(BoxDecorationBreak::Slice)),
            "clone" => Some(DeclKind::BoxDecorationBreak(BoxDecorationBreak::Clone)),
            _ => None,
        },
        // Fase 7.523 — `line-snap` (CSS Line Grid). HEREDA.
        "line-snap" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::LineSnap(LineSnap::None)),
            "baseline" => Some(DeclKind::LineSnap(LineSnap::Baseline)),
            "contain" => Some(DeclKind::LineSnap(LineSnap::Contain)),
            _ => None,
        },
        // Fase 7.524 — `line-grid` (CSS Line Grid). HEREDA.
        "line-grid" => match value.trim().to_ascii_lowercase().as_str() {
            "match" => Some(DeclKind::LineGrid(LineGrid::Match)),
            "create" => Some(DeclKind::LineGrid(LineGrid::Create)),
            _ => None,
        },
        // Fase 7.525 — `initial-letter` shorthand (CSS Inline 3). HEREDA.
        // Parse opaco hasta que un layout de drop-cap lo necesite.
        // Fase 7.747 — alias `-webkit-initial-letter` → estándar.
        "initial-letter" | "-webkit-initial-letter" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::InitialLetter(None))
            } else {
                Some(DeclKind::InitialLetter(Some(v.to_string())))
            }
        }
        // Fase 7.526 — `highlight` (CSS Highlight API). HEREDA.
        "highlight" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::Highlight(None))
            } else {
                Some(DeclKind::Highlight(Some(v.to_string())))
            }
        }
        // Fase 7.527 — `ruby-merge` (CSS Ruby 1). HEREDA.
        "ruby-merge" => match value.trim().to_ascii_lowercase().as_str() {
            "separate" => Some(DeclKind::RubyMerge(RubyMerge::Separate)),
            "collapse" => Some(DeclKind::RubyMerge(RubyMerge::Collapse)),
            "auto" => Some(DeclKind::RubyMerge(RubyMerge::Auto)),
            _ => None,
        },
        // Fase 7.528 — `text-spacing` shorthand (CSS Text 4). HEREDA.
        // Parse opaco — `normal` reservado a None.
        "text-spacing" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::TextSpacing(None))
            } else {
                Some(DeclKind::TextSpacing(Some(v.to_string())))
            }
        }
        // Fase 7.529 — `speak-as` (CSS Speech 1). HEREDA.
        "speak-as" => match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::SpeakAs(SpeakAs::Normal)),
            "spell-out" => Some(DeclKind::SpeakAs(SpeakAs::SpellOut)),
            "digits" => Some(DeclKind::SpeakAs(SpeakAs::Digits)),
            "literal-punctuation" => Some(DeclKind::SpeakAs(SpeakAs::LiteralPunctuation)),
            "no-punctuation" => Some(DeclKind::SpeakAs(SpeakAs::NoPunctuation)),
            _ => None,
        },
        // Fase 7.530 — `voice-balance` (CSS Speech 1). -100..100. HEREDA.
        // Keywords `left|center|right|leftwards|rightwards` → -100/0/100/-50/50.
        "voice-balance" => match value.trim().to_ascii_lowercase().as_str() {
            "left" => Some(DeclKind::VoiceBalance(-100.0)),
            "leftwards" => Some(DeclKind::VoiceBalance(-50.0)),
            "center" => Some(DeclKind::VoiceBalance(0.0)),
            "rightwards" => Some(DeclKind::VoiceBalance(50.0)),
            "right" => Some(DeclKind::VoiceBalance(100.0)),
            other => other
                .parse::<f32>()
                .ok()
                .filter(|n| (-100.0..=100.0).contains(n))
                .map(DeclKind::VoiceBalance),
        },
        // Fase 7.531-7.533 — `voice-{pitch,rate,volume}` (CSS Speech 1).
        // Parse opaco — `medium`/`normal` reservados a None.
        "voice-pitch" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::VoicePitch(None))
            } else {
                Some(DeclKind::VoicePitch(Some(v.to_string())))
            }
        }
        "voice-rate" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::VoiceRate(None))
            } else {
                Some(DeclKind::VoiceRate(Some(v.to_string())))
            }
        }
        "voice-volume" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::VoiceVolume(None))
            } else {
                Some(DeclKind::VoiceVolume(Some(v.to_string())))
            }
        }
        // Fase 7.534-7.537 — `pause-{before,after}` y `rest-{before,after}`
        // (CSS Speech 1). Parse opaco — `none` reservado a None.
        "pause-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::PauseBefore(None))
            } else {
                Some(DeclKind::PauseBefore(Some(v.to_string())))
            }
        }
        "pause-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::PauseAfter(None))
            } else {
                Some(DeclKind::PauseAfter(Some(v.to_string())))
            }
        }
        "rest-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::RestBefore(None))
            } else {
                Some(DeclKind::RestBefore(Some(v.to_string())))
            }
        }
        "rest-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::RestAfter(None))
            } else {
                Some(DeclKind::RestAfter(Some(v.to_string())))
            }
        }
        // Fase 7.538 — `cue-fade-duration` (CSS Speech 1). `<time>`.
        "cue-fade-duration" => parse_time_seconds(value.trim()).map(DeclKind::CueFadeDuration),
        // Fase 7.539-7.541 — `cue-{before,after}` y `cue` shorthand (CSS Speech 1).
        // Parse opaco — `none` reservado a None.
        "cue-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::CueBefore(None))
            } else {
                Some(DeclKind::CueBefore(Some(v.to_string())))
            }
        }
        "cue-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::CueAfter(None))
            } else {
                Some(DeclKind::CueAfter(Some(v.to_string())))
            }
        }
        "cue" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::Cue(None))
            } else {
                Some(DeclKind::Cue(Some(v.to_string())))
            }
        }
        // Fase 7.542 — `navigation-up` (CSS UI 3 legacy). Parse opaco —
        // `auto` reservado a None.
        "navigation-up" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavigationUp(None))
            } else {
                Some(DeclKind::NavigationUp(Some(v.to_string())))
            }
        }
        // Fase 7.543 — `glyph-orientation-horizontal` (SVG 1.1 legacy).
        // `<angle>` en grados; sólo aceptamos 0/90/180/270 y los keywords
        // `0deg`/`90deg`/... — gramática extendida por simplicidad.
        "glyph-orientation-horizontal" => {
            let v = value.trim().to_ascii_lowercase();
            let num = v.strip_suffix("deg").unwrap_or(&v);
            num.parse::<f32>().ok().map(DeclKind::GlyphOrientationHorizontal)
        }
        // Fase 7.544-7.546 — `navigation-{down,left,right}` (CSS UI 3 legacy).
        // Parse opaco — `auto` reservado a None.
        "navigation-down" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavigationDown(None))
            } else {
                Some(DeclKind::NavigationDown(Some(v.to_string())))
            }
        }
        "navigation-left" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavigationLeft(None))
            } else {
                Some(DeclKind::NavigationLeft(Some(v.to_string())))
            }
        }
        "navigation-right" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavigationRight(None))
            } else {
                Some(DeclKind::NavigationRight(Some(v.to_string())))
            }
        }
        // Fase 7.547 — `counter-increment-style` (CSS Lists 4). Parse opaco
        // — `decimal` reservado a None.
        "counter-increment-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("decimal") {
                Some(DeclKind::CounterIncrementStyle(None))
            } else {
                Some(DeclKind::CounterIncrementStyle(Some(v.to_string())))
            }
        }
        // Fase 7.548 — `overflow-clip-box` (CSS Overflow legacy).
        "overflow-clip-box" => match value.trim().to_ascii_lowercase().as_str() {
            "padding-box" => Some(DeclKind::OverflowClipBox(OverflowClipBox::PaddingBox)),
            "content-box" => Some(DeclKind::OverflowClipBox(OverflowClipBox::ContentBox)),
            _ => None,
        },
        // Fase 7.549-7.552 — familia `mask-border-*` (CSS Masking 1). Parse
        // opaco; el sentinel reservado va a `None`.
        // Fase 7.609-7.613 — `-webkit-mask-box-image-*` son los alias vendor
        // (de facto) de `mask-border-*`: enrutan al mismo handler/almacén.
        "mask-border-source" | "-webkit-mask-box-image-source" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::MaskBorderSource(None))
            } else {
                Some(DeclKind::MaskBorderSource(Some(v.to_string())))
            }
        }
        "mask-border-slice" | "-webkit-mask-box-image-slice" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::MaskBorderSlice(None))
            } else {
                Some(DeclKind::MaskBorderSlice(Some(v.to_string())))
            }
        }
        "mask-border-width" | "-webkit-mask-box-image-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::MaskBorderWidth(None))
            } else {
                Some(DeclKind::MaskBorderWidth(Some(v.to_string())))
            }
        }
        "mask-border-outset" | "-webkit-mask-box-image-outset" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::MaskBorderOutset(None))
            } else {
                Some(DeclKind::MaskBorderOutset(Some(v.to_string())))
            }
        }
        // Fase 7.553 — `mask-border-repeat` (CSS Masking 1); Fase 7.613 alias.
        "mask-border-repeat" | "-webkit-mask-box-image-repeat" => match value.trim().to_ascii_lowercase().as_str() {
            "stretch" => Some(DeclKind::MaskBorderRepeat(MaskBorderRepeat::Stretch)),
            "repeat" => Some(DeclKind::MaskBorderRepeat(MaskBorderRepeat::Repeat)),
            "round" => Some(DeclKind::MaskBorderRepeat(MaskBorderRepeat::Round)),
            "space" => Some(DeclKind::MaskBorderRepeat(MaskBorderRepeat::Space)),
            _ => None,
        },
        // Fase 7.554 — `mask-border-mode` (CSS Masking 1).
        "mask-border-mode" => match value.trim().to_ascii_lowercase().as_str() {
            "luminance" => Some(DeclKind::MaskBorderMode(MaskBorderMode::Luminance)),
            "alpha" => Some(DeclKind::MaskBorderMode(MaskBorderMode::Alpha)),
            _ => None,
        },
        // Fase 7.555 — `caret-animation` (CSS UI 4).
        "caret-animation" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::CaretAnimation(CaretAnimation::Auto)),
            "manual" => Some(DeclKind::CaretAnimation(CaretAnimation::Manual)),
            _ => None,
        },
        // Fase 7.556 — `scroll-marker-group` (CSS Overflow 5).
        "scroll-marker-group" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::ScrollMarkerGroup(ScrollMarkerGroup::None)),
            "before" => Some(DeclKind::ScrollMarkerGroup(ScrollMarkerGroup::Before)),
            "after" => Some(DeclKind::ScrollMarkerGroup(ScrollMarkerGroup::After)),
            _ => None,
        },
        // Fase 7.557 — `scroll-initial-target` (CSS Overflow 5).
        "scroll-initial-target" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::ScrollInitialTarget(ScrollInitialTarget::None)),
            "nearest" => Some(DeclKind::ScrollInitialTarget(ScrollInitialTarget::Nearest)),
            _ => None,
        },
        // Fase 7.558 — `corner-shape` (CSS Borders 4). Parse opaco —
        // `round` reservado a None.
        "corner-shape" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("round") {
                Some(DeclKind::CornerShape(None))
            } else {
                Some(DeclKind::CornerShape(Some(v.to_string())))
            }
        }
        // Fase 7.559 — `hyphenate-limit-lines` (CSS Text 4). `no-limit` → None.
        "hyphenate-limit-lines" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("no-limit") {
                Some(DeclKind::HyphenateLimitLines(None))
            } else {
                v.parse::<u32>().ok().map(|n| DeclKind::HyphenateLimitLines(Some(n)))
            }
        }
        // Fase 7.560 — `hyphenate-limit-last` (CSS Text 4).
        "hyphenate-limit-last" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::HyphenateLimitLast(HyphenateLimitLast::None)),
            "always" => Some(DeclKind::HyphenateLimitLast(HyphenateLimitLast::Always)),
            "column" => Some(DeclKind::HyphenateLimitLast(HyphenateLimitLast::Column)),
            "page" => Some(DeclKind::HyphenateLimitLast(HyphenateLimitLast::Page)),
            "spread" => Some(DeclKind::HyphenateLimitLast(HyphenateLimitLast::Spread)),
            _ => None,
        },
        // Fase 7.561 — `hyphenate-limit-zone` (CSS Text 4). `0` → None.
        "hyphenate-limit-zone" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::HyphenateLimitZone(None))
            } else {
                Some(DeclKind::HyphenateLimitZone(Some(v.to_string())))
            }
        }
        // Fase 7.562 — `interest-target` (interest invokers). Parse opaco.
        "interest-target" => {
            let v = value.trim();
            if v.is_empty() || v.eq_ignore_ascii_case("none") {
                Some(DeclKind::InterestTarget(None))
            } else {
                Some(DeclKind::InterestTarget(Some(v.to_string())))
            }
        }
        // Fase 7.563 — `interest-delay-start`. `normal` → None.
        "interest-delay-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::InterestDelayStart(None))
            } else {
                Some(DeclKind::InterestDelayStart(Some(v.to_string())))
            }
        }
        // Fase 7.564 — `interest-delay-end`. `normal` → None.
        "interest-delay-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::InterestDelayEnd(None))
            } else {
                Some(DeclKind::InterestDelayEnd(Some(v.to_string())))
            }
        }
        // Fase 7.565 — `azimuth` (CSS 2.1 aural). Parse opaco; `center` → None.
        "azimuth" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("center") {
                Some(DeclKind::Azimuth(None))
            } else {
                Some(DeclKind::Azimuth(Some(v.to_string())))
            }
        }
        // Fase 7.566 — `elevation` (CSS 2.1 aural). Parse opaco; `level` → None.
        "elevation" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("level") {
                Some(DeclKind::Elevation(None))
            } else {
                Some(DeclKind::Elevation(Some(v.to_string())))
            }
        }
        // Fase 7.567 — `richness` (CSS 2.1 aural). Número 0–100, clamp.
        "richness" => value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|n| DeclKind::Richness(n.clamp(0.0, 100.0))),
        // Fase 7.568 — `stress` (CSS 2.1 aural). Número 0–100, clamp.
        "stress" => value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|n| DeclKind::Stress(n.clamp(0.0, 100.0))),
        // Fase 7.569-7.571 — `pitch`/`speech-rate`/`volume` (CSS 2.1 aural).
        // Parse opaco; `medium` → None.
        "pitch" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::Pitch(None))
            } else {
                Some(DeclKind::Pitch(Some(v.to_string())))
            }
        }
        "speech-rate" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::SpeechRate(None))
            } else {
                Some(DeclKind::SpeechRate(Some(v.to_string())))
            }
        }
        "volume" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::Volume(None))
            } else {
                Some(DeclKind::Volume(Some(v.to_string())))
            }
        }
        // Fase 7.572 — `speak` (CSS 2.1 aural). Distinto de `speak-as`.
        "speak" => match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::Speak(Speak::Normal)),
            "none" => Some(DeclKind::Speak(Speak::None)),
            "spell-out" => Some(DeclKind::Speak(Speak::SpellOut)),
            _ => None,
        },
        // Fase 7.573 — `play-during` (CSS 2.1 aural). Parse opaco; `auto` → None.
        "play-during" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::PlayDuring(None))
            } else {
                Some(DeclKind::PlayDuring(Some(v.to_string())))
            }
        }
        // Fase 7.574 — `text-decoration-skip` (CSS Text Decor 4, shorthand
        // legacy). Parse opaco; `auto` → None.
        // Fase 7.743 — alias `-webkit-text-decoration-skip` → estándar.
        "text-decoration-skip" | "-webkit-text-decoration-skip" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::TextDecorationSkip(None))
            } else {
                Some(DeclKind::TextDecorationSkip(Some(v.to_string())))
            }
        }
        // Fase 7.575 — `text-decoration-skip-box` (CSS Text Decor 4).
        "text-decoration-skip-box" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::TextDecorationSkipBox(TextDecorationSkipBox::None)),
            "all" => Some(DeclKind::TextDecorationSkipBox(TextDecorationSkipBox::All)),
            _ => None,
        },
        // Fase 7.576 — `text-decoration-skip-self` (CSS Text Decor 4).
        "text-decoration-skip-self" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::TextDecorationSkipSelf(None))
            } else {
                Some(DeclKind::TextDecorationSkipSelf(Some(v.to_string())))
            }
        }
        // Fase 7.577 — `text-decoration-skip-spaces` (CSS Text Decor 4).
        // `start end` (default) → None.
        "text-decoration-skip-spaces" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("start end") {
                Some(DeclKind::TextDecorationSkipSpaces(None))
            } else {
                Some(DeclKind::TextDecorationSkipSpaces(Some(v.to_string())))
            }
        }
        // Fase 7.578 — `text-decoration-skip-inset` (CSS Text Decor 4).
        "text-decoration-skip-inset" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::TextDecorationSkipInset(TextDecorationSkipInset::None)),
            "auto" => Some(DeclKind::TextDecorationSkipInset(TextDecorationSkipInset::Auto)),
            _ => None,
        },
        // Fase 7.579 — `-webkit-text-stroke-width`. px o número desnudo;
        // keywords thin/medium/thick → 1/3/5.
        "-webkit-text-stroke-width" => {
            let v = value.trim().to_ascii_lowercase();
            match v.as_str() {
                "thin" => Some(DeclKind::WebkitTextStrokeWidth(1.0)),
                "medium" => Some(DeclKind::WebkitTextStrokeWidth(3.0)),
                "thick" => Some(DeclKind::WebkitTextStrokeWidth(5.0)),
                _ => {
                    let num = v.strip_suffix("px").unwrap_or(&v);
                    num.parse::<f32>().ok().map(DeclKind::WebkitTextStrokeWidth)
                }
            }
        }
        // Fase 7.580-7.581 — `-webkit-text-{stroke,fill}-color`. Parse opaco;
        // `currentcolor` → None.
        "-webkit-text-stroke-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitTextStrokeColor(None))
            } else {
                Some(DeclKind::WebkitTextStrokeColor(Some(v.to_string())))
            }
        }
        "-webkit-text-fill-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitTextFillColor(None))
            } else {
                Some(DeclKind::WebkitTextFillColor(Some(v.to_string())))
            }
        }
        // Fase 7.582 — `font-smooth` (no estándar). Parse opaco; `auto` → None.
        "font-smooth" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::FontSmooth(None))
            } else {
                Some(DeclKind::FontSmooth(Some(v.to_string())))
            }
        }
        // Fase 7.583 — `text-group-align` (CSS Text 4).
        "text-group-align" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::TextGroupAlign(TextGroupAlign::None)),
            "start" => Some(DeclKind::TextGroupAlign(TextGroupAlign::Start)),
            "end" => Some(DeclKind::TextGroupAlign(TextGroupAlign::End)),
            "left" => Some(DeclKind::TextGroupAlign(TextGroupAlign::Left)),
            "right" => Some(DeclKind::TextGroupAlign(TextGroupAlign::Right)),
            "center" => Some(DeclKind::TextGroupAlign(TextGroupAlign::Center)),
            _ => None,
        },
        // Fase 7.584 — `continue` (CSS Overflow 4).
        "continue" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::Continue(Continue::Auto)),
            "discard" => Some(DeclKind::Continue(Continue::Discard)),
            _ => None,
        },
        // Fase 7.585 — `block-ellipsis` (CSS Overflow 4). Parse opaco;
        // `none` → None (también `auto` se conserva como string).
        "block-ellipsis" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::BlockEllipsis(None))
            } else {
                Some(DeclKind::BlockEllipsis(Some(v.to_string())))
            }
        }
        // Fase 7.586 — `max-lines` (CSS Overflow 4). `none` → None.
        "max-lines" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::MaxLines(None))
            } else {
                v.parse::<u32>().ok().map(|n| DeclKind::MaxLines(Some(n)))
            }
        }
        // Fase 7.587 — `region-fragment` (CSS Regions 1).
        "region-fragment" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::RegionFragment(RegionFragment::Auto)),
            "break" => Some(DeclKind::RegionFragment(RegionFragment::Break)),
            _ => None,
        },
        // Fase 7.588 — `overflow-style` (CSS Marquee/Basic UI legacy).
        // Parse opaco; `auto` → None.
        "overflow-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::OverflowStyle(None))
            } else {
                Some(DeclKind::OverflowStyle(Some(v.to_string())))
            }
        }
        // Fase 7.589 — `marquee-style` (CSS Marquee).
        "marquee-style" => match value.trim().to_ascii_lowercase().as_str() {
            "scroll" => Some(DeclKind::MarqueeStyle(MarqueeStyle::Scroll)),
            "slide" => Some(DeclKind::MarqueeStyle(MarqueeStyle::Slide)),
            "alternate" => Some(DeclKind::MarqueeStyle(MarqueeStyle::Alternate)),
            _ => None,
        },
        // Fase 7.590 — `marquee-direction` (CSS Marquee).
        "marquee-direction" => match value.trim().to_ascii_lowercase().as_str() {
            "forward" => Some(DeclKind::MarqueeDirection(MarqueeDirection::Forward)),
            "reverse" => Some(DeclKind::MarqueeDirection(MarqueeDirection::Reverse)),
            _ => None,
        },
        // Fase 7.591 — `marquee-speed` (CSS Marquee).
        "marquee-speed" => match value.trim().to_ascii_lowercase().as_str() {
            "slow" => Some(DeclKind::MarqueeSpeed(MarqueeSpeed::Slow)),
            "normal" => Some(DeclKind::MarqueeSpeed(MarqueeSpeed::Normal)),
            "fast" => Some(DeclKind::MarqueeSpeed(MarqueeSpeed::Fast)),
            _ => None,
        },
        // Fase 7.592 — `marquee-loop` (CSS Marquee). `infinite` → None.
        "marquee-loop" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("infinite") {
                Some(DeclKind::MarqueeLoop(None))
            } else {
                v.parse::<i32>().ok().map(|n| DeclKind::MarqueeLoop(Some(n)))
            }
        }
        // Fase 7.593 — `marquee-increment` (CSS Marquee). `6px` (default) → None.
        "marquee-increment" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("6px") {
                Some(DeclKind::MarqueeIncrement(None))
            } else {
                Some(DeclKind::MarqueeIncrement(Some(v.to_string())))
            }
        }
        // Fase 7.594-7.598 — familia `nav-*` (CSS UI 3 legacy). Parse opaco;
        // `auto` → None.
        "nav-index" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavIndex(None))
            } else {
                Some(DeclKind::NavIndex(Some(v.to_string())))
            }
        }
        "nav-up" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavUp(None))
            } else {
                Some(DeclKind::NavUp(Some(v.to_string())))
            }
        }
        "nav-down" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavDown(None))
            } else {
                Some(DeclKind::NavDown(Some(v.to_string())))
            }
        }
        "nav-left" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavLeft(None))
            } else {
                Some(DeclKind::NavLeft(Some(v.to_string())))
            }
        }
        "nav-right" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavRight(None))
            } else {
                Some(DeclKind::NavRight(Some(v.to_string())))
            }
        }
        // Fase 7.599-7.602 — `-webkit-box-{orient,direction,align,pack}`
        // (flexbox viejo). Parse opaco; sentinel default → None.
        "-webkit-box-orient" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("inline-axis") {
                Some(DeclKind::WebkitBoxOrient(None))
            } else {
                Some(DeclKind::WebkitBoxOrient(Some(v.to_string())))
            }
        }
        "-webkit-box-direction" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::WebkitBoxDirection(None))
            } else {
                Some(DeclKind::WebkitBoxDirection(Some(v.to_string())))
            }
        }
        "-webkit-box-align" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("stretch") {
                Some(DeclKind::WebkitBoxAlign(None))
            } else {
                Some(DeclKind::WebkitBoxAlign(Some(v.to_string())))
            }
        }
        "-webkit-box-pack" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("start") {
                Some(DeclKind::WebkitBoxPack(None))
            } else {
                Some(DeclKind::WebkitBoxPack(Some(v.to_string())))
            }
        }
        // Fase 7.603 — `-webkit-box-flex` (flexbox viejo). Número desnudo.
        "-webkit-box-flex" => value
            .trim()
            .parse::<f32>()
            .ok()
            .map(DeclKind::WebkitBoxFlex),
        // Fase 7.604 — `-webkit-box-ordinal-group` (flexbox viejo). `1` → None.
        "-webkit-box-ordinal-group" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "1" {
                Some(DeclKind::WebkitBoxOrdinalGroup(None))
            } else {
                v.parse::<u32>().ok().map(|n| DeclKind::WebkitBoxOrdinalGroup(Some(n)))
            }
        }
        // Fase 7.605-7.606 — `-webkit-font-smoothing` / `-moz-osx-font-smoothing`
        // (no estándar). Parse opaco; `auto` → None.
        "-webkit-font-smoothing" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitFontSmoothing(None))
            } else {
                Some(DeclKind::WebkitFontSmoothing(Some(v.to_string())))
            }
        }
        "-moz-osx-font-smoothing" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::MozOsxFontSmoothing(None))
            } else {
                Some(DeclKind::MozOsxFontSmoothing(Some(v.to_string())))
            }
        }
        // Fase 7.607 — `-webkit-tap-highlight-color`. Parse opaco.
        "-webkit-tap-highlight-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else { Some(DeclKind::WebkitTapHighlightColor(Some(v.to_string()))) }
        }
        // Fase 7.608 — `zoom`. Parse opaco; `normal` → None.
        "zoom" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::Zoom(None))
            } else {
                Some(DeclKind::Zoom(Some(v.to_string())))
            }
        }
        // Fase 7.614-7.616 — `column-break-{before,after,inside}` (Multicol
        // legacy). Parse opaco; `auto` → None.
        "column-break-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ColumnBreakBefore(None))
            } else {
                Some(DeclKind::ColumnBreakBefore(Some(v.to_string())))
            }
        }
        "column-break-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ColumnBreakAfter(None))
            } else {
                Some(DeclKind::ColumnBreakAfter(Some(v.to_string())))
            }
        }
        "column-break-inside" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ColumnBreakInside(None))
            } else {
                Some(DeclKind::ColumnBreakInside(Some(v.to_string())))
            }
        }
        // Fase 7.617 — `user-modify` (+ alias `-webkit-user-modify`). `read-only` → None.
        "user-modify" | "-webkit-user-modify" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("read-only") {
                Some(DeclKind::UserModify(None))
            } else {
                Some(DeclKind::UserModify(Some(v.to_string())))
            }
        }
        // Fase 7.618 — `-webkit-touch-callout` (iOS). `default` → None.
        "-webkit-touch-callout" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("default") {
                Some(DeclKind::WebkitTouchCallout(None))
            } else {
                Some(DeclKind::WebkitTouchCallout(Some(v.to_string())))
            }
        }
        // Fase 7.619 — `-webkit-user-drag`. `auto` → None.
        "-webkit-user-drag" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitUserDrag(None))
            } else {
                Some(DeclKind::WebkitUserDrag(Some(v.to_string())))
            }
        }
        // Fase 7.620 — `-webkit-rtl-ordering`. `logical` → None.
        "-webkit-rtl-ordering" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("logical") {
                Some(DeclKind::WebkitRtlOrdering(None))
            } else {
                Some(DeclKind::WebkitRtlOrdering(Some(v.to_string())))
            }
        }
        // Fase 7.621 — `-webkit-text-security`. `none` → None.
        "-webkit-text-security" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitTextSecurity(None))
            } else {
                Some(DeclKind::WebkitTextSecurity(Some(v.to_string())))
            }
        }
        // Fase 7.622 — `-webkit-nbsp-mode`. `normal` → None.
        "-webkit-nbsp-mode" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::WebkitNbspMode(None))
            } else {
                Some(DeclKind::WebkitNbspMode(Some(v.to_string())))
            }
        }
        // Fase 7.623 — `-webkit-locale`. `auto` → None.
        "-webkit-locale" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitLocale(None))
            } else {
                Some(DeclKind::WebkitLocale(Some(v.to_string())))
            }
        }
        // Fase 7.624 — `-webkit-column-axis`. `auto` → None.
        "-webkit-column-axis" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitColumnAxis(None))
            } else {
                Some(DeclKind::WebkitColumnAxis(Some(v.to_string())))
            }
        }
        // Fase 7.625 — `-webkit-column-progression`. `normal` → None.
        "-webkit-column-progression" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::WebkitColumnProgression(None))
            } else {
                Some(DeclKind::WebkitColumnProgression(Some(v.to_string())))
            }
        }
        // Fase 7.626 — `-webkit-app-region` (Chrome/Electron). `none` → None.
        "-webkit-app-region" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitAppRegion(None))
            } else {
                Some(DeclKind::WebkitAppRegion(Some(v.to_string())))
            }
        }
        // Fase 7.627 — `-webkit-highlight`. `none` → None.
        "-webkit-highlight" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitHighlight(None))
            } else {
                Some(DeclKind::WebkitHighlight(Some(v.to_string())))
            }
        }
        // Fase 7.628 — `-webkit-box-reflect`. `none` → None.
        "-webkit-box-reflect" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBoxReflect(None))
            } else {
                Some(DeclKind::WebkitBoxReflect(Some(v.to_string())))
            }
        }
        // Fase 7.644 — `-webkit-mask-composite`. `add` → None.
        "-webkit-mask-composite" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("add") {
                Some(DeclKind::WebkitMaskComposite(None))
            } else {
                Some(DeclKind::WebkitMaskComposite(Some(v.to_string())))
            }
        }
        // Fase 7.645 — `-webkit-mask-position-x`. `center` → None.
        "-webkit-mask-position-x" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitMaskPositionX(None))
            } else {
                Some(DeclKind::WebkitMaskPositionX(Some(v.to_string())))
            }
        }
        // Fase 7.646 — `-webkit-mask-position-y`. `center` → None.
        "-webkit-mask-position-y" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitMaskPositionY(None))
            } else {
                Some(DeclKind::WebkitMaskPositionY(Some(v.to_string())))
            }
        }
        // Fase 7.647 — `-webkit-mask-repeat-x`. `repeat` → None.
        "-webkit-mask-repeat-x" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("repeat") {
                Some(DeclKind::WebkitMaskRepeatX(None))
            } else {
                Some(DeclKind::WebkitMaskRepeatX(Some(v.to_string())))
            }
        }
        // Fase 7.648 — `-webkit-mask-repeat-y`. `repeat` → None.
        "-webkit-mask-repeat-y" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("repeat") {
                Some(DeclKind::WebkitMaskRepeatY(None))
            } else {
                Some(DeclKind::WebkitMaskRepeatY(Some(v.to_string())))
            }
        }
        // Fase 7.649 — `-webkit-margin-start` (alias legacy de
        // margin-inline-start). `0` → None.
        "-webkit-margin-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitMarginStart(None))
            } else {
                Some(DeclKind::WebkitMarginStart(Some(v.to_string())))
            }
        }
        // Fase 7.650 — `-webkit-margin-end`. `0` → None.
        "-webkit-margin-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitMarginEnd(None))
            } else {
                Some(DeclKind::WebkitMarginEnd(Some(v.to_string())))
            }
        }
        // Fase 7.651 — `-webkit-margin-before` (block-start). `0` → None.
        "-webkit-margin-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitMarginBefore(None))
            } else {
                Some(DeclKind::WebkitMarginBefore(Some(v.to_string())))
            }
        }
        // Fase 7.652 — `-webkit-margin-after` (block-end). `0` → None.
        "-webkit-margin-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitMarginAfter(None))
            } else {
                Some(DeclKind::WebkitMarginAfter(Some(v.to_string())))
            }
        }
        // Fase 7.653 — `-webkit-padding-start`. `0` → None.
        "-webkit-padding-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitPaddingStart(None))
            } else {
                Some(DeclKind::WebkitPaddingStart(Some(v.to_string())))
            }
        }
        // Fase 7.654 — `-webkit-padding-end`. `0` → None.
        "-webkit-padding-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitPaddingEnd(None))
            } else {
                Some(DeclKind::WebkitPaddingEnd(Some(v.to_string())))
            }
        }
        // Fase 7.655 — `-webkit-padding-before` (block-start). `0` → None.
        "-webkit-padding-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitPaddingBefore(None))
            } else {
                Some(DeclKind::WebkitPaddingBefore(Some(v.to_string())))
            }
        }
        // Fase 7.656 — `-webkit-padding-after` (block-end). `0` → None.
        "-webkit-padding-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitPaddingAfter(None))
            } else {
                Some(DeclKind::WebkitPaddingAfter(Some(v.to_string())))
            }
        }
        // Fase 7.657 — `-webkit-logical-width` (inline-size). `auto` → None.
        "-webkit-logical-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitLogicalWidth(None))
            } else {
                Some(DeclKind::WebkitLogicalWidth(Some(v.to_string())))
            }
        }
        // Fase 7.658 — `-webkit-logical-height` (block-size). `auto` → None.
        "-webkit-logical-height" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitLogicalHeight(None))
            } else {
                Some(DeclKind::WebkitLogicalHeight(Some(v.to_string())))
            }
        }
        // Fase 7.664 — `-webkit-transform-origin-x`. `50%`/`center` → None.
        "-webkit-transform-origin-x" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "50%" || v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitTransformOriginX(None))
            } else {
                Some(DeclKind::WebkitTransformOriginX(Some(v.to_string())))
            }
        }
        // Fase 7.665 — `-webkit-transform-origin-y`. `50%`/`center` → None.
        "-webkit-transform-origin-y" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "50%" || v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitTransformOriginY(None))
            } else {
                Some(DeclKind::WebkitTransformOriginY(Some(v.to_string())))
            }
        }
        // Fase 7.666 — `-webkit-transform-origin-z`. `0` → None.
        "-webkit-transform-origin-z" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitTransformOriginZ(None))
            } else {
                Some(DeclKind::WebkitTransformOriginZ(Some(v.to_string())))
            }
        }
        // Fase 7.667 — `-webkit-perspective-origin-x`. `50%`/`center` → None.
        "-webkit-perspective-origin-x" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "50%" || v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitPerspectiveOriginX(None))
            } else {
                Some(DeclKind::WebkitPerspectiveOriginX(Some(v.to_string())))
            }
        }
        // Fase 7.668 — `-webkit-perspective-origin-y`. `50%`/`center` → None.
        "-webkit-perspective-origin-y" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "50%" || v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitPerspectiveOriginY(None))
            } else {
                Some(DeclKind::WebkitPerspectiveOriginY(Some(v.to_string())))
            }
        }
        // Fase 7.669 — `-webkit-min-logical-width` (min-inline-size). `auto` → None.
        "-webkit-min-logical-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitMinLogicalWidth(None))
            } else {
                Some(DeclKind::WebkitMinLogicalWidth(Some(v.to_string())))
            }
        }
        // Fase 7.670 — `-webkit-max-logical-width` (max-inline-size). `none` → None.
        "-webkit-max-logical-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitMaxLogicalWidth(None))
            } else {
                Some(DeclKind::WebkitMaxLogicalWidth(Some(v.to_string())))
            }
        }
        // Fase 7.671 — `-webkit-min-logical-height` (min-block-size). `auto` → None.
        "-webkit-min-logical-height" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitMinLogicalHeight(None))
            } else {
                Some(DeclKind::WebkitMinLogicalHeight(Some(v.to_string())))
            }
        }
        // Fase 7.672 — `-webkit-max-logical-height` (max-block-size). `none` → None.
        "-webkit-max-logical-height" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitMaxLogicalHeight(None))
            } else {
                Some(DeclKind::WebkitMaxLogicalHeight(Some(v.to_string())))
            }
        }
        // Fase 7.673 — `-webkit-background-composite`. `source-over` → None.
        "-webkit-background-composite" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("source-over") {
                Some(DeclKind::WebkitBackgroundComposite(None))
            } else {
                Some(DeclKind::WebkitBackgroundComposite(Some(v.to_string())))
            }
        }
        // Fase 7.674 — `-webkit-border-before` (border-block-start). `none` → None.
        "-webkit-border-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderBefore(None))
            } else {
                Some(DeclKind::WebkitBorderBefore(Some(v.to_string())))
            }
        }
        // Fase 7.675 — `-webkit-border-after` (border-block-end). `none` → None.
        "-webkit-border-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderAfter(None))
            } else {
                Some(DeclKind::WebkitBorderAfter(Some(v.to_string())))
            }
        }
        // Fase 7.676 — `-webkit-border-start` (border-inline-start). `none` → None.
        "-webkit-border-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderStart(None))
            } else {
                Some(DeclKind::WebkitBorderStart(Some(v.to_string())))
            }
        }
        // Fase 7.677 — `-webkit-border-end` (border-inline-end). `none` → None.
        "-webkit-border-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderEnd(None))
            } else {
                Some(DeclKind::WebkitBorderEnd(Some(v.to_string())))
            }
        }
        // Fase 7.678 — `-webkit-border-horizontal-spacing`. `0` → None.
        "-webkit-border-horizontal-spacing" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitBorderHorizontalSpacing(None))
            } else {
                Some(DeclKind::WebkitBorderHorizontalSpacing(Some(v.to_string())))
            }
        }
        // Fase 7.679 — `-webkit-flow-into` (CSS Regions). `none` → None.
        "-webkit-flow-into" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitFlowInto(None))
            } else {
                Some(DeclKind::WebkitFlowInto(Some(v.to_string())))
            }
        }
        // Fase 7.680 — `-webkit-flow-from` (CSS Regions). `none` → None.
        "-webkit-flow-from" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitFlowFrom(None))
            } else {
                Some(DeclKind::WebkitFlowFrom(Some(v.to_string())))
            }
        }
        // Fase 7.681 — `-webkit-region-break-before`. `auto` → None.
        "-webkit-region-break-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitRegionBreakBefore(None))
            } else {
                Some(DeclKind::WebkitRegionBreakBefore(Some(v.to_string())))
            }
        }
        // Fase 7.682 — `-webkit-region-break-after`. `auto` → None.
        "-webkit-region-break-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitRegionBreakAfter(None))
            } else {
                Some(DeclKind::WebkitRegionBreakAfter(Some(v.to_string())))
            }
        }
        // Fase 7.683 — `-webkit-region-break-inside`. `auto` → None.
        "-webkit-region-break-inside" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitRegionBreakInside(None))
            } else {
                Some(DeclKind::WebkitRegionBreakInside(Some(v.to_string())))
            }
        }
        // Fase 7.698 — `-webkit-border-before-color`. `currentcolor` → None.
        "-webkit-border-before-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitBorderBeforeColor(None))
            } else {
                Some(DeclKind::WebkitBorderBeforeColor(Some(v.to_string())))
            }
        }
        // Fase 7.699 — `-webkit-border-before-style`. `none` → None.
        "-webkit-border-before-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderBeforeStyle(None))
            } else {
                Some(DeclKind::WebkitBorderBeforeStyle(Some(v.to_string())))
            }
        }
        // Fase 7.700 — `-webkit-border-before-width`. `medium` → None.
        "-webkit-border-before-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::WebkitBorderBeforeWidth(None))
            } else {
                Some(DeclKind::WebkitBorderBeforeWidth(Some(v.to_string())))
            }
        }
        // Fase 7.701 — `-webkit-border-after-color`. `currentcolor` → None.
        "-webkit-border-after-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitBorderAfterColor(None))
            } else {
                Some(DeclKind::WebkitBorderAfterColor(Some(v.to_string())))
            }
        }
        // Fase 7.702 — `-webkit-border-after-style`. `none` → None.
        "-webkit-border-after-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderAfterStyle(None))
            } else {
                Some(DeclKind::WebkitBorderAfterStyle(Some(v.to_string())))
            }
        }
        // Fase 7.703 — `-webkit-border-after-width`. `medium` → None.
        "-webkit-border-after-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::WebkitBorderAfterWidth(None))
            } else {
                Some(DeclKind::WebkitBorderAfterWidth(Some(v.to_string())))
            }
        }
        // Fase 7.704 — `-webkit-border-start-color`. `currentcolor` → None.
        "-webkit-border-start-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitBorderStartColor(None))
            } else {
                Some(DeclKind::WebkitBorderStartColor(Some(v.to_string())))
            }
        }
        // Fase 7.705 — `-webkit-border-start-style`. `none` → None.
        "-webkit-border-start-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderStartStyle(None))
            } else {
                Some(DeclKind::WebkitBorderStartStyle(Some(v.to_string())))
            }
        }
        // Fase 7.706 — `-webkit-border-start-width`. `medium` → None.
        "-webkit-border-start-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::WebkitBorderStartWidth(None))
            } else {
                Some(DeclKind::WebkitBorderStartWidth(Some(v.to_string())))
            }
        }
        // Fase 7.707 — `-webkit-border-end-color`. `currentcolor` → None.
        "-webkit-border-end-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitBorderEndColor(None))
            } else {
                Some(DeclKind::WebkitBorderEndColor(Some(v.to_string())))
            }
        }
        // Fase 7.708 — `-webkit-border-end-style`. `none` → None.
        "-webkit-border-end-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderEndStyle(None))
            } else {
                Some(DeclKind::WebkitBorderEndStyle(Some(v.to_string())))
            }
        }
        // Fase 7.709 — `-webkit-border-end-width`. `medium` → None.
        "-webkit-border-end-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::WebkitBorderEndWidth(None))
            } else {
                Some(DeclKind::WebkitBorderEndWidth(Some(v.to_string())))
            }
        }
        // Fase 7.730 — `-webkit-margin-top-collapse`. `collapse` → None.
        "-webkit-margin-top-collapse" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("collapse") {
                Some(DeclKind::WebkitMarginTopCollapse(None))
            } else {
                Some(DeclKind::WebkitMarginTopCollapse(Some(v.to_string())))
            }
        }
        // Fase 7.731 — `-webkit-margin-bottom-collapse`. `collapse` → None.
        "-webkit-margin-bottom-collapse" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("collapse") {
                Some(DeclKind::WebkitMarginBottomCollapse(None))
            } else {
                Some(DeclKind::WebkitMarginBottomCollapse(Some(v.to_string())))
            }
        }
        // Fase 7.732 — `-webkit-margin-collapse` (shorthand). `collapse` → None.
        "-webkit-margin-collapse" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("collapse") {
                Some(DeclKind::WebkitMarginCollapse(None))
            } else {
                Some(DeclKind::WebkitMarginCollapse(Some(v.to_string())))
            }
        }
        // Fase 7.733 — `-webkit-border-vertical-spacing`. `0` → None.
        "-webkit-border-vertical-spacing" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitBorderVerticalSpacing(None))
            } else {
                Some(DeclKind::WebkitBorderVerticalSpacing(Some(v.to_string())))
            }
        }
        // Fase 7.734 — `-webkit-mask-source-type`. `alpha` → None.
        "-webkit-mask-source-type" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("alpha") {
                Some(DeclKind::WebkitMaskSourceType(None))
            } else {
                Some(DeclKind::WebkitMaskSourceType(Some(v.to_string())))
            }
        }
        // Fase 7.750 — `-webkit-marquee-direction`. `auto` → None.
        "-webkit-marquee-direction" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitMarqueeDirection(None))
            } else {
                Some(DeclKind::WebkitMarqueeDirection(Some(v.to_string())))
            }
        }
        // Fase 7.751 — `-webkit-marquee-increment`. `6px` → None.
        "-webkit-marquee-increment" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("6px") {
                Some(DeclKind::WebkitMarqueeIncrement(None))
            } else {
                Some(DeclKind::WebkitMarqueeIncrement(Some(v.to_string())))
            }
        }
        // Fase 7.752 — `-webkit-marquee-repetition`. `infinite` → None.
        "-webkit-marquee-repetition" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("infinite") {
                Some(DeclKind::WebkitMarqueeRepetition(None))
            } else {
                Some(DeclKind::WebkitMarqueeRepetition(Some(v.to_string())))
            }
        }
        // Fase 7.753 — `-webkit-marquee-speed`. `normal` → None.
        "-webkit-marquee-speed" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::WebkitMarqueeSpeed(None))
            } else {
                Some(DeclKind::WebkitMarqueeSpeed(Some(v.to_string())))
            }
        }
        // Fase 7.754 — `-webkit-marquee-style`. `scroll` → None.
        "-webkit-marquee-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("scroll") {
                Some(DeclKind::WebkitMarqueeStyle(None))
            } else {
                Some(DeclKind::WebkitMarqueeStyle(Some(v.to_string())))
            }
        }
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
        // Fase 7.684-7.688 — la familia `-webkit-column-*` es el alias vendor
        // (de facto) de `column-*`: enruta al mismo parser/almacén.
        "column-count" | "-webkit-column-count" => {
            Some(DeclKind::ColumnCount(parse_column_count(value)))
        }
        "column-width" | "-webkit-column-width" => {
            parse_length_or_pct(value).map(DeclKind::ColumnWidth)
        }
        "column-rule-width" | "-webkit-column-rule-width" => {
            parse_length_px(value).map(DeclKind::ColumnRuleWidth)
        }
        "column-rule-color" | "-webkit-column-rule-color" => {
            if is_current_color(value) {
                Some(DeclKind::ColumnRuleColor(None))
            } else {
                parse_color(value).map(|c| DeclKind::ColumnRuleColor(Some(c)))
            }
        }
        // `column-rule-style` y `column-rule` van por `parse_declarations`.
        "column-fill" => parse_column_fill(value).map(DeclKind::ColumnFill),
        "column-span" | "-webkit-column-span" => {
            parse_column_span(value).map(DeclKind::ColumnSpan)
        }
        // `page-break-inside` (legacy CSS 2.1) = `break-inside` (subset).
        "break-inside" | "page-break-inside" => {
            parse_break_inside(value).map(DeclKind::BreakInside)
        }
        "table-layout" => parse_table_layout(value).map(DeclKind::TableLayout),
        "border-collapse" => parse_border_collapse(value).map(DeclKind::BorderCollapse),
        "border-spacing" => parse_border_spacing(value).map(|(h, v)| DeclKind::BorderSpacing { h, v }),
        // Fase 7.640 — `-epub-caption-side` (EPUB) alias de `caption-side`.
        "caption-side" | "-epub-caption-side" => {
            parse_caption_side(value).map(DeclKind::CaptionSide)
        }
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
        // Fase 7.748 — alias `-webkit-print-color-adjust` → estándar.
        "print-color-adjust" | "color-adjust" | "-webkit-print-color-adjust" => {
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
        // Fase 7.634-7.636 — la familia `-webkit-text-emphasis-*` es el alias
        // vendor (de facto) de `text-emphasis-*`: mismo parser/almacén.
        // Fase 7.642-7.643 — los `-epub-text-emphasis-{style,color}` (EPUB) al
        // mismo destino que los estándar/webkit.
        "text-emphasis-style" | "-webkit-text-emphasis-style"
        | "-epub-text-emphasis-style" => {
            parse_text_emphasis_style(value).map(DeclKind::TextEmphasisStyle)
        }
        "text-emphasis-color" | "-webkit-text-emphasis-color"
        | "-epub-text-emphasis-color" => {
            if is_current_color(value) {
                Some(DeclKind::TextEmphasisColor(None))
            } else {
                parse_color(value).map(|c| DeclKind::TextEmphasisColor(Some(c)))
            }
        }
        "text-emphasis-position" | "-webkit-text-emphasis-position" => {
            parse_text_emphasis_position(value).map(DeclKind::TextEmphasisPosition)
        }
        // `text-emphasis` shorthand: ver `parse_declarations`.
        // Fase 7.749 — alias `-webkit-ruby-position` → estándar.
        "ruby-position" | "-webkit-ruby-position" => parse_ruby_position(value).map(DeclKind::RubyPosition),
        // Fase 7.662 — `-webkit-transform-origin` alias vendor del shorthand.
        "transform-origin" | "-webkit-transform-origin" => {
            parse_transform_origin(value).map(DeclKind::TransformOrigin)
        }
        // Fase 7.740 — alias `-webkit-transform-style` → estándar.
        "transform-style" | "-webkit-transform-style" => {
            parse_transform_style(value).map(DeclKind::TransformStyle)
        }
        // Fase 7.741 — alias `-webkit-perspective` → estándar.
        "perspective" | "-webkit-perspective" => parse_perspective(value).map(DeclKind::Perspective),
        // Fase 7.663 — `-webkit-perspective-origin` alias vendor del shorthand.
        "perspective-origin" | "-webkit-perspective-origin" => {
            parse_perspective_origin(value).map(DeclKind::PerspectiveOrigin)
        }
        // Fase 7.742 — alias `-webkit-backface-visibility` → estándar.
        "backface-visibility" | "-webkit-backface-visibility" => {
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
        // Fase 7.631 — `-webkit-line-break` alias vendor de `line-break`.
        "line-break" | "-webkit-line-break" => {
            parse_line_break(value).map(DeclKind::LineBreak)
        }
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
        // Fase 7.693-7.697 — la familia `-webkit-mask-*` (longhands) es el
        // alias vendor (de facto) de `mask-*`: mismo parser/almacén.
        "mask-clip" | "-webkit-mask-clip" => parse_mask_clip(value).map(DeclKind::MaskClip),
        "mask-composite" => {
            parse_mask_composite(value).map(DeclKind::MaskComposite)
        }
        "mask-origin" | "-webkit-mask-origin" => {
            parse_mask_origin(value).map(DeclKind::MaskOrigin)
        }
        "mask-repeat" | "-webkit-mask-repeat" => {
            // Reusa `parse_background_repeat` (devuelve `DeclKind::BackgroundRepeat`);
            // extraemos el valor y lo reemitimos como `MaskRepeat`.
            match parse_background_repeat(value) {
                Some(DeclKind::BackgroundRepeat(r)) => {
                    Some(DeclKind::MaskRepeat(r))
                }
                _ => None,
            }
        }
        "mask-position" | "-webkit-mask-position" => match parse_background_position(value) {
            Some(DeclKind::BackgroundPosition(p)) => {
                Some(DeclKind::MaskPosition(p))
            }
            _ => None,
        },
        "mask-size" | "-webkit-mask-size" => match parse_background_size(value) {
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
        // Fase 7.722 — `-webkit-transform` alias vendor de `transform`.
        "transform" | "-webkit-transform" => {
            parse_transforms(value).map(DeclKind::Transforms)
        }
        "grid-template-columns" => {
            parse_grid_template(value).map(DeclKind::GridTemplateColumns)
        }
        "grid-template-rows" => parse_grid_template(value).map(DeclKind::GridTemplateRows),
        // Fase 7.723-7.724 — `-webkit-animation` / `-webkit-transition` alias
        // vendor de los shorthands `animation` / `transition`.
        "animation" | "-webkit-animation" => parse_animation(value),
        "transition" | "-webkit-transition" => parse_transition(value),
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
fn parse_block_step_piece(tok: &str) -> Option<DeclKind> {
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
/// Devuelve el primer item de una lista separada por coma. Si no hay
/// coma, devuelve el string completo. Espacios al borde recortados.
/// Fase 7.514+ (longhands animation que sólo guardan el primer item).
fn first_comma(s: &str) -> &str {
    match s.find(',') {
        Some(i) => s[..i].trim(),
        None => s.trim(),
    }
}

/// Parsea `<time>` CSS: `<n>s` o `<n>ms`. Devuelve segundos.
/// Fase 7.515.
fn parse_time_seconds(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix("ms") {
        return num.trim().parse::<f32>().ok().map(|n| n / 1000.0);
    }
    if let Some(num) = s.strip_suffix('s') {
        return num.trim().parse::<f32>().ok();
    }
    None
}

/// Parsea `<easing-function>` por keyword (sin cubic-bezier/steps por
/// ahora — un parser completo vive en parser/sheet.rs si lo necesitás).
/// Fase 7.516.
fn parse_easing_keyword(s: &str) -> Option<EasingFunction> {
    match s.trim().to_ascii_lowercase().as_str() {
        "linear" => Some(EasingFunction::Linear),
        "ease" => Some(EasingFunction::Ease),
        "ease-in" => Some(EasingFunction::EaseIn),
        "ease-out" => Some(EasingFunction::EaseOut),
        "ease-in-out" => Some(EasingFunction::EaseInOut),
        "step-start" => Some(EasingFunction::StepStart),
        "step-end" => Some(EasingFunction::StepEnd),
        _ => None,
    }
}

/// Parsea `image-resolution: [ from-image || <resolution> ] && snap?`.
/// Devuelve `Some(ImageResolution::FromImage)` cuando aparece sólo
/// `from-image` (con o sin `snap`). Resoluciones aceptadas: `<n>dppx`,
/// `<n>dpi`, `<n>dpcm`. Cualquier orden entre los tokens. CSS Images 4.
/// Fase 7.485.
pub(crate) fn parse_image_resolution(s: &str) -> Option<ImageResolution> {
    let lower = s.trim().to_ascii_lowercase();
    let mut from_image = false;
    let mut snap = false;
    let mut dppx: Option<f32> = None;
    for tok in lower.split_whitespace() {
        match tok {
            "from-image" => from_image = true,
            "snap" => snap = true,
            other => {
                if let Some(num) = other.strip_suffix("dppx") {
                    dppx = num.parse::<f32>().ok();
                } else if let Some(num) = other.strip_suffix("dpi") {
                    dppx = num.parse::<f32>().ok().map(|n| n / 96.0);
                } else if let Some(num) = other.strip_suffix("dpcm") {
                    dppx = num.parse::<f32>().ok().map(|n| n * 2.54 / 96.0);
                } else {
                    return None;
                }
            }
        }
    }
    match (from_image, dppx) {
        (true, None) => Some(ImageResolution::FromImage),
        (_, Some(d)) if d > 0.0 => Some(ImageResolution::Resolution { dppx: d, snap }),
        _ => None,
    }
}

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
