use super::*;

pub(crate) fn parse_weight(s: &str) -> Option<u16> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        "lighter" => Some(300),
        "bolder" => Some(700),
        num => num.parse().ok(),
    }
}

pub(crate) fn parse_font_style(s: &str) -> Option<FontStyle> {
    // CSS spec: normal | italic | oblique [<angle>?]. Tratamos oblique
    // como italic — parley/fontique sintetizan si la fuente no tiene
    // oblique nativo.
    let v = s.trim().to_ascii_lowercase();
    if v == "normal" {
        Some(FontStyle::Normal)
    } else if v == "italic" || v.starts_with("oblique") {
        Some(FontStyle::Italic)
    } else {
        None
    }
}

pub(crate) fn parse_display(s: &str) -> Option<Display> {
    match s.trim().to_ascii_lowercase().as_str() {
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "inline-block" => Some(Display::InlineBlock),
        // Fase 7.844 — alias vendor del flexbox viejo: `-webkit-flex`/`-webkit-box`
        // → flex; `-webkit-inline-flex` → inline-flex (aprox, sin la sintaxis
        // box-orient/box-flex de 2009).
        "flex" | "-webkit-flex" | "-webkit-box" => Some(Display::Flex),
        "inline-flex" | "-webkit-inline-flex" | "-webkit-inline-box" => {
            Some(Display::InlineFlex)
        }
        "grid" => Some(Display::Grid),
        "inline-grid" => Some(Display::InlineGrid),
        "none" => Some(Display::None),
        // Fase 7.877 — valores que el modelo no distingue (sin BFC explícito,
        // table layout, ni "sin caja"). Aproximaciones al variant más cercano:
        // `flow-root`/`list-item`/`contents` y la familia table → Block;
        // inline-table → InlineBlock. (`contents` debería no generar caja; sin
        // eso, Block mantiene los hijos visibles — divergencia documentada.)
        "flow-root" | "list-item" | "contents" | "table" | "table-row"
        | "table-cell" | "table-row-group" | "table-header-group"
        | "table-footer-group" | "table-column" | "table-column-group"
        | "table-caption" | "ruby" => Some(Display::Block),
        "inline-table" => Some(Display::InlineBlock),
        // Sintaxis de dos valores `<outside> <inside>` (CSS Display 3): el
        // `outside` decide el variant; `flow-root` interior con `inline` → BFC
        // inline ≈ inline-block.
        other if other.contains(' ') => {
            let parts: Vec<&str> = other.split_whitespace().collect();
            let inline = parts.contains(&"inline");
            let flow_root = parts.contains(&"flow-root");
            Some(match (inline, flow_root) {
                (true, true) => Display::InlineBlock,
                (true, false) => Display::Inline,
                (false, _) => Display::Block,
            })
        }
        _ => None,
    }
}

pub(crate) fn parse_flex_direction(s: &str) -> Option<FlexDirection> {
    match s.trim().to_ascii_lowercase().as_str() {
        "row" => Some(FlexDirection::Row),
        "row-reverse" => Some(FlexDirection::RowReverse),
        "column" => Some(FlexDirection::Column),
        "column-reverse" => Some(FlexDirection::ColumnReverse),
        _ => None,
    }
}

pub(crate) fn parse_flex_wrap(s: &str) -> Option<FlexWrap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
}

/// Quita los qualifiers de overflow-alignment (`safe`/`unsafe`, cuyo
/// comportamiento de seguridad no implementamos) y colapsa `first baseline`/
/// `last baseline` → `baseline`. Devuelve el keyword núcleo en minúsculas.
/// Fase 7.840 — lo aplican todos los parsers de align-*/justify-*.
fn normalize_alignment(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    if s == "first baseline" || s == "last baseline" {
        return "baseline".to_string();
    }
    if let Some(rest) = s.strip_prefix("safe ").or_else(|| s.strip_prefix("unsafe ")) {
        return rest.trim().to_string();
    }
    s
}

pub(crate) fn parse_justify_content(s: &str) -> Option<JustifyContent> {
    match normalize_alignment(s).as_str() {
        "start" | "flex-start" | "left" => Some(JustifyContent::Start),
        "center" => Some(JustifyContent::Center),
        "end" | "flex-end" | "right" => Some(JustifyContent::End),
        "space-between" => Some(JustifyContent::SpaceBetween),
        "space-around" => Some(JustifyContent::SpaceAround),
        "space-evenly" => Some(JustifyContent::SpaceEvenly),
        _ => None,
    }
}

