//! Parsing de selectores: `parse_selector`/`parse_compound`/`parse_attr_match`,
//! normalización de combinadores, strip de pseudo-elementos y de comentarios.
//! Sub-módulo de `parser` (regla #1). `use super::*`.
use super::*;

pub(crate) fn parse_selector(sel: &str) -> Option<Selector> {
    let sel = sel.trim();
    // Strip pseudo-element del final (`::before`/`::after`). CSS también
    // acepta la sintaxis legacy `:before`/`:after` con un sólo `:` —
    // las aceptamos por compatibilidad. Pueden venir adheridas al
    // último compound (`p::before`) o solas (`::before` matchea
    // implícitamente al universal).
    let (sel, pseudo_element) = strip_pseudo_element(sel);
    if sel.is_empty() {
        let compound = Compound {
            tag: TagPart::Universal,
            ids: vec![],
            classes: vec![],
            attrs: vec![],
            pseudos: vec![],
        };
        return Some(Selector {
            compounds: vec![compound],
            combinators: vec![],
            pseudo_element,
        });
    }
    // Tokenizamos: cada compound es una secuencia sin espacios ni
    // combinadores; los combinadores ('>', '+', '~') están separados por
    // whitespace en CSS canónico o pegados. Normalizamos respetando lo
    // que viva dentro de `[...]` o `(...)`.
    let normalized = normalize_combinators(sel);
    let mut compounds: Vec<Compound> = Vec::new();
    let mut combinators: Vec<Combinator> = Vec::new();
    let mut pending_combinator: Option<Combinator> = None;
    let mut first = true;
    for tok in split_ws_top_level(&normalized) {
        match tok {
            ">" => pending_combinator = Some(Combinator::Child),
            "+" => pending_combinator = Some(Combinator::AdjacentSibling),
            "~" => pending_combinator = Some(Combinator::GeneralSibling),
            _ => {
                let compound = parse_compound(tok)?;
                if first {
                    first = false;
                } else {
                    combinators.push(pending_combinator.take().unwrap_or(Combinator::Descendant));
                }
                compounds.push(compound);
            }
        }
    }
    if compounds.is_empty() {
        return None;
    }
    if pending_combinator.is_some() {
        return None;
    }
    Some(Selector { compounds, combinators, pseudo_element })
}

/// Si `sel` termina con `::before`/`::after` (o legacy `:before`/`:after`),
/// devuelve `(prefix, Some(PseudoElement))`. Sino devuelve `(sel, None)`.
pub(crate) fn strip_pseudo_element(sel: &str) -> (&str, Option<PseudoElement>) {
    let sel = sel.trim_end();
    // Fase 7.934 — pseudo-element moderno `::ident` o `::ident(args)` a nivel
    // superior (fuera de `[...]`/`(...)`). Tomamos el primer `::`; lo que sigue
    // es el nombre (+ args opcionales). `::before`/`::after` → variantes con
    // box; el resto (`::selection`, `::marker`, `::part()`…) → `Other` inerte.
    if let Some(pos) = find_double_colon(sel) {
        let prefix = &sel[..pos];
        let rest = sel[pos + 2..].trim();
        let name_end = rest.find('(').unwrap_or(rest.len());
        let name = rest[..name_end].trim().to_ascii_lowercase();
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            // `::` suelto o con basura → dejamos que el compound lo rechace.
            return (sel, None);
        }
        let pe = match name.as_str() {
            "before" => PseudoElement::Before,
            "after" => PseudoElement::After,
            _ => PseudoElement::Other,
        };
        return (prefix, Some(pe));
    }
    let lower = sel.to_ascii_lowercase();
    for (suffix, pe) in [
        (":before", PseudoElement::Before),
        (":after", PseudoElement::After),
        // CSS2 legacy de un solo `:` para los cuatro originales.
        (":first-line", PseudoElement::Other),
        (":first-letter", PseudoElement::Other),
    ] {
        if let Some(prefix) = lower.strip_suffix(suffix) {
            // Cuidado: `:before` no debe matchear cuando es parte de
            // `:before-leaf` (no es un pseudo válido en CSS). Pero al
            // ser sufijo exacto del string, esto no aplica acá. Sí
            // garantizamos que el prefijo no termine en alfanumérico
            // (caso `p:beforex` — el parseo falla al no encontrar
            // pseudoclase válida y lo rechazamos abajo). Acá basta.
            return (&sel[..prefix.len()], Some(pe));
        }
    }
    (sel, None)
}

