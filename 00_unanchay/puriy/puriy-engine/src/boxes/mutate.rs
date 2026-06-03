//! Mutación y restyle del box tree ya construido: las APIs que el chrome usa
//! para reflejar cambios del JS (`synthesize_box_node`, set attribute/style/
//! classList, sync checked, restyle por cascada) + `set_box_visual` y la
//! herencia/propagación de color/font, parsers simples de color/px, y los
//! walks (`find_y`, count). Extraído de `boxes/mod.rs` (regla #1). Comparte
//! tipos del crate vía `use super::*`.
use super::*;

impl BoxTree {
    /// Cuenta total de boxes (incluyendo la raíz).
    pub fn descendants_count(&self) -> usize {
        count(&self.root)
    }

    /// Recorre el árbol pre-order y aplica `f` a cada box.
    pub fn walk(&self, mut f: impl FnMut(&BoxNode)) {
        walk_inner(&self.root, &mut f);
    }

    /// Estima la posición vertical (px desde el top del documento) del
    /// nodo con `element_id == id`. Usada por fragment navigation
    /// (`<a href="#foo">`) — el chrome ajusta `scroll_y` a este valor.
    /// La estimación suma margin+padding de bloques y `font_size *
    /// line_height` de hojas de texto en orden DFS; ignora layout real
    /// (taffy todavía no corrió cuando el chrome resuelve el click), así
    /// que el salto puede caer ~1 línea arriba o abajo del target. Es
    /// suficiente para que el usuario vea el destino sin perderse.
    pub fn find_element_y(&self, id: &str) -> Option<f32> {
        let mut acc = 0.0_f32;
        find_y_inner(&self.root, id, &mut acc)
    }

    /// Reemplaza el contenido de texto del subárbol del nodo con
    /// `element_id == id` por `new_text`. Implementa el caso simple de
    /// `el.textContent = X` desde JS:
    ///
    /// 1. Si el nodo target tiene `.text` directo, se reemplaza.
    /// 2. Sino, se reemplaza el PRIMER text leaf en orden DFS del
    ///    subárbol; los hijos no-text quedan intactos.
    /// 3. Si no hay text leaves, no se hace nada (Fase 7.5c — caso raro;
    ///    requeriría sintetizar un nuevo BoxNode con estilo del padre).
    ///
    /// Devuelve `true` si se aplicó la mutación, `false` si no se
    /// encontró el id o no había text leaves. Spec real de `textContent`
    /// es "reemplazar TODO el subárbol con un único text node"; nuestra
    /// aproximación cubre el 90% de los usos reales (clocks, contadores,
    /// banners) sin un refactor del modelo del box tree.
    pub fn set_element_text_content(&mut self, id: &str, new_text: &str) -> bool {
        replace_text_content(&mut self.root, id, new_text)
    }

    /// Aplica una mutación de estilo (proveniente de `el.style.X = Y`)
    /// al nodo con `element_id == id`. `prop` en kebab-case (`color`,
    /// `background-color`, `display`, `font-size`, `visibility`).
    ///
    /// Devuelve `true` si la mutación se aplicó. Props desconocidas o
    /// values no parseables devuelven `false` (silencioso — los setters
    /// JS publican igual; el chrome aplica sólo lo que sabe). Subset
    /// limitado a propósito; ampliar cuando aparezcan casos reales.
    pub fn set_element_style(&mut self, id: &str, prop: &str, value: &str) -> bool {
        set_element_style_inner(&mut self.root, id, prop, value)
    }

    /// Reemplaza la lista de clases del nodo `element_id == id` por
    /// `classes`. NO re-corre la cascada — el caller debe llamar
    /// [`Self::restyle`] después (típicamente una sola vez tras drenar
    /// todas las mutaciones de un evento). Devuelve `true` si encontró el
    /// nodo. Mantiene el atributo `class` de `attributes` en sync para que
    /// el DOM espejo del restyle lea las clases nuevas. Fase 7.184.
    pub fn set_element_class_list(&mut self, id: &str, classes: Vec<String>) -> bool {
        set_class_list_inner(&mut self.root, id, classes)
    }

    /// Sincroniza el atributo `checked` (presencia) de cada control de
    /// formulario con `checks[i]`, en orden DFS (el mismo que indexa el
    /// `input_checks` del chrome), para que un restyle re-evalúe
    /// `:checked`/`:checked + label`. NO recascadea — el caller llama
    /// [`Self::restyle`] después. Fase 7.187.
    pub fn sync_checked_from(&mut self, checks: &[bool]) {
        let mut counter = 0usize;
        sync_checked_inner(&mut self.root, checks, &mut counter);
    }

