//! Style engine — parser CSS minimal sobre `cssparser`.
//!
//! Para Fase 2 soportamos sólo:
//! - selectores type (`p`, `div`, `h1`) y universal (`*`)
//! - propiedades `color`, `background-color`, `display`, `font-size`,
//!   `margin`, `padding`
//! - inline `style="..."` en cada elemento
//!
//! No hay cascada con especificidad real ni `!important`. Stylo entero
//! entra en Fase 3 cuando el chrome Llimphi consuma estilos jerárquicos
//! complejos. Por ahora, una pasada de regla→nodo con override por
//! inline style alcanza para renderizar páginas simples (example.com,
//! landing del propio repo).

use markup5ever_rcdom::Handle;

use crate::boxes::{Color, Display};
use crate::dom::{self, DomTree};

/// Estilo computado por nodo. Defaults razonables — un nodo sin reglas
/// que matchen igual produce un box renderizable (texto negro sobre
/// transparente).
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: Color,
    pub background: Option<Color>,
    pub font_size: f32,
    pub font_weight: u16,
    pub margin: f32,
    pub padding: f32,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::Inline,
            color: Color::BLACK,
            background: None,
            font_size: 16.0,
            font_weight: 400,
            margin: 0.0,
            padding: 0.0,
        }
    }
}

/// Almacena reglas parseadas + función de "computar para nodo".
pub struct StyleEngine {
    rules: Vec<Rule>,
}

impl StyleEngine {
    /// Construye el engine desde el DOM: parsea cada `<style>` inline +
    /// inyecta el UA stylesheet (los defaults HTML que cssparser no
    /// conoce).
    pub fn from_dom(dom: &DomTree) -> Self {
        let mut rules = ua_stylesheet();
        for sheet in dom.collect_inline_stylesheets() {
            rules.extend(parse_stylesheet(&sheet));
        }
        Self { rules }
    }

    /// Computa el estilo de un nodo Element. Aplica en orden: UA →
    /// stylesheets del documento → atributo `style="..."`. El último
    /// gana (cascada simplificada).
    pub fn compute(&self, node: &Handle) -> ComputedStyle {
        let mut style = ComputedStyle::default();
        let Some(local) = dom::element_name(node) else {
            return style;
        };
        // Defaults por tag — `div`/`p`/`h1` son block.
        style.display = default_display(&local);

        // Defaults por tag para weight (h1..h6 y b/strong = bold) antes
        // de la cascada — cualquier regla de autor las puede override.
        style.font_weight = default_weight(&local);

        for rule in &self.rules {
            if rule.matches(node) {
                rule.apply(&mut style);
            }
        }

        if let Some(inline) = dom::attr(node, "style") {
            for decl in parse_declarations(&inline) {
                decl.apply(&mut style);
            }
        }
        style
    }
}

#[derive(Debug)]
struct Rule {
    selector: Selector,
    decls: Vec<Decl>,
}

/// Selector encadenado — alterna compound + combinador. `compounds[0]`
/// es el ancestro/hermano más lejano; `compounds.last()` es el sujeto.
/// `combinators[i]` es el combinador entre `compounds[i]` y
/// `compounds[i+1]`.
#[derive(Debug, Clone)]
struct Selector {
    compounds: Vec<Compound>,
    combinators: Vec<Combinator>,
}

/// Combinador CSS entre dos compounds consecutivos.
#[derive(Debug, Clone, Copy)]
enum Combinator {
    /// Whitespace — descendiente cualquier nivel.
    Descendant,
    /// `>` — hijo directo.
    Child,
    /// `+` — hermano adyacente inmediato.
    AdjacentSibling,
    /// `~` — hermano general (posterior, mismo padre).
    GeneralSibling,
}

