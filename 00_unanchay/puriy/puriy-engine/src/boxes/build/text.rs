//! Helpers de texto/listas/enlaces: whitespace, text-transform, marcadores de lista, alfa/romano, href.
//! Extraído de `boxes/build.rs` (regla #1). Sin cambios de lógica.
use super::*;

/// Colapso de whitespace según `white-space`:
/// - `Normal` / `NoWrap`: runs internos → un espacio, leading/trailing
///   reducidos a uno; newlines colapsan igual.
/// - `Pre`: todo preservado.
/// - `PreWrap`: igual que Pre — el wrap es responsabilidad del layout.
/// - `PreLine`: runs de espacio/tab → un espacio, newlines preservados.
pub(crate) fn collapse_whitespace(s: &str, ws: WhiteSpace) -> String {
    match ws {
        WhiteSpace::Pre | WhiteSpace::PreWrap => s.to_string(),
        WhiteSpace::PreLine => {
            // Colapsa espacios/tabs (no '\n') a uno solo, preserva newlines.
            let mut out = String::with_capacity(s.len());
            let mut prev_space = false;
            for c in s.chars() {
                if c == '\n' {
                    out.push(c);
                    prev_space = false;
                } else if c.is_whitespace() {
                    if !prev_space {
                        out.push(' ');
                        prev_space = true;
                    }
                } else {
                    out.push(c);
                    prev_space = false;
                }
            }
            out
        }
        WhiteSpace::Normal | WhiteSpace::NoWrap => {
            let leading = s.chars().next().is_some_and(|c| c.is_whitespace());
            let trailing = s.chars().last().is_some_and(|c| c.is_whitespace());
            let core: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
            if core.is_empty() {
                // Sólo whitespace: lo dejamos como " " para no perder el
                // separador entre inlines vecinos.
                return if leading || trailing { " ".to_string() } else { String::new() };
            }
            let mut out = String::with_capacity(core.len() + 2);
            if leading {
                out.push(' ');
            }
            out.push_str(&core);
            if trailing {
                out.push(' ');
            }
            out
        }
    }
}

/// Aplica `text-transform` al texto. Capitalize convierte la primera
/// letra de cada palabra (separada por whitespace) a mayúscula.
pub(crate) fn apply_text_transform(s: String, t: TextTransform) -> String {
    match t {
        TextTransform::None => s,
        TextTransform::Uppercase => s.to_uppercase(),
        TextTransform::Lowercase => s.to_lowercase(),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(s.len());
            let mut start_of_word = true;
            for c in s.chars() {
                if c.is_whitespace() {
                    out.push(c);
                    start_of_word = true;
                } else if start_of_word {
                    out.extend(c.to_uppercase());
                    start_of_word = false;
                } else {
                    out.push(c);
                }
            }
            out
        }
    }
}