pub(crate) fn parse_align_items(s: &str) -> Option<AlignItems> {
    match normalize_alignment(s).as_str() {
        "start" | "flex-start" => Some(AlignItems::Start),
        "center" => Some(AlignItems::Center),
        "end" | "flex-end" => Some(AlignItems::End),
        "stretch" => Some(AlignItems::Stretch),
        "baseline" => Some(AlignItems::Baseline),
        _ => None,
    }
}

/// `align-content`. `normal` y `baseline` colapsan a `Normal` (default de
/// taffy ≈ stretch); el resto mapea directo. `start`/`end` aceptan también
/// la variante `flex-*`.
pub(crate) fn parse_align_content(s: &str) -> Option<AlignContent> {
    match normalize_alignment(s).as_str() {
        "normal" | "baseline" => Some(AlignContent::Normal),
        "start" | "flex-start" => Some(AlignContent::Start),
        "center" => Some(AlignContent::Center),
        "end" | "flex-end" => Some(AlignContent::End),
        "stretch" => Some(AlignContent::Stretch),
        "space-between" => Some(AlignContent::SpaceBetween),
        "space-around" => Some(AlignContent::SpaceAround),
        "space-evenly" => Some(AlignContent::SpaceEvenly),
        _ => None,
    }
}

/// `justify-items` (grid). Reusa el subset de `align-items` y agrega
/// `left`/`right` (que en escritura LTR equivalen a start/end). `normal`
/// se descarta → queda el default None. `auto`/`legacy` también.
pub(crate) fn parse_justify_items(s: &str) -> Option<AlignItems> {
    match normalize_alignment(s).as_str() {
        "left" => Some(AlignItems::Start),
        "right" => Some(AlignItems::End),
        other => parse_align_items(other),
    }
}

/// `justify-self` (grid item). Reusa `align-self` + `left`/`right`.
pub(crate) fn parse_justify_self(s: &str) -> Option<AlignSelf> {
    match normalize_alignment(s).as_str() {
        "left" => Some(AlignSelf::Start),
        "right" => Some(AlignSelf::End),
        other => parse_align_self(other),
    }
}

/// `place-content: <align-content> [<justify-content>]`. Un solo valor
/// setea ambos ejes. Cada mitad se valida con su parser propio; las que no
/// parsean se descartan (el otro eje igual se aplica).
pub(crate) fn parse_place_content_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut it = value.split_whitespace();
    let Some(a) = it.next() else { return out };
    let b = it.next().unwrap_or(a);
    if let Some(ac) = parse_align_content(a) {
        out.push(Decl { kind: DeclKind::AlignContent(ac), important });
    }
    if let Some(jc) = parse_justify_content(b) {
        out.push(Decl { kind: DeclKind::JustifyContent(jc), important });
    }
    out
}

/// `place-items: <align-items> [<justify-items>]`. Un solo valor = ambos.
pub(crate) fn parse_place_items_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut it = value.split_whitespace();
    let Some(a) = it.next() else { return out };
    let b = it.next().unwrap_or(a);
    if let Some(ai) = parse_align_items(a) {
        out.push(Decl { kind: DeclKind::AlignItems(ai), important });
    }
    if let Some(ji) = parse_justify_items(b) {
        out.push(Decl { kind: DeclKind::JustifyItems(ji), important });
    }
    out
}

/// `place-self: <align-self> [<justify-self>]`. Un solo valor = ambos.
pub(crate) fn parse_place_self_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut it = value.split_whitespace();
    let Some(a) = it.next() else { return out };
    let b = it.next().unwrap_or(a);
    if let Some(asf) = parse_align_self(a) {
        out.push(Decl { kind: DeclKind::AlignSelf(asf), important });
    }
    if let Some(jsf) = parse_justify_self(b) {
        out.push(Decl { kind: DeclKind::JustifySelf(jsf), important });
    }
    out
}

