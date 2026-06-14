//! Text-decoration, list-style, content, contadores, motor calc, font-size, line-height, colores nombrados.
//! Value-parsers extraídos de `decls.rs` (regla #1). Lógica intacta.
use super::*;

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
pub(crate) fn parse_text_decoration_shorthand(value: &str, important: bool) -> Vec<Decl> {
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
    let raw = s.trim();
    // Fase 7.904 — marcador string (`list-style-type: "→"`) o `symbols(...)`
    // (CSS Counter Styles 3): marcadores custom que el enum no modela; los
    // aproximamos a `Disc`. Mejor que descartar (dejaría el marker heredado).
    if (raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2)
        || raw.to_ascii_lowercase().starts_with("symbols(")
    {
        return Some(ListStyleType::Disc);
    }
    match raw.to_ascii_lowercase().as_str() {
        "none" => Some(ListStyleType::None),
        "disc" => Some(ListStyleType::Disc),
        "circle" => Some(ListStyleType::Circle),
        "square" => Some(ListStyleType::Square),
        "decimal" => Some(ListStyleType::Decimal),
        "lower-alpha" | "lower-latin" => Some(ListStyleType::LowerAlpha),
        "upper-alpha" | "upper-latin" => Some(ListStyleType::UpperAlpha),
        "lower-roman" => Some(ListStyleType::LowerRoman),
        "upper-roman" => Some(ListStyleType::UpperRoman),
        // Fase 7.867 — estilos de contador adicionales (CSS Counter Styles 3).
        // El enum no los modela uno a uno; los aproximamos al pariente más
        // cercano que SÍ pintamos: los numéricos → `Decimal`, los alfabéticos
        // de otros alfabetos → `LowerAlpha`/`UpperAlpha`, los triángulos de
        // `<details>` → `Disc`. Mejor que descartar (que dejaría el marker
        // heredado).
        "decimal-leading-zero" | "cjk-decimal" | "arabic-indic" | "armenian"
        | "georgian" | "hebrew" | "cjk-ideographic" | "japanese-informal"
        | "japanese-formal" | "korean-hangul-formal" | "simp-chinese-informal"
        | "trad-chinese-informal"
        // Fase 7.910 — variantes -formal/-hanja restantes (CSS Counter Styles 3).
        | "simp-chinese-formal" | "trad-chinese-formal" | "korean-hanja-formal"
        | "korean-hanja-informal" | "korean-hangul" | "cjk-heavenly-stem"
        | "cjk-earthly-branch" | "ethiopic-numeric" => Some(ListStyleType::Decimal),
        "lower-greek" | "lower-armenian" => Some(ListStyleType::LowerAlpha),
        "upper-armenian" | "upper-greek" | "upper-latin-symbol" => {
            Some(ListStyleType::UpperAlpha)
        }
        "disclosure-open" | "disclosure-closed" => Some(ListStyleType::Disc),
        _ => None,
    }
}

pub(crate) fn parse_text_align(s: &str) -> Option<TextAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        // Fase 7.845 — `match-parent` (sin contexto de dirección, en LTR ≈
        // start = left) y alias vendor `-webkit-center`/`-moz-center` → center.
        "left" | "start" | "match-parent" => Some(TextAlign::Left),
        "center" | "-webkit-center" | "-moz-center" => Some(TextAlign::Center),
        "right" | "end" => Some(TextAlign::Right),
        // Fase 7.856 — `justify-all` fuerza justificar también la última
        // línea; sin esa distinción en el modelo, colapsa a `Justify`.
        "justify" | "justify-all" => Some(TextAlign::Justify),
        _ => None,
    }
}