/// Construye el texto del marker de un `<li>`. Para tipos numerados
/// (`decimal`/`*-alpha`/`*-roman`) calcula la posición del item entre sus
/// hermanos `<li>` del mismo padre, respetando `<ol start>` y
/// `<li value>`. Devuelve `None` si `list-style-type: none`.
///
/// Marcadores con número usan `"N. "` (período + un espacio) — alineado
/// con el comportamiento de browsers. Marcadores con símbolo usan
/// `"<sym>  "` (doble espacio) para dar el airecito que tenía el bullet
/// hardcoded original.
pub(crate) fn li_marker(
    node: &Handle,
    kind: &ListStyleType,
    counter_styles: &[crate::style::CounterStyleRule],
) -> Option<String> {
    match kind {
        ListStyleType::None => None,
        // Fase 7.1216 — marcador string literal (verbatim, el autor controla
        // el espaciado). String vacío `""` suprime el marcador.
        ListStyleType::Str(s) => {
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        // Fase 7.1218 — `@counter-style` referenciado por nombre. Si no está
        // registrado (o su sistema no lo soportamos), cae a `decimal`.
        ListStyleType::Named(name) => Some(
            format_named_counter(name, ol_item_position(node), counter_styles, 0)
                .unwrap_or_else(|| format!("{}. ", ol_item_position(node))),
        ),
        ListStyleType::Disc => Some("• ".into()),
        ListStyleType::Circle => Some("◦ ".into()),
        ListStyleType::Square => Some("▪ ".into()),
        ListStyleType::Decimal => Some(format!("{}. ", ol_item_position(node))),
        ListStyleType::LowerAlpha => {
            Some(format!("{}. ", to_alpha(ol_item_position(node), false)))
        }
        ListStyleType::UpperAlpha => {
            Some(format!("{}. ", to_alpha(ol_item_position(node), true)))
        }
        ListStyleType::LowerRoman => {
            Some(format!("{}. ", to_roman(ol_item_position(node), false)))
        }
        ListStyleType::UpperRoman => {
            Some(format!("{}. ", to_roman(ol_item_position(node), true)))
        }
    }
}

/// Quita comillas simples/dobles de un descriptor de `@counter-style`.
fn unq(s: &str) -> &str {
    let t = s.trim();
    if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')))
    {
        &t[1..t.len() - 1]
    } else {
        t
    }
}

/// Tokeniza la lista `symbols` de un `@counter-style` en símbolos individuales.
/// Tokens separados por whitespace; cada uno puede ir entre comillas
/// (`"◆" "◇"`) o suelto (`A B C`). Las comillas se retiran.
fn parse_symbols(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.push(c);
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                } else if c.is_whitespace() {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                } else {
                    cur.push(c);
                }
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Formatea `n` con el `@counter-style` registrado bajo `name` (CSS Counter
/// Styles 3). Soporta los sistemas symbols-based más comunes —`cyclic`,
/// `symbolic`, `fixed`— con `prefix`/`suffix` (suffix default `". "`). Sistemas
/// `numeric`/`alphabetic`/`additive`/`extends` u otros casos (símbolos vacíos,
/// fuera de rango en `fixed`/`symbolic`) caen a `fallback` (recursivo, con tope
/// anti-ciclo) o, en última instancia, devuelven `None` para que el caller use
/// `decimal`. Fase 7.1218.
pub(crate) fn format_named_counter(
    name: &str,
    n: i32,
    styles: &[crate::style::CounterStyleRule],
    depth: u8,
) -> Option<String> {
    if depth > 8 {
        return None; // tope anti-ciclo → el caller usa decimal
    }
    let rule = styles.iter().find(|r| r.name.eq_ignore_ascii_case(name))?;
    let fb = |styles: &[crate::style::CounterStyleRule]| {
        rule.fallback
            .as_deref()
            .map(|f| f.trim())
            .filter(|f| !f.is_empty())
            .and_then(|f| format_named_counter(f, n, styles, depth + 1))
    };
    let system_raw = rule.system.as_deref().unwrap_or("symbolic");
    let mut sysw = system_raw.split_whitespace();
    let sys = sysw.next().unwrap_or("symbolic").to_ascii_lowercase();
    let symbols = parse_symbols(rule.symbols.as_deref().unwrap_or(""));
    let len = symbols.len() as i32;
    if len == 0 {
        return fb(styles);
    }
    let core = match sys.as_str() {
        // Recorre los símbolos cíclicamente; admite n<=0 vía rem_euclid.
        "cyclic" => symbols[((n - 1).rem_euclid(len)) as usize].clone(),
        // Cada símbolo una vez desde `first` (default 1); fuera de rango → fallback.
        "fixed" => {
            let first: i32 = sysw.next().and_then(|s| s.parse().ok()).unwrap_or(1);
            let idx = n - first;
            if idx < 0 || idx >= len {
                return fb(styles);
            }
            symbols[idx as usize].clone()
        }
        // Símbolo cíclico repetido ⌈n/len⌉ veces (sólo n>=1).
        "symbolic" => {
            if n < 1 {
                return fb(styles);
            }
            let idx = ((n - 1) % len) as usize;
            let count = ((n - 1) / len + 1) as usize;
            symbols[idx].repeat(count)
        }
        // numeric/alphabetic/additive/extends u otros: no modelados → fallback.
        _ => return fb(styles),
    };
    let prefix = rule.prefix.as_deref().map(unq).unwrap_or("");
    // suffix default ". " (initial del descriptor en CSS Counter Styles 3).
    let suffix = rule.suffix.as_deref().map(unq).unwrap_or(". ");
    Some(format!("{prefix}{core}{suffix}"))
}

/// Posición 1-indexed del `<li>` entre sus hermanos `<li>` del padre.
/// Respeta `<ol start="N">` (arranca el contador en N) y `<li value="N">`
/// (resetea el contador al valor dado para ese item y los siguientes).
/// Si `node` no es un `<li>` o no tiene padre, devuelve 1.
pub(crate) fn ol_item_position(node: &Handle) -> i32 {
    let Some(parent) = parent_handle(node) else { return 1 };
    let parent_is_ol = dom::element_name(&parent).as_deref() == Some("ol");
    let mut counter: i32 = if parent_is_ol {
        dom::attr(&parent, "start").and_then(|s| s.trim().parse().ok()).unwrap_or(1)
    } else {
        1
    };
    for child in parent.children.borrow().iter() {
        if dom::element_name(child).as_deref() != Some("li") {
            continue;
        }
        if let Some(v) = dom::attr(child, "value").and_then(|s| s.trim().parse::<i32>().ok()) {
            counter = v;
        }
        if std::rc::Rc::ptr_eq(child, node) {
            return counter;
        }
        counter += 1;
    }
    counter
}

/// Misma idea que `style::parent_of`. Lo duplicamos acá para no tocar
/// la visibilidad del helper en `style.rs`.
pub(crate) fn parent_handle(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let restored = weak.clone();
    node.parent.set(restored);
    weak.and_then(|w| w.upgrade())
}

/// Convierte 1..N a alpha bijectiva base-26 (1=a, 26=z, 27=aa, 28=ab…).
/// Valores `<= 0` caen a `"0"` — el marker numérico igual se imprime.
pub(crate) fn to_alpha(mut n: i32, upper: bool) -> String {
    if n <= 0 {
        return n.to_string();
    }
    let mut buf: Vec<u8> = Vec::new();
    while n > 0 {
        n -= 1;
        let d = (n % 26) as u8;
        buf.push(if upper { b'A' + d } else { b'a' + d });
        n /= 26;
    }
    buf.reverse();
    // SAFETY: sólo ASCII A-Z/a-z.
    String::from_utf8(buf).expect("alpha ascii-only")
}

/// Romanos 1..3999. Fuera del rango caemos a decimal — matchea el
/// comportamiento de browsers (Chromium también).
pub(crate) fn to_roman(n: i32, upper: bool) -> String {
    if !(1..=3999).contains(&n) {
        return n.to_string();
    }
    const VALUES: &[(i32, &str, &str)] = &[
        (1000, "M", "m"),
        (900, "CM", "cm"),
        (500, "D", "d"),
        (400, "CD", "cd"),
        (100, "C", "c"),
        (90, "XC", "xc"),
        (50, "L", "l"),
        (40, "XL", "xl"),
        (10, "X", "x"),
        (9, "IX", "ix"),
        (5, "V", "v"),
        (4, "IV", "iv"),
        (1, "I", "i"),
    ];
    let mut n = n;
    let mut out = String::new();
    for (val, up, lo) in VALUES {
        while n >= *val {
            out.push_str(if upper { up } else { lo });
            n -= val;
        }
    }
    out
}

pub(crate) fn resolve_href(base: Option<&url::Url>, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    // Schemes que NO son web: el chrome no debería intentar navegar a ellos.
    let lc = href.to_ascii_lowercase();
    if lc.starts_with("javascript:")
        || lc.starts_with("mailto:")
        || lc.starts_with("tel:")
        || lc.starts_with("sms:")
        || lc.starts_with("data:")
    {
        return None;
    }
    // Fragmentos puros (`#foo`): resuelven a la URL actual + fragment.
    // El chrome detecta same-page navigation (mismo URL sans fragment)
    // y scrollea al elemento con id matching en lugar de recargar.
    if href.starts_with('#') {
        return base.and_then(|b| b.join(href).ok()).map(|u| u.to_string());
    }
    if let Ok(abs) = url::Url::parse(href) {
        // Sólo http/https son navegables por puriy hoy. file://, ftp://,
        // etc. quedan ignorados para no romper la pestaña.
        return match abs.scheme() {
            "http" | "https" | "about" => Some(abs.into()),
            _ => None,
        };
    }
    base.and_then(|b| b.join(href).ok()).and_then(|abs| {
        match abs.scheme() {
            "http" | "https" | "about" => Some(abs.into()),
            _ => None,
        }
    })
}

