use super::*;

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
            // El 1er arg de `repeat` puede tener comas internas si el track es
            // `minmax(a, b)` → split sólo en la 1ª coma de nivel superior.
            let Some((count_raw, track_raw)) = split_first_top_comma(inner) else {
                continue;
            };
            let track = parse_one_grid_track(track_raw.trim())?;
            let count = match count_raw.trim().to_ascii_lowercase().as_str() {
                // Fase 7.859 — `auto-fill`/`auto-fit`: sin ancho de container
                // al parsear, estimamos N = viewport / min-track (si el min es
                // px); si no, 1 track. Divergencia documentada.
                "auto-fill" | "auto-fit" => auto_repeat_count(track_raw.trim()),
                other => other.parse::<i32>().ok()?,
            };
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
    // Fase 7.859 — `minmax(min, max)`. El modelo es de tamaño único: tomamos
    // el `max` (el tamaño "ideal" al que crece el track, p.ej. `1fr`); si el
    // max es intrínseco/auto, caemos al `min`; si ambos lo son, `auto`. Es la
    // aproximación que mejor refleja el patrón usual `minmax(<px>, 1fr)`.
    if let Some(inner) = strip_fn(s, "minmax") {
        let Some((min_raw, max_raw)) = split_first_top_comma(inner) else {
            return None;
        };
        let max = parse_one_grid_track(max_raw.trim());
        if let Some(m) = max {
            if !matches!(m, GridTrackSize::Auto) {
                return Some(m);
            }
        }
        return parse_one_grid_track(min_raw.trim());
    }
    // Fase 7.859 — `fit-content(<len>)` ≈ track con tope = ese length → lo
    // aproximamos al length (clamp superior implícito que no modelamos).
    if let Some(inner) = strip_fn(s, "fit-content") {
        return parse_one_grid_track(inner.trim());
    }
    if let Some(num) = s.strip_suffix("fr") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(GridTrackSize::Fr(v));
    }
    // `min-content`/`max-content` como track entero → auto (sin layout intrínseco).
    if s.eq_ignore_ascii_case("min-content") || s.eq_ignore_ascii_case("max-content") {
        return Some(GridTrackSize::Auto);
    }
    if let Some(lv) = parse_length_or_pct(s) {
        return Some(match lv {
            LengthVal::Px(v) => GridTrackSize::Px(v),
            LengthVal::Pct(v) => GridTrackSize::Pct(v),
            // Fase 7.849 — tracks intrínsecos (min/max/fit-content) aún no se
            // modelan en `GridTrackSize`; aproximamos a `auto`.
            LengthVal::Auto
            | LengthVal::MinContent
            | LengthVal::MaxContent
            | LengthVal::FitContent => GridTrackSize::Auto,
        });
    }
    None
}

/// Parte `s` en (antes, después) de la PRIMERA coma de nivel superior
/// (respeta paréntesis anidados). `None` si no hay coma top-level.
fn split_first_top_comma(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Estima el número de repeticiones de `repeat(auto-fill|auto-fit, <track>)`.
/// Si el track es `minmax(<px>, …)` con un mínimo en px, devuelve
/// `floor(viewport_width / min_px)` (≥1); si no hay un piso px conocido, 1.
/// Aproximación: no hay ancho de container real en este punto del pipeline.
fn auto_repeat_count(track: &str) -> i32 {
    let min_px = strip_fn(track, "minmax")
        .and_then(split_first_top_comma)
        .and_then(|(min_raw, _)| match parse_one_grid_track(min_raw.trim()) {
            Some(GridTrackSize::Px(v)) if v > 0.0 => Some(v),
            _ => None,
        });
    match min_px {
        Some(px) => ((resolve_viewport().width / px).floor() as i32).max(1),
        None => 1,
    }
}