/// Acepta `auto`, `Npx`, `Nrem`/`Nem` (→ px), `N%`. Sin unidad y
/// distinto de `0` → falla (a diferencia de `parse_length_px`, que
/// asume px).
/// Devuelve el primer item de una lista separada por coma. Si no hay
/// coma, devuelve el string completo. Espacios al borde recortados.
/// Fase 7.514+ (longhands animation que sólo guardan el primer item).
pub(crate) fn first_comma(s: &str) -> &str {
    // Fase 7.855 — sólo cuenta una coma de NIVEL SUPERIOR; las internas de
    // `cubic-bezier(a, b, c, d)`/`steps(n, end)` no parten el primer item.
    let mut depth: i32 = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => return s[..i].trim(),
            _ => {}
        }
    }
    s.trim()
}

/// Parsea `<time>` CSS: `<n>s` o `<n>ms`. Devuelve segundos.
/// Fase 7.515.
pub(crate) fn parse_time_seconds(s: &str) -> Option<f32> {
    let s = s.trim();
    // Fase 7.877 — `calc()` sobre tiempos (el evaluador da segundos).
    if is_math_fn(s) {
        return match eval_calc(s)? {
            CalcVal::Number(n) if n.is_finite() => Some(n),
            _ => None,
        };
    }
    if let Some(num) = s.strip_suffix("ms") {
        return num.trim().parse::<f32>().ok().map(|n| n / 1000.0);
    }
    if let Some(num) = s.strip_suffix('s') {
        return num.trim().parse::<f32>().ok();
    }
    None
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

/// Tamaño máximo (`max-width`/`max-height` y sus lógicos): igual que
/// `parse_length_or_pct` pero acepta `none` (el valor *inicial* de las
/// props max-*) → `LengthVal::Auto`, que el bridge a taffy interpreta como
/// "sin límite". Sin esto, `max-width: none` (reset muy común) se descartaba
/// y dejaba el máximo cascadeado previo. Fase 7.830.
pub(crate) fn parse_max_size(s: &str) -> Option<LengthVal> {
    if s.trim().eq_ignore_ascii_case("none") {
        Some(LengthVal::Auto)
    } else {
        parse_length_or_pct(s)
    }
}

pub(crate) fn parse_length_or_pct(s: &str) -> Option<LengthVal> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") {
        return Some(LengthVal::Auto);
    }
    // Fase 7.906 — `calc-size(<basis>, <calc>)` (CSS Sizing 4, interpolate-size).
    // Sin interpolación real, resolvemos al tamaño base (1er argumento).
    if s.len() > 10 && s[..10].eq_ignore_ascii_case("calc-size(") {
        if let Some(body) = s[10..].strip_suffix(')') {
            let basis = first_comma(body).trim();
            // `any` como basis = sin restricción → `auto`.
            if basis.eq_ignore_ascii_case("any") {
                return Some(LengthVal::Auto);
            }
            return parse_length_or_pct(basis);
        }
    }
    // Fase 7.849 — keywords de tamaño intrínseco. `fit-content(<len>)` (forma
    // funcional) cae también a `FitContent` (el argumento no se modela aún).
    if s.eq_ignore_ascii_case("min-content") {
        return Some(LengthVal::MinContent);
    }
    if s.eq_ignore_ascii_case("max-content") {
        return Some(LengthVal::MaxContent);
    }
    if s.eq_ignore_ascii_case("fit-content")
        || s.to_ascii_lowercase().starts_with("fit-content(")
    {
        return Some(LengthVal::FitContent);
    }
    // Fase 7.861 — `stretch` (CSS Sizing 4) y sus alias vendor de "llená el
    // espacio disponible": en un bloque eso es justo lo que hace `auto`.
    if s.eq_ignore_ascii_case("stretch")
        || s.eq_ignore_ascii_case("-webkit-fill-available")
        || s.eq_ignore_ascii_case("-moz-available")
        || s.eq_ignore_ascii_case("fill-available")
    {
        return Some(LengthVal::Auto);
    }
    if let Some(num) = s.strip_suffix('%') {
        return num.trim().parse::<f32>().ok().map(LengthVal::Pct);
    }
    // Fase 7.907 — `attr()` (fuera de content), `anchor()` y `anchor-size()`
    // necesitan DOM/geometría que el parser no tiene. Si traen un FALLBACK
    // (último argumento de nivel superior) lo resolvemos; sin fallback, drop.
    for fname in ["attr", "anchor-size", "anchor"] {
        if let Some(inner) = strip_named_fn(s, fname) {
            return last_top_comma(inner).and_then(|fb| parse_length_or_pct(fb.trim()));
        }
    }
    // Funciones matemáticas: `calc()`/`min()`/`max()`/`clamp()` (anidables,
    // con precedencia `*`/`/` sobre `+`/`-` y paréntesis).
    if is_math_fn(s) {
        return eval_calc(s).and_then(calcval_to_length);
    }
    parse_length_px(s).map(LengthVal::Px)
}