    /// Re-aplica la cascada CSS a TODO el árbol reusando las reglas
    /// retenidas (`self.styles`). Necesario tras un cambio de `classList`
    /// u otra mutación que altere qué reglas matchean: un cambio en una
    /// clase puede afectar descendientes (selectores descendientes,
    /// herencia) y hermanos posteriores (`+`/`~`), así que recascadeamos
    /// el documento entero. Reconstruye un DOM rcdom-espejo (sólo
    /// elementos) del box tree y corre el MISMO motor de cascada que el
    /// build inicial — sin duplicar el matcher. Fase 7.184.
    ///
    /// Limitaciones (documentadas en el SDD): no re-dropea ni resucita
    /// nodos `display:none` (los que arrancaron ocultos al cargar nunca se
    /// boxearon; los que están en el árbol sí togglean display); no
    /// recolapsa márgenes (preserva el `margin` ya colapsado); no re-deriva
    /// contenido de pseudo-elements ni animaciones.
    pub fn restyle(&mut self) {
        let BoxTree { root, styles, .. } = self;
        if root.tag.is_some() {
            if let Some(mirror) = mirror_element(root) {
                restyle_apply(root, &mirror, None, styles);
            }
        } else {
            // Root sintético (wrapper sin tag): aplica a sus hijos elemento
            // como top-level (parent None), igual que `build` con `<body>`.
            let doc = markup5ever_rcdom::Node::new(markup5ever_rcdom::NodeData::Document);
            collect_mirror_children(root, &doc);
            let mc = doc.children.borrow();
            let mut mi = 0usize;
            restyle_children(&mut root.children, &mc, &mut mi, None, styles);
        }
    }

    /// Setea / actualiza el atributo `name` del nodo `id`. `name` va con
    /// su prefijo completo (`data-foo`, `aria-checked`, `href`, etc.) y
    /// debe venir ya en lowercase kebab. Devuelve `true` si encontró el
    /// nodo. Fase 7.16 — `el.setAttribute(name, value)` publica esta
    /// mutación; también la usan internamente los setters `data-*`.
    pub fn set_element_attribute(&mut self, id: &str, name: &str, value: &str) -> bool {
        set_attribute_inner(&mut self.root, id, name, Some(value))
    }

    /// Borra el atributo `name` del nodo `id`. Devuelve `true` si
    /// encontró el nodo (haya o no existido la key). Fase 7.16.
    pub fn remove_element_attribute(&mut self, id: &str, name: &str) -> bool {
        set_attribute_inner(&mut self.root, id, name, None)
    }

    /// Wrapper sobre `set_element_attribute` que reconstruye `data-<key>`
    /// para preservar la API de Fase 7.11. `key` va sin prefijo.
    pub fn set_element_dataset(&mut self, id: &str, key: &str, value: &str) -> bool {
        self.set_element_attribute(id, &format!("data-{}", key), value)
    }

    /// Wrapper sobre `remove_element_attribute`. Fase 7.11/7.16.
    pub fn remove_element_dataset(&mut self, id: &str, key: &str) -> bool {
        self.remove_element_attribute(id, &format!("data-{}", key))
    }

    /// Agrega `child` como último hijo del nodo `parent_id`. Fase 7.12.
    /// Devuelve `true` si encontró el parent. El `child` viene sintético
    /// (creado por `synthesize_box_node`); no se valida que su id sea
    /// único en el árbol.
    pub fn append_child_to(&mut self, parent_id: &str, child: BoxNode) -> bool {
        if let Some(parent) = find_node_mut(&mut self.root, parent_id) {
            // Fase 7.14 — heredar font/color/etc. del parent antes
            // de insertar. Sin esto, los nodos sintéticos quedan con
            // defaults (black/16px) ignorando el contexto visual del
            // padre.
            let mut child = child;
            inherit_style_to_child(parent, &mut child);
            parent.children.push(child);
            true
        } else {
            false
        }
    }

