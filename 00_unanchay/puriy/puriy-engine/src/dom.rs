//! DOM — wrapper sobre `markup5ever_rcdom::RcDom`.
//!
//! El DOM Rc-based es el "sidecar" oficial de `html5ever`: no soporta
//! mutación concurrente ni eventos JS, pero es exactamente lo que
//! necesitamos para Fase 2 (parsear y volcar a box tree). Si en Fase 3+
//! necesitamos `getElementById` rápido o reflow tras DOM mutation,
//! migramos a Stylo + un DOM custom; por ahora `RcDom` es de sobra.

use std::cell::RefCell;
use std::rc::Rc;

use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, ParseOpts};
use markup5ever::interface::QualName;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// DOM parseado. Internamente usa [`RcDom`].
pub struct DomTree {
    dom: RcDom,
}

impl DomTree {
    /// Parsea una cadena HTML (la cabecera "<!DOCTYPE html>" es opcional —
    /// html5ever asume HTML5 quirks-aware por default).
    pub fn parse(html: &str) -> Self {
        let dom = parse_document(RcDom::default(), ParseOpts::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())
            .expect("RcDom::read_from no falla con &[u8] en memoria");
        Self { dom }
    }

    /// Nodo raíz (Document → primer hijo es <html>).
    pub fn document(&self) -> Handle {
        self.dom.document.clone()
    }

    /// Recupera el `<title>` plano si existe.
    pub fn title(&self) -> Option<String> {
        let title = find_first(&self.document(), "title")?;
        Some(collect_text(&title))
    }

    /// Devuelve el primer nodo cuyo nombre local coincide.
    pub fn find(&self, local: &str) -> Option<Handle> {
        find_first(&self.document(), local)
    }

    /// Itera in-order todos los `<style>` y devuelve sus textos
    /// concatenados — entrada para el [`StyleEngine`](crate::StyleEngine).
    pub fn collect_inline_stylesheets(&self) -> Vec<String> {
        let mut out = Vec::new();
        walk(&self.document(), &mut |node| {
            if let NodeData::Element { name, .. } = &node.data {
                if name.local.as_ref() == "style" {
                    out.push(collect_text(node));
                }
            }
        });
        out
    }
}

/// DFS pre-order. La closure recibe cada Handle.
pub(crate) fn walk(node: &Handle, f: &mut impl FnMut(&Handle)) {
    f(node);
    for child in node.children.borrow().iter() {
        walk(child, f);
    }
}

fn find_first(node: &Handle, local: &str) -> Option<Handle> {
    if let NodeData::Element { name, .. } = &node.data {
        if name.local.as_ref() == local {
            return Some(node.clone());
        }
    }
    for child in node.children.borrow().iter() {
        if let Some(found) = find_first(child, local) {
            return Some(found);
        }
    }
    None
}

/// Concatena todos los nodos Text descendientes (sin formateo).
pub(crate) fn collect_text(node: &Handle) -> String {
    let mut out = String::new();
    collect_text_inner(node, &mut out);
    out.trim().to_string()
}

fn collect_text_inner(node: &Handle, out: &mut String) {
    if let NodeData::Text { contents } = &node.data {
        out.push_str(&contents.borrow());
    }
    for child in node.children.borrow().iter() {
        collect_text_inner(child, out);
    }
}

/// Lee el atributo `name` de un nodo Element (case-insensitive sobre el
/// nombre local). Devuelve `None` si no es un Element o el atributo no
/// existe.
pub(crate) fn attr(node: &Handle, name: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &node.data else {
        return None;
    };
    let attrs: &RefCell<Vec<markup5ever::interface::Attribute>> = attrs;
    for a in attrs.borrow().iter() {
        if a.name.local.as_ref().eq_ignore_ascii_case(name) {
            return Some(a.value.to_string());
        }
    }
    None
}

/// Nombre local de un Element, en lowercase ASCII. `None` si el nodo no
/// es un Element.
pub(crate) fn element_name(node: &Handle) -> Option<String> {
    let NodeData::Element { name, .. } = &node.data else {
        return None;
    };
    let _: &QualName = name;
    Some(name.local.as_ref().to_ascii_lowercase())
}

#[allow(dead_code)] // utilitario p/ Rc clone, exportado por si style.rs lo usa
pub(crate) fn children(node: &Handle) -> Vec<Handle> {
    node.children.borrow().iter().map(Rc::clone).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_title() {
        let html = "<!doctype html><html><head><title>Hola</title></head><body><p>texto</p></body></html>";
        let dom = DomTree::parse(html);
        assert_eq!(dom.title().as_deref(), Some("Hola"));
    }

    #[test]
    fn encuentra_body() {
        let html = "<html><body><p>x</p></body></html>";
        let dom = DomTree::parse(html);
        assert!(dom.find("body").is_some());
        assert!(dom.find("p").is_some());
        assert!(dom.find("inexistente").is_none());
    }

    #[test]
    fn extrae_style_inline() {
        let html = "<html><head><style>p { color: red }</style></head><body></body></html>";
        let dom = DomTree::parse(html);
        let sheets = dom.collect_inline_stylesheets();
        assert_eq!(sheets.len(), 1);
        assert!(sheets[0].contains("color: red"));
    }
}