/// `gap: V` ⇒ row=V, column=V. `gap: R C` ⇒ row=R, column=C. Coincide
/// con la semántica CSS shorthand (primer valor = row, segundo = column).
pub(crate) fn parse_gap(value: &str) -> Option<(f32, f32)> {
    // Fase 7.835 — `normal` (valor inicial de row/column-gap) → 0 en nuestro
    // modelo (sin gap). Cada token puede ser `normal` o una length.
    fn gap_token(s: &str) -> Option<f32> {
        if s.eq_ignore_ascii_case("normal") {
            Some(0.0)
        } else {
            // Fase 7.853 — acepta `calc()`/`min()`/`max()`/`clamp()`; el split
            // top-level respeta los espacios internos de la función.
            parse_length_px_or_calc(s)
        }
    }
    let parts = split_top_level_ws(value);
    match parts.as_slice() {
        [v] => {
            let v = gap_token(v)?;
            Some((v, v))
        }
        [r, c] => Some((gap_token(r)?, gap_token(c)?)),
        _ => None,
    }
}

pub(crate) fn parse_box_sizing(s: &str) -> Option<BoxSizing> {
    match s.trim().to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

pub(crate) fn parse_overflow(s: &str) -> Option<Overflow> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Overflow::Visible),
        // hidden/clip/auto/scroll todos los tratamos como Hidden por
        // ahora (no soportamos scroll real; clip y hidden cortan igual).
        // Fase 7.833 — `overlay` (alias legacy de `auto`) → Hidden.
        "hidden" | "clip" | "auto" | "scroll" | "overlay" => Some(Overflow::Hidden),
        _ => None,
    }
}

pub(crate) fn parse_white_space(s: &str) -> Option<WhiteSpace> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpace::Normal),
        "nowrap" => Some(WhiteSpace::NoWrap),
        "pre" => Some(WhiteSpace::Pre),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "pre-line" => Some(WhiteSpace::PreLine),
        // Fase 7.843 — `break-spaces` ≈ pre-wrap (difiere sólo en el manejo de
        // los espacios al final de línea, que no modelamos): aproximación.
        "break-spaces" => Some(WhiteSpace::PreWrap),
        _ => None,
    }
}

pub(crate) fn parse_text_transform(s: &str) -> Option<TextTransform> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        "capitalize" => Some(TextTransform::Capitalize),
        // Fase 7.873 — `full-width`/`full-size-kana`/`math-auto` transforman
        // ancho/forma de glifos, no el caso; el shaper no lo aplica, así que
        // colapsan a `None` (no-op) en vez de descartar la declaración.
        "full-width" | "full-size-kana" | "math-auto" => Some(TextTransform::None),
        _ => None,
    }
}

/// Acepta `0..1` o `0%..100%`. Clampa.
pub(crate) fn parse_opacity(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct / 100.0).clamp(0.0, 1.0));
    }
    // Fase 7.872 — acepta `calc()`/min/max/clamp que resuelva a número.
    parse_number_or_calc(s).map(|v| v.clamp(0.0, 1.0))
}

pub(crate) fn parse_align_self(s: &str) -> Option<AlignSelf> {
    match normalize_alignment(s).as_str() {
        "auto" => Some(AlignSelf::Auto),
        "start" | "flex-start" => Some(AlignSelf::Start),
        "center" => Some(AlignSelf::Center),
        "end" | "flex-end" => Some(AlignSelf::End),
        "stretch" => Some(AlignSelf::Stretch),
        "baseline" => Some(AlignSelf::Baseline),
        _ => None,
    }
}