/// Simple compound — un Tag + 0..N ids/clases/atributos/pseudoclases en
/// cadena (sin espacios). Ejemplos válidos: `a.btn`, `p#hero.alert`,
/// `input[type="checkbox"]`, `li:first-child`, `a[href^="https"]:last-of-type`.
#[derive(Debug, Clone)]
struct Compound {
    tag: TagPart,
    ids: Vec<String>,
    classes: Vec<String>,
    attrs: Vec<AttrMatch>,
    pseudos: Vec<Pseudo>,
}

#[derive(Debug, Clone)]
enum TagPart {
    Universal,
    Type(String),
}

#[derive(Debug, Clone)]
struct AttrMatch {
    name: String,
    op: AttrOp,
    value: String,
}

#[derive(Debug, Clone, Copy)]
enum AttrOp {
    /// `[attr]` — sólo presencia
    Present,
    /// `[attr=value]` — igualdad exacta
    Equals,
    /// `[attr^=value]` — empieza con
    Prefix,
    /// `[attr$=value]` — termina con
    Suffix,
    /// `[attr*=value]` — contiene substring
    Contains,
}

/// Pseudoclases estructurales — puramente posicionales, sin estado de UI.
/// `:hover`, `:focus`, `:active` siguen sin soporte porque requieren
/// tracking del chrome.
#[derive(Debug, Clone, Copy)]
enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
}

impl Compound {
    fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        let Some(local) = dom::element_name(node) else {
            return false;
        };
        if let TagPart::Type(t) = &self.tag {
            if !t.eq_ignore_ascii_case(&local) {
                return false;
            }
        }
        for want in &self.ids {
            if dom::attr(node, "id").as_deref() != Some(want.as_str()) {
                return false;
            }
        }
        if !self.classes.is_empty() {
            let attr = dom::attr(node, "class").unwrap_or_default();
            let present: Vec<&str> = attr.split_whitespace().collect();
            for want in &self.classes {
                if !present.iter().any(|c| c == want) {
                    return false;
                }
            }
        }
        for am in &self.attrs {
            if !attr_matches(node, am) {
                return false;
            }
        }
        for p in &self.pseudos {
            if !pseudo_matches(node, *p) {
                return false;
            }
        }
        true
    }
}

fn attr_matches(node: &markup5ever_rcdom::Handle, am: &AttrMatch) -> bool {
    let actual = dom::attr(node, &am.name);
    match am.op {
        AttrOp::Present => actual.is_some(),
        op => {
            let Some(actual) = actual else { return false };
            match op {
                AttrOp::Equals => actual == am.value,
                AttrOp::Prefix => actual.starts_with(&am.value),
                AttrOp::Suffix => actual.ends_with(&am.value),
                AttrOp::Contains => actual.contains(&am.value),
                AttrOp::Present => unreachable!(),
            }
        }
    }
}

fn pseudo_matches(node: &markup5ever_rcdom::Handle, p: Pseudo) -> bool {
    let Some(parent) = parent_of(node) else { return false };
    let kids = parent.children.borrow();
    let mut elems: Vec<markup5ever_rcdom::Handle> = Vec::new();
    for c in kids.iter() {
        if dom::element_name(c).is_some() {
            elems.push(c.clone());
        }
    }
    let Some(pos) = elems.iter().position(|c| std::rc::Rc::ptr_eq(c, node)) else {
        return false;
    };
    match p {
        Pseudo::FirstChild => pos == 0,
        Pseudo::LastChild => pos + 1 == elems.len(),
        Pseudo::OnlyChild => elems.len() == 1,
        Pseudo::FirstOfType => {
            let my_tag = dom::element_name(node).unwrap_or_default();
            elems[..pos]
                .iter()
                .all(|c| dom::element_name(c).map(|t| t != my_tag).unwrap_or(true))
        }
        Pseudo::LastOfType => {
            let my_tag = dom::element_name(node).unwrap_or_default();
            elems[pos + 1..]
                .iter()
                .all(|c| dom::element_name(c).map(|t| t != my_tag).unwrap_or(true))
        }
    }
}

