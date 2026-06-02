//! Value-parsers de propiedades: `parse_color` (público) + funciones de color,
//! parsers de enums (display/flex/align/...), longitudes, gradientes, transforms,
//! grid, sombras de texto, y `evaluate_media_query` (público) + supports. Sub-
//! módulo de `parser` (regla #1). `use super::*`.
use super::*;

/// Parsea un color CSS (`#rgb`/`#rrggbb`/`#rrggbbaa`, `rgb()`/`rgba()`,
/// `hsl()`/`hsla()`, named colors) a [`Color`]. Público para que el chrome
/// pinte `fillStyle`/`strokeStyle` de canvas (Fase 7.196). `None` si no
/// parsea.
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    // hex #RRGGBB / #RGB / #RRGGBBAA / #RGBA
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::rgb(r, g, b));
        }
        if hex.len() == 3 {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            return Some(Color::rgb(r, g, b));
        }
        if hex.len() == 8 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            return Some(Color { r, g, b, a });
        }
        if hex.len() == 4 {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
            return Some(Color { r, g, b, a });
        }
    }
    // rgb()/rgba() — coma legacy o whitespace moderno, con alpha por
    // 4to arg o sufijo `/ alpha`.
    if let Some(args) = strip_fn(s, "rgba").or_else(|| strip_fn(s, "rgb")) {
        return parse_rgb_func(args);
    }
    if let Some(args) = strip_fn(s, "hsla").or_else(|| strip_fn(s, "hsl")) {
        return parse_hsl_func(args);
    }
    // Nombres comunes.
    NAMED_COLORS.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, c)| *c)
}

/// Si `s` es de la forma `name(…)`, devuelve los argumentos crudos
/// (sin paréntesis). Tolera espacios entre el nombre y `(`. Match del
/// nombre case-insensitive.
pub(crate) fn strip_fn<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let s = s.trim();
    if !s.get(..name.len())?.eq_ignore_ascii_case(name) {
        return None;
    }
    let rest = s[name.len()..].trim_start();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.trim())
}

/// Parsea los argumentos de `rgb(…)` o `rgba(…)`. Acepta sintaxis
/// legacy (separador coma, alpha como 4to arg) y moderna (whitespace
/// + `/ alpha`). Cada canal RGB tolera entero 0-255 o porcentaje. El
/// alpha tolera fracción 0-1 o porcentaje.
pub(crate) fn parse_rgb_func(args: &str) -> Option<Color> {
    let (rgb, alpha) = split_color_args(args)?;
    if rgb.len() != 3 {
        return None;
    }
    let r = parse_color_chan(rgb[0])?;
    let g = parse_color_chan(rgb[1])?;
    let b = parse_color_chan(rgb[2])?;
    let a = match alpha {
        Some(a_str) => parse_alpha(a_str)?,
        None => 255,
    };
    Some(Color { r, g, b, a })
}

/// Parsea `hsl(…)` / `hsla(…)`. H = grados (0-360, se wrappea), S/L =
/// porcentaje (0-100). Alpha igual que rgba.
pub(crate) fn parse_hsl_func(args: &str) -> Option<Color> {
    let (parts, alpha) = split_color_args(args)?;
    if parts.len() != 3 {
        return None;
    }
    let h = parse_hue(parts[0])?;
    let s = parse_pct(parts[1])?;
    let l = parse_pct(parts[2])?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    let a = match alpha {
        Some(a_str) => parse_alpha(a_str)?,
        None => 255,
    };
    Some(Color { r, g, b, a })
}