/// `flex: <grow> [<shrink>] [<basis>]`. Casos especiales:
/// - `flex: none` → `0 0 auto`
/// - `flex: auto` → `1 1 auto`
/// - `flex: <number>` → `N 1 0%` (basis 0%, common preset)
/// Devuelve 3 decls atómicas (grow + shrink + basis).
/// Propiedades lógicas de caja (`margin-inline`/`margin-block`/`padding-*` y
/// sus `-start`/`-end`), mapeadas a las físicas asumiendo LTR + escritura
/// horizontal (el caso por defecto). `inline` ↔ left/right, `block` ↔
/// top/bottom; `start`=left/top, `end`=right/bottom. Las dos-lados aceptan
/// 1–2 valores (`margin-inline: 10px` o `10px 20px`). Devuelve `None` si el
/// nombre no es una propiedad lógica conocida. Fase 7.191.
pub(crate) fn parse_logical_box(prop: &str, value: &str, important: bool) -> Option<Vec<Decl>> {
    use DeclKind::{
        MarginBottom, MarginLeft, MarginRight, MarginTop, PaddingBottom, PaddingLeft,
        PaddingRight, PaddingTop,
    };
    use DeclKind::{InsetBottom, InsetLeft, InsetRight, InsetTop};
    let lower = prop.to_ascii_lowercase();
    // `inset-inline`/`inset-block` y sus `-start`/`-end`: usan `LengthVal`
    // (length/%/auto), no `f32` como margin/padding — firma aparte.
    let inset_two: Option<(fn(LengthVal) -> DeclKind, fn(LengthVal) -> DeclKind)> =
        match lower.as_str() {
            "inset-inline" => Some((InsetLeft, InsetRight)),
            "inset-block" => Some((InsetTop, InsetBottom)),
            _ => None,
        };
    if let Some((a, b)) = inset_two {
        let parts: Vec<&str> = value.split_whitespace().collect();
        let vals: Vec<LengthVal> =
            parts.iter().filter_map(|p| parse_length_or_pct_or_auto(p)).collect();
        if vals.is_empty() || vals.len() != parts.len() {
            return Some(Vec::new());
        }
        let (s, e) = if vals.len() == 1 { (vals[0], vals[0]) } else { (vals[0], vals[1]) };
        return Some(vec![
            Decl { kind: a(s), important },
            Decl { kind: b(e), important },
        ]);
    }
    let inset_single: Option<fn(LengthVal) -> DeclKind> = match lower.as_str() {
        "inset-inline-start" => Some(InsetLeft),
        "inset-inline-end" => Some(InsetRight),
        "inset-block-start" => Some(InsetTop),
        "inset-block-end" => Some(InsetBottom),
        _ => None,
    };
    if let Some(ctor) = inset_single {
        return Some(
            parse_length_or_pct_or_auto(value)
                .map(|v| vec![Decl { kind: ctor(v), important }])
                .unwrap_or_default(),
        );
    }
    // Fase 7.857 — `margin-*` lógicos con `auto`. El eje inline (left/right)
    // mapea a los flags de centrado (igual que los longhands físicos en
    // `parse_declarations`); el eje block (top/bottom) no centra → 0. Se
    // resuelve acá antes del camino numérico genérico (que descarta `auto`).
    if let Some(decls) = parse_logical_margin_auto(&lower, value, important) {
        return Some(decls);
    }
    // Lados emparejados (1–2 valores): (start_ctor, end_ctor).
    let two: Option<(fn(f32) -> DeclKind, fn(f32) -> DeclKind)> = match lower.as_str() {
        "margin-inline" => Some((MarginLeft, MarginRight)),
        "margin-block" => Some((MarginTop, MarginBottom)),
        "padding-inline" => Some((PaddingLeft, PaddingRight)),
        "padding-block" => Some((PaddingTop, PaddingBottom)),
        _ => None,
    };
    if let Some((a, b)) = two {
        let parts: Vec<&str> = value.split_whitespace().collect();
        let vals: Vec<f32> = parts.iter().filter_map(|p| parse_length_px(p)).collect();
        if vals.is_empty() || vals.len() != parts.len() {
            return Some(Vec::new());
        }
        let (s, e) = if vals.len() == 1 { (vals[0], vals[0]) } else { (vals[0], vals[1]) };
        return Some(vec![
            Decl { kind: a(s), important },
            Decl { kind: b(e), important },
        ]);
    }
    // Un solo lado (`-start`/`-end`).
    let single: Option<fn(f32) -> DeclKind> = match lower.as_str() {
        "margin-inline-start" => Some(MarginLeft),
        "margin-inline-end" => Some(MarginRight),
        "margin-block-start" => Some(MarginTop),
        "margin-block-end" => Some(MarginBottom),
        "padding-inline-start" => Some(PaddingLeft),
        "padding-inline-end" => Some(PaddingRight),
        "padding-block-start" => Some(PaddingTop),
        "padding-block-end" => Some(PaddingBottom),
        _ => None,
    };
    let ctor = single?;
    Some(
        parse_length_px(value)
            .map(|v| vec![Decl { kind: ctor(v), important }])
            .unwrap_or_default(),
    )
}

/// Lado físico que un margen lógico edita, con su naturaleza de centrado.
enum MarginSide {
    Left,
    Right,
    /// Eje block (top/bottom): `auto` → 0, no centra.
    Block(bool), // true = bottom
}

