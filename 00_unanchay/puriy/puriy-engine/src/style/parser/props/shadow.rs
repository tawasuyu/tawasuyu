use super::*;

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