/// Tokeniza los args de un color function. Devuelve `(canales, alpha?)`.
/// Resuelve coma vs whitespace y la sintaxis moderna `r g b / a`.
pub(crate) fn split_color_args(args: &str) -> Option<(Vec<&str>, Option<&str>)> {
    let args = args.trim();
    // Sintaxis moderna: `R G B / A`. La barra separa el alpha.
    if let Some(slash) = args.find('/') {
        let main = args[..slash].trim();
        let alpha = args[slash + 1..].trim();
        let parts: Vec<&str> = main.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }
        return Some((parts, Some(alpha)));
    }
    // Legacy: comas separan TODO (incluido el alpha).
    if args.contains(',') {
        let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
        if parts.len() == 4 {
            let (rgb, a) = parts.split_at(3);
            return Some((rgb.to_vec(), Some(a[0])));
        }
        return Some((parts, None));
    }
    // Moderna sin alpha: solo whitespace.
    let parts: Vec<&str> = args.split_whitespace().collect();
    Some((parts, None))
}

/// Canal RGB: entero 0-255 o porcentaje 0%-100%.
pub(crate) fn parse_color_chan(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct.clamp(0.0, 100.0) * 2.55).round() as u8);
    }
    s.parse::<i32>().ok().map(|n| n.clamp(0, 255) as u8)
}

/// Alpha: fracción 0.0-1.0 o porcentaje 0%-100%.
pub(crate) fn parse_alpha(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct.clamp(0.0, 100.0) * 2.55).round() as u8);
    }
    let f: f32 = s.parse().ok()?;
    Some((f.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Hue: `Ndeg` o número crudo (grados implícitos). `Nrad`/`Nturn` no
/// soportados — caen a `None` y la función devuelve `None`.
pub(crate) fn parse_hue(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix("deg").unwrap_or(s);
    s.trim().parse().ok()
}

/// Porcentaje 0%-100% → fracción 0.0-1.0.
pub(crate) fn parse_pct(s: &str) -> Option<f32> {
    let s = s.trim().strip_suffix('%')?;
    let pct: f32 = s.trim().parse().ok()?;
    Some((pct / 100.0).clamp(0.0, 1.0))
}

/// HSL→RGB estándar (CSS Color Module L3). h en grados, s/l en 0..1.
pub(crate) fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h_prime.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_u8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r1), to_u8(g1), to_u8(b1))
}

/// Parsea un value tipo `margin: <1..4 longitudes>`. Devuelve `None` si
/// algún token no es longitud válida o si hay menos de 1 / más de 4.
pub(crate) fn parse_sides(value: &str) -> Option<Sides<f32>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let parsed: Vec<f32> = parts
        .iter()
        .map(|t| parse_length_px(t))
        .collect::<Option<Vec<_>>>()?;
    Some(match parsed.as_slice() {
        [a] => Sides::all(*a),
        [v, h] => Sides { top: *v, right: *h, bottom: *v, left: *h },
        [t, h, b] => Sides { top: *t, right: *h, bottom: *b, left: *h },
        [t, r, b, l] => Sides { top: *t, right: *r, bottom: *b, left: *l },
        _ => return None,
    })
}

const NAMED_COLORS: &[(&str, Color)] = &[
    ("black", Color::BLACK),
    ("white", Color::WHITE),
    ("red", Color::rgb_const(255, 0, 0)),
    ("green", Color::rgb_const(0, 128, 0)),
    ("blue", Color::rgb_const(0, 0, 255)),
    ("gray", Color::rgb_const(128, 128, 128)),
    ("grey", Color::rgb_const(128, 128, 128)),
    ("silver", Color::rgb_const(192, 192, 192)),
    ("maroon", Color::rgb_const(128, 0, 0)),
    ("yellow", Color::rgb_const(255, 255, 0)),
    ("olive", Color::rgb_const(128, 128, 0)),
    ("lime", Color::rgb_const(0, 255, 0)),
    ("aqua", Color::rgb_const(0, 255, 255)),
    ("cyan", Color::rgb_const(0, 255, 255)),
    ("teal", Color::rgb_const(0, 128, 128)),
    ("navy", Color::rgb_const(0, 0, 128)),
    ("fuchsia", Color::rgb_const(255, 0, 255)),
    ("magenta", Color::rgb_const(255, 0, 255)),
    ("purple", Color::rgb_const(128, 0, 128)),
    ("orange", Color::rgb_const(255, 165, 0)),
    ("pink", Color::rgb_const(255, 192, 203)),
    ("brown", Color::rgb_const(165, 42, 42)),
    ("gold", Color::rgb_const(255, 215, 0)),
    ("indigo", Color::rgb_const(75, 0, 130)),
    ("violet", Color::rgb_const(238, 130, 238)),
    ("crimson", Color::rgb_const(220, 20, 60)),
    ("darkblue", Color::rgb_const(0, 0, 139)),
    ("darkgreen", Color::rgb_const(0, 100, 0)),
    ("darkred", Color::rgb_const(139, 0, 0)),
    ("darkgray", Color::rgb_const(169, 169, 169)),
    ("lightgray", Color::rgb_const(211, 211, 211)),
    ("lightblue", Color::rgb_const(173, 216, 230)),
    ("lightgreen", Color::rgb_const(144, 238, 144)),
    ("transparent", Color::TRANSPARENT),
];

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
        "flex" => Some(Display::Flex),
        "inline-flex" => Some(Display::InlineFlex),
        "grid" => Some(Display::Grid),
        "inline-grid" => Some(Display::InlineGrid),
        "none" => Some(Display::None),
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

