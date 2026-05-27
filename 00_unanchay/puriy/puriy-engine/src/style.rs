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
    /// Ancho explícito. `Auto` = el default block-fills-parent.
    pub width: LengthVal,
    /// Tope superior — útil para containers narrow ("max-width:800px").
    pub max_width: LengthVal,
    /// Alineación horizontal del texto dentro del box.
    pub text_align: TextAlign,
    /// Altura de línea como multiplicador del font-size. `None` =
    /// default razonable (1.4) en el caller.
    pub line_height: Option<f32>,
    /// Ancho del border en px. `0` = sin border.
    pub border_width: f32,
    /// Color del border. `None` = no se dibuja aunque `border_width > 0`.
    pub border_color: Option<Color>,
    /// Radio del corner-radius en px (0 = esquinas vivas).
    pub border_radius: f32,
}

/// Valor longitud de CSS reducido al subset que soportamos: `auto`,
/// `Npx`, `N%`. `em`/`rem` se resuelven a px en parse time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthVal {
    Auto,
    Px(f32),
    Pct(f32),
}

/// Alineación horizontal del contenido inline dentro de un bloque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
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
            width: LengthVal::Auto,
            max_width: LengthVal::Auto,
            text_align: TextAlign::Left,
            line_height: None,
            border_width: 0.0,
            border_color: None,
            border_radius: 0.0,
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
    /// gana (cascada simplificada). Sin inheritance — el caller debe
    /// usar [`Self::compute_with_parent`] si necesita propagación.
    pub fn compute(&self, node: &Handle) -> ComputedStyle {
        self.compute_with_parent(node, None)
    }

    /// Variante con inheritance CSS. Si `parent` está dado, las
    /// propiedades heredables (`color`, `font_size`, `font_weight`,
    /// `text_align`, `line_height`) se inicializan con el valor del
    /// padre antes de aplicar reglas y `style=`. Propiedades no
    /// heredables (`background`, `display`, `margin`, `padding`,
    /// `width`, `max_width`) siempre arrancan en el default.
    pub fn compute_with_parent(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
    ) -> ComputedStyle {
        self.compute_with_parent_in_state(node, parent, false)
    }

    /// Variante con hover. Si `hover_active=true`, los selectores con
    /// `:hover` también matchean. Permite computar el "estilo bajo el
    /// mouse" sin un mouse real — el chrome lo usa para precalcular
    /// `hover_fill` en el render.
    pub fn compute_with_parent_in_state(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
        hover_active: bool,
    ) -> ComputedStyle {
        let mut style = ComputedStyle::default();
        if let Some(p) = parent {
            style.color = p.color;
            style.font_size = p.font_size;
            style.font_weight = p.font_weight;
            style.text_align = p.text_align;
            style.line_height = p.line_height;
        }
        let Some(local) = dom::element_name(node) else {
            return style;
        };
        // Defaults por tag — `div`/`p`/`h1` son block. `display` no
        // hereda, así que siempre se setea según el tag local.
        style.display = default_display(&local);

        // `font_weight` por tag (h1..h6/b/strong/th = bold) override
        // el heredado — un `<b>` dentro de un `<p>` no-bold sigue
        // siendo bold.
        let weight_default = default_weight(&local);
        if weight_default != 400 {
            style.font_weight = weight_default;
        }

        // Cascada en dos pasadas:
        //   1. Decls normales, ordenadas por (specificity, source_index).
        //   2. Decls `!important`, ordenadas igual.
        // Cada decl individual lleva su flag — una misma regla puede
        // tener decls normales y `!important` mezcladas.
        let matched: Vec<(u32, usize, &Rule)> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.matches_in_state(node, hover_active))
            .map(|(i, r)| (r.selector.specificity(), i, r))
            .collect();
        let inline_decls: Vec<Decl> = dom::attr(node, "style")
            .map(|s| parse_declarations(&s))
            .unwrap_or_default();

        // PASADA 1 — normales.
        let mut normal_apps: Vec<(u32, usize, &Decl)> = Vec::new();
        for (spec, src, rule) in &matched {
            for d in &rule.decls {
                if !d.important {
                    normal_apps.push((*spec, *src, d));
                }
            }
        }
        normal_apps.sort_by_key(|(spec, idx, _)| (*spec, *idx));
        for (_, _, d) in normal_apps {
            d.apply(&mut style);
        }
        // Inline normal (especificidad 1000) cierra la pasada normal.
        for d in &inline_decls {
            if !d.important {
                d.apply(&mut style);
            }
        }

        // PASADA 2 — `!important`. Cualquier important de cualquier
        // regla vence cualquier normal — y entre importants, vuelve a
        // mandar especificidad/orden.
        let mut imp_apps: Vec<(u32, usize, &Decl)> = Vec::new();
        for (spec, src, rule) in &matched {
            for d in &rule.decls {
                if d.important {
                    imp_apps.push((*spec, *src, d));
                }
            }
        }
        imp_apps.sort_by_key(|(spec, idx, _)| (*spec, *idx));
        for (_, _, d) in imp_apps {
            d.apply(&mut style);
        }
        // Inline `!important` (efectiva 10_000 en CSS real, pero acá
        // simplemente cierra la pasada — gana todo lo anterior).
        for d in &inline_decls {
            if d.important {
                d.apply(&mut style);
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

impl Selector {
    /// Especificidad CSS — número compuesto `a*100 + b*10 + c` donde:
    /// - `a` = cuentas de `#id` en toda la cadena
    /// - `b` = cuentas de `.class`, `[attr]`, `:pseudo-class`
    /// - `c` = cuentas de tags (`p`, `div`, …); `*` y combinadores no
    ///   suman
    ///
    /// Inline `style="..."` no pasa por acá; el caller le otorga 1000
    /// implícito al aplicarlo después de los selectores.
    fn specificity(&self) -> u32 {
        let mut ids = 0u32;
        let mut classes_etc = 0u32;
        let mut types = 0u32;
        for c in &self.compounds {
            ids += c.ids.len() as u32;
            classes_etc += c.classes.len() as u32;
            classes_etc += c.attrs.len() as u32;
            classes_etc += c.pseudos.len() as u32;
            if matches!(c.tag, TagPart::Type(_)) {
                types += 1;
            }
        }
        ids * 100 + classes_etc * 10 + types
    }
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

/// Pseudoclases soportadas — la mayoría estructurales (puramente
/// posicionales). `Hover` se evalúa según un flag externo que pasa el
/// caller (`hover_active`); el chrome se encarga de mantenerlo
/// correlacionado con la posición del mouse.
#[derive(Debug, Clone, Copy)]
enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    Hover,
}

impl Compound {
    fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        self.matches_in_state(node, false)
    }

    /// Variante con flag `hover_active` — sólo afecta el matching de
    /// `:hover`. El resto del compound se evalúa idéntico.
    fn matches_in_state(
        &self,
        node: &markup5ever_rcdom::Handle,
        hover_active: bool,
    ) -> bool {
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
            if !pseudo_matches(node, *p, hover_active) {
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

fn pseudo_matches(
    node: &markup5ever_rcdom::Handle,
    p: Pseudo,
    hover_active: bool,
) -> bool {
    if matches!(p, Pseudo::Hover) {
        return hover_active;
    }
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
        Pseudo::Hover => unreachable!("Hover ya fue resuelto arriba"),
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
    #[allow(dead_code)]
    fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        self.matches_in_state(node, false)
    }

    fn matches_in_state(&self, node: &markup5ever_rcdom::Handle, hover_active: bool) -> bool {
        let compounds = &self.selector.compounds;
        if compounds.is_empty() {
            return false;
        }
        // El sujeto (último) debe matchear el nodo.
        if !compounds.last().unwrap().matches_in_state(node, hover_active) {
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

/// Una declaración CSS individual + flag `!important`.
#[derive(Debug, Clone)]
struct Decl {
    kind: DeclKind,
    important: bool,
}

#[derive(Debug, Clone)]
enum DeclKind {
    Color(Color),
    Background(Color),
    Display(Display),
    FontSize(f32),
    FontWeight(u16),
    Margin(f32),
    Padding(f32),
    Width(LengthVal),
    MaxWidth(LengthVal),
    TextAlign(TextAlign),
    LineHeight(f32),
    BorderWidth(f32),
    BorderColor(Color),
    /// `border-style: solid` activa el dibujo del border; `none`/`hidden`
    /// lo desactiva (color → None).
    BorderEnabled(bool),
    BorderRadius(f32),
}

impl Decl {
    fn apply(&self, s: &mut ComputedStyle) {
        match &self.kind {
            DeclKind::Color(c) => s.color = *c,
            DeclKind::Background(c) => s.background = Some(*c),
            DeclKind::Display(d) => s.display = *d,
            DeclKind::FontSize(v) => s.font_size = *v,
            DeclKind::FontWeight(w) => s.font_weight = *w,
            DeclKind::Margin(v) => s.margin = *v,
            DeclKind::Padding(v) => s.padding = *v,
            DeclKind::Width(v) => s.width = *v,
            DeclKind::MaxWidth(v) => s.max_width = *v,
            DeclKind::TextAlign(a) => s.text_align = *a,
            DeclKind::LineHeight(v) => s.line_height = Some(*v),
            DeclKind::BorderWidth(v) => s.border_width = *v,
            DeclKind::BorderColor(c) => s.border_color = Some(*c),
            DeclKind::BorderEnabled(on) => {
                if !*on {
                    s.border_color = None;
                    s.border_width = 0.0;
                }
            }
            DeclKind::BorderRadius(v) => s.border_radius = *v,
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
        Rule {
            selector: ty("h1"),
            decls: vec![
                Decl { kind: DeclKind::FontSize(32.0), important: false },
                Decl { kind: DeclKind::Margin(20.0), important: false },
            ],
        },
        Rule {
            selector: ty("h2"),
            decls: vec![
                Decl { kind: DeclKind::FontSize(24.0), important: false },
                Decl { kind: DeclKind::Margin(18.0), important: false },
            ],
        },
        Rule {
            selector: ty("p"),
            decls: vec![Decl { kind: DeclKind::Margin(12.0), important: false }],
        },
        Rule {
            selector: ty("body"),
            decls: vec![Decl { kind: DeclKind::Padding(8.0), important: false }],
        },
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
                    "hover" => Pseudo::Hover,
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
    // Cada decl separada por `;`. Detectamos `!important` recortando
    // el sufijo del value antes de pasarlo al parser de tipo. La
    // shorthand `border:` se expande inline a 1..3 decls atómicas.
    let mut out = Vec::new();
    for chunk in css.split(';') {
        let Some((prop, value)) = chunk.split_once(':') else {
            continue;
        };
        let prop = prop.trim();
        let value = value.trim();
        let (value, important) = match strip_important(value) {
            Some(stripped) => (stripped, true),
            None => (value, false),
        };
        if prop.eq_ignore_ascii_case("border") {
            out.extend(parse_border_shorthand(value, important));
            continue;
        }
        if let Some(kind) = decl_kind_from_pair(prop, value) {
            out.push(Decl { kind, important });
        }
    }
    out
}

/// Si `value` termina en `!important` (con o sin espacios), devuelve la
/// porción antes del bang. Sino, `None`.
fn strip_important(value: &str) -> Option<&str> {
    let v = value.trim_end();
    if v.len() < "!important".len() {
        return None;
    }
    let tail = &v[v.len() - "!important".len()..];
    if tail.eq_ignore_ascii_case("!important") {
        Some(v[..v.len() - "!important".len()].trim_end())
    } else {
        None
    }
}

fn decl_kind_from_pair(prop: &str, value: &str) -> Option<DeclKind> {
    match prop.to_ascii_lowercase().as_str() {
        "color" => parse_color(value).map(DeclKind::Color),
        "background-color" | "background" => parse_color(value).map(DeclKind::Background),
        "display" => parse_display(value).map(DeclKind::Display),
        "font-size" => parse_length_px(value).map(DeclKind::FontSize),
        "font-weight" => parse_weight(value).map(DeclKind::FontWeight),
        "margin" => parse_length_px(value).map(DeclKind::Margin),
        "padding" => parse_length_px(value).map(DeclKind::Padding),
        "width" => parse_length_or_pct(value).map(DeclKind::Width),
        "max-width" => parse_length_or_pct(value).map(DeclKind::MaxWidth),
        "text-align" => parse_text_align(value).map(DeclKind::TextAlign),
        "line-height" => parse_line_height(value).map(DeclKind::LineHeight),
        "border-width" => parse_length_px(value).map(DeclKind::BorderWidth),
        "border-color" => parse_color(value).map(DeclKind::BorderColor),
        "border-style" => parse_border_style(value).map(DeclKind::BorderEnabled),
        "border-radius" => parse_length_px(value).map(DeclKind::BorderRadius),
        // `border: 1px solid #ccc` — shorthand. Devolvemos un único
        // DeclKind sintético: en realidad ya hay 3 sub-decls que el
        // caller debe emitir, así que delegamos a una ruta especial vía
        // parse_declarations (ver más arriba). Acá no podemos producir
        // varios, así que ignoramos — la entrada se rellena en
        // parse_declarations cuando ve `border`.
        "border" => None,
        _ => None,
    }
}

fn parse_border_style(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "solid" | "dashed" | "dotted" | "double" => Some(true),
        "none" | "hidden" => Some(false),
        _ => None,
    }
}

/// Parsea el shorthand `border: <width> <style> <color>` (componentes en
/// cualquier orden). Devuelve hasta 3 decls. Si falta el style, se asume
/// `solid`. Cualquier "none" en la posición de style desactiva el border.
fn parse_border_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut width: Option<f32> = None;
    let mut color: Option<Color> = None;
    let mut style_on: Option<bool> = None;
    for tok in value.split_whitespace() {
        if let Some(w) = parse_length_px(tok) {
            width = Some(w);
            continue;
        }
        if let Some(c) = parse_color(tok) {
            color = Some(c);
            continue;
        }
        if let Some(s) = parse_border_style(tok) {
            style_on = Some(s);
            continue;
        }
    }
    // Defaults razonables: si hay width+color sin style, asumimos solid.
    if style_on.is_none() && (width.is_some() || color.is_some()) {
        style_on = Some(true);
    }
    let mut out = Vec::new();
    if let Some(on) = style_on {
        out.push(Decl { kind: DeclKind::BorderEnabled(on), important });
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::BorderWidth(w), important });
    }
    if let Some(c) = color {
        out.push(Decl { kind: DeclKind::BorderColor(c), important });
    }
    out
}

fn parse_text_align(s: &str) -> Option<TextAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => Some(TextAlign::Left),
        "center" => Some(TextAlign::Center),
        "right" | "end" => Some(TextAlign::Right),
        "justify" => Some(TextAlign::Justify),
        _ => None,
    }
}

/// Acepta `auto`, `Npx`, `Nrem`/`Nem` (→ px), `N%`. Sin unidad y
/// distinto de `0` → falla (a diferencia de `parse_length_px`, que
/// asume px).
fn parse_length_or_pct(s: &str) -> Option<LengthVal> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") {
        return Some(LengthVal::Auto);
    }
    if let Some(num) = s.strip_suffix('%') {
        return num.trim().parse::<f32>().ok().map(LengthVal::Pct);
    }
    parse_length_px(s).map(LengthVal::Px)
}

