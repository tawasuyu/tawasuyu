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
                    // especificidad de su argumento MÁS específico (para un
                    // solo arg, idéntico a contar sus partes).
                    Pseudo::Not(list) | Pseudo::Is(list) => {
                        extra += list.iter().map(compound_specificity).max().unwrap_or(0);
                    }
                    // `:has(...)` aporta la especificidad de su argumento más
                    // específico (CSS spec, igual que `:is`).
                    Pseudo::Has(rels) => {
                        extra += rels
                            .iter()
                            .map(|r| compound_specificity(&r.compound))
                            .max()
                            .unwrap_or(0);
                    }
                    // `:where(...)` no aporta especificidad.
                    Pseudo::Where(_) => {}
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

/// Especificidad (a*100 + b*10 + c) de UN compound, para resolver el aporte
/// de `:is(...)`. Aproxima `:is`/`:where` anidados como una clase (raro).
pub(crate) fn compound_specificity(c: &Compound) -> u32 {
    let ids = c.ids.len() as u32;
    let mut classes = c.classes.len() as u32 + c.attrs.len() as u32;
    let types = u32::from(matches!(c.tag, TagPart::Type(_)));
    for p in &c.pseudos {
        match p {
            Pseudo::Where(_) => {}
            _ => classes += 1,
        }
    }
    ids * 100 + classes * 10 + types
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
    /// `:nth-child(an+b)` — match si la posición 1-indexed del nodo en
    /// el padre satisface `pos = a*k + b` para algún `k >= 0`.
    NthChild {
        a: i32,
        b: i32,
    },
    /// `:not(a, b, ...)` — negación de una lista de compounds simples (sin
    /// combinadores ni `:not` anidado). Matchea si NINGUNO matchea.
    Not(Vec<Compound>),
    /// `:nth-of-type(an+b)` — posición 1-indexed entre hermanos del MISMO tag.
    NthOfType {
        a: i32,
        b: i32,
    },
    /// `:nth-last-child(an+b)` — posición contando desde el final.
    NthLastChild {
        a: i32,
        b: i32,
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
    /// `:is(a, b, ...)` — matchea si CUALQUIER compound de la lista matchea.
    /// Especificidad: la del argumento más específico (CSS spec).
    Is(Vec<Compound>),
    /// `:where(a, b, ...)` — como `:is` pero aporta especificidad CERO.
    Where(Vec<Compound>),
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
    /// Pseudo-clase estándar reconocida pero NO evaluable con el estado que
    /// rastreamos (validación de formularios, estado de media/popover/dialog,
    /// `:active`/`:visited`/`:target`…). Se parsea para NO tirar la regla
    /// entera (comportamiento de browser real, donde estos selectores son
    /// válidos); evalúa al `bool` guardado. `:scope` → `true` (transparente).
    /// Fase 7.933.
    Inert(bool),
}

/// Una relative selector de `:has(...)`: un combinador (descendiente por
/// defecto) + un compound. `:has(> .a)` → `{Child, .a}`.
#[derive(Debug, Clone)]
pub(crate) struct RelativeSelector {
    pub(crate) combinator: Combinator,
    pub(crate) compound: Compound,
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
                .any(|c| c.matches_in_state(node, hover_active, focus_active))
        }
        Pseudo::Checked => return has("checked") || has("selected"),
        Pseudo::Disabled => return has("disabled"),
        Pseudo::Enabled => return is_form_control(node) && !has("disabled"),
        Pseudo::Required => return has("required"),
        Pseudo::Optional => return is_form_control(node) && !has("required"),
        Pseudo::ReadOnly => return has("readonly"),
        Pseudo::ReadWrite => return is_editable_control(node) && !has("readonly"),
        Pseudo::Is(list) | Pseudo::Where(list) => {
            return list
                .iter()
                .any(|c| c.matches_in_state(node, hover_active, focus_active))
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
        | Pseudo::Is(_)
        | Pseudo::Where(_)
        | Pseudo::Empty
        | Pseudo::Root
        | Pseudo::AnyLink
        | Pseudo::Has(_)
        | Pseudo::Lang(_)
        | Pseudo::Inert(_) => unreachable!("ya resueltos arriba"),
        Pseudo::FirstChild => pos == 0,
        Pseudo::LastChild => pos + 1 == elems.len(),
        Pseudo::OnlyChild => elems.len() == 1,
        Pseudo::FirstOfType => type_pos == 0,
        Pseudo::LastOfType => type_pos + 1 == same_type.len(),
        Pseudo::OnlyOfType => same_type.len() == 1,
        Pseudo::NthChild { a, b } => nth_matches((pos + 1) as i32, *a, *b),
        Pseudo::NthLastChild { a, b } => nth_matches((elems.len() - pos) as i32, *a, *b),
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
    match rel.combinator {
        Combinator::Descendant => {
            any_descendant_matches(node, &rel.compound, hover_active, focus_active)
        }
        Combinator::Child => dom::children(node)
            .iter()
            .any(|c| rel.compound.matches_in_state(c, hover_active, focus_active)),
        Combinator::AdjacentSibling => following_element_siblings(node)
            .first()
            .is_some_and(|s| rel.compound.matches_in_state(s, hover_active, focus_active)),
        Combinator::GeneralSibling => following_element_siblings(node)
            .iter()
            .any(|s| rel.compound.matches_in_state(s, hover_active, focus_active)),
    }
}

/// `true` si algún descendiente (cualquier nivel, excluye el propio nodo)
/// matchea el compound.
fn any_descendant_matches(
    node: &markup5ever_rcdom::Handle,
    compound: &Compound,
    hover_active: bool,
    focus_active: bool,
) -> bool {
    for c in node.children.borrow().iter() {
        if dom::element_name(c).is_none() {
            continue;
        }
        if compound.matches_in_state(c, hover_active, focus_active)
            || any_descendant_matches(c, compound, hover_active, focus_active)
        {
            return true;
        }
    }
    false
}

/// `:lang(...)` — el idioma efectivo del elemento (atributo `lang` propio o
/// del ancestro más cercano) matchea si es igual a algún tag pedido o es un
/// subtag suyo (`lang="en-US"` ↔ `:lang(en)`). Case-insensitive.
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
        let compounds = &self.selector.compounds;
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
