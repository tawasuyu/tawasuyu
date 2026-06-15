//! Selectores y matching: `Rule`, `Selector`/`Compound`/`Combinator`,
//! pseudo-clases (`Pseudo`) y de tipo, matchers de atributos, especificidad,
//! `:nth-*`, y el contenido de pseudo-elementos (`ContentItem`/`PseudoElement`
//! + `resolve_content_items`). Extraído de `style/mod.rs` (regla #1). Comparte
//! los tipos del módulo `style` y del crate vía `use super::*`.
use super::*;

#[derive(Debug, Clone)]
pub(crate) struct Rule {
    pub(crate) selector: Selector,
    pub(crate) decls: Vec<Decl>,
}

/// Resuelve una lista de `ContentItem` a la string final que se pintará
/// como leaf de texto. Counters se buscan en `counters`; ausentes
/// resuelven a `0` (CSS spec: el contador implícito vale 0 si no se
/// resetó). Attrs se leen del `node` (el padre del pseudo-element);
/// ausentes resuelven a `""`.
pub fn resolve_content_items(
    items: &[ContentItem],
    node: &markup5ever_rcdom::Handle,
    counters: &std::collections::HashMap<String, i32>,
) -> String {
    let mut out = String::new();
    for it in items {
        match it {
            ContentItem::Text(s) => out.push_str(s),
            ContentItem::Counter(name) => {
                let v = counters.get(name).copied().unwrap_or(0);
                out.push_str(&v.to_string());
            }
            ContentItem::Attr(name) => {
                if let Some(v) = dom::attr(node, name) {
                    out.push_str(&v);
                }
            }
            // `Url` se materializa como `<img>` sintético en boxes —
            // acá lo saltamos, el caller hace dispatch sobre los items.
            ContentItem::Url(_) => {}
        }
    }
    out
}

/// Item dentro del valor de `content:` para `::before`/`::after`. Un
/// `content:` puede tener varios items concatenados — `Text`/`Counter`/
/// `Attr` se resuelven a string y los runs adyacentes se mergean en un
/// solo text leaf; `Url` se materializa como un `<img>` sintético
/// separado, en línea con los demás items.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentItem {
    /// Literal string entre comillas — el más común.
    Text(String),
    /// `counter(name)` — el valor actual del contador con ese nombre,
    /// formateado como decimal por ahora (CSS spec permite list-style-type
    /// como segundo arg; queda para más adelante).
    Counter(String),
    /// `attr(name)` — el valor del atributo `name` del elemento padre del
    /// pseudo. Strings vacíos si el atributo no existe.
    Attr(String),
    /// `url(...)` — genera un `<img>` sintético inline-block con el
    /// recurso descargado. Si la descarga/decode falla, se omite (no
    /// fallback a texto — CSS spec dice que un url() inválido suprime
    /// la generación del pseudo).
    Url(String),
}

/// Pseudo-elemento attachado al selector. Genera un box hijo sintético
/// del nodo matching, no parte del DOM real. `content: "..."` define
/// qué texto pintar. El chrome lo trata como un text leaf inline
/// regular insertado al inicio (`Before`) o al final (`After`) de los
/// children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
    /// Pseudo-elemento estándar reconocido pero NO renderizado (`::selection`,
    /// `::marker`, `::placeholder`, `::first-line`, `::backdrop`, `::part()`…).
    /// Se parsea para NO tirar la regla; como nunca computamos un box para él
    /// (`compute_pseudo` sólo se llama con `Before`/`After`) y el filtro de
    /// cascada exige `pseudo_element == target_pseudo`, sus declaraciones
    /// quedan inertes (no se filtran al elemento real). Fase 7.934.
    Other,
}