pub(crate) fn parse_justify_content(s: &str) -> Option<JustifyContent> {
    match s.trim().to_ascii_lowercase().as_str() {
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
    match s.trim().to_ascii_lowercase().as_str() {
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
    match s.trim().to_ascii_lowercase().as_str() {
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
    match s.trim().to_ascii_lowercase().as_str() {
        "left" => Some(AlignItems::Start),
        "right" => Some(AlignItems::End),
        other => parse_align_items(other),
    }
}

/// `justify-self` (grid item). Reusa `align-self` + `left`/`right`.
pub(crate) fn parse_justify_self(s: &str) -> Option<AlignSelf> {
    match s.trim().to_ascii_lowercase().as_str() {
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
    let parts: Vec<&str> = value.split_whitespace().collect();
    match parts.as_slice() {
        [v] => {
            let v = parse_length_px(v)?;
            Some((v, v))
        }
        [r, c] => Some((parse_length_px(r)?, parse_length_px(c)?)),
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
        "hidden" | "clip" | "auto" | "scroll" => Some(Overflow::Hidden),
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
        _ => None,
    }
}

pub(crate) fn parse_text_transform(s: &str) -> Option<TextTransform> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        "capitalize" => Some(TextTransform::Capitalize),
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
    s.parse::<f32>().ok().map(|v| v.clamp(0.0, 1.0))
}

pub(crate) fn parse_align_self(s: &str) -> Option<AlignSelf> {
    match s.trim().to_ascii_lowercase().as_str() {
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
    let mut style_active: Option<bool> = None;
    for tok in value.split_whitespace() {
        if width.is_none() {
            if let Some(w) = parse_length_px(tok) {
                width = Some(w);
                continue;
            }
        }
        if style_active.is_none() {
            if let Some(active) = parse_border_style(tok) {
                style_active = Some(active);
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
    if let Some(c) = color {
        out.push(Decl { kind: DeclKind::OutlineColor(c), important });
    }
    if style_active.is_some() {
        out.push(Decl { kind: DeclKind::OutlineStyle(true), important });
    }
    out
}

/// `background-image: linear-gradient(...)` o `none`. Devuelve un
/// `DeclKind` listo (Background o BackgroundGradient o None).
pub(crate) fn parse_background_image(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(DeclKind::BackgroundGradientNone);
    }
    if let Some(args) = strip_fn(v, "linear-gradient") {
        return parse_linear_gradient(args).map(DeclKind::BackgroundGradient);
    }
    if let Some(args) = strip_fn(v, "url") {
        // url('foo') / url("foo") / url(foo) — trimea comillas.
        let raw = args.trim();
        let unquoted = raw
            .strip_prefix('"').and_then(|s| s.strip_suffix('"'))
            .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(raw);
        let url = unquoted.trim();
        if url.is_empty() {
            return None;
        }
        return Some(DeclKind::BackgroundImageUrl(url.to_string()));
    }
    // Otros gradientes (`radial-gradient`, `conic-gradient`) o `cross-fade`
    // no soportados — silencio.
    None
}

/// Parsea el contenido de `linear-gradient(...)`. Sintaxis aceptada:
/// - `linear-gradient(<angle>?, <stop>, <stop>, ...)`
/// - `linear-gradient(to <side>?, <stop>, <stop>, ...)`
/// `<angle>` en `Ndeg` o `Nturn` (turn × 360 = grados). Default 180
/// (top→bottom). `to right`=90, `to left`=270, `to top`=0, `to bottom`=180,
/// combinaciones diagonales (`to top right`=45) también. Stops: `<color>
/// <pos>?` donde pos es `N%` o `Npx`.
pub(crate) fn parse_linear_gradient(args: &str) -> Option<LinearGradient> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    if parts.len() < 2 {
        return None;
    }
    let (angle_deg, stops_start) = parse_gradient_direction(parts[0]);
    let stops_start_idx = if angle_deg.is_some() { 1 } else { 0 };
    let angle_deg = angle_deg.unwrap_or(180.0);
    let mut stops: Vec<GradientStop> = Vec::new();
    for raw in &parts[stops_start_idx..] {
        if let Some(s) = parse_gradient_stop(raw) {
            stops.push(s);
        }
    }
    if stops.len() < 2 {
        return None;
    }
    let _ = stops_start;
    Some(LinearGradient { angle_deg, stops })
}

/// Si el token es una dirección/ángulo válido devuelve `(Some(deg),
/// true)`; si no encaja, `(None, false)` para que el caller lo trate
/// como stop.
pub(crate) fn parse_gradient_direction(s: &str) -> (Option<f32>, bool) {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("to ") {
        let deg = match rest.trim() {
            "top" => 0.0,
            "right" => 90.0,
            "bottom" => 180.0,
            "left" => 270.0,
            "top right" | "right top" => 45.0,
            "bottom right" | "right bottom" => 135.0,
            "bottom left" | "left bottom" => 225.0,
            "top left" | "left top" => 315.0,
            _ => return (None, false),
        };
        return (Some(deg), true);
    }
    if let Some(num) = lower.strip_suffix("deg") {
        if let Ok(v) = num.trim().parse::<f32>() {
            return (Some(v), true);
        }
    }
    if let Some(num) = lower.strip_suffix("turn") {
        if let Ok(v) = num.trim().parse::<f32>() {
            return (Some(v * 360.0), true);
        }
    }
    (None, false)
}

pub(crate) fn parse_gradient_stop(s: &str) -> Option<GradientStop> {
    let s = s.trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.as_slice() {
        [c] => Some(GradientStop { color: parse_color(c)?, pos: None }),
        [c, p] => {
            let color = parse_color(c)?;
            let pos = if let Some(pct) = p.strip_suffix('%') {
                pct.trim().parse::<f32>().ok().map(|v| (v / 100.0).clamp(0.0, 1.0))
            } else if let Some(px) = parse_length_px(p) {
                // Aproximación: tratamos px como 0..1 dividiendo por 100.
                // En el wild la mayoría usa %, así que esta heurística
                // raramente importa.
                Some((px / 100.0).clamp(0.0, 1.0))
            } else {
                None
            };
            Some(GradientStop { color, pos })
        }
        _ => None,
    }
}

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
        "sticky" => Some(Position::Sticky),
        _ => None,
    }
}

pub(crate) fn parse_vertical_align(s: &str) -> Option<VerticalAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        "baseline" => Some(VerticalAlign::Baseline),
        "top" | "text-top" => Some(VerticalAlign::Top),
        "middle" => Some(VerticalAlign::Middle),
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
        "auto" => Some(PointerEvents::Auto),
        "none" => Some(PointerEvents::None),
        _ => None,
    }
}

/// `text-shadow: <x> <y> [blur] <color>[, <x> <y> [blur] <color>]*`.
/// `none` → vector vacío. Devuelve None si ningún shadow es válido.
pub(crate) fn parse_text_shadows(value: &str) -> Option<Vec<TextShadow>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for sh in v.split(',') {
        if let Some(s) = parse_one_text_shadow(sh) {
            out.push(s);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn parse_one_text_shadow(s: &str) -> Option<TextShadow> {
    let mut lengths: Vec<f32> = Vec::with_capacity(3);
    let mut color: Option<Color> = None;
    for tok in s.split_whitespace() {
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
    Some(TextShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_px: lengths.get(2).copied().unwrap_or(0.0),
        color: color.unwrap_or(Color::BLACK),
    })
}

/// `transform: none` o cadena de funciones (`rotate(45deg) scale(2)
/// translate(10px, 20px)`). Acepta `translate(x)`, `translate(x, y)`,
/// `translateX(x)`, `translateY(y)`, `scale(s)`, `scale(sx, sy)`,
/// `scaleX(sx)`, `scaleY(sy)`, `rotate(Ndeg|Nrad|Nturn)`.
pub(crate) fn parse_transforms(value: &str) -> Option<Vec<Transform>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    let mut rest = v;
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let open = rest.find('(')?;
        let name = rest[..open].trim().to_ascii_lowercase();
        let mut depth = 1usize;
        let bytes = rest[open + 1..].as_bytes();
        let mut close = None;
        for (i, &c) in bytes.iter().enumerate() {
            match c {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let args = &rest[open + 1..open + 1 + close];
        let tr = parse_transform_fn(&name, args)?;
        out.push(tr);
        rest = &rest[open + 1 + close + 1..];
    }
    Some(out)
}

pub(crate) fn parse_transform_fn(name: &str, args: &str) -> Option<Transform> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    match name {
        "translate" => match parts.as_slice() {
            [x] => Some(Transform::Translate(parse_length_px(x)?, 0.0)),
            [x, y] => Some(Transform::Translate(parse_length_px(x)?, parse_length_px(y)?)),
            _ => None,
        },
        "translatex" => Some(Transform::Translate(parse_length_px(parts[0])?, 0.0)),
        "translatey" => Some(Transform::Translate(0.0, parse_length_px(parts[0])?)),
        "scale" => match parts.as_slice() {
            [s] => {
                let v = s.parse::<f32>().ok()?;
                Some(Transform::Scale(v, v))
            }
            [sx, sy] => {
                Some(Transform::Scale(sx.parse().ok()?, sy.parse().ok()?))
            }
            _ => None,
        },
        "scalex" => Some(Transform::Scale(parts[0].parse().ok()?, 1.0)),
        "scaley" => Some(Transform::Scale(1.0, parts[0].parse().ok()?)),
        "rotate" => {
            let arg = parts[0];
            let deg = if let Some(n) = arg.strip_suffix("deg") {
                n.trim().parse::<f32>().ok()?
            } else if let Some(n) = arg.strip_suffix("rad") {
                let v: f32 = n.trim().parse().ok()?;
                v.to_degrees()
            } else if let Some(n) = arg.strip_suffix("turn") {
                let v: f32 = n.trim().parse().ok()?;
                v * 360.0
            } else {
                // Sin unidad: asumir deg.
                arg.parse::<f32>().ok()?
            };
            Some(Transform::Rotate(deg))
        }
        _ => None,
    }
}

/// `grid-template-columns: <track-list>`. Subset soportado:
/// - `auto`
/// - `Npx` / `N%`
/// - `Nfr`
/// - `repeat(N, <track>)` con repeat de un solo track
/// Tokens separados por whitespace.
pub(crate) fn parse_grid_template(value: &str) -> Option<Vec<GridTrackSize>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out: Vec<GridTrackSize> = Vec::new();
    // Tokenize: respeta nesting de paréntesis para repeat(N, X).
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in v.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    for tok in tokens {
        if let Some(inner) = strip_fn(&tok, "repeat") {
            let parts: Vec<&str> = inner.splitn(2, ',').collect();
            if parts.len() != 2 {
                continue;
            }
            let count: i32 = parts[0].trim().parse().ok()?;
            let track = parse_one_grid_track(parts[1].trim())?;
            for _ in 0..count.max(0) {
                out.push(track);
            }
        } else if let Some(t) = parse_one_grid_track(&tok) {
            out.push(t);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn parse_one_grid_track(s: &str) -> Option<GridTrackSize> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") {
        return Some(GridTrackSize::Auto);
    }
    if let Some(num) = s.strip_suffix("fr") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(GridTrackSize::Fr(v));
    }
    if let Some(lv) = parse_length_or_pct(s) {
        return Some(match lv {
            LengthVal::Px(v) => GridTrackSize::Px(v),
            LengthVal::Pct(v) => GridTrackSize::Pct(v),
            LengthVal::Auto => GridTrackSize::Auto,
        });
    }
    None
}

/// Evalúa una condición de `@media` contra el viewport por defecto. Subset:
/// `(max-width: Npx)`, `(min-width: Npx)`, encadenados por ` and `.
/// `screen`/`all` se ignoran (siempre true).
/// Evalúa una media query (`@media` en CSS y `window.matchMedia()` en JS) contra
/// el viewport actual. Soporta listas separadas por `,` (OR), `not`/`only`,
/// el combinador ` and `, tipos de media (`screen`/`all`/`print`/`speech`) y
/// las features: `min/max/exact-width`, `min/max/exact-height`, `orientation`
/// (portrait/landscape), `min/max/exact-resolution` (`Ndppx`/`Ndpi`/`Nx` vs
/// `vp.dpr`) y `prefers-color-scheme`/`prefers-reduced-motion` (reportamos
/// light / no-reduce). Features desconocidas se ignoran (no descalifican), igual
/// que el comportamiento previo, para no romper CSS que las use de forma
/// progresiva. Pública porque el chrome (`puriy-llimphi`) la reusa para resolver
/// `matchMedia` contra el viewport real de la ventana.
pub fn evaluate_media_query(condition: &str, vp: Viewport) -> bool {
    let cond = condition.trim().to_ascii_lowercase();
    if cond.is_empty() {
        return true;
    }
    // Media query LIST: separada por comas, matchea si CUALQUIER componente lo hace.
    if cond.contains(',') {
        return cond.split(',').any(|q| evaluate_media_query(q, vp));
    }
    // `not` a nivel de query invierte el resultado completo.
    if let Some(rest) = cond.strip_prefix("not ") {
        return !evaluate_media_query_terms(rest.trim(), vp);
    }
    evaluate_media_query_terms(&cond, vp)
}

/// Evalúa los términos unidos por ` and ` de una query ya sin `,`/`not` de tope.
pub(crate) fn evaluate_media_query_terms(cond: &str, vp: Viewport) -> bool {
    for part in cond.split(" and ").map(|s| s.trim()) {
        if part.is_empty() {
            continue;
        }
        // Tipos de media.
        if part == "all" || part == "screen" {
            continue;
        }
        if part == "print" || part == "speech" || part == "tty" {
            return false;
        }
        let part = part.strip_prefix("only ").unwrap_or(part).trim();
        // Esperamos `(feature)` o `(feature: value)`.
        let Some(inner) = part.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
            // Token no reconocido (tipo de media raro): no matchea.
            return false;
        };
        if !evaluate_media_feature(inner.trim(), vp) {
            return false;
        }
    }
    true
}