/// Si `s` es `name(...)` (case-insensitive, sin espacio antes del `(`),
/// devuelve los argumentos internos sin los paréntesis. Fase 7.907.
fn strip_named_fn<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let s = s.trim();
    if s.len() <= name.len() + 1 || !s[..name.len()].eq_ignore_ascii_case(name) {
        return None;
    }
    s[name.len()..].trim_start().strip_prefix('(')?.strip_suffix(')')
}

/// Parte tras la ÚLTIMA coma de nivel superior (el fallback de `attr()`/
/// `anchor()`). `None` si no hay coma a nivel superior. Fase 7.907.
fn last_top_comma(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut last = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => last = Some(i),
            _ => {}
        }
    }
    last.map(|i| &s[i + 1..])
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
    /// Ángulo en grados (Fase 7.903). Lo producen los tokens `deg`/`rad`/
    /// `grad`/`turn` y las trig inversas; las trig directas (`sin`/`cos`/
    /// `tan`) lo consumen y devuelven `Number`.
    Angle(f32),
}

/// `true` si `s` arranca con una función matemática CSS (`calc`/`min`/
/// `max`/`clamp`) seguida de `(`.
pub(crate) fn is_math_fn(s: &str) -> bool {
    let l = s.trim_start().to_ascii_lowercase();
    // Fase 7.854 — funciones de paso (CSS Values 4): `round`/`mod`/`rem`.
    // Fase 7.871 — exponenciales/signo: `abs`/`sign`/`sqrt`/`pow`/`hypot`/
    // `exp`/`log`.
    // Fase 7.903 — trigonométricas (CSS Values 4): `sin`/`cos`/`tan` y sus
    // inversas `asin`/`acos`/`atan`/`atan2`.
    [
        "calc(", "min(", "max(", "clamp(", "round(", "mod(", "rem(", "abs(", "sign(", "sqrt(",
        "pow(", "hypot(", "exp(", "log(", "sin(", "cos(", "tan(", "asin(", "acos(", "atan(",
        "atan2(",
    ]
    .iter()
    .any(|p| l.starts_with(p))
}