/// Selector encadenado — alterna compound + combinador. `compounds[0]`
/// es el ancestro/hermano más lejano; `compounds.last()` es el sujeto.
/// `combinators[i]` es el combinador entre `compounds[i]` y
/// `compounds[i+1]`. `pseudo_element` (si Some) indica que la regla
/// genera un `::before` o `::after` del sujeto en lugar de aplicar al
/// nodo mismo.
#[derive(Debug, Clone)]
pub(crate) struct Selector {
    pub(crate) compounds: Vec<Compound>,
    pub(crate) combinators: Vec<Combinator>,
    pub(crate) pseudo_element: Option<PseudoElement>,
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
    pub(crate) fn specificity(&self) -> u32 {
        let mut ids = 0u32;
        let mut classes_etc = 0u32;
        let mut types = 0u32;
        // Aportes ya pre-multiplicados (de `:is(...)`, que suma la
        // especificidad de su argumento más específico, CSS spec).
        let mut extra = 0u32;
        for c in &self.compounds {
            ids += c.ids.len() as u32;
            classes_etc += c.classes.len() as u32;
            classes_etc += c.attrs.len() as u32;
            for p in &c.pseudos {
                match p {
                    // CSS spec: `:not(...)` e `:is(...)` aportan la
                    // especificidad de su argumento MÁS específico (selector
                    // complejo completo, Fase 7.938).
                    Pseudo::Not(list) | Pseudo::Is(list) => {
                        extra += list.iter().map(Selector::specificity).max().unwrap_or(0);
                    }
                    // `:has(...)` aporta la especificidad de su argumento más
                    // específico (CSS spec, igual que `:is`).
                    Pseudo::Has(rels) => {
                        extra += rels
                            .iter()
                            .map(|r| r.selector.specificity())
                            .max()
                            .unwrap_or(0);
                    }
                    // `:where(...)` no aporta especificidad.
                    Pseudo::Where(_) => {}
                    // `:nth-child(... of S)` / `:nth-last-child(... of S)`:
                    // la pseudo-clase cuenta como una (b), más la
                    // especificidad del selector más específico de `S`
                    // (CSS Selectors 4).
                    Pseudo::NthChild { of: Some(list), .. }
                    | Pseudo::NthLastChild { of: Some(list), .. } => {
                        classes_etc += 1;
                        extra += list.iter().map(Selector::specificity).max().unwrap_or(0);
                    }
                    _ => classes_etc += 1,
                }
            }
            if matches!(c.tag, TagPart::Type(_)) {
                types += 1;
            }
        }
        ids * 100 + classes_etc * 10 + types + extra
    }
}

/// Combinador CSS entre dos compounds consecutivos.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Combinator {
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
pub(crate) struct Compound {
    pub(crate) tag: TagPart,
    pub(crate) ids: Vec<String>,
    pub(crate) classes: Vec<String>,
    pub(crate) attrs: Vec<AttrMatch>,
    pub(crate) pseudos: Vec<Pseudo>,
}

#[derive(Debug, Clone)]
pub(crate) enum TagPart {
    Universal,
    Type(String),
}