/// Evalúa UNA feature `(feature)` o `(feature: value)` contra el viewport.
pub(crate) fn evaluate_media_feature(inner: &str, vp: Viewport) -> bool {
    let Some((feature, val)) = inner.split_once(':').map(|(a, b)| (a.trim(), b.trim())) else {
        // Feature booleana (sin valor): matchea si la capacidad "existe".
        return matches!(inner, "color" | "grid" | "hover" | "pointer");
    };
    match feature {
        "max-width" => parse_length_px(val).is_some_and(|l| vp.width <= l),
        "min-width" => parse_length_px(val).is_some_and(|l| vp.width >= l),
        "width" => parse_length_px(val).is_some_and(|l| (vp.width - l).abs() < 0.5),
        "max-height" => parse_length_px(val).is_some_and(|l| vp.height <= l),
        "min-height" => parse_length_px(val).is_some_and(|l| vp.height >= l),
        "height" => parse_length_px(val).is_some_and(|l| (vp.height - l).abs() < 0.5),
        "orientation" => match val {
            "portrait" => vp.height >= vp.width,
            "landscape" => vp.width > vp.height,
            _ => false,
        },
        "min-resolution" => parse_resolution_dppx(val).is_some_and(|r| vp.dpr >= r),
        "max-resolution" => parse_resolution_dppx(val).is_some_and(|r| vp.dpr <= r),
        "resolution" => parse_resolution_dppx(val).is_some_and(|r| (vp.dpr - r).abs() < 0.01),
        "min-aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| vp.width / vp.height >= r)
        }
        "max-aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| vp.width / vp.height <= r)
        }
        "aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| (vp.width / vp.height - r).abs() < 0.01)
        }
        // Preferencias del usuario: reportamos tema claro y sin reducción.
        "prefers-color-scheme" => val == "light" || val == "no-preference",
        "prefers-reduced-motion" => val == "no-preference",
        "prefers-contrast" => val == "no-preference",
        "hover" => val == "hover",
        "any-hover" => val == "hover",
        "pointer" => val == "fine",
        "any-pointer" => val == "fine",
        // Feature desconocida: no descalifica (comportamiento previo lenient).
        _ => true,
    }
}