/// Convierte un `CalcVal` final a `LengthVal`. Un número crudo sólo es
/// válido si es 0 (un número no es una longitud). Mezcla px+pct degrada a
/// `Pct` (se pierde el offset px — sin container width, igual que el calc
/// histórico). Ver [`parse_length_or_pct`].
pub(crate) fn calcval_to_length(v: CalcVal) -> Option<LengthVal> {
    match v {
        CalcVal::Number(n) if n == 0.0 => Some(LengthVal::Px(0.0)),
        CalcVal::Number(_) | CalcVal::Angle(_) => None,
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

/// Parsea un token a píxeles aceptando `calc()`/`min()`/`max()`/`clamp()`
/// además de las longitudes simples de [`parse_length_px`]. Un calc con
/// componente `%` cae a `None` (los campos f32 de margin/padding no modelan
/// porcentaje). Fase 7.847.
pub(crate) fn parse_length_px_or_calc(t: &str) -> Option<f32> {
    let t = t.trim();
    if is_math_fn(t) {
        return match eval_calc(t)? {
            CalcVal::Number(n) if n == 0.0 => Some(0.0),
            CalcVal::Length { px, pct } if pct == 0.0 => Some(px),
            _ => None,
        };
    }
    parse_length_px(t)
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
            // Tras la 1ª letra el nombre admite dígitos (`atan2`). Fase 7.903.
            while self.i < self.b.len() && self.b[self.i].is_ascii_alphanumeric() {
                self.i += 1;
            }
            let mut name = self.src[start..self.i].to_ascii_lowercase();
            // CSS no permite whitespace entre el nombre y `(`.
            if self.peek() != Some(b'(') {
                // Fase 7.871 — constantes numéricas de calc (CSS Values 4):
                // `pi`, `e`, `infinity`, `-infinity` (sin signo acá), `NaN`.
                return match name.as_str() {
                    "pi" => Some(CalcVal::Number(std::f32::consts::PI)),
                    "e" => Some(CalcVal::Number(std::f32::consts::E)),
                    "infinity" => Some(CalcVal::Number(f32::INFINITY)),
                    "nan" => Some(CalcVal::Number(f32::NAN)),
                    _ => None,
                };
            }
            self.i += 1;
            // Fase 7.854 — `round(<strategy>, A, B)` con estrategia opcional
            // (`nearest`/`up`/`down`/`to-zero`). Se consume acá porque es un
            // keyword, no una expresión; se anexa al name como `round:<strat>`.
            if name == "round" {
                if let Some(strat) = self.try_round_strategy() {
                    name = format!("round:{strat}");
                }
            }
            let args = self.args()?;
            return apply_math_fn(&name, &args);
        }
        self.number()
    }

    /// Intenta leer una estrategia de `round()` (`nearest`/`up`/`down`/
    /// `to-zero`) seguida de coma. Si lo que sigue NO es una de esas, no
    /// consume nada (devuelve `None`) y el primer arg se parsea como expr.
    fn try_round_strategy(&mut self) -> Option<&'static str> {
        let save = self.i;
        self.ws();
        let start = self.i;
        while self.i < self.b.len()
            && (self.b[self.i].is_ascii_alphabetic() || self.b[self.i] == b'-')
        {
            self.i += 1;
        }
        let word = self.src[start..self.i].to_ascii_lowercase();
        let strat = match word.as_str() {
            "nearest" => "nearest",
            "up" => "up",
            "down" => "down",
            "to-zero" => "to-zero",
            _ => {
                self.i = save;
                return None;
            }
        };
        self.ws();
        if self.peek() == Some(b',') {
            self.i += 1;
            Some(strat)
        } else {
            self.i = save;
            None
        }
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
    // Fase 7.875 — `<angle>` dentro de calc → `Number(grados)`. Conviven con
    // los números puros (un calc de ángulo da grados); en un contexto de
    // longitud el `Number` resultante se rechaza igual (no es una length).
    if let Some(deg) = token_angle_degrees(t) {
        return Some(CalcVal::Angle(deg));
    }
    // Fase 7.877 — `<time>` dentro de calc → `Number(segundos)`. Mismo modelo
    // que los ángulos: un calc de tiempo da segundos.
    if let Some(secs) = token_time_seconds(t) {
        return Some(CalcVal::Number(secs));
    }
    parse_length_px(t).map(|px| CalcVal::Length { px, pct: 0.0 })
}

/// `<time>` con unidad → segundos. `ms` antes que `s` (sufijo solapado).
/// Sin aceptar calc (evita recursión). Fase 7.877.
fn token_time_seconds(t: &str) -> Option<f32> {
    if let Some(r) = t.strip_suffix("ms") {
        return r.trim().parse::<f32>().ok().map(|v| v / 1000.0);
    }
    if let Some(r) = t.strip_suffix('s') {
        return r.trim().parse::<f32>().ok();
    }
    None
}