impl Rule {
    fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        let compounds = &self.selector.compounds;
        if compounds.is_empty() {
            return false;
        }
        // El sujeto (último) debe matchear el nodo.
        if !compounds.last().unwrap().matches(node) {
            return false;
        }
        if compounds.len() == 1 {
            return true;
        }
        // Avanzamos derecha→izquierda, encadenando combinadores. Cada
        // combinador define cómo viajar al "siguiente" candidato:
        //   Descendant/Child  → ancestro
        //   Adjacent/General  → hermano anterior
        let combs = &self.selector.combinators;
        // El combinador entre compounds[i-1] y compounds[i] vive en
        // combs[i-1]. Recorremos desde compounds[len-2] hacia 0.
        let mut subject = node.clone();
        let mut i = compounds.len() - 1;
        while i > 0 {
            let comb = combs[i - 1];
            let target = &compounds[i - 1];
            match comb {
                Combinator::Child => {
                    let Some(p) = parent_of(&subject) else { return false };
                    if !target.matches(&p) {
                        return false;
                    }
                    subject = p;
                }
                Combinator::Descendant => {
                    let mut cur = parent_of(&subject);
                    loop {
                        let Some(n) = cur else { return false };
                        if target.matches(&n) {
                            subject = n;
                            break;
                        }
                        cur = parent_of(&n);
                    }
                }
                Combinator::AdjacentSibling => {
                    let Some(prev) = prev_element_sibling(&subject) else { return false };
                    if !target.matches(&prev) {
                        return false;
                    }
                    subject = prev;
                }
                Combinator::GeneralSibling => {
                    let mut cur = prev_element_sibling(&subject);
                    loop {
                        let Some(n) = cur else { return false };
                        if target.matches(&n) {
                            subject = n;
                            break;
                        }
                        cur = prev_element_sibling(&n);
                    }
                }
            }
            i -= 1;
        }
        true
    }
    fn apply(&self, style: &mut ComputedStyle) {
        for d in &self.decls {
            d.apply(style);
        }
    }
}

fn parent_of(node: &markup5ever_rcdom::Handle) -> Option<markup5ever_rcdom::Handle> {
    let weak = node.parent.take();
    let restored = weak.clone();
    node.parent.set(restored);
    weak.and_then(|w| w.upgrade())
}

/// Hermano Element anterior (saltea texto/whitespace nodes). Devuelve
/// `None` si no hay padre o si no hay Element previo bajo el mismo padre.
fn prev_element_sibling(
    node: &markup5ever_rcdom::Handle,
) -> Option<markup5ever_rcdom::Handle> {
    let parent = parent_of(node)?;
    let kids = parent.children.borrow();
    let mut last_elem: Option<markup5ever_rcdom::Handle> = None;
    for child in kids.iter() {
        if std::rc::Rc::ptr_eq(child, node) {
            return last_elem;
        }
        if dom::element_name(child).is_some() {
            last_elem = Some(child.clone());
        }
    }
    None
}

#[derive(Debug, Clone)]
enum Decl {
    Color(Color),
    Background(Color),
    Display(Display),
    FontSize(f32),
    FontWeight(u16),
    Margin(f32),
    Padding(f32),
}

impl Decl {
    fn apply(&self, s: &mut ComputedStyle) {
        match self {
            Decl::Color(c) => s.color = *c,
            Decl::Background(c) => s.background = Some(*c),
            Decl::Display(d) => s.display = *d,
            Decl::FontSize(v) => s.font_size = *v,
            Decl::FontWeight(w) => s.font_weight = *w,
            Decl::Margin(v) => s.margin = *v,
            Decl::Padding(v) => s.padding = *v,
        }
    }
}

fn default_display(tag: &str) -> Display {
    match tag {
        "html" | "body" | "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol"
        | "li" | "header" | "footer" | "section" | "article" | "nav" | "main" | "aside"
        | "form" | "pre" | "blockquote" | "hr" => Display::Block,
        "head" | "title" | "style" | "script" | "meta" | "link" => Display::None,
        _ => Display::Inline,
    }
}