/// Parsea un aspect-ratio de media query a un float `ancho/alto`. Acepta la
/// forma `W/H` (`16/9`) y el número suelto (`1.5`). `None` si no parsea o el
/// alto es cero.
pub(crate) fn parse_aspect_ratio(val: &str) -> Option<f32> {
    let v = val.trim();
    if let Some((w, h)) = v.split_once('/') {
        let w: f32 = w.trim().parse().ok()?;
        let h: f32 = h.trim().parse().ok()?;
        if h == 0.0 {
            return None;
        }
        Some(w / h)
    } else {
        v.parse::<f32>().ok()
    }
}

/// Parsea una resolución de media query a `dppx` (dots per px). Acepta
/// `Ndppx`, `Nx` (alias de dppx) y `Ndpi` (96dpi = 1dppx). `None` si no parsea.
pub(crate) fn parse_resolution_dppx(val: &str) -> Option<f32> {
    let v = val.trim();
    if let Some(n) = v.strip_suffix("dppx").or_else(|| v.strip_suffix('x')) {
        n.trim().parse::<f32>().ok()
    } else if let Some(n) = v.strip_suffix("dpi") {
        n.trim().parse::<f32>().ok().map(|d| d / 96.0)
    } else if let Some(n) = v.strip_suffix("dpcm") {
        n.trim().parse::<f32>().ok().map(|d| d / 96.0 * 2.54)
    } else {
        None
    }
}

/// Evalúa una condición `@supports (prop: value)` ⇒ true si nuestro
/// parser puede convertirla a algún DeclKind. Subset minimal: no
/// soporta `and`/`or`/`not` por ahora.
pub(crate) fn evaluate_supports_query(condition: &str) -> bool {
    let cond = condition.trim();
    let Some(inner) = cond.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
        return false;
    };
    let Some((prop, val)) = inner.split_once(':') else {
        return false;
    };
    decl_kind_from_pair(prop.trim(), val.trim()).is_some()
}

/// Indica que `cssparser` está enlazado aunque el subset actual no use
/// la API completa — la presencia del crate evita que `cargo` lo
/// pruebe y deja el camino abierto para Fase 3.
#[doc(hidden)]
pub fn _cssparser_anchor() {
    let _ = cssparser::ParserInput::new("");
}