/// `<angle>` con unidad → grados, sin aceptar calc (para evitar recursión
/// dentro del propio evaluador). `deg`/`rad`/`grad`/`turn`. Fase 7.875.
fn token_angle_degrees(t: &str) -> Option<f32> {
    let (num, unit) = if let Some(r) = t.strip_suffix("deg") {
        (r, 1.0)
    } else if let Some(r) = t.strip_suffix("grad") {
        (r, 360.0 / 400.0)
    } else if let Some(r) = t.strip_suffix("rad") {
        (r, 180.0 / std::f32::consts::PI)
    } else if let Some(r) = t.strip_suffix("turn") {
        (r, 360.0)
    } else {
        return None;
    };
    num.trim().parse::<f32>().ok().map(|n| n * unit)
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
        // Fase 7.904 — `math` (CSS Fonts 4 / MathML): escalado automático por
        // nivel de script. Sin MathML, degrada a heredado (×1).
        "math" => return Some(DeclKind::FontSizeRel(1.0)),
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
        (CalcVal::Angle(x), CalcVal::Angle(y)) => Some(CalcVal::Angle(x + sign * y)),
        // Sumar número + longitud (o dimensiones distintas) es inválido en CSS.
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
        (CalcVal::Number(x), CalcVal::Angle(a)) | (CalcVal::Angle(a), CalcVal::Number(x)) => {
            Some(CalcVal::Angle(a * x))
        }
        // dimensión * dimensión (longitud²/ángulo²) es inválido.
        _ => None,
    }
}

fn calc_div(a: CalcVal, b: CalcVal) -> Option<CalcVal> {
    match (a, b) {
        (CalcVal::Number(x), CalcVal::Number(y)) if y != 0.0 => Some(CalcVal::Number(x / y)),
        (CalcVal::Length { px, pct }, CalcVal::Number(y)) if y != 0.0 => {
            Some(CalcVal::Length { px: px / y, pct: pct / y })
        }
        (CalcVal::Angle(a), CalcVal::Number(y)) if y != 0.0 => Some(CalcVal::Angle(a / y)),
        // ángulo / ángulo = número adimensional (CSS Values 4).
        (CalcVal::Angle(a), CalcVal::Angle(b)) if b != 0.0 => Some(CalcVal::Number(a / b)),
        _ => None,
    }
}

