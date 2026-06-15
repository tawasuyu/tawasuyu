use super::*;

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

/// Comparador de la sintaxis de rango de Media Queries 4.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum RangeCmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl RangeCmp {
    fn apply(self, lhs: f32, rhs: f32) -> bool {
        match self {
            RangeCmp::Lt => lhs < rhs,
            RangeCmp::Le => lhs <= rhs,
            RangeCmp::Gt => lhs > rhs,
            RangeCmp::Ge => lhs >= rhs,
            RangeCmp::Eq => (lhs - rhs).abs() < 0.5,
        }
    }
    /// Invierte el sentido (para `value op feature` → `feature flip(op) value`).
    fn flip(self) -> RangeCmp {
        match self {
            RangeCmp::Lt => RangeCmp::Gt,
            RangeCmp::Le => RangeCmp::Ge,
            RangeCmp::Gt => RangeCmp::Lt,
            RangeCmp::Ge => RangeCmp::Le,
            RangeCmp::Eq => RangeCmp::Eq,
        }
    }
}

/// Valor actual de una media feature de rango contra el viewport.
fn range_feature_current(name: &str, vp: Viewport) -> Option<f32> {
    match name {
        "width" | "inline-size" => Some(vp.width),
        "height" | "block-size" => Some(vp.height),
        "aspect-ratio" => Some(vp.width / vp.height),
        "resolution" => Some(vp.dpr),
        _ => None,
    }
}

/// Parsea el valor de comparación según la feature de rango.
fn range_feature_value(name: &str, val: &str) -> Option<f32> {
    match name {
        "width" | "inline-size" | "height" | "block-size" => parse_length_px(val),
        "aspect-ratio" => parse_aspect_ratio(val),
        "resolution" => parse_resolution_dppx(val),
        _ => None,
    }
}

/// Intenta evaluar la sintaxis de rango de MQ4: `(width >= 600px)`,
/// `(600px < width)`, `(400px <= width <= 800px)`. `None` si el `inner` no
/// es una expresión de rango (lo maneja el path `feature: value`).
pub(crate) fn try_eval_media_range(inner: &str, vp: Viewport) -> Option<bool> {
    // Sólo es rango si hay un comparador `<`/`>`/`=` (el path normal usa `:`).
    if !inner.contains(['<', '>', '=']) {
        return None;
    }
    // Tokeniza en palabras y comparadores (con o sin espacios).
    let mut words: Vec<String> = Vec::new();
    let mut ops: Vec<RangeCmp> = Vec::new();
    let mut order: Vec<bool> = Vec::new(); // true = word, false = op
    let bytes = inner.as_bytes();
    let mut i = 0;
    let mut cur = String::new();
    let flush = |cur: &mut String, words: &mut Vec<String>, order: &mut Vec<bool>| {
        let t = cur.trim();
        if !t.is_empty() {
            words.push(t.to_string());
            order.push(true);
        }
        cur.clear();
    };
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'<' || c == b'>' || c == b'=' {
            flush(&mut cur, &mut words, &mut order);
            let op = if (c == b'<' || c == b'>') && bytes.get(i + 1) == Some(&b'=') {
                i += 2;
                if c == b'<' { RangeCmp::Le } else { RangeCmp::Ge }
            } else {
                i += 1;
                match c {
                    b'<' => RangeCmp::Lt,
                    b'>' => RangeCmp::Gt,
                    _ => RangeCmp::Eq,
                }
            };
            ops.push(op);
            order.push(false);
            continue;
        }
        cur.push(c as char);
        i += 1;
    }
    flush(&mut cur, &mut words, &mut order);
    // Patrón válido: alterna word/op empezando y terminando en word.
    let alternating_ok = order.iter().enumerate().all(|(idx, is_word)| *is_word == (idx % 2 == 0));
    if !alternating_ok {
        return None;
    }
    match (words.as_slice(), ops.as_slice()) {
        // `feature op value` o `value op feature`.
        ([a, b], [op]) => {
            if let Some(cur) = range_feature_current(a, vp) {
                let v = range_feature_value(a, b)?;
                Some(op.apply(cur, v))
            } else if let Some(cur) = range_feature_current(b, vp) {
                let v = range_feature_value(b, a)?;
                Some(op.flip().apply(cur, v))
            } else {
                None
            }
        }
        // `v1 op1 feature op2 v2` (la feature está en el medio).
        ([v1, f, v2], [op1, op2]) => {
            let cur = range_feature_current(f, vp)?;
            let lo = range_feature_value(f, v1)?;
            let hi = range_feature_value(f, v2)?;
            Some(op1.flip().apply(cur, lo) && op2.apply(cur, hi))
        }
        _ => None,
    }
}