/// Posición del primer `::` a nivel superior (fuera de `[...]` y `(...)`).
/// `None` si no hay. Fase 7.934.
fn find_double_colon(sel: &str) -> Option<usize> {
    let bytes = sel.as_bytes();
    let mut in_bracket = false;
    let mut paren_depth: u32 = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        match bytes[i] {
            b'[' => in_bracket = true,
            b']' => in_bracket = false,
            b'(' if !in_bracket => paren_depth += 1,
            b')' if !in_bracket => paren_depth = paren_depth.saturating_sub(1),
            b':' if !in_bracket && paren_depth == 0 && bytes[i + 1] == b':' => {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Inserta espacios alrededor de `>`/`+`/`~` para que `split_whitespace`
/// los aísle como tokens propios. Si caen dentro de `[…]` o `(…)` los
/// dejamos intactos — `[href*="a>b"]` o `:not(a+b)` deben pasar al
/// compound parser sin romperse.
pub(crate) fn normalize_combinators(sel: &str) -> String {
    let mut out = String::with_capacity(sel.len() + 4);
    let mut in_bracket = false;
    let mut paren_depth: u32 = 0;
    for c in sel.chars() {
        match c {
            '[' => {
                in_bracket = true;
                out.push(c);
            }
            ']' => {
                in_bracket = false;
                out.push(c);
            }
            '(' => {
                paren_depth += 1;
                out.push(c);
            }
            ')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                out.push(c);
            }
            '>' | '+' | '~' if !in_bracket && paren_depth == 0 => {
                out.push(' ');
                out.push(c);
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// Parsea un compound: opcional tag/`*` seguido de cualquier número de
/// `.class`, `#id`, `[attr...]`, `:pseudo`. Devuelve `None` si encuentra
/// caracteres no esperados, una pseudo no soportada, o `::pseudo-element`.
pub(crate) fn parse_compound(sel: &str) -> Option<Compound> {
    let bytes = sel.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0;
    // Tag opcional (puede ser `*` o un nombre).
    let tag = if bytes[0] == b'*' {
        i = 1;
        TagPart::Universal
    } else if is_ident_byte(bytes[0]) {
        let start = i;
        while i < bytes.len() && is_ident_byte(bytes[i]) {
            i += 1;
        }
        TagPart::Type(sel[start..i].to_string())
    } else {
        TagPart::Universal
    };
    let mut ids = Vec::new();
    let mut classes = Vec::new();
    let mut attrs = Vec::new();
    let mut pseudos = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'.' | b'#' => {
                let marker = bytes[i];
                i += 1;
                let start = i;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let ident = sel[start..i].to_string();
                if marker == b'.' {
                    classes.push(ident);
                } else {
                    ids.push(ident);
                }
            }
            b'[' => {
                let inner_start = i + 1;
                let rel_close = sel[inner_start..].find(']')?;
                let inner = &sel[inner_start..inner_start + rel_close];
                attrs.push(parse_attr_match(inner)?);
                i = inner_start + rel_close + 1;
            }
            b':' => {
                i += 1;
                // `::pseudo-element` (e.g. ::before) — rechazamos.
                if i < bytes.len() && bytes[i] == b':' {
                    return None;
                }
                let start = i;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let name = sel[start..i].to_ascii_lowercase();
                // Funcionales: `:nth-child(...)`, `:not(...)`. Detectamos
                // y consumimos los argumentos.
                if i < bytes.len() && bytes[i] == b'(' {
                    let arg_start = i + 1;
                    let rel_close = sel[arg_start..].find(')')?;
                    let arg = &sel[arg_start..arg_start + rel_close];
                    let p = match name.as_str() {
                        "nth-child" => {
                            // Fase 7.933 — `:nth-child(An+B of <sel>)` (CSS
                            // Selectors 4): parseamos el An+B e ignoramos el
                            // filtro `of <sel>` (aproximación: matchea sin el
                            // filtro de tipo). Mejor que tirar la regla.
                            let (a, b) = parse_nth_arg(nth_strip_of(arg))?;
                            Pseudo::NthChild { a, b }
                        }
                        "nth-of-type" => {
                            let (a, b) = parse_nth_arg(arg)?;
                            Pseudo::NthOfType { a, b }
                        }
                        "nth-last-child" => {
                            let (a, b) = parse_nth_arg(nth_strip_of(arg))?;
                            Pseudo::NthLastChild { a, b }
                        }
                        "nth-last-of-type" => {
                            let (a, b) = parse_nth_arg(arg)?;
                            Pseudo::NthLastOfType { a, b }
                        }
                        "not" => {
                            // CSS4: lista de compounds (`:not(.a, .b)`).
                            let mut inner = Vec::new();
                            for part in arg.split(',') {
                                let c = parse_compound(part.trim())?;
                                // Anti-recursión: `:not(:not(...))` rechazamos.
                                if c.pseudos.iter().any(|p| matches!(p, Pseudo::Not(_))) {
                                    return None;
                                }
                                inner.push(c);
                            }
                            if inner.is_empty() {
                                return None;
                            }
                            Pseudo::Not(inner)
                        }
                        "has" => {
                            // `:has(<rel-sel-list>)` — cada relative selector
                            // es un combinador opcional (descendiente por
                            // defecto) + un compound. Lista separada por coma.
                            let mut rels = Vec::new();
                            for part in arg.split(',') {
                                let part = part.trim();
                                if part.is_empty() {
                                    return None;
                                }
                                let (combinator, rest) = match part.as_bytes()[0] {
                                    b'>' => (Combinator::Child, part[1..].trim()),
                                    b'+' => (Combinator::AdjacentSibling, part[1..].trim()),
                                    b'~' => (Combinator::GeneralSibling, part[1..].trim()),
                                    _ => (Combinator::Descendant, part),
                                };
                                let compound = parse_compound(rest)?;
                                // Anti-recursión: no soportamos `:has` anidado.
                                if compound.pseudos.iter().any(|p| matches!(p, Pseudo::Has(_))) {
                                    return None;
                                }
                                rels.push(RelativeSelector { combinator, compound });
                            }
                            if rels.is_empty() {
                                return None;
                            }
                            Pseudo::Has(rels)
                        }
                        "lang" => {
                            // `:lang(en, fr)` — lista de tags de idioma.
                            let tags: Vec<String> = arg
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                            if tags.is_empty() {
                                return None;
                            }
                            Pseudo::Lang(tags)
                        }
                        "is" | "where" => {
                            // Lista de compounds separados por coma (sin
                            // combinadores adentro — `parse_compound` parsea
                            // uno solo). Split naive por `,` (no contempla
                            // comas dentro de `[attr="a,b"]`, caso raro).
                            let mut inner = Vec::new();
                            for part in arg.split(',') {
                                inner.push(parse_compound(part.trim())?);
                            }
                            if inner.is_empty() {
                                return None;
                            }
                            if name == "is" {
                                Pseudo::Is(inner)
                            } else {
                                Pseudo::Where(inner)
                            }
                        }
                        // Fase 7.933 — pseudo-clases funcionales estándar que
                        // reconocemos pero no evaluamos (custom states, shadow
                        // DOM, dir): inertes para no tirar la regla. `:dir()`
                        // real queda pendiente. El argumento debe ser no vacío.
                        "dir" | "state" | "host" | "host-context" => {
                            if arg.trim().is_empty() {
                                return None;
                            }
                            Pseudo::Inert(false)
                        }
                        _ => return None,
                    };
                    pseudos.push(p);
                    i = arg_start + rel_close + 1;
                    continue;
                }
                let p = match name.as_str() {
                    "first-child" => Pseudo::FirstChild,
                    "last-child" => Pseudo::LastChild,
                    "only-child" => Pseudo::OnlyChild,
                    "first-of-type" => Pseudo::FirstOfType,
                    "last-of-type" => Pseudo::LastOfType,
                    "only-of-type" => Pseudo::OnlyOfType,
                    "hover" => Pseudo::Hover,
                    "focus" | "focus-visible" | "focus-within" => Pseudo::Focus,
                    "checked" => Pseudo::Checked,
                    "disabled" => Pseudo::Disabled,
                    "enabled" => Pseudo::Enabled,
                    "required" => Pseudo::Required,
                    "optional" => Pseudo::Optional,
                    "read-only" => Pseudo::ReadOnly,
                    "read-write" => Pseudo::ReadWrite,
                    "empty" => Pseudo::Empty,
                    "root" => Pseudo::Root,
                    "link" | "any-link" => Pseudo::AnyLink,
                    // Fase 7.933 — pseudo-clases estándar que reconocemos para
                    // NO tirar la regla, pero que no podemos evaluar con el
                    // estado que rastreamos → inertes (no matchean nunca).
                    // Validación de formularios, rango, autofill, defaults:
                    "valid" | "invalid" | "in-range" | "out-of-range"
                    | "placeholder-shown" | "user-valid" | "user-invalid"
                    | "default" | "indeterminate" | "autofill" | "blank"
                    | "read-write-plaintext-only"
                    // Interacción/enlace que no rastreamos:
                    | "active" | "visited" | "target" | "target-within"
                    | "current" | "past" | "future" | "local-link"
                    // Estado de media/popover/dialog/fullscreen:
                    | "popover-open" | "modal" | "fullscreen" | "open" | "closed"
                    | "playing" | "paused" | "muted" | "seeking" | "buffering"
                    | "stalled" | "volume-locked" | "picture-in-picture"
                    // Shadow DOM (no implementado): host sin paréntesis.
                    | "host" => Pseudo::Inert(false),
                    // `:scope` sin contexto de scoping ≈ transparente (matchea
                    // al propio elemento): inerte que SIEMPRE matchea.
                    "scope" => Pseudo::Inert(true),
                    _ => return None,
                };
                pseudos.push(p);
            }
            _ => return None,
        }
    }
    if matches!(tag, TagPart::Universal)
        && ids.is_empty()
        && classes.is_empty()
        && attrs.is_empty()
        && pseudos.is_empty()
        && sel != "*"
    {
        return None;
    }
    Some(Compound { tag, ids, classes, attrs, pseudos })
}

/// Quita el filtro `of <selector>` de un argumento `:nth-child()` (CSS
/// Selectors 4), devolviendo sólo la parte `An+B`. `"2 of .item"` → `"2"`.
/// Fase 7.933.
fn nth_strip_of(arg: &str) -> &str {
    let lower = arg.to_ascii_lowercase();
    // Buscamos el token ` of ` (rodeado de whitespace) a nivel superior.
    if let Some(pos) = lower.find(" of ") {
        arg[..pos].trim()
    } else {
        arg.trim()
    }
}

/// Parsea el interior de `[...]`: `name`, `name=val`, `name="val"`,
/// `name^=val`, `name$=val`, `name*=val`. Devuelve `None` si el formato
/// no encaja.
pub(crate) fn parse_attr_match(inner: &str) -> Option<AttrMatch> {
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    let ops: &[(&str, AttrOp)] = &[
        ("^=", AttrOp::Prefix),
        ("$=", AttrOp::Suffix),
        ("*=", AttrOp::Contains),
        ("=", AttrOp::Equals),
    ];
    for (sym, op) in ops {
        if let Some(pos) = inner.find(sym) {
            let name = inner[..pos].trim().to_string();
            if name.is_empty() {
                return None;
            }
            let mut raw = inner[pos + sym.len()..].trim();
            // Flag de case-sensitivity CSS4: ` i` (insensible) / ` s`
            // (sensible, default). Es un token suelto al final, separado por
            // whitespace del valor (`[a=b i]`, `[a="b" i]`).
            let mut case_insensitive = false;
            for (flag, ci) in [('i', true), ('I', true), ('s', false), ('S', false)] {
                if let Some(stripped) = raw.strip_suffix(flag) {
                    if stripped.ends_with(char::is_whitespace) {
                        case_insensitive = ci;
                        raw = stripped.trim_end();
                        break;
                    }
                }
            }
            let value = raw.trim_matches(|c| c == '"' || c == '\'').to_string();
            return Some(AttrMatch { name, op: *op, value, case_insensitive });
        }
    }
    Some(AttrMatch {
        name: inner.to_string(),
        op: AttrOp::Present,
        value: String::new(),
        case_insensitive: false,
    })
}

pub(crate) fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

pub(crate) fn strip_comments(css: &str) -> String {
    // Operamos a nivel byte para detectar `/*…*/`, pero copiamos slices
    // de la `&str` original para preservar UTF-8 multi-byte (un push de
    // bytes individuales `as char` rompe runs no-ASCII como "▸").
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    let mut chunk_start = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // Volcamos el chunk pendiente antes del comentario.
            out.push_str(&css[chunk_start..i]);
            if let Some(end) = css[i + 2..].find("*/") {
                i += 2 + end + 2;
                chunk_start = i;
                continue;
            }
            return out;
        }
        i += 1;
    }
    out.push_str(&css[chunk_start..]);
    out
}