#[derive(Debug, Clone)]
pub(crate) struct AttrMatch {
    pub(crate) name: String,
    pub(crate) op: AttrOp,
    pub(crate) value: String,
    /// Flag `i` de CSS4 (`[attr=val i]`) — comparación case-insensitive.
    /// Default `false` (case-sensitive, equivalente al flag `s`).
    pub(crate) case_insensitive: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum AttrOp {
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
#[derive(Debug, Clone)]
pub(crate) enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    Hover,
    /// `:focus` — flag externo del caller. Sólo aporta a la cascada
    /// cuando el chrome computa el estilo "como si el nodo estuviera
    /// focado"; el engine no sabe qué nodo lo está y deja la decisión
    /// al chrome.
    Focus,
    /// `:nth-child(an+b [of S]?)` — match si la posición 1-indexed del nodo
    /// satisface `pos = a*k + b` para algún `k >= 0`. Con `of S` (CSS
    /// Selectors 4), la posición se cuenta SÓLO entre los hermanos que
    /// matchean la lista `S`, y el nodo además debe matchear `S`.
    NthChild {
        a: i32,
        b: i32,
        of: Option<Vec<Selector>>,
    },
    /// `:not(a, b, ...)` — negación de una lista de selectores (CSS Selectors
    /// 4: complejos permitidos, ej `:not(.a > .b)`). Matchea si NINGUNO
    /// matchea con el nodo como sujeto. Fase 7.938.
    Not(Vec<Selector>),
    /// `:nth-of-type(an+b)` — posición 1-indexed entre hermanos del MISMO tag.
    NthOfType {
        a: i32,
        b: i32,
    },
    /// `:nth-last-child(an+b [of S]?)` — posición contando desde el final;
    /// con `of S` se cuenta sólo entre hermanos que matchean `S`.
    NthLastChild {
        a: i32,
        b: i32,
        of: Option<Vec<Selector>>,
    },
    /// `:nth-last-of-type(an+b)` — posición desde el final entre el mismo tag.
    NthLastOfType {
        a: i32,
        b: i32,
    },
    /// `:only-of-type` — único hermano de su tag.
    OnlyOfType,
    // === Pseudo-clases de estado (basadas en atributos del elemento) ===
    /// `:checked` — `<input>`/`<option>` con el atributo `checked`/`selected`.
    Checked,
    /// `:disabled` — control con atributo `disabled`.
    Disabled,
    /// `:enabled` — control de formulario SIN `disabled`.
    Enabled,
    /// `:required` — control con atributo `required`.
    Required,
    /// `:optional` — control de formulario SIN `required`.
    Optional,
    /// `:read-only` — control con atributo `readonly`.
    ReadOnly,
    /// `:read-write` — control editable (input/textarea/contenteditable) sin
    /// `readonly`.
    ReadWrite,
    /// `:placeholder-shown` — `<input>`/`<textarea>` con atributo `placeholder`
    /// no vacío y valor vacío (input sin `value`/vacío; textarea sin texto).
    /// Derivable del DOM estático. Fase 7.1212.
    PlaceholderShown,
    /// `:default` — control "por defecto" de un formulario: checkbox/radio con
    /// `checked`, `<option selected>`, o el primer botón submit del form.
    /// Fase 7.1212.
    Default,
    /// `:in-range` / `:out-of-range` — `<input>` con limitación de rango
    /// (`type` number/range/date/…, con `min`/`max`) cuyo `value` cae dentro
    /// (`true`) o fuera (`false`) del rango. Fase 7.1212.
    InRange(bool),
    /// `:is(a, b, ...)` — matchea si CUALQUIER selector de la lista matchea
    /// (complejos permitidos, CSS Selectors 4). Especificidad: la del
    /// argumento más específico (CSS spec). Fase 7.938.
    Is(Vec<Selector>),
    /// `:where(a, b, ...)` — como `:is` pero aporta especificidad CERO.
    Where(Vec<Selector>),
    /// `:empty` — elemento sin hijos elemento ni texto no-whitespace
    /// (comentarios ignorados, CSS Selectors 4).
    Empty,
    /// `:root` — el elemento raíz del documento (`<html>` en HTML).
    Root,
    /// `:link` / `:any-link` — `<a>`/`<area>`/`<link>` con atributo `href`.
    /// (No distinguimos visitado/no-visitado: no rastreamos historial.)
    AnyLink,
    /// `:has(<rel-sel-list>)` — relacional. Matchea si ALGUNA relative
    /// selector matchea contra el subárbol/hermanos del elemento.
    Has(Vec<RelativeSelector>),
    /// `:lang(en, fr)` — el idioma del elemento (atributo `lang` propio o
    /// del ancestro más cercano) coincide con (o es subtag de) alguno.
    Lang(Vec<String>),
    /// `:dir(rtl)` / `:dir(ltr)` — direccionalidad resuelta del elemento
    /// (atributo `dir` propio o del ancestro más cercano; default `ltr`;
    /// `auto` se aproxima a `ltr` — no analizamos el contenido). `true` =
    /// matchea `rtl`. Fase 7.940.
    Dir(bool),
    /// Pseudo-clase estándar reconocida pero NO evaluable con el estado que
    /// rastreamos (validación de formularios, estado de media/popover/dialog,
    /// `:active`/`:visited`/`:target`…). Se parsea para NO tirar la regla
    /// entera (comportamiento de browser real, donde estos selectores son
    /// válidos); evalúa al `bool` guardado. `:scope` → `true` (transparente).
    /// Fase 7.933.
    Inert(bool),
}

/// Una relative selector de `:has(...)`: un combinador (descendiente por
/// defecto) + un selector COMPLEJO cuyo sujeto se busca relativo al ancla.
/// `:has(> .a)` → `{Child, .a}`; `:has(.a > .b)` → `{Descendant, .a > .b}`.
/// Fase 7.938.
#[derive(Debug, Clone)]
pub(crate) struct RelativeSelector {
    pub(crate) combinator: Combinator,
    pub(crate) selector: Selector,
}

impl Compound {
    pub(crate) fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        self.matches_in_state(node, false, false)
    }