/// Evalúa UNA feature `(feature)` o `(feature: value)` contra el viewport.
pub(crate) fn evaluate_media_feature(inner: &str, vp: Viewport) -> bool {
    // Sintaxis de rango MQ4 (`width >= 600px`, `400px <= width <= 800px`).
    if let Some(r) = try_eval_media_range(inner, vp) {
        return r;
    }
    let Some((feature, val)) = inner.split_once(':').map(|(a, b)| (a.trim(), b.trim())) else {
        // Feature booleana (sin valor): matchea si el valor de la feature NO
        // es su valor "cero"/none (CSS MQ4 §2.4). puriy es un renderer de
        // escritorio con color, JS y refresco rápido. `monochrome`/`color-index`
        // valen 0 → booleano falso.
        return matches!(
            inner,
            "color" | "grid" | "hover" | "pointer" | "any-hover" | "any-pointer"
                | "scripting" | "update"
        );
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
        "prefers-reduced-data" => val == "no-preference",
        "prefers-reduced-transparency" => val == "no-preference",
        "hover" => val == "hover",
        "any-hover" => val == "hover",
        "pointer" => val == "fine",
        "any-pointer" => val == "fine",
        // === Fase 7.1213 — features con respuesta definitiva para un renderer
        // de escritorio (antes caían en `_ => true` y matcheaban cualquier
        // valor, incluido el incorrecto). ===
        // puriy ejecuta JS de forma persistente.
        "scripting" => val == "enabled",
        // Refresco rápido (no e-ink).
        "update" => val == "fast",
        // Soporta CSS Grid.
        "grid" => val == "1",
        // Pantalla a color (no monocroma) sin paleta indexada.
        "monochrome" => val == "0",
        "color-index" => val == "0",
        // `min-monochrome`/`min-color-index: N` matchea sólo si N <= 0 (nuestro
        // monochrome/color-index valen 0).
        "min-monochrome" | "min-color-index" => val.parse::<i32>().is_ok_and(|n| n <= 0),
        "max-monochrome" | "max-color-index" => val.parse::<i32>().is_ok_and(|n| n >= 0),
        // Gama sRGB (no P3/rec2020).
        "color-gamut" => val == "srgb",
        // Rango dinámico estándar (no HDR).
        "dynamic-range" | "video-dynamic-range" => val == "standard",
        // Escaneo progresivo.
        "scan" => val == "progressive",
        // Scrolleable en ambos ejes.
        "overflow-block" => val == "scroll",
        "overflow-inline" => val == "scroll",
        // Sin modo de colores forzados ni inversión.
        "forced-colors" => val == "none",
        "inverted-colors" => val == "none",
        // Navegador estándar (no PWA instalada).
        "display-mode" => val == "browser",
        // Feature desconocida de verdad: no descalifica (lenient, forward-compat).
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

/// Evalúa una condición `@supports`: una declaración `(prop: value)` es
/// soportada si nuestro parser la convierte a algún `DeclKind`. Soporta
/// `and`/`or`/`not`, agrupación con paréntesis y `selector(<sel>)`
/// (recursivo). Las keywords se reconocen en minúsculas.
pub(crate) fn evaluate_supports_query(condition: &str) -> bool {
    let cond = condition.trim();
    // `not <cond>`.
    if let Some(rest) = strip_supports_not(cond) {
        return !evaluate_supports_query(rest);
    }
    // `a and b and ...` (a nivel de paréntesis 0).
    let and_parts = split_supports(cond, "and");
    if and_parts.len() > 1 {
        return and_parts.iter().all(|p| evaluate_supports_query(p));
    }
    // `a or b or ...`.
    let or_parts = split_supports(cond, "or");
    if or_parts.len() > 1 {
        return or_parts.iter().any(|p| evaluate_supports_query(p));
    }
    // `selector(<sel>)` — soportado si el selector parsea.
    if let Some(sel) = cond
        .strip_prefix("selector(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_selector(sel.trim()).is_some();
    }
    // Grupo o declaración entre paréntesis.
    if let Some(inner) = strip_supports_parens(cond) {
        let inner = inner.trim();
        if let Some((prop, val)) = split_top_colon(inner) {
            return decl_kind_from_pair(prop.trim(), val.trim()).is_some();
        }
        // Agrupación `( <cond> )`.
        return evaluate_supports_query(inner);
    }
    false
}

/// `not <cond>` / `not(<cond>)` (whitespace o `(` tras el keyword).
fn strip_supports_not(s: &str) -> Option<&str> {
    let rest = s.trim().strip_prefix("not")?;
    let c = rest.chars().next()?;
    (c.is_whitespace() || c == '(').then(|| rest.trim_start())
}

/// Divide `s` por ` kw ` (whitespace a ambos lados) a profundidad de
/// paréntesis 0. Devuelve `[s]` si no hay separador.
fn split_supports<'a>(s: &'a str, kw: &str) -> Vec<&'a str> {
    let bytes = s.as_bytes();
    let kwb = kw.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b' ' if depth == 0 => {
                let j = i + 1;
                if bytes[j..].starts_with(kwb) && bytes.get(j + kwb.len()) == Some(&b' ') {
                    parts.push(s[start..i].trim());
                    i = j + kwb.len() + 1;
                    start = i;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(s[start..].trim());
    parts
}

/// Si `s` está envuelto por un par de paréntesis que se cierran al final
/// (no `(a) ... (b)`), devuelve el interior.
fn strip_supports_parens(s: &str) -> Option<&str> {
    let s = s.trim();
    let inner = s.strip_prefix('(')?.strip_suffix(')')?;
    let mut depth = 0i32;
    for c in inner.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return None; // se cierra antes del final → no envuelve todo
                }
            }
            _ => {}
        }
    }
    (depth == 0).then_some(inner)
}

/// Primer `:` a profundidad de paréntesis 0 → `(prop, value)`.
fn split_top_colon(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b':' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}