fn default_weight(tag: &str) -> u16 {
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "b" | "strong" | "th" => 700,
        _ => 400,
    }
}

/// UA stylesheet mínimo — defaults HTML5 que cssparser por sí solo no
/// inyecta. Mantén corto: sólo lo necesario para no devolver páginas
/// "blancas" sin reglas autor.
fn ua_stylesheet() -> Vec<Rule> {
    fn ty(s: &str) -> Selector {
        Selector {
            compounds: vec![Compound {
                tag: TagPart::Type(s.into()),
                ids: vec![],
                classes: vec![],
                attrs: vec![],
                pseudos: vec![],
            }],
            combinators: vec![],
        }
    }
    vec![
        Rule { selector: ty("h1"), decls: vec![Decl::FontSize(32.0), Decl::Margin(20.0)] },
        Rule { selector: ty("h2"), decls: vec![Decl::FontSize(24.0), Decl::Margin(18.0)] },
        Rule { selector: ty("p"), decls: vec![Decl::Margin(12.0)] },
        Rule { selector: ty("body"), decls: vec![Decl::Padding(8.0)] },
    ]
}

// ----- parser -----
//
// Para Fase 2 no usamos cssparser AtRule/QualifiedRule (su API rotó
// entre 0.33→0.35 y nuestro subset cabe en 30 líneas). Si Fase 3 mete
// nesting / `@media` / `!important`, migrar a `cssparser::StyleSheetParser`
// con un visitor.

fn parse_stylesheet(css: &str) -> Vec<Rule> {
    let mut out = Vec::new();
    // Strip comentarios /* ... */
    let css = strip_comments(css);
    let bytes = css.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Encuentra '{' que abre el body.
        let Some(brace) = css[i..].find('{') else { break };
        let sel_raw = css[i..i + brace].trim();
        i += brace + 1;
        let Some(close) = css[i..].find('}') else { break };
        let body = &css[i..i + close];
        i += close + 1;
        if sel_raw.is_empty() {
            continue;
        }
        // Selectores múltiples separados por ',': uno por uno.
        for sel in sel_raw.split(',') {
            let sel = sel.trim();
            let Some(selector) = parse_selector(sel) else {
                // Selectores no soportados (combinadores, pseudoclases,
                // atributos) ignoramos en silencio — la regla queda
                // inerte y el documento sigue parseando.
                continue;
            };
            out.push(Rule { selector, decls: parse_declarations(body) });
        }
    }
    out
}