/// Maneja `margin-*` lógicos cuando el valor incluye `auto`. Devuelve `None`
/// si la prop no es un margen lógico o no contiene ningún `auto` (→ el camino
/// numérico genérico la resuelve). Fase 7.857.
fn parse_logical_margin_auto(lower: &str, value: &str, important: bool) -> Option<Vec<Decl>> {
    use DeclKind::{
        MarginBottom, MarginLeft, MarginLeftAuto, MarginRight, MarginRightAuto, MarginTop,
    };
    // (lado_start, lado_end?) — `None` en end = prop de un solo lado.
    let sides: (MarginSide, Option<MarginSide>) = match lower {
        "margin-inline" => (MarginSide::Left, Some(MarginSide::Right)),
        "margin-block" => (MarginSide::Block(false), Some(MarginSide::Block(true))),
        "margin-inline-start" => (MarginSide::Left, None),
        "margin-inline-end" => (MarginSide::Right, None),
        "margin-block-start" => (MarginSide::Block(false), None),
        "margin-block-end" => (MarginSide::Block(true), None),
        _ => return None,
    };
    let parts: Vec<&str> = value.split_whitespace().collect();
    let has_auto = parts.iter().any(|p| p.eq_ignore_ascii_case("auto"));
    if !has_auto {
        return None; // sin `auto` → camino numérico genérico.
    }
    // Un solo lado no admite 2 valores; pares admiten 1 (replica) ó 2.
    let (start_tok, end_tok) = match (parts.as_slice(), &sides.1) {
        ([s], _) => (*s, *s),
        ([s, e], Some(_)) => (*s, *e),
        _ => return Some(Vec::new()), // forma inválida → descartar
    };
    let emit = |side: &MarginSide, tok: &str, out: &mut Vec<Decl>| -> bool {
        let is_auto = tok.eq_ignore_ascii_case("auto");
        match side {
            MarginSide::Left if is_auto => {
                out.push(Decl { kind: MarginLeft(0.0), important });
                out.push(Decl { kind: MarginLeftAuto(true), important });
            }
            MarginSide::Right if is_auto => {
                out.push(Decl { kind: MarginRight(0.0), important });
                out.push(Decl { kind: MarginRightAuto(true), important });
            }
            MarginSide::Block(_) if is_auto => {
                let ctor = if matches!(side, MarginSide::Block(true)) { MarginBottom } else { MarginTop };
                out.push(Decl { kind: ctor(0.0), important });
            }
            _ => {
                // Token de longitud: parsea o señala fallo.
                let Some(px) = parse_length_px_or_calc(tok) else { return false };
                let kind = match side {
                    MarginSide::Left => MarginLeft(px),
                    MarginSide::Right => MarginRight(px),
                    MarginSide::Block(true) => MarginBottom(px),
                    MarginSide::Block(false) => MarginTop(px),
                };
                out.push(Decl { kind, important });
            }
        }
        true
    };
    let mut out = Vec::new();
    if !emit(&sides.0, start_tok, &mut out) {
        return Some(Vec::new());
    }
    if let Some(end) = &sides.1 {
        if !emit(end, end_tok, &mut out) {
            return Some(Vec::new());
        }
    }
    Some(out)
}

/// `inset: <t> [r] [b] [l]` — 1..4 valores con la distribución de `margin`
/// (1→todos, 2→TB/LR, 3→T/LR/B, 4→TRBL). Cada valor acepta length/%/auto.
/// Expande a los cuatro longhands `top`/`right`/`bottom`/`left`. Fase 7.189.
pub(crate) fn parse_inset_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let vals: Vec<LengthVal> =
        parts.iter().filter_map(|p| parse_length_or_pct_or_auto(p)).collect();
    // Si algún token no parsea, descartamos el shorthand entero (CSS spec).
    if vals.is_empty() || vals.len() != parts.len() {
        return Vec::new();
    }
    let (t, r, b, l) = match vals.as_slice() {
        [a] => (*a, *a, *a, *a),
        [a, b2] => (*a, *b2, *a, *b2),
        [a, b2, c] => (*a, *b2, *c, *b2),
        [a, b2, c, d, ..] => (*a, *b2, *c, *d),
        [] => return Vec::new(),
    };
    vec![
        Decl { kind: DeclKind::InsetTop(t), important },
        Decl { kind: DeclKind::InsetRight(r), important },
        Decl { kind: DeclKind::InsetBottom(b), important },
        Decl { kind: DeclKind::InsetLeft(l), important },
    ]
}

