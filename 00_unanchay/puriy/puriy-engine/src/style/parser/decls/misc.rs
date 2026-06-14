//! Parsers varios: nth-arg, box-shadow, cursor, filtros, blend, scroll-snap, sides.
//! Value-parsers extraídos de `decls.rs` (regla #1). Lógica intacta.
use super::*;

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

/// `clip` (CSS2.1, deprecada): `auto | rect(<t>, <r>, <b>, <l>)`. Cada lado
/// es `<length> | auto` (`auto` → `None`). Acepta el separador con coma (forma
/// canónica) y sin coma (forma legacy `rect(0 0 0 0)`). Case-insensitive.
pub(crate) fn parse_clip(value: &str) -> Option<Clip> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") {
        return Some(Clip::Auto);
    }
    let lower = v.to_ascii_lowercase();
    let inner = lower.strip_prefix("rect(")?.strip_suffix(')')?.trim();
    let parts: Vec<&str> = if inner.contains(',') {
        inner.split(',').map(|p| p.trim()).collect()
    } else {
        inner.split_whitespace().collect()
    };
    if parts.len() != 4 {
        return None;
    }
    let side = |s: &str| -> Option<Option<f32>> {
        if s.eq_ignore_ascii_case("auto") {
            Some(None)
        } else {
            parse_length_px(s).map(Some)
        }
    };
    Some(Clip::Rect {
        top: side(parts[0])?,
        right: side(parts[1])?,
        bottom: side(parts[2])?,
        left: side(parts[3])?,
    })
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
        "auto-phrase" => Some(WordBreak::AutoPhrase), // CSS Text 4 (Fase 7.917)
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
        // Fase 7.910 — aliases legacy SVG 1.1 (`writing-mode` con valores
        // `lr`/`rl`/`tb`): `lr`/`lr-tb`/`rl`/`rl-tb` = horizontal; `tb`/`tb-rl`
        // = vertical-rl; `tb-lr` = vertical-lr.
        "lr" | "lr-tb" | "rl" | "rl-tb" => Some(WritingMode::HorizontalTb),
        "tb" | "tb-rl" => Some(WritingMode::VerticalRl),
        "tb-lr" => Some(WritingMode::VerticalLr),
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
pub(crate) fn parse_angle_deg(s: &str) -> Option<f32> {
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