fn apply_math_fn(name: &str, args: &[CalcVal]) -> Option<CalcVal> {
    match name {
        "calc" => (args.len() == 1).then(|| args[0]),
        "min" => reduce_minmax(args, true),
        "max" => reduce_minmax(args, false),
        "clamp" if args.len() == 3 => clamp_calc(args[0], args[1], args[2]),
        // Fase 7.854 — funciones de paso. `round` con estrategia opcional
        // (`round:<strat>`); `mod`/`rem` toman exactamente 2 args.
        "round" if args.len() == 2 => stepped(args[0], args[1], "nearest"),
        n if n.starts_with("round:") && args.len() == 2 => {
            stepped(args[0], args[1], &n["round:".len()..])
        }
        "mod" if args.len() == 2 => modrem(args[0], args[1], true),
        "rem" if args.len() == 2 => modrem(args[0], args[1], false),
        // Fase 7.871 — `abs` preserva dimensión; `sign` devuelve número puro.
        "abs" if args.len() == 1 => Some(rebuild_like(args[0], scalar_of(args[0]).abs())),
        "sign" if args.len() == 1 => Some(CalcVal::Number(scalar_of(args[0]).signum_css())),
        // Exponenciales/raíz: operan sobre NÚMEROS puros (sin dimensión).
        "sqrt" if args.len() == 1 => num_fn(args, |a| a[0].sqrt()),
        "exp" if args.len() == 1 => num_fn(args, |a| a[0].exp()),
        "pow" if args.len() == 2 => num_fn(args, |a| a[0].powf(a[1])),
        "log" if args.len() == 1 => num_fn(args, |a| a[0].ln()),
        "log" if args.len() == 2 => num_fn(args, |a| a[0].log(a[1])),
        // Fase 7.903 — trig directas: arg `<angle>` (grados) o `<number>`
        // (radianes); devuelven número puro. `tan(90deg)` → ±∞.
        "sin" if args.len() == 1 => Some(CalcVal::Number(trig_angle_rad(args[0]).sin())),
        "cos" if args.len() == 1 => Some(CalcVal::Number(trig_angle_rad(args[0]).cos())),
        "tan" if args.len() == 1 => Some(CalcVal::Number(trig_angle_rad(args[0]).tan())),
        // Inversas: arg número puro; devuelven `<angle>` en grados.
        "asin" if args.len() == 1 => num_arg(args[0]).map(|n| CalcVal::Angle(n.asin().to_degrees())),
        "acos" if args.len() == 1 => num_arg(args[0]).map(|n| CalcVal::Angle(n.acos().to_degrees())),
        "atan" if args.len() == 1 => num_arg(args[0]).map(|n| CalcVal::Angle(n.atan().to_degrees())),
        // `atan2(y, x)`: dos args de la misma dimensión; ángulo en grados.
        "atan2" if args.len() == 2 => {
            let (y, x) = comparable_scalars(args[0], args[1])?;
            Some(CalcVal::Angle(y.atan2(x).to_degrees()))
        }
        // `hypot(a, b, ...)`: misma dimensión que el 1er arg.
        "hypot" if !args.is_empty() => {
            if !all_comparable(args) {
                return None;
            }
            let sum: f32 = args.iter().map(|v| scalar_of(*v).powi(2)).sum();
            Some(rebuild_like(args[0], sum.sqrt()))
        }
        _ => None,
    }
}

/// Número crudo o `calc()`/min/max/... que resuelva a un NÚMERO puro (sin
/// dimensión). Para props que toman `<number>` (opacity/flex-grow/order/
/// z-index). Un resultado con dimensión (px/%) → `None`. Fase 7.872.
pub(crate) fn parse_number_or_calc(s: &str) -> Option<f32> {
    let s = s.trim();
    if is_math_fn(s) {
        return match eval_calc(s)? {
            CalcVal::Number(n) => Some(n),
            _ => None,
        };
    }
    s.parse::<f32>().ok()
}

/// Escalar de un `CalcVal` (px+pct con uno en 0, o el número).
fn scalar_of(v: CalcVal) -> f32 {
    match v {
        CalcVal::Number(n) => n,
        CalcVal::Length { px, pct } => px + pct,
        CalcVal::Angle(a) => a,
    }
}

/// Argumento de una trig directa → radianes. Un `Angle` (grados) se convierte;
/// un `Number` puro ya está en radianes (CSS Values 4 §10). Una longitud cae a
/// su escalar (caso inválido que el caller no debería producir). Fase 7.903.
fn trig_angle_rad(v: CalcVal) -> f32 {
    match v {
        CalcVal::Angle(deg) => deg.to_radians(),
        other => scalar_of(other),
    }
}

/// Escalar si el arg es un número puro; `None` si tiene dimensión. Para las
/// trig inversas, que sólo aceptan `<number>`. Fase 7.903.
fn num_arg(v: CalcVal) -> Option<f32> {
    match v {
        CalcVal::Number(n) => Some(n),
        _ => None,
    }
}