    /// Quita el primer descendiente con `element_id == child_id` que
    /// sea hijo directo del nodo `parent_id`. Devuelve `true` si quitó
    /// algo. Fase 7.12.
    pub fn remove_child_by_id(&mut self, parent_id: &str, child_id: &str) -> bool {
        if let Some(parent) = find_node_mut(&mut self.root, parent_id) {
            let before = parent.children.len();
            parent
                .children
                .retain(|c| c.element_id.as_deref() != Some(child_id));
            parent.children.len() < before
        } else {
            false
        }
    }

    /// Inserta `child` antes del primer hijo directo de `parent_id`
    /// cuyo `element_id == ref_id`. Si `ref_id` no se encuentra, hace
    /// fallback a append. Devuelve `true` si encontró el parent.
    /// Fase 7.14.
    pub fn insert_child_before(
        &mut self,
        parent_id: &str,
        child: BoxNode,
        ref_id: &str,
    ) -> bool {
        if let Some(parent) = find_node_mut(&mut self.root, parent_id) {
            let pos = parent
                .children
                .iter()
                .position(|c| c.element_id.as_deref() == Some(ref_id));
            let mut child = child;
            inherit_style_to_child(parent, &mut child);
            match pos {
                Some(i) => parent.children.insert(i, child),
                None => parent.children.push(child),
            }
            true
        } else {
            false
        }
    }
}

/// Fase 7.14 — copia las propiedades CSS-heredables del padre al child
/// sintético recién insertado, y propaga al text leaf interno si existe.
/// Sin esto, los nodos creados por `createElement` quedan con defaults
/// de `empty_root()` (color black, font_size 16px, etc.), ignorando el
/// contexto visual del padre.
///
/// Heredables (CSS spec): `color`, `font_size`, `font_weight`,
/// `font_style`, `font_family`, `line_height`, `text_align`,
/// `text_decoration`, `white_space`, `text_transform`. NO heredables:
/// `background`, `display`, `margin`, `padding`, `width`, etc.
pub(crate) fn inherit_style_to_child(parent: &BoxNode, child: &mut BoxNode) {
    child.color = parent.color;
    child.font_size = parent.font_size;
    child.font_weight = parent.font_weight;
    child.font_style = parent.font_style;
    child.font_family = parent.font_family.clone();
    child.line_height = parent.line_height;
    child.text_align = parent.text_align;
    child.text_decoration = parent.text_decoration;
    child.text_decoration_color = parent.text_decoration_color;
    child.text_decoration_style = parent.text_decoration_style;
    child.white_space = parent.white_space;
    child.text_transform = parent.text_transform;
    // Propagar al text leaf interno (primer hijo si es text node).
    for c in child.children.iter_mut() {
        if c.text.is_some() {
            c.color = child.color;
            c.font_size = child.font_size;
            c.font_weight = child.font_weight;
            c.font_style = child.font_style;
            c.font_family = child.font_family.clone();
            c.line_height = child.line_height;
            c.text_decoration = child.text_decoration;
            c.text_decoration_color = child.text_decoration_color;
            c.text_decoration_style = child.text_decoration_style;
        }
    }
}

/// Construye un `BoxNode` sintético para `el.appendChild(createElement(...))`.
/// Inicializa con defaults de `empty_root()` y customiza tag/id/text/
/// class_list/input_initial según los campos provenientes del payload
/// JS. Display elegido por tag: bloques comunes (`div`/`p`/`h1..h6`/
/// `ul`/`ol`/`li`/`section`/`article`/`header`/`footer`/`nav`/`main`)
/// son block; el resto es inline. UA stylesheet no se re-aplica — los
/// estilos se mantienen en defaults. Fase 7.12.
pub fn synthesize_box_node(
    tag: &str,
    id: Option<&str>,
    text_content: &str,
    class_list: Vec<String>,
    value: Option<&str>,
) -> BoxNode {
    // Fase 7.19 — tag vacío significa text node (createTextNode). El
    // BoxNode resultante es inline sin tag y con `text = Some(content)`.
    // El padre lo trata como cualquier otro text leaf; herencia de
    // estilos via inherit_style_to_child al append.
    if tag.is_empty() {
        let mut leaf = empty_root();
        leaf.display = Display::Inline;
        leaf.tag = None;
        leaf.text = Some(text_content.to_string());
        leaf.element_id = id.map(|s| s.to_string());
        return leaf;
    }
    let mut node = empty_root();
    node.tag = Some(tag.to_string());
    node.element_id = id.map(|s| s.to_string());
    node.class_list = class_list;
    // Display por tag — heurística simple (sin UA cascade). Suficiente
    // para que appendChild de `<li>`, `<div>`, `<p>` rendere como bloque.
    let display = match tag.to_ascii_lowercase().as_str() {
        "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "li"
        | "section" | "article" | "header" | "footer" | "nav" | "main" | "aside"
        | "blockquote" | "pre" | "table" | "tr" | "tbody" | "thead" | "tfoot"
        | "figure" | "figcaption" | "address" | "hr" => Display::Block,
        "br" => Display::Block,
        "span" | "a" | "b" | "i" | "em" | "strong" | "small" | "code" | "u" | "s"
        | "del" | "ins" | "mark" | "sub" | "sup" | "kbd" | "var" | "samp" | "abbr"
        | "cite" | "q" | "time" | "label" => Display::Inline,
        _ => Display::Block,
    };
    node.display = display;
    // textContent: si es no-vacío, agregamos un text leaf como único
    // hijo. Hereda font_size/color del nodo padre via cascade — pero
    // como acá no corremos cascade, usamos los defaults.
    if !text_content.is_empty() {
        let mut leaf = empty_root();
        leaf.display = Display::Inline;
        leaf.tag = None;
        leaf.text = Some(text_content.to_string());
        leaf.font_size = node.font_size;
        leaf.color = node.color;
        node.children.push(leaf);
    }
    // input_initial para inputs con value pre-set.
    if let Some(v) = value {
        if !v.is_empty() {
            node.input_initial = Some(v.to_string());
        }
    }
    node
}