/// Parsea un selector encadenado. Soporta:
/// - simples compound: `*`, `tag`, `.class`, `#id`, `a.btn`, `p#hero.alert`
/// - selectores de atributo: `[href]`, `[type="text"]`, `[href^="https"]`,
///   `[src$=".png"]`, `[class*="foo"]`
/// - pseudoclases estructurales: `:first-child`, `:last-child`,
///   `:only-child`, `:first-of-type`, `:last-of-type`
/// - combinadores: descendiente (whitespace), hijo directo `>`,
///   hermano adyacente `+`, hermano general `~`
///
/// Pseudoclases de estado (`:hover`, `:focus`, `:active`), `:not(...)`,
/// `:nth-child(...)` y pseudo-elementos (`::before`) siguen sin soporte —
/// el selector entero se ignora si los menciona.
fn parse_selector(sel: &str) -> Option<Selector> {
    let sel = sel.trim();
    if sel.is_empty() {
        return Some(Selector {
            compounds: vec![Compound {
                tag: TagPart::Universal,
                ids: vec![],
                classes: vec![],
                attrs: vec![],
                pseudos: vec![],
            }],
            combinators: vec![],
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
    for tok in normalized.split_whitespace() {
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
    // Si el selector terminó con un combinador colgado, es inválido.
    if pending_combinator.is_some() {
        return None;
    }
    Some(Selector { compounds, combinators })
}

/// Inserta espacios alrededor de `>`/`+`/`~` para que `split_whitespace`
/// los aísle como tokens propios. Si caen dentro de `[…]` o `(…)` los
/// dejamos intactos — `[href*="a>b"]` o `:not(a+b)` deben pasar al
/// compound parser sin romperse.
fn normalize_combinators(sel: &str) -> String {
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
fn parse_compound(sel: &str) -> Option<Compound> {
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
                // Pseudoclases con paréntesis (`:not(...)`, `:nth-child(...)`)
                // no soportadas todavía — rechazamos el selector entero.
                if i < bytes.len() && bytes[i] == b'(' {
                    return None;
                }
                let p = match name.as_str() {
                    "first-child" => Pseudo::FirstChild,
                    "last-child" => Pseudo::LastChild,
                    "only-child" => Pseudo::OnlyChild,
                    "first-of-type" => Pseudo::FirstOfType,
                    "last-of-type" => Pseudo::LastOfType,
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

/// Parsea el interior de `[...]`: `name`, `name=val`, `name="val"`,
/// `name^=val`, `name$=val`, `name*=val`. Devuelve `None` si el formato
/// no encaja.
fn parse_attr_match(inner: &str) -> Option<AttrMatch> {
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
            let raw = inner[pos + sym.len()..].trim();
            let value = raw.trim_matches(|c| c == '"' || c == '\'').to_string();
            return Some(AttrMatch { name, op: *op, value });
        }
    }
    Some(AttrMatch {
        name: inner.to_string(),
        op: AttrOp::Present,
        value: String::new(),
    })
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            if let Some(end) = css[i + 2..].find("*/") {
                i += 2 + end + 2;
                continue;
            }
            break;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn parse_declarations(css: &str) -> Vec<Decl> {
    // Cada decl separada por `;`.
    let mut out = Vec::new();
    for chunk in css.split(';') {
        let Some((prop, value)) = chunk.split_once(':') else {
            continue;
        };
        if let Some(d) = decl_from_pair(prop.trim(), value.trim()) {
            out.push(d);
        }
    }
    out
}

fn decl_from_pair(prop: &str, value: &str) -> Option<Decl> {
    match prop.to_ascii_lowercase().as_str() {
        "color" => parse_color(value).map(Decl::Color),
        "background-color" | "background" => parse_color(value).map(Decl::Background),
        "display" => parse_display(value).map(Decl::Display),
        "font-size" => parse_length_px(value).map(Decl::FontSize),
        "font-weight" => parse_weight(value).map(Decl::FontWeight),
        "margin" => parse_length_px(value).map(Decl::Margin),
        "padding" => parse_length_px(value).map(Decl::Padding),
        _ => None,
    }
}

fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    // hex #RRGGBB
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
    }
    // Nombres comunes — lista mínima.
    NAMED_COLORS.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, c)| *c)
}

const NAMED_COLORS: &[(&str, Color)] = &[
    ("black", Color::BLACK),
    ("white", Color::WHITE),
    ("red", Color::rgb_const(255, 0, 0)),
    ("green", Color::rgb_const(0, 128, 0)),
    ("blue", Color::rgb_const(0, 0, 255)),
    ("gray", Color::rgb_const(128, 128, 128)),
    ("silver", Color::rgb_const(192, 192, 192)),
    ("transparent", Color::TRANSPARENT),
];

fn parse_weight(s: &str) -> Option<u16> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        "lighter" => Some(300),
        "bolder" => Some(700),
        num => num.parse().ok(),
    }
}

fn parse_display(s: &str) -> Option<Display> {
    match s.trim().to_ascii_lowercase().as_str() {
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "inline-block" => Some(Display::InlineBlock),
        "none" => Some(Display::None),
        _ => None,
    }
}