/// `flex-flow: <direction> || <wrap>` (en cualquier orden) → `flex-direction`
/// + `flex-wrap`. Fase 7.189.
pub(crate) fn parse_flex_flow_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    for tok in value.split_whitespace() {
        if let Some(d) = parse_flex_direction(tok) {
            out.push(Decl { kind: DeclKind::FlexDirection(d), important });
        } else if let Some(w) = parse_flex_wrap(tok) {
            out.push(Decl { kind: DeclKind::FlexWrap(w), important });
        }
    }
    out
}

pub(crate) fn parse_flex_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let v = value.trim().to_ascii_lowercase();
    let (grow, shrink, basis) = if v == "none" {
        (0.0_f32, 0.0_f32, LengthVal::Auto)
    } else if v == "auto" {
        (1.0_f32, 1.0_f32, LengthVal::Auto)
    } else if v == "initial" {
        (0.0_f32, 1.0_f32, LengthVal::Auto)
    } else {
        let parts: Vec<&str> = value.split_whitespace().collect();
        match parts.as_slice() {
            [g] => {
                // `flex: 1` ⇒ `1 1 0%`
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                (g, 1.0, LengthVal::Pct(0.0))
            }
            [g, s_or_b] => {
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                // El segundo puede ser shrink (número solo) o basis (longitud).
                if let Some(b) = parse_length_or_pct(s_or_b) {
                    (g, 1.0, b)
                } else if let Some(s) = s_or_b.parse::<f32>().ok() {
                    (g, s, LengthVal::Pct(0.0))
                } else {
                    return Vec::new();
                }
            }
            [g, s, b] => {
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                let Some(s) = s.parse::<f32>().ok() else {
                    return Vec::new();
                };
                let Some(b) = parse_length_or_pct(b) else {
                    return Vec::new();
                };
                (g, s, b)
            }
            _ => return Vec::new(),
        }
    };
    vec![
        Decl { kind: DeclKind::FlexGrow(grow), important },
        Decl { kind: DeclKind::FlexShrink(shrink), important },
        Decl { kind: DeclKind::FlexBasis(basis), important },
    ]
}

/// `outline: <width> <style> <color>`. Tokens en cualquier orden.
pub(crate) fn parse_outline_shorthand(value: &str, important: bool) -> Vec<Decl> {
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
        // `outline-style: none` apaga: width=0 + color=None.
        out.push(Decl { kind: DeclKind::OutlineStyle(false), important });
        return out;
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::OutlineWidth(w), important });
    }
    if current {
        out.push(Decl { kind: DeclKind::CurrentColor(ColorTarget::Outline), important });
    } else if let Some(c) = color {
        out.push(Decl { kind: DeclKind::OutlineColor(c), important });
    }
    if style_active.is_some() {
        out.push(Decl { kind: DeclKind::OutlineStyle(true), important });
    }
    if let Some(ls) = line_style {
        out.push(Decl { kind: DeclKind::OutlineStylePattern(ls), important });
    }
    out
}

/// Shorthand `margin:` con soporte de `auto` por lado (centrado). Emite
/// longhands px (auto→0) más los flags `MarginLeftAuto`/`MarginRightAuto`.
/// El `auto` vertical no centra en block flow: se trata como 0. Vacío si
/// algún token no parsea como px ni `auto`.
pub(crate) fn parse_margin_shorthand(value: &str, important: bool) -> Vec<Decl> {
    // Fase 7.847 — tokeniza respetando paréntesis para no partir `calc(…)`.
    let toks = split_top_level_ws(value);
    if toks.is_empty() || toks.len() > 4 {
        return Vec::new();
    }
    // Cada token → (px, es_auto).
    let mut sides: Vec<(f32, bool)> = Vec::with_capacity(toks.len());
    for t in &toks {
        if t.eq_ignore_ascii_case("auto") {
            sides.push((0.0, true));
        } else if let Some(px) = parse_length_px_or_calc(t) {
            sides.push((px, false));
        } else {
            return Vec::new();
        }
    }
    // Expande 1/2/3/4 valores a (top, right, bottom, left).
    let (t, r, b, l) = match sides.as_slice() {
        [a] => (*a, *a, *a, *a),
        [v, h] => (*v, *h, *v, *h),
        [t, h, bo] => (*t, *h, *bo, *h),
        [t, r, bo, le] => (*t, *r, *bo, *le),
        _ => return Vec::new(),
    };
    // Los longhands px limpian el flag auto; los flags van DESPUÉS para
    // que el orden de aplicación deje el auto en pie cuando corresponde.
    vec![
        Decl { kind: DeclKind::MarginTop(t.0), important },
        Decl { kind: DeclKind::MarginRight(r.0), important },
        Decl { kind: DeclKind::MarginBottom(b.0), important },
        Decl { kind: DeclKind::MarginLeft(l.0), important },
        Decl { kind: DeclKind::MarginLeftAuto(l.1), important },
        Decl { kind: DeclKind::MarginRightAuto(r.1), important },
    ]
}