/// Acepta multiplicador adimensional (`1.5`, `1.6`), `Npx`, `Nem`/`Nrem`.
/// Devuelve siempre un multiplicador (px se divide por 16; `em`/`rem`
/// salen como ya están). Imperfecto pero alcanza para Fase 4.
fn parse_line_height(s: &str) -> Option<f32> {
    let s = s.trim();
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
    fn parsea_width_max_width() {
        let s = parse_stylesheet("p { width: 80%; max-width: 800px } div { width: auto }");
        assert_eq!(s.len(), 2);
        assert!(matches!(s[0].decls[0].kind, DeclKind::Width(LengthVal::Pct(80.0))));
        assert!(matches!(s[0].decls[1].kind, DeclKind::MaxWidth(LengthVal::Px(800.0))));
        assert!(matches!(s[1].decls[0].kind, DeclKind::Width(LengthVal::Auto)));
    }

    #[test]
    fn parsea_text_align() {
        let s = parse_stylesheet("h1 { text-align: center } p { text-align: right }");
        assert!(matches!(s[0].decls[0].kind, DeclKind::TextAlign(TextAlign::Center)));
        assert!(matches!(s[1].decls[0].kind, DeclKind::TextAlign(TextAlign::Right)));
    }

    #[test]
    fn parsea_line_height() {
        let s = parse_stylesheet("p { line-height: 1.5 } h1 { line-height: 32px }");
        // 1.5 → 1.5
        assert!(matches!(s[0].decls[0].kind, DeclKind::LineHeight(v) if (v - 1.5).abs() < 1e-6));
        // 32px sobre font-size 16px estimado → 2.0
        assert!(matches!(s[1].decls[0].kind, DeclKind::LineHeight(v) if (v - 2.0).abs() < 1e-6));
    }

    #[test]
    fn computa_width_y_text_align() {
        let html = r#"<html><head><style>
            .narrow{max-width:600px;text-align:center;line-height:1.6}
        </style></head><body><div class="narrow">x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let st = eng.compute(&div);
        assert_eq!(st.max_width, LengthVal::Px(600.0));
        assert_eq!(st.text_align, TextAlign::Center);
        assert!((st.line_height.unwrap() - 1.6).abs() < 1e-6);
    }

    #[test]
    fn hereda_color_y_font_size_del_padre() {
        // `<p style="color:red; font-size:20px">foo <em>bar</em></p>` —
        // el `<em>` no tiene regla propia pero hereda color y tamaño.
        let html = r#"<html><body><p style="color:red; font-size:20px">foo<em>bar</em></p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, None);
        assert_eq!(p_style.color, Color::rgb(255, 0, 0));
        let em = dom.find("em").unwrap();
        let em_style = eng.compute_with_parent(&em, Some(&p_style));
        assert_eq!(em_style.color, Color::rgb(255, 0, 0));
        assert!((em_style.font_size - 20.0).abs() < 1e-6);
    }

    #[test]
    fn no_hereda_propiedades_no_heredables() {
        // background y margin/padding NO heredan.
        let html = r#"<html><body><div style="background:red; margin:30px"><p>x</p></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let div_style = eng.compute_with_parent(&div, None);
        assert_eq!(div_style.background, Some(Color::rgb(255, 0, 0)));
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, Some(&div_style));
        assert_eq!(p_style.background, None);
        // margin del <p> es 12px (UA default), no 30px del padre.
        assert!((p_style.margin - 12.0).abs() < 1e-6);
    }

    #[test]
    fn font_weight_bold_local_no_propaga_a_padre_no_bold() {
        // Un `<b>` dentro de `<p>` no-bold sigue siendo bold.
        let html = "<html><body><p>foo<b>bar</b></p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, None);
        assert_eq!(p_style.font_weight, 400);
        let b = dom.find("b").unwrap();
        let b_style = eng.compute_with_parent(&b, Some(&p_style));
        assert_eq!(b_style.font_weight, 700);
    }

    #[test]
    fn box_tree_propaga_color_a_hoja_de_texto() {
        // Verifica el bug original: el text leaf debe heredar el color
        // del `<p>` padre.
        let html = r#"<html><body><p style="color: #00ff00">verde</p></body></html>"#;
        let eng = crate::Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut leaf_colors = Vec::new();
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("verde") {
                leaf_colors.push(b.color);
            }
        });
        assert_eq!(leaf_colors.len(), 1);
        assert_eq!(leaf_colors[0], Color::rgb(0, 0xff, 0));
    }

    #[test]
    fn specificity_calculada_correctamente() {
        // `body p` = 0,0,2 → 2
        let s1 = parse_selector("body p").unwrap();
        assert_eq!(s1.specificity(), 2);
        // `.menu li` = 0,1,1 → 11
        let s2 = parse_selector(".menu li").unwrap();
        assert_eq!(s2.specificity(), 11);
        // `#hero` = 1,0,0 → 100
        let s3 = parse_selector("#hero").unwrap();
        assert_eq!(s3.specificity(), 100);
        // `a.btn[href^="https"]:first-child` = 0,3,1 → 31
        let s4 = parse_selector(r#"a.btn[href^="https"]:first-child"#).unwrap();
        assert_eq!(s4.specificity(), 31);
        // `nav > a#x.y` = 1,1,2 → 112
        let s5 = parse_selector("nav > a#x.y").unwrap();
        assert_eq!(s5.specificity(), 112);
    }

    #[test]
    fn id_vence_a_tag_aunque_llegue_antes() {
        // `#hero { color: blue }` está ANTES que `body p { color: red }`
        // en el stylesheet — sin especificidad, el último (rojo) ganaba.
        // Con especificidad, el #id (100 > 2) gana azul.
        let html = r#"<html><head><style>
            #hero { color: blue }
            body p { color: red }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn clase_vence_a_tag() {
        // `.alert` (10) > `p` (1) aunque ambos matcheen.
        let html = r#"<html><head><style>
            .alert { color: red }
            p { color: blue }
        </style></head><body><p class="alert">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn inline_style_vence_a_id() {
        // Inline tiene especificidad implícita 1000 — gana sobre `#hero`.
        let html = r##"<html><head><style>
            #hero { color: blue }
        </style></head><body><p id="hero" style="color: green">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn empate_de_especificidad_gana_el_ultimo() {
        // Dos selectores con misma especificidad: gana el que llega después.
        let html = r#"<html><head><style>
            p { color: red }
            p { color: blue }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn important_vence_normal_de_mayor_especificidad() {
        // `body p { color: red !important }` (spec=2) debe vencer a
        // `#hero { color: blue }` (spec=100) — important rompe la
        // jerarquía de especificidad dentro del mismo origen.
        let html = r#"<html><head><style>
            body p { color: red !important }
            #hero { color: blue }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn important_inline_vence_important_de_id() {
        // Inline !important vence cualquier !important de selector.
        let html = r##"<html><head><style>
            #hero { color: red !important }
        </style></head><body><p id="hero" style="color: green !important">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn normal_inline_pierde_contra_important_de_regla() {
        // Inline normal (1000) pierde contra !important de cualquier selector.
        let html = r##"<html><head><style>
            p { color: red !important }
        </style></head><body><p style="color: green">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn parsea_border_shorthand() {
        let html = r#"<html><head><style>
            .a { border: 2px solid #ff0000 }
            .b { border: 1px dashed blue !important }
            .c { border: none }
            .d { border-radius: 8px }
        </style></head><body>
          <div class="a"></div><div class="b"></div>
          <div class="c"></div><div class="d"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 4);
        let a = eng.compute(&divs[0]);
        assert!((a.border_width - 2.0).abs() < 1e-6);
        assert_eq!(a.border_color, Some(Color::rgb(255, 0, 0)));
        let b = eng.compute(&divs[1]);
        assert!((b.border_width - 1.0).abs() < 1e-6);
        assert_eq!(b.border_color, Some(Color::rgb(0, 0, 255)));
        let c = eng.compute(&divs[2]);
        assert_eq!(c.border_color, None); // `none` deshabilita
        assert!((c.border_width - 0.0).abs() < 1e-6);
        let d = eng.compute(&divs[3]);
        assert!((d.border_radius - 8.0).abs() < 1e-6);
    }

    #[test]
    fn parsea_border_propiedades_individuales() {
        let html = r#"<html><head><style>
            div { border-width: 3px; border-color: #00ff00; border-style: solid; border-radius: 5px }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let st = eng.compute(&div);
        assert!((st.border_width - 3.0).abs() < 1e-6);
        assert_eq!(st.border_color, Some(Color::rgb(0, 0xff, 0)));
        assert!((st.border_radius - 5.0).abs() < 1e-6);
    }

    #[test]
    fn hover_state_activa_regla_solo_cuando_corresponde() {
        // `.btn:hover { background: red }`: matchea con hover_active=true,
        // no matchea sin él.
        let html = r##"<html><head><style>
            .btn:hover { background: #ff0000 }
            .btn { background: #ffffff }
        </style></head><body><a class="btn">x</a></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let base = eng.compute_with_parent_in_state(&a, None, false);
        let hover = eng.compute_with_parent_in_state(&a, None, true);
        assert_eq!(base.background, Some(Color::rgb(255, 255, 255)));
        assert_eq!(hover.background, Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn hover_pseudo_aporta_a_specificity() {
        // `.btn:hover` debe tener specificity 0,2,0 → 20 (clase 10 + pseudo 10)
        let s = parse_selector(".btn:hover").unwrap();
        assert_eq!(s.specificity(), 20);
    }

    #[test]
    fn box_tree_expone_hover_background() {
        let html = r##"<html><head><style>
            .btn { background: white }
            .btn:hover { background: #ffaa00 }
        </style></head><body><a class="btn">x</a></body></html>"##;
        let eng = crate::Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut hover_bgs = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                hover_bgs.push(b.hover_background);
            }
        });
        assert_eq!(hover_bgs.len(), 1);
        assert_eq!(hover_bgs[0], Some(Color::rgb(0xff, 0xaa, 0)));
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