/// Busca el primer descendiente (incluyendo el root) con `element_id`
/// igual a `target` y devuelve `&mut` a él. Pre-order DFS. None si no
/// existe. Fase 7.12 — helper para mutaciones estructurales.
pub(crate) fn find_node_mut<'a>(root: &'a mut BoxNode, target: &str) -> Option<&'a mut BoxNode> {
    if root.element_id.as_deref() == Some(target) {
        return Some(root);
    }
    for c in root.children.iter_mut() {
        if let Some(found) = find_node_mut(c, target) {
            return Some(found);
        }
    }
    None
}

pub(crate) fn set_attribute_inner(
    node: &mut BoxNode,
    target: &str,
    name: &str,
    value: Option<&str>,
) -> bool {
    if node.element_id.as_deref() == Some(target) {
        node.attributes.retain(|(k, _)| k != name);
        if let Some(v) = value {
            node.attributes.push((name.to_string(), v.to_string()));
        }
        return true;
    }
    for c in node.children.iter_mut() {
        if set_attribute_inner(c, target, name, value) {
            return true;
        }
    }
    false
}

pub(crate) fn set_element_style_inner(
    node: &mut BoxNode,
    target: &str,
    prop: &str,
    value: &str,
) -> bool {
    if node.element_id.as_deref() == Some(target) {
        // Persistimos la declaración inline en el atributo `style` para que
        // un restyle posterior (classList) la re-aplique con prioridad inline
        // — sin esto, la cascada pisaría lo que JS seteó vía `el.style.X`.
        upsert_inline_style_attr(node, prop, value);
        return apply_style_to_node(node, prop, value);
    }
    for c in node.children.iter_mut() {
        if set_element_style_inner(c, target, prop, value) {
            return true;
        }
    }
    false
}