/// Shorthand `font:` — expande a font-style / font-weight / font-size /
/// line-height / font-family. Sintaxis CSS:
///   font: [ <style> || <variant> || <weight> || <stretch> ]?
///         <size> [ / <line-height> ]? <family>
/// `size` y `family` son obligatorios; las palabras de fuente de sistema
/// (`caption`/`menu`/…) no se soportan y devuelven vacío. `font-variant`
/// y `font-stretch` se reconocen para no romper el parseo pero se ignoran
/// (no tenemos esos ejes todavía).
pub(crate) fn parse_font_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let v = value.trim();
    // Fase 7.863 — palabras de fuente de sistema (`caption`/`menu`/…). Sin un
    // tema de UA que aporte la familia/tamaño reales, aplicamos el efecto más
    // útil: fijar el `font-size` al de una fuente de UI estándar (13px, o 11px
    // para `small-caption`). La familia queda en el default del runtime.
    match v.to_ascii_lowercase().as_str() {
        "small-caption" => return vec![Decl { kind: DeclKind::FontSize(11.0), important }],
        "caption" | "icon" | "menu" | "message-box" | "status-bar" => {
            return vec![Decl { kind: DeclKind::FontSize(13.0), important }];
        }
        _ => {}
    }
    // El `/` separa size de line-height (`16px/1.5` o `16px / 1.5`). Lo
    // rodeamos de espacios para tokenizar uniforme; font-family no usa `/`.
    let spaced = v.replace('/', " / ");
    let mut words = spaced.split_whitespace().peekable();

    let mut style: Option<FontStyle> = None;
    let mut weight: Option<u16> = None;
    let mut size: Option<f32> = None;

    // Prefijo: style || variant || weight || stretch en cualquier orden,
    // hasta toparnos con el token de tamaño. Los pesos numéricos (100..900)
    // se reconocen ANTES de probar el tamaño para no confundir `300` con un
    // `font-size` crudo.
    while let Some(&w) = words.peek() {
        if w == "/" {
            break;
        }
        let wl = w.to_ascii_lowercase();
        match wl.as_str() {
            // `normal` aplica a style/variant/weight: no cambia nada.
            "normal" => {}
            "italic" | "oblique" => style = Some(FontStyle::Italic),
            "bold" | "bolder" => weight = Some(700),
            "lighter" => weight = Some(300),
            "100" | "200" | "300" | "400" | "500" | "600" | "700" | "800" | "900" => {
                weight = wl.parse().ok();
            }
            // font-variant / font-stretch keywords: reconocidos pero ignorados.
            "small-caps" | "all-small-caps" | "ultra-condensed" | "extra-condensed"
            | "condensed" | "semi-condensed" | "semi-expanded" | "expanded"
            | "extra-expanded" | "ultra-expanded" => {}
            // No es keyword de prefijo → debe ser el tamaño (px/em/rem/calc…).
            _ => {
                if let Some(px) = parse_px_or_math(w) {
                    size = Some(px);
                    words.next();
                }
                break;
            }
        }
        words.next();
    }

    let Some(fs) = size else {
        return Vec::new();
    };
    let mut out = vec![Decl { kind: DeclKind::FontSize(fs), important }];
    if let Some(s) = style {
        out.push(Decl { kind: DeclKind::FontStyle(s), important });
    }
    if let Some(w) = weight {
        out.push(Decl { kind: DeclKind::FontWeight(w), important });
    }

    // line-height opcional tras `/`.
    if words.peek() == Some(&"/") {
        words.next();
        if let Some(lh_word) = words.next() {
            if let Some(lh) = parse_line_height(lh_word) {
                out.push(Decl { kind: DeclKind::LineHeight(lh), important });
            }
        }
    }

    // Resto = font-family (obligatoria; puede traer varias familias por coma).
    let family: Vec<&str> = words.collect();
    if !family.is_empty() {
        out.push(Decl { kind: DeclKind::FontFamily(family.join(" ")), important });
    }
    out
}
