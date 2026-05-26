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

/// Selector compound — secuencia de simples conectada por combinador
/// "descendiente" (whitespace en CSS). `parts[0]` = ancestro más
/// lejano; `parts.last()` = sujeto. Sin combinadores `>`/`+`/`~`, sin
/// pseudoclases — la fase actual sólo necesita descendientes.
#[derive(Debug, Clone)]
struct Selector {
    parts: Vec<Simple>,
}

#[derive(Debug, Clone)]
enum Simple {
    Universal,
    Type(String),
    Id(String),
    Class(String),
}

impl Simple {
    fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        let Some(local) = dom::element_name(node) else {
            return false;
        };
        match self {
            Simple::Universal => true,
            Simple::Type(t) => t.eq_ignore_ascii_case(&local),
            Simple::Id(want) => {
                dom::attr(node, "id").as_deref().map(|v| v == want).unwrap_or(false)
            }
            Simple::Class(want) => dom::attr(node, "class")
                .map(|c| c.split_whitespace().any(|cls| cls == want))
                .unwrap_or(false),
        }
    }
}

impl Rule {
    fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        let parts = &self.selector.parts;
        if parts.is_empty() {
            return false;
        }
        // El último simple debe matchear el sujeto.
        if !parts.last().unwrap().matches(node) {
            return false;
        }
        if parts.len() == 1 {
            return true;
        }
        // Los anteriores (en orden, derecha→izquierda) deben matchear
        // *algún* ancestro — greedy: por cada uno avanzamos hacia arriba
        // hasta encontrar match, sino fallamos.
        let mut remaining = &parts[..parts.len() - 1];
        let mut current = parent_of(node);
        while !remaining.is_empty() {
            let Some(n) = current else { return false };
            let last = remaining.last().unwrap();
            if last.matches(&n) {
                remaining = &remaining[..remaining.len() - 1];
            }
            current = parent_of(&n);
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
        Selector { parts: vec![Simple::Type(s.into())] }
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

/// Parsea un selector compound. Soporta:
/// - simples: `*`, `tag`, `.class`, `#id`
/// - combinador descendiente (whitespace): `a b`, `.menu li`, `#hero h2`
///
/// Combinadores `>`, `+`, `~`, pseudoclases (`:hover`), atributos (`[…]`)
/// no soportados — la regla se ignora.
fn parse_selector(sel: &str) -> Option<Selector> {
    let sel = sel.trim();
    if sel.is_empty() {
        return Some(Selector { parts: vec![Simple::Universal] });
    }
    if sel.contains('>') || sel.contains('+') || sel.contains('~')
        || sel.contains(':') || sel.contains('[')
    {
        return None;
    }
    let mut parts = Vec::new();
    for token in sel.split_whitespace() {
        parts.push(parse_simple(token)?);
    }
    if parts.is_empty() {
        return None;
    }
    Some(Selector { parts })
}

fn parse_simple(sel: &str) -> Option<Simple> {
    if sel == "*" {
        return Some(Simple::Universal);
    }
    if let Some(rest) = sel.strip_prefix('.') {
        if rest.chars().all(is_ident_char) && !rest.is_empty() {
            return Some(Simple::Class(rest.to_string()));
        }
        return None;
    }
    if let Some(rest) = sel.strip_prefix('#') {
        if rest.chars().all(is_ident_char) && !rest.is_empty() {
            return Some(Simple::Id(rest.to_string()));
        }
        return None;
    }
    if sel.chars().all(is_ident_char) && !sel.is_empty() {
        return Some(Simple::Type(sel.to_string()));
    }
    None
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
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
        assert_eq!(rules[0].selector.parts.len(), 1);
        assert!(matches!(&rules[0].selector.parts[0], Simple::Type(t) if t == "p"));
        assert_eq!(rules[0].decls.len(), 2);
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
    fn cascada_inline_sobrescribe() {
        let html = "<html><head><style>p { color: red }</style></head><body><p style='color:blue'>x</p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let style = eng.compute(&p);
        assert_eq!(style.color, Color::rgb(0, 0, 255));
    }
}