/// Inserta o actualiza una declaración `prop: value` en el atributo `style`
/// del nodo (kebab `prop`). Mantiene el resto de las declaraciones inline.
/// Usado para que `el.style.X = Y` (Fase 7.8) persista a través del restyle
/// (Fase 7.184), que re-parsea el atributo `style` desde el DOM espejo.
pub(crate) fn upsert_inline_style_attr(node: &mut BoxNode, prop: &str, value: &str) {
    let prop = prop.trim();
    if prop.is_empty() {
        return;
    }
    let existing = node
        .attributes
        .iter()
        .find(|(k, _)| k == "style")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let mut decls: Vec<(String, String)> = Vec::new();
    for seg in existing.split(';') {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        if let Some((k, v)) = seg.split_once(':') {
            decls.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    if let Some(slot) = decls.iter_mut().find(|(k, _)| k == prop) {
        slot.1 = value.trim().to_string();
    } else {
        decls.push((prop.to_string(), value.trim().to_string()));
    }
    let serialized = decls
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("; ");
    if let Some(slot) = node.attributes.iter_mut().find(|(k, _)| k == "style") {
        slot.1 = serialized;
    } else {
        node.attributes.push(("style".to_string(), serialized));
    }
}

pub(crate) fn apply_style_to_node(node: &mut BoxNode, prop: &str, value: &str) -> bool {
    let val = value.trim();
    match prop {
        "color" => {
            if let Some(c) = parse_simple_color(val) {
                node.color = c;
                propagate_text_color(node, c);
                return true;
            }
        }
        "background" | "background-color" => {
            if val.eq_ignore_ascii_case("none") || val.eq_ignore_ascii_case("transparent") {
                node.background = None;
                return true;
            }
            if let Some(c) = parse_simple_color(val) {
                node.background = Some(c);
                return true;
            }
        }
        "display" => {
            let d = match val.to_ascii_lowercase().as_str() {
                "none" => Some(Display::None),
                "block" => Some(Display::Block),
                "inline" => Some(Display::Inline),
                "inline-block" => Some(Display::InlineBlock),
                "flex" => Some(Display::Flex),
                "grid" => Some(Display::Grid),
                _ => None,
            };
            if let Some(d) = d {
                node.display = d;
                return true;
            }
        }
        "font-size" => {
            if let Some(px) = parse_px(val) {
                node.font_size = px;
                propagate_font_size(node, px);
                return true;
            }
        }
        "visibility" => {
            // Aproximación: hidden → display:none (perdemos el espacio
            // reservado; spec real lo mantiene). Suficiente para toggle
            // show/hide del 90% de los casos.
            if val.eq_ignore_ascii_case("hidden") {
                node.display = Display::None;
                return true;
            }
            if val.eq_ignore_ascii_case("visible") {
                return true; // no-op por ahora
            }
        }
        _ => {}
    }
    false
}

// ===================== Fase 7.184 — restyle on classList =====================

/// Reemplaza recursivamente la `class_list` (y el atributo `class` espejo)
/// del nodo con `element_id == id`. Devuelve `true` si lo encontró.
pub(crate) fn set_class_list_inner(node: &mut BoxNode, id: &str, classes: Vec<String>) -> bool {
    if node.element_id.as_deref() == Some(id) {
        // Sincroniza el atributo `class` para que el DOM espejo del restyle
        // (que lee de `attributes`) y `class_list` no diverjan.
        let joined = classes.join(" ");
        if let Some(slot) = node.attributes.iter_mut().find(|(k, _)| k == "class") {
            slot.1 = joined;
        } else if !joined.is_empty() {
            node.attributes.push(("class".to_string(), joined));
        }
        node.class_list = classes;
        return true;
    }
    for c in node.children.iter_mut() {
        if set_class_list_inner(c, id, classes.clone()) {
            return true;
        }
    }
    false
}

/// Recorre en DFS los controles (`input_kind.is_some()`) y fija/quita el
/// atributo `checked` de cada uno según `checks[counter]`.
pub(crate) fn sync_checked_inner(node: &mut BoxNode, checks: &[bool], counter: &mut usize) {
    if node.input_kind.is_some() {
        let checked = checks.get(*counter).copied().unwrap_or(false);
        let has = node.attributes.iter().any(|(k, _)| k == "checked");
        if checked && !has {
            node.attributes.push(("checked".to_string(), String::new()));
        } else if !checked && has {
            node.attributes.retain(|(k, _)| k != "checked");
        }
        *counter += 1;
    }
    for c in node.children.iter_mut() {
        sync_checked_inner(c, checks, counter);
    }
}

/// Construye un Element rcdom espejo de un BoxNode elemento (`tag.is_some()`),
/// con `id`/`class`/`style` + el resto de `attributes`, y sus hijos elemento
/// (aplanando wrappers `tag=None`). Devuelve `None` si el box no es elemento.
pub(crate) fn mirror_element(b: &BoxNode) -> Option<Handle> {
    use markup5ever::interface::{Attribute, QualName};
    use markup5ever::{LocalName, Namespace};
    use markup5ever_rcdom::Node;
    use std::cell::RefCell;

    let tag = b.tag.as_deref()?;
    let mk_attr = |name: &str, val: &str| Attribute {
        name: QualName::new(None, Namespace::from(""), LocalName::from(name)),
        value: val.into(),
    };
    let mut attrs: Vec<Attribute> = Vec::new();
    // `id`/`class` desde los campos canónicos (la mutación de classList los
    // actualiza ahí); el resto desde `attributes` sin pisar id/class.
    if let Some(id) = b.element_id.as_deref() {
        attrs.push(mk_attr("id", id));
    }
    if !b.class_list.is_empty() {
        attrs.push(mk_attr("class", &b.class_list.join(" ")));
    }
    for (k, v) in &b.attributes {
        if k == "id" || k == "class" {
            continue;
        }
        attrs.push(mk_attr(k, v));
    }
    let elem = Node::new(NodeData::Element {
        name: QualName::new(None, Namespace::from(""), LocalName::from(tag)),
        attrs: RefCell::new(attrs),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    });
    collect_mirror_children(b, &elem);
    Some(elem)
}

/// Empuja los Element espejo de los hijos ELEMENTO de `b` bajo
/// `parent_mirror`, aplanando los wrappers `tag=None` (text leaves, markers,
/// pseudo-content) — en el DOM real esos no son ancestros de los elementos.
pub(crate) fn collect_mirror_children(b: &BoxNode, parent_mirror: &Handle) {
    use std::rc::Rc;
    for child in &b.children {
        if child.tag.is_some() {
            if let Some(cm) = mirror_element(child) {
                cm.parent.set(Some(Rc::downgrade(parent_mirror)));
                parent_mirror.children.borrow_mut().push(cm);
            }
        } else {
            collect_mirror_children(child, parent_mirror);
        }
    }
}

/// Computa el estilo re-cascadeado de `b` (un elemento, pareado con su
/// `mirror`) y lo aplica; luego recursa sobre sus hijos. `mirror.children`
/// está en el mismo orden (elementos aplanados) que recorre `restyle_children`.
pub(crate) fn restyle_apply(
    b: &mut BoxNode,
    mirror: &Handle,
    parent_cs: Option<&ComputedStyle>,
    styles: &StyleEngine,
) {
    let cs = styles.compute_with_parent(mirror, parent_cs);
    // Deltas de hover/focus, igual criterio que `build_node`.
    let hover_bg = {
        let h = styles.compute_with_parent_in_state(mirror, parent_cs, true);
        (h.background != cs.background).then_some(h.background).flatten()
    };
    let focus_bg = {
        let f = styles.compute_with_parent_for_state(mirror, parent_cs, false, true);
        (f.background != cs.background).then_some(f.background).flatten()
    };
    set_box_visual(b, &cs, hover_bg, focus_bg);
    let mc = mirror.children.borrow();
    let mut mi = 0usize;
    restyle_children(&mut b.children, &mc, &mut mi, Some(&cs), styles);
}

/// Recorre los hijos de un elemento, pareando cada hijo ELEMENTO con el
/// siguiente espejo (`mc[mi]`) y propagando estilo a los text leaves. Los
/// wrappers `tag=None` se atraviesan transparentes (sin consumir espejo).
pub(crate) fn restyle_children(
    children: &mut [BoxNode],
    mc: &[Handle],
    mi: &mut usize,
    parent_cs: Option<&ComputedStyle>,
    styles: &StyleEngine,
) {
    for child in children.iter_mut() {
        if child.tag.is_some() {
            if let Some(cm) = mc.get(*mi) {
                restyle_apply(child, cm, parent_cs, styles);
            }
            *mi += 1;
        } else {
            if let Some(p) = parent_cs {
                set_leaf_inherited(child, p);
            }
            // Wrapper sin tag: atravesar a sus hijos manteniendo el mismo
            // espejo/cursor (sus elementos son hijos del MISMO ancestro).
            restyle_children(&mut child.children, mc, mi, parent_cs, styles);
        }
    }
}

/// Sobrescribe los campos visuales derivados del estilo en un BoxNode
/// existente, preservando estructura/text/imagen/link/inputs y el `margin`
/// ya colapsado (no recolapsamos en restyle).
pub(crate) fn set_box_visual(b: &mut BoxNode, s: &ComputedStyle, hover_bg: Option<Color>, focus_bg: Option<Color>) {
    b.display = s.display;
    b.background = s.background;
    b.color = s.color;
    b.font_size = s.font_size;
    b.font_weight = s.font_weight;
    b.font_style = s.font_style;
    b.font_family = s.font_family.clone();
    b.padding = s.padding;
    b.width = s.width;
    b.height = s.height;
    b.max_width = s.max_width;
    b.text_align = s.text_align;
    b.line_height = s.line_height;
    b.border_widths = s.border_widths;
    b.border_colors = s.border_colors;
    b.border_radii = s.border_radii;
    b.hover_background = hover_bg;
    b.focus_background = focus_bg;
    b.box_shadow = s.box_shadow;
    b.z_index = s.z_index;
    b.flex_direction = s.flex_direction;
    b.justify_content = s.justify_content;
    b.align_items = s.align_items;
    b.align_content = s.align_content;
    b.justify_items = s.justify_items;
    b.justify_self = s.justify_self;
    b.flex_wrap = s.flex_wrap;
    b.gap_row = s.gap_row;
    b.gap_column = s.gap_column;
    b.box_sizing = s.box_sizing;
    b.min_width = s.min_width;
    b.min_height = s.min_height;
    b.max_height = s.max_height;
    b.aspect_ratio = s.aspect_ratio;
    b.overflow = s.overflow;
    b.white_space = s.white_space;
    b.text_transform = s.text_transform;
    b.opacity = s.opacity;
    b.align_self = s.align_self;
    b.flex_grow = s.flex_grow;
    b.flex_shrink = s.flex_shrink;
    b.flex_basis = s.flex_basis;
    b.outline = s.outline;
    b.background_gradient = s.background_gradient.clone();
    b.background_size = s.background_size;
    b.background_position = s.background_position;
    b.background_repeat = s.background_repeat;
    b.background_origin = s.background_origin;
    b.background_clip = s.background_clip;
    b.position = s.position;
    b.inset_top = s.inset_top;
    b.inset_right = s.inset_right;
    b.inset_bottom = s.inset_bottom;
    b.inset_left = s.inset_left;
    b.vertical_align = s.vertical_align;
    b.visibility = s.visibility;
    b.pointer_events = s.pointer_events;
    b.object_fit = s.object_fit;
    b.object_position = s.object_position;
    b.text_indent = s.text_indent;
    b.word_spacing = s.word_spacing;
    b.letter_spacing = s.letter_spacing;
    b.text_shadows = s.text_shadows.clone();
    b.transforms = s.transforms.clone();
    b.grid_template_columns = s.grid_template_columns.clone();
    b.grid_template_rows = s.grid_template_rows.clone();
    b.text_decoration = s.text_decoration;
    b.text_decoration_color = s.text_decoration_color;
    b.text_decoration_style = s.text_decoration_style;
}

/// Propaga las propiedades CSS heredables del estilo del padre a una hoja
/// de texto (mismo subconjunto que copia `compute_internal` del padre).
pub(crate) fn set_leaf_inherited(leaf: &mut BoxNode, p: &ComputedStyle) {
    leaf.color = p.color;
    leaf.font_size = p.font_size;
    leaf.font_weight = p.font_weight;
    leaf.font_style = p.font_style;
    leaf.font_family = p.font_family.clone();
    leaf.text_align = p.text_align;
    leaf.line_height = p.line_height;
    leaf.text_decoration = p.text_decoration;
    leaf.text_decoration_color = p.text_decoration_color;
    leaf.text_decoration_style = p.text_decoration_style;
    leaf.white_space = p.white_space;
    leaf.text_transform = p.text_transform;
    leaf.text_shadows = p.text_shadows.clone();
    leaf.word_spacing = p.word_spacing;
    leaf.letter_spacing = p.letter_spacing;
    leaf.text_indent = p.text_indent;
    leaf.visibility = p.visibility;
    leaf.pointer_events = p.pointer_events;
}

pub(crate) fn propagate_text_color(node: &mut BoxNode, c: Color) {
    if node.text.is_some() {
        node.color = c;
    }
    for child in node.children.iter_mut() {
        propagate_text_color(child, c);
    }
}

pub(crate) fn propagate_font_size(node: &mut BoxNode, size: f32) {
    if node.text.is_some() {
        node.font_size = size;
    }
    for child in node.children.iter_mut() {
        propagate_font_size(child, size);
    }
}

/// Parser mínimo de colores para `el.style.X = Y`. Acepta: `#rgb`,
/// `#rrggbb`, palabras CSS comunes (red, blue, green, black, white,
/// gray, yellow, orange, pink, purple, cyan, magenta, transparent).
pub(crate) fn parse_simple_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    let lower = s.to_ascii_lowercase();
    let (r, g, b) = match lower.as_str() {
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "blue" => (0, 0, 255),
        "yellow" => (255, 255, 0),
        "orange" => (255, 165, 0),
        "pink" => (255, 192, 203),
        "purple" => (128, 0, 128),
        "cyan" | "aqua" => (0, 255, 255),
        "magenta" | "fuchsia" => (255, 0, 255),
        "gray" | "grey" => (128, 128, 128),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "darkgray" | "darkgrey" => (169, 169, 169),
        _ => return None,
    };
    Some(Color { r, g, b, a: 255 })
}

pub(crate) fn parse_hex_color(hex: &str) -> Option<Color> {
    let h = hex.trim();
    match h.len() {
        3 => {
            let r = u8::from_str_radix(&h[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&h[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&h[2..3].repeat(2), 16).ok()?;
            Some(Color { r, g, b, a: 255 })
        }
        6 => {
            let r = u8::from_str_radix(&h[0..2], 16).ok()?;
            let g = u8::from_str_radix(&h[2..4], 16).ok()?;
            let b = u8::from_str_radix(&h[4..6], 16).ok()?;
            Some(Color { r, g, b, a: 255 })
        }
        _ => None,
    }
}

pub(crate) fn parse_px(s: &str) -> Option<f32> {
    let t = s.trim();
    let stripped = t.strip_suffix("px").unwrap_or(t);
    stripped.trim().parse::<f32>().ok()
}

pub(crate) fn replace_text_content(node: &mut BoxNode, target: &str, new_text: &str) -> bool {
    if node.element_id.as_deref() == Some(target) {
        return replace_first_text_leaf(node, new_text);
    }
    for c in node.children.iter_mut() {
        if replace_text_content(c, target, new_text) {
            return true;
        }
    }
    false
}

pub(crate) fn replace_first_text_leaf(node: &mut BoxNode, new_text: &str) -> bool {
    if node.text.is_some() {
        node.text = Some(new_text.to_string());
        return true;
    }
    for c in node.children.iter_mut() {
        if replace_first_text_leaf(c, new_text) {
            return true;
        }
    }
    false
}

pub(crate) fn find_y_inner(b: &BoxNode, target: &str, acc: &mut f32) -> Option<f32> {
    if b.element_id.as_deref() == Some(target) {
        return Some(*acc);
    }
    if b.text.is_some() {
        // Hoja de texto: una línea de altura font_size * line_height.
        *acc += b.font_size * b.line_height.unwrap_or(1.2);
        return None;
    }
    // Block-ish: contribución de borders verticales del lado top.
    *acc += b.margin.top + b.padding.top;
    for c in &b.children {
        if let Some(y) = find_y_inner(c, target, acc) {
            return Some(y);
        }
    }
    *acc += b.padding.bottom + b.margin.bottom;
    None
}

impl BoxTree {
    /// Estima la y del N-ésimo (1-based) leaf de texto cuyo contenido
    /// contiene `query_lower` (la query debe venir ya lowercased — el
    /// caller suele hacerlo una vez fuera del walk). Usado por la find
    /// bar para auto-scroll al match actual con Enter/Shift+Enter.
    pub fn find_y_of_match(&self, query_lower: &str, nth_1based: usize) -> Option<f32> {
        if query_lower.is_empty() || nth_1based == 0 {
            return None;
        }
        let mut acc = 0.0_f32;
        let mut seen = 0_usize;
        find_match_y_inner(&self.root, query_lower, nth_1based, &mut acc, &mut seen)
    }
}

pub(crate) fn find_match_y_inner(
    b: &BoxNode,
    query: &str,
    target_nth: usize,
    acc: &mut f32,
    seen: &mut usize,
) -> Option<f32> {
    if let Some(text) = &b.text {
        if text.to_lowercase().contains(query) {
            *seen += 1;
            if *seen == target_nth {
                return Some(*acc);
            }
        }
        *acc += b.font_size * b.line_height.unwrap_or(1.2);
        return None;
    }
    *acc += b.margin.top + b.padding.top;
    for c in &b.children {
        if let Some(y) = find_match_y_inner(c, query, target_nth, acc, seen) {
            return Some(y);
        }
    }
    *acc += b.padding.bottom + b.margin.bottom;
    None
}

pub(crate) fn count(b: &BoxNode) -> usize {
    1 + b.children.iter().map(count).sum::<usize>()
}

pub(crate) fn walk_inner(b: &BoxNode, f: &mut impl FnMut(&BoxNode)) {
    f(b);
    for c in &b.children {
        walk_inner(c, f);
    }
}