/// Acepta `12px`, `1.5rem` (tratada como em*16), `0`. Sin unidad → px.
fn parse_length_px(s: &str) -> Option<f32> {
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
    s.parse().ok()
}

/// Indica que `cssparser` está enlazado aunque el subset actual no use
/// la API completa — la presencia del crate evita que `cargo` lo
/// pruebe y deja el camino abierto para Fase 3.
#[doc(hidden)]
pub fn _cssparser_anchor() {
    let _ = cssparser::ParserInput::new("");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_hex_color() {
        assert_eq!(parse_color("#ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("red"), Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn parsea_length() {
        assert_eq!(parse_length_px("12px"), Some(12.0));
        assert_eq!(parse_length_px("1.5em"), Some(24.0));
        assert_eq!(parse_length_px("0"), Some(0.0));
        assert_eq!(parse_length_px("xyz"), None);
    }

    #[test]
    fn parsea_regla_simple() {
        let rules = parse_stylesheet("p { color: red; font-size: 14px; }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.compounds.len(), 1);
        assert!(matches!(
            &rules[0].selector.compounds[0].tag,
            TagPart::Type(t) if t == "p"
        ));
        assert_eq!(rules[0].decls.len(), 2);
    }

    #[test]
    fn selector_compound_matchea() {
        // `a.btn` matchea sólo `<a class="btn">`.
        let html = r##"<html><head><style>a.btn{color:red}</style></head><body>
                <a class="btn" href="#">click</a>
                <a href="#">otro</a>
                <span class="btn">no soy a</span>
            </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut anchors = Vec::new();
        let mut spans = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("a") => anchors.push(n.clone()),
                Some("span") => spans.push(n.clone()),
                _ => {}
            }
        });
        assert_eq!(anchors.len(), 2);
        assert_eq!(spans.len(), 1);
        // anchors[0] tiene class="btn"
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(255, 0, 0));
        // anchors[1] sin class
        assert_eq!(eng.compute(&anchors[1]).color, Color::BLACK);
        // span.btn no es <a>
        assert_eq!(eng.compute(&spans[0]).color, Color::BLACK);
    }

    #[test]
    fn selector_hijo_directo_matchea() {
        // `ul > li` matchea `<li>` que es hijo *directo* de `<ul>`. Un
        // `<li>` dentro de `<ol>` adentro de `<ul>` no debe matchear.
        let html = r#"<html><head><style>ul > li{color:#0a0}</style></head>
            <body>
              <ul><li>directo</li></ul>
              <ol><li>indirecto</li></ol>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 2);
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_hermano_adyacente_matchea() {
        // `h2 + p` matchea sólo el primer `<p>` inmediatamente después
        // de un `<h2>`.
        let html = r#"<html><head><style>h2+p{color:#00f}</style></head>
            <body>
              <h2>t</h2><p>uno</p><p>dos</p>
              <p>aislado</p>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_hermano_general_matchea() {
        // `h2 ~ p` matchea TODOS los `<p>` hermanos posteriores a un `<h2>`.
        let html = r#"<html><head><style>h2~p{color:#00f}</style></head>
            <body>
              <p>antes</p><h2>t</h2><p>uno</p><span>x</span><p>dos</p>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        // El primero está antes del h2 → no aplica.
        assert_eq!(eng.compute(&ps[0]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[1]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn selector_descendiente_matchea() {
        // `.menu li` matchea sólo los `<li>` dentro de `.menu`.
        let html = r#"<html><head><style>.menu li{color:#00aa00}</style></head>
            <body>
              <ul class="menu"><li>uno</li><li>dos</li></ul>
              <ul><li>tres</li></ul>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 3);
        // Los dos primeros viven en .menu → verde
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[1]).color, Color::rgb(0, 0xaa, 0));
        // El tercero no
        assert_eq!(eng.compute(&lis[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_class_matchea() {
        let html = r#"<html><head><style>.alert{color:red}</style></head><body><p class="alert">x</p><p>y</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ps: Vec<_> = {
            let mut acc = Vec::new();
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::element_name(n).as_deref() == Some("p") {
                    acc.push(n.clone());
                }
            });
            acc
        };
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_id_matchea() {
        let html = r#"<html><head><style>#hero{color:#0000ff}</style></head><body><p id="hero">x</p><p>y</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_presente() {
        // `[href]` matchea cualquier elemento con atributo `href`.
        let html = r#"<html><head><style>[href]{color:red}</style></head>
            <body><a href="x">link</a><a>sin</a><span>no a</span></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut elems = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if matches!(
                crate::dom::element_name(n).as_deref(),
                Some("a") | Some("span")
            ) {
                elems.push(n.clone());
            }
        });
        // a[href] → rojo; a sin href → negro; span → negro
        assert_eq!(eng.compute(&elems[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&elems[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&elems[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_equals() {
        // `input[type="checkbox"]` matchea sólo el checkbox.
        let html = r##"<html><head><style>input[type="checkbox"]{color:#00aa00}</style></head>
            <body>
              <input type="checkbox">
              <input type="text">
              <input>
            </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut inputs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("input") {
                inputs.push(n.clone());
            }
        });
        assert_eq!(inputs.len(), 3);
        assert_eq!(eng.compute(&inputs[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&inputs[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&inputs[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_prefix_suffix_contains() {
        let html = r##"<html><head><style>
            a[href^="https"]{color:#00f}
            img[src$=".png"]{color:#0f0}
            div[class*="warn"]{color:#f00}
        </style></head>
        <body>
            <a href="https://x">seguro</a>
            <a href="http://x">inseguro</a>
            <img src="logo.png">
            <img src="logo.jpg">
            <div class="banner warn-strong">!!</div>
            <div class="banner">--</div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut anchors = Vec::new();
        let mut imgs = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| match crate::dom::element_name(n).as_deref() {
            Some("a") => anchors.push(n.clone()),
            Some("img") => imgs.push(n.clone()),
            Some("div") => divs.push(n.clone()),
            _ => {}
        });
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&anchors[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&imgs[0]).color, Color::rgb(0, 255, 0));
        assert_eq!(eng.compute(&imgs[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&divs[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&divs[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_first_last_only_child() {
        let html = r#"<html><head><style>
            li:first-child{color:#00f}
            li:last-child{background:#0f0}
            p:only-child{color:#f0f}
        </style></head>
        <body>
          <ul><li>a</li><li>b</li><li>c</li></ul>
          <section><p>solo</p></section>
          <section><p>uno</p><p>dos</p></section>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| match crate::dom::element_name(n).as_deref() {
            Some("li") => lis.push(n.clone()),
            Some("p") => ps.push(n.clone()),
            _ => {}
        });
        // li:first-child sólo el primero
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
        // li:last-child sólo el tercero (background)
        assert!(eng.compute(&lis[0]).background.is_none());
        assert_eq!(eng.compute(&lis[2]).background, Some(Color::rgb(0, 255, 0)));
        // p:only-child el primero (único en su section), no los otros dos
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(255, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_first_last_of_type() {
        let html = r#"<html><head><style>
            p:first-of-type{color:#00f}
            p:last-of-type{color:#0a0}
        </style></head>
        <body>
          <div>x</div>
          <p>uno</p>
          <span>y</span>
          <p>dos</p>
          <p>tres</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        // primer <p> → azul (es :first-of-type aunque haya <div> y <span> antes)
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        // del medio → ninguno (last gana cascada al último pero a este ninguno)
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        // último <p> → verde
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0, 0xaa, 0));
    }

    #[test]
    fn cascada_inline_sobrescribe() {
        let html = "<html><head><style>p { color: red }</style></head><body><p style='color:blue'>x</p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let style = eng.compute(&p);
        assert_eq!(style.color, Color::rgb(0, 0, 255));
    }
}