/// Aplica `f` exigiendo que TODOS los args sean números puros (dimensionless).
fn num_fn(args: &[CalcVal], f: impl Fn(&[f32]) -> f32) -> Option<CalcVal> {
    let nums: Vec<f32> = args
        .iter()
        .map(|v| match v {
            CalcVal::Number(n) => Some(*n),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some(CalcVal::Number(f(&nums)))
}

trait SignumCss {
    /// `sign()` de CSS: -1/0/+1 (a diferencia de `f32::signum`, que para 0.0
    /// devuelve +1.0). `NaN` → 0.
    fn signum_css(self) -> f32;
}
impl SignumCss for f32 {
    fn signum_css(self) -> f32 {
        if self.is_nan() || self == 0.0 {
            0.0
        } else if self > 0.0 {
            1.0
        } else {
            -1.0
        }
    }
}

/// Aplica una función de dos args dimensionalmente comparables (`round`/`mod`/
/// `rem`), operando sobre el escalar resuelto y reconstruyendo el `CalcVal`
/// con la dimensión del primer arg. `None` si son incomparables o el paso es 0.
fn stepped(a: CalcVal, b: CalcVal, op: &str) -> Option<CalcVal> {
    let (va, vb) = comparable_scalars(a, b)?;
    if vb == 0.0 {
        return None; // round/mod/rem con paso 0 → indefinido (NaN en spec).
    }
    let r = match op {
        "nearest" => (va / vb).round() * vb,
        "up" => (va / vb).ceil() * vb,
        "down" => (va / vb).floor() * vb,
        "to-zero" => (va / vb).trunc() * vb,
        _ => return None,
    };
    Some(rebuild_like(a, r))
}

fn modrem(a: CalcVal, b: CalcVal, is_mod: bool) -> Option<CalcVal> {
    let (va, vb) = comparable_scalars(a, b)?;
    if vb == 0.0 {
        return None;
    }
    // `mod`: resultado con el signo del divisor (euclídeo-CSS). `rem`: signo
    // del dividendo (el `%` de Rust).
    let r = if is_mod {
        let m = va % vb;
        if m != 0.0 && (m < 0.0) != (vb < 0.0) { m + vb } else { m }
    } else {
        va % vb
    };
    Some(rebuild_like(a, r))
}

/// Extrae los escalares de dos `CalcVal` si son comparables (misma dimensión).
fn comparable_scalars(a: CalcVal, b: CalcVal) -> Option<(f32, f32)> {
    if !all_comparable(&[a, b]) {
        return None;
    }
    Some((scalar_of(a), scalar_of(b)))
}

/// Reconstruye un `CalcVal` con la dimensión de `like` y el escalar `r`.
fn rebuild_like(like: CalcVal, r: f32) -> CalcVal {
    match like {
        CalcVal::Number(_) => CalcVal::Number(r),
        CalcVal::Angle(_) => CalcVal::Angle(r),
        CalcVal::Length { pct, .. } if pct == 0.0 => CalcVal::Length { px: r, pct: 0.0 },
        CalcVal::Length { .. } => CalcVal::Length { px: 0.0, pct: r },
    }
}

/// `true` si todos los valores son comparables (misma dimensión): todos
/// número, todos px puro, o todos pct puro.
fn all_comparable(vs: &[CalcVal]) -> bool {
    vs.iter().all(|v| matches!(v, CalcVal::Number(_)))
        || vs.iter().all(|v| matches!(v, CalcVal::Angle(_)))
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
    let best = args.iter().map(|v| scalar_of(*v)).reduce(pick)?;
    Some(rebuild_like(first, best))
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
    // Fase 7.865 — `calc()`/min/max/clamp. Un número crudo es el multiplicador;
    // una longitud px se normaliza a múltiplo del font-size base (16px).
    if is_math_fn(s) {
        return match eval_calc(s)? {
            CalcVal::Number(n) => Some(n),
            CalcVal::Length { px, pct } if pct == 0.0 => Some(px / 16.0),
            _ => None,
        };
    }
    // Fase 7.873 — `line-height: <percentage>`. Relativo al font-size; como
    // multiplicador, `150%` = 1.5.
    if let Some(num) = s.strip_suffix('%') {
        return num.trim().parse::<f32>().ok().map(|p| p / 100.0);
    }
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