    /// Variante con flags de estado externos (`hover_active`,
    /// `focus_active`) — los `:hover` y `:focus` matchean cuando el
    /// caller los activa.
    pub(crate) fn matches_in_state(
        &self,
        node: &markup5ever_rcdom::Handle,
        hover_active: bool,
        focus_active: bool,
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
            if !pseudo_matches(node, p, hover_active, focus_active) {
                return false;
            }
        }
        true
    }
}

pub(crate) fn attr_matches(node: &markup5ever_rcdom::Handle, am: &AttrMatch) -> bool {
    let actual = dom::attr(node, &am.name);
    match am.op {
        AttrOp::Present => actual.is_some(),
        op => {
            let Some(actual) = actual else { return false };
            // Con el flag `i` (CSS4) comparamos en minúsculas ASCII.
            if am.case_insensitive {
                let a = actual.to_ascii_lowercase();
                let v = am.value.to_ascii_lowercase();
                return match op {
                    AttrOp::Equals => a == v,
                    AttrOp::Prefix => a.starts_with(&v),
                    AttrOp::Suffix => a.ends_with(&v),
                    AttrOp::Contains => a.contains(&v),
                    AttrOp::Present => unreachable!(),
                };
            }
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

pub(crate) fn pseudo_matches(
    node: &markup5ever_rcdom::Handle,
    p: &Pseudo,
    hover_active: bool,
    focus_active: bool,
) -> bool {
    // Resueltos sin mirar el padre: flags externos, negación, y pseudo-clases
    // de estado basadas en atributos del propio elemento.
    let has = |name: &str| dom::attr(node, name).is_some();
    match p {
        Pseudo::Hover => return hover_active,
        Pseudo::Focus => return focus_active,
        Pseudo::Not(list) => {
            return !list
                .iter()
                .any(|s| selector_matches_subject(s, node, hover_active, focus_active))
        }
        Pseudo::Checked => return has("checked") || has("selected"),
        Pseudo::Disabled => return has("disabled"),
        Pseudo::Enabled => return is_form_control(node) && !has("disabled"),
        Pseudo::Required => return has("required"),
        Pseudo::Optional => return is_form_control(node) && !has("required"),
        Pseudo::ReadOnly => return has("readonly"),
        Pseudo::ReadWrite => return is_editable_control(node) && !has("readonly"),
        Pseudo::PlaceholderShown => return placeholder_shown(node),
        Pseudo::Default => return is_default_element(node),
        Pseudo::InRange(want_in) => {
            return match range_state(node) {
                Some(in_range) => in_range == *want_in,
                None => false,
            }
        }
        Pseudo::Is(list) | Pseudo::Where(list) => {
            return list
                .iter()
                .any(|s| selector_matches_subject(s, node, hover_active, focus_active))
        }
        Pseudo::Empty => return is_empty_element(node),
        Pseudo::Root => return dom::element_name(node).as_deref() == Some("html"),
        Pseudo::AnyLink => {
            return matches!(
                dom::element_name(node).as_deref(),
                Some("a") | Some("area") | Some("link")
            ) && dom::attr(node, "href").is_some()
        }
        Pseudo::Has(rels) => {
            return rels
                .iter()
                .any(|r| has_relative_match(node, r, hover_active, focus_active))
        }
        Pseudo::Lang(tags) => return lang_matches(node, tags),
        Pseudo::Dir(want_rtl) => return dir_matches(node) == *want_rtl,
        Pseudo::Inert(b) => return *b,
        _ => {}
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
    // Índice (0-based) entre hermanos del MISMO tag, y total de ese tag.
    let my_tag = dom::element_name(node).unwrap_or_default();
    let same_type: Vec<usize> = elems
        .iter()
        .enumerate()
        .filter(|(_, c)| dom::element_name(c).as_deref() == Some(my_tag.as_str()))
        .map(|(i, _)| i)
        .collect();
    let type_pos = same_type.iter().position(|&i| i == pos).unwrap_or(0);
    match p {
        Pseudo::Hover
        | Pseudo::Focus
        | Pseudo::Not(_)
        | Pseudo::Checked
        | Pseudo::Disabled
        | Pseudo::Enabled
        | Pseudo::Required
        | Pseudo::Optional
        | Pseudo::ReadOnly
        | Pseudo::ReadWrite
        | Pseudo::PlaceholderShown
        | Pseudo::Default
        | Pseudo::InRange(_)
        | Pseudo::Is(_)
        | Pseudo::Where(_)
        | Pseudo::Empty
        | Pseudo::Root
        | Pseudo::AnyLink
        | Pseudo::Has(_)
        | Pseudo::Lang(_)
        | Pseudo::Dir(_)
        | Pseudo::Inert(_) => unreachable!("ya resueltos arriba"),
        Pseudo::FirstChild => pos == 0,
        Pseudo::LastChild => pos + 1 == elems.len(),
        Pseudo::OnlyChild => elems.len() == 1,
        Pseudo::FirstOfType => type_pos == 0,
        Pseudo::LastOfType => type_pos + 1 == same_type.len(),
        Pseudo::OnlyOfType => same_type.len() == 1,
        // `:nth-child(An+B of S)` (CSS Selectors 4): el nodo debe matchear
        // `S` y la posición se cuenta sólo entre los hermanos que matchean
        // `S`. Sin `of`, posición entre todos los hermanos-elemento.
        Pseudo::NthChild { a, b, of } => match of {
            None => nth_matches((pos + 1) as i32, *a, *b),
            Some(list) => nth_of_matches(&elems, pos, list, *a, *b, false, hover_active, focus_active),
        },
        Pseudo::NthLastChild { a, b, of } => match of {
            None => nth_matches((elems.len() - pos) as i32, *a, *b),
            Some(list) => nth_of_matches(&elems, pos, list, *a, *b, true, hover_active, focus_active),
        },
        Pseudo::NthOfType { a, b } => nth_matches((type_pos + 1) as i32, *a, *b),
        Pseudo::NthLastOfType { a, b } => {
            nth_matches((same_type.len() - type_pos) as i32, *a, *b)
        }
    }
}

/// `:empty` — sin hijos elemento ni texto no-whitespace. Los comentarios
/// y processing-instructions se ignoran (CSS Selectors 4).
pub(crate) fn is_empty_element(node: &markup5ever_rcdom::Handle) -> bool {
    for c in node.children.borrow().iter() {
        if dom::element_name(c).is_some() {
            return false;
        }
        if let markup5ever_rcdom::NodeData::Text { contents } = &c.data {
            if !contents.borrow().trim().is_empty() {
                return false;
            }
        }
    }
    true
}

/// `:has(...)` — evalúa una relative selector contra el subárbol/hermanos.
pub(crate) fn has_relative_match(
    node: &markup5ever_rcdom::Handle,
    rel: &RelativeSelector,
    hover_active: bool,
    focus_active: bool,
) -> bool {
    let sel = &rel.selector;
    match rel.combinator {
        Combinator::Descendant => {
            any_descendant_matches(node, sel, hover_active, focus_active)
        }
        Combinator::Child => dom::children(node)
            .iter()
            .any(|c| selector_matches_subject(sel, c, hover_active, focus_active)),
        Combinator::AdjacentSibling => following_element_siblings(node)
            .first()
            .is_some_and(|s| selector_matches_subject(sel, s, hover_active, focus_active)),
        Combinator::GeneralSibling => following_element_siblings(node)
            .iter()
            .any(|s| selector_matches_subject(sel, s, hover_active, focus_active)),
    }
}

/// `true` si algún descendiente (cualquier nivel, excluye el propio nodo)
/// matchea el compound.
fn any_descendant_matches(
    node: &markup5ever_rcdom::Handle,
    sel: &Selector,
    hover_active: bool,
    focus_active: bool,
) -> bool {
    for c in node.children.borrow().iter() {
        if dom::element_name(c).is_none() {
            continue;
        }
        if selector_matches_subject(sel, c, hover_active, focus_active)
            || any_descendant_matches(c, sel, hover_active, focus_active)
        {
            return true;
        }
    }
    false
}

/// `:lang(...)` — el idioma efectivo del elemento (atributo `lang` propio o
/// del ancestro más cercano) matchea si es igual a algún tag pedido o es un
/// subtag suyo (`lang="en-US"` ↔ `:lang(en)`). Case-insensitive.
/// Direccionalidad resuelta de un elemento (`:dir()`): busca el atributo `dir`
/// propio o del ancestro más cercano. `rtl`→true; `ltr`/`auto`/ausente→false
/// (`auto` se aproxima a `ltr`, no analizamos el contenido). Fase 7.940.
pub(crate) fn dir_matches(node: &markup5ever_rcdom::Handle) -> bool {
    let mut cur = Some(node.clone());
    while let Some(n) = cur {
        if let Some(d) = dom::attr(&n, "dir") {
            let d = d.trim().to_ascii_lowercase();
            if d == "rtl" {
                return true;
            } else if d == "ltr" || d == "auto" {
                return false;
            }
        }
        cur = parent_of(&n);
    }
    false
}

pub(crate) fn lang_matches(node: &markup5ever_rcdom::Handle, tags: &[String]) -> bool {
    let mut cur = Some(node.clone());
    let lang = loop {
        let Some(n) = cur else { return false };
        if let Some(l) = dom::attr(&n, "lang") {
            let l = l.trim();
            if !l.is_empty() {
                break l.to_ascii_lowercase();
            }
        }
        cur = parent_of(&n);
    };
    tags.iter().any(|t| {
        let t = t.trim().to_ascii_lowercase();
        !t.is_empty() && (lang == t || lang.starts_with(&format!("{t}-")))
    })
}

/// Hermanos-elemento que siguen a `node` bajo el mismo padre, en orden.
fn following_element_siblings(
    node: &markup5ever_rcdom::Handle,
) -> Vec<markup5ever_rcdom::Handle> {
    let Some(parent) = parent_of(node) else {
        return Vec::new();
    };
    let kids = parent.children.borrow();
    let mut out = Vec::new();
    let mut after = false;
    for c in kids.iter() {
        if after && dom::element_name(c).is_some() {
            out.push(c.clone());
        }
        if std::rc::Rc::ptr_eq(c, node) {
            after = true;
        }
    }
    out
}

/// `true` si la posición CSS 1-indexed `p_css` satisface `a*k + b` para algún
/// `k >= 0`. Compartido por `:nth-child`/`:nth-of-type`/`:nth-last-*`.
pub(crate) fn nth_matches(p_css: i32, a: i32, b: i32) -> bool {
    let diff = p_css - b;
    if a == 0 {
        diff == 0
    } else if a > 0 {
        diff >= 0 && diff % a == 0
    } else {
        diff <= 0 && diff % a == 0
    }
}

/// `:nth-child(An+B of S)` / `:nth-last-child(... of S)` (CSS Selectors 4):
/// el nodo en posición `pos` (índice 0-based dentro de `elems`, los hermanos
/// elemento) matchea si (1) él mismo matchea la lista `S` y (2) su posición
/// 1-indexed *entre los hermanos que matchean `S`* satisface `An+B`. Con
/// `from_end` la posición se cuenta desde el final. El nodo es `elems[pos]`.
#[allow(clippy::too_many_arguments)]
fn nth_of_matches(
    elems: &[markup5ever_rcdom::Handle],
    pos: usize,
    list: &[Selector],
    a: i32,
    b: i32,
    from_end: bool,
    hover_active: bool,
    focus_active: bool,
) -> bool {
    let matches_s = |n: &markup5ever_rcdom::Handle| {
        list.iter()
            .any(|s| selector_matches_subject(s, n, hover_active, focus_active))
    };
    // El nodo debe matchear S, si no nunca cuenta.
    if !matches_s(&elems[pos]) {
        return false;
    }
    // Índices (en `elems`) de los hermanos que matchean S, en orden de documento.
    let matching: Vec<usize> = elems
        .iter()
        .enumerate()
        .filter(|(_, c)| matches_s(c))
        .map(|(i, _)| i)
        .collect();
    let Some(idx) = matching.iter().position(|&i| i == pos) else {
        return false;
    };
    let css_pos = if from_end {
        (matching.len() - idx) as i32
    } else {
        (idx + 1) as i32
    };
    nth_matches(css_pos, a, b)
}

/// `:placeholder-shown` — `<input>` o `<textarea>` con `placeholder` no vacío
/// cuyo valor está vacío. Para `<input>` el valor es el atributo `value`
/// (ausente o vacío ⇒ placeholder visible); tipos que no soportan placeholder
/// (checkbox/radio/etc.) nunca matchean. Para `<textarea>` el valor es su texto.
pub(crate) fn placeholder_shown(node: &markup5ever_rcdom::Handle) -> bool {
    let ph = dom::attr(node, "placeholder").unwrap_or_default();
    if ph.is_empty() {
        return false;
    }
    match dom::element_name(node).as_deref() {
        Some("input") => {
            // Sólo tipos textuales soportan placeholder.
            let ty = dom::attr(node, "type").unwrap_or_else(|| "text".into());
            let textual = matches!(
                ty.to_ascii_lowercase().as_str(),
                "text" | "search" | "url" | "tel" | "email" | "password" | "number"
            );
            textual && dom::attr(node, "value").unwrap_or_default().is_empty()
        }
        Some("textarea") => is_empty_element(node)
            || node.children.borrow().iter().all(|c| match &c.data {
                markup5ever_rcdom::NodeData::Text { contents } => {
                    contents.borrow().trim().is_empty()
                }
                _ => dom::element_name(c).is_none(),
            }),
        _ => false,
    }
}

/// `:default` — control por defecto de un formulario. Casos derivables del
/// DOM estático: checkbox/radio con `checked`, `<option selected>`, y el
/// primer botón submit (`<button>` sin type o type=submit, o
/// `<input type=submit|image>`) en orden de documento dentro de su `<form>`.
pub(crate) fn is_default_element(node: &markup5ever_rcdom::Handle) -> bool {
    let name = dom::element_name(node);
    match name.as_deref() {
        Some("option") => return dom::attr(node, "selected").is_some(),
        Some("input") => {
            let ty = dom::attr(node, "type").unwrap_or_else(|| "text".into());
            let ty = ty.to_ascii_lowercase();
            if matches!(ty.as_str(), "checkbox" | "radio") {
                return dom::attr(node, "checked").is_some();
            }
            if matches!(ty.as_str(), "submit" | "image") {
                return is_first_submit_in_form(node);
            }
            return false;
        }
        Some("button") => {
            let ty = dom::attr(node, "type").unwrap_or_else(|| "submit".into());
            if ty.eq_ignore_ascii_case("submit") {
                return is_first_submit_in_form(node);
            }
            return false;
        }
        _ => false,
    }
}

/// `true` si `node` es el primer control submit (en orden de documento) dentro
/// del `<form>` ancestro más cercano — el botón submit por defecto del form.
fn is_first_submit_in_form(node: &markup5ever_rcdom::Handle) -> bool {
    // Sube al <form> ancestro.
    let mut form = parent_of(node);
    while let Some(f) = form.clone() {
        if dom::element_name(&f).as_deref() == Some("form") {
            break;
        }
        form = parent_of(&f);
    }
    let Some(form) = form else { return false };
    if dom::element_name(&form).as_deref() != Some("form") {
        return false;
    }
    // Busca el primer submit en orden de documento bajo el form.
    let mut found: Option<markup5ever_rcdom::Handle> = None;
    fn walk_first(
        n: &markup5ever_rcdom::Handle,
        found: &mut Option<markup5ever_rcdom::Handle>,
    ) {
        if found.is_some() {
            return;
        }
        for c in n.children.borrow().iter() {
            if found.is_some() {
                return;
            }
            if is_submit_control(c) {
                *found = Some(c.clone());
                return;
            }
            walk_first(c, found);
        }
    }
    walk_first(&form, &mut found);
    found.is_some_and(|f| std::rc::Rc::ptr_eq(&f, node))
}

fn is_submit_control(node: &markup5ever_rcdom::Handle) -> bool {
    match dom::element_name(node).as_deref() {
        Some("button") => dom::attr(node, "type")
            .unwrap_or_else(|| "submit".into())
            .eq_ignore_ascii_case("submit"),
        Some("input") => {
            let ty = dom::attr(node, "type").unwrap_or_default().to_ascii_lowercase();
            matches!(ty.as_str(), "submit" | "image")
        }
        _ => false,
    }
}

/// Estado de rango de un `<input>` con limitación (`type` number/range/date/…
/// con `min`/`max`). `Some(true)` = dentro de rango, `Some(false)` = fuera,
/// `None` = el elemento no tiene limitación de rango aplicable (ni `:in-range`
/// ni `:out-of-range` matchean). Tipos numéricos se comparan como f64; los
/// de fecha/hora como string ISO (ordenable lexicográficamente).
pub(crate) fn range_state(node: &markup5ever_rcdom::Handle) -> Option<bool> {
    if dom::element_name(node).as_deref() != Some("input") {
        return None;
    }
    let ty = dom::attr(node, "type")?.to_ascii_lowercase();
    let numeric = matches!(ty.as_str(), "number" | "range");
    let datelike = matches!(
        ty.as_str(),
        "date" | "month" | "week" | "time" | "datetime-local"
    );
    if !numeric && !datelike {
        return None;
    }
    let min = dom::attr(node, "min");
    let max = dom::attr(node, "max");
    if min.is_none() && max.is_none() {
        return None; // sin limitación → no matchea ninguno
    }
    // Valor vacío con limitación de rango ⇒ in-range (CSS spec).
    let value = dom::attr(node, "value").unwrap_or_default();
    if value.trim().is_empty() {
        return Some(true);
    }
    if numeric {
        let v: f64 = value.trim().parse().ok()?;
        if let Some(mn) = min.as_deref().and_then(|s| s.trim().parse::<f64>().ok()) {
            if v < mn {
                return Some(false);
            }
        }
        if let Some(mx) = max.as_deref().and_then(|s| s.trim().parse::<f64>().ok()) {
            if v > mx {
                return Some(false);
            }
        }
        Some(true)
    } else {
        // Fecha/hora: ISO 8601 es comparable como string.
        let v = value.trim();
        if let Some(mn) = min.as_deref().map(str::trim) {
            if !mn.is_empty() && v < mn {
                return Some(false);
            }
        }
        if let Some(mx) = max.as_deref().map(str::trim) {
            if !mx.is_empty() && v > mx {
                return Some(false);
            }
        }
        Some(true)
    }
}

/// Tags de control de formulario (para `:enabled`/`:optional`).
pub(crate) fn is_form_control(node: &markup5ever_rcdom::Handle) -> bool {
    matches!(
        dom::element_name(node).as_deref(),
        Some("input" | "select" | "textarea" | "button" | "option" | "optgroup" | "fieldset")
    )
}

/// Controles editables (para `:read-write`): `<textarea>`, `<input>` y
/// cualquier elemento con `contenteditable`.
pub(crate) fn is_editable_control(node: &markup5ever_rcdom::Handle) -> bool {
    matches!(dom::element_name(node).as_deref(), Some("textarea" | "input"))
        || dom::attr(node, "contenteditable").is_some_and(|v| v != "false")
}

impl Rule {
    #[allow(dead_code)]
    pub(crate) fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        self.matches_in_state(node, false, false)
    }

    pub(crate) fn matches_in_state(
        &self,
        node: &markup5ever_rcdom::Handle,
        hover_active: bool,
        focus_active: bool,
    ) -> bool {
        selector_matches_subject(&self.selector, node, hover_active, focus_active)
    }
}

/// Matchea un `Selector` complejo (compounds + combinadores) contra `node`
/// como SUJETO (el compound más a la derecha debe matchear `node`; avanza
/// derecha→izquierda por la cadena). Reutilizado por `Rule::matches_in_state`
/// y por las pseudo-clases funcionales `:is()`/`:where()`/`:not()`/`:has()`
/// que aceptan selectores complejos (CSS Selectors 4). Fase 7.938.
pub(crate) fn selector_matches_subject(
    selector: &Selector,
    node: &markup5ever_rcdom::Handle,
    hover_active: bool,
    focus_active: bool,
) -> bool {
    {
        let compounds = &selector.compounds;
        if compounds.is_empty() {
            return false;
        }
        // El sujeto (último) debe matchear el nodo. Los ancestros/hermanos
        // siguen matcheando sin los flags activos (un `:hover/:focus`
        // sólo aplica al sujeto del selector, no propagamos el estado
        // por la cadena — es suficiente para 90% del CSS real).
        if !compounds.last().unwrap().matches_in_state(node, hover_active, focus_active) {
            return false;
        }
        if compounds.len() == 1 {
            return true;
        }
        // Avanzamos derecha→izquierda, encadenando combinadores. Cada
        // combinador define cómo viajar al "siguiente" candidato:
        //   Descendant/Child  → ancestro
        //   Adjacent/General  → hermano anterior
        let combs = &selector.combinators;
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

pub(crate) fn parent_of(node: &markup5ever_rcdom::Handle) -> Option<markup5ever_rcdom::Handle> {
    let weak = node.parent.take();
    let restored = weak.clone();
    node.parent.set(restored);
    weak.and_then(|w| w.upgrade())
}

/// Hermano Element anterior (saltea texto/whitespace nodes). Devuelve
/// `None` si no hay padre o si no hay Element previo bajo el mismo padre.
pub(crate) fn prev_element_sibling(
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
