//! Construcción del box tree desde el DOM + StyleEngine: `build`/`build_node`
//! (walk recursivo que computa estilos y arma cada `BoxNode`), recolección de
//! forms y de `<svg>` (parseo de prims/paths), texto inline, colapso de
//! márgenes, y prefetch/decodificación de imágenes (`fetch_image_src`).
//! Repartido en submódulos hermanos. Extraído de `boxes/mod.rs` (regla #1).
use super::*;

mod svg;
pub(crate) use svg::*;
mod node;
pub(crate) use node::*;
mod inline;
pub(crate) use inline::*;
mod images;
pub use images::*;
mod text;
pub(crate) use text::*;
// `computed_ext` sólo aporta un impl de enlace (sin nombres que reexportar).
mod computed_ext;

pub fn build(dom: &DomTree, styles: &StyleEngine, base_url: &str) -> BoxTree {
    // `<base href="...">` en el `<head>` override la base URL. Si está
    // ausente o inválido, fallback al URL del documento.
    let doc_base = url::Url::parse(base_url).ok();
    let base = dom
        .base_href()
        .as_deref()
        .and_then(|href| {
            // El base href puede ser absoluto o relativo al URL del doc.
            url::Url::parse(href)
                .ok()
                .or_else(|| doc_base.as_ref().and_then(|b| b.join(href).ok()))
        })
        .or(doc_base);
    let body = dom.find("body").unwrap_or_else(|| dom.document());
    // Prefetch paralelo de imágenes: pre-walk del DOM antes del build
    // recolecta todas las URLs de `<img>`/`<picture>` (resueltas contra
    // base) y las baja en paralelo con un pool de workers. Las bytes
    // quedan en la cache global; el `fetch_and_decode` síncrono dentro
    // de `build_node` después hace cache hit. Esto convierte el parse
    // de una página con 20 imágenes de "20 round-trips serializados"
    // a "ceil(20/N) round-trips". `background-image: url(...)` no
    // entra al pre-walk todavía — vive en CSS y requiere computar
    // styles primero.
    prefetch_image_urls(&dom.document(), base.as_ref());
    // Segundo pass de prefetch: `background-image: url(...)` vive en
    // CSS — necesita styles computados, así que va después del primer
    // pre-walk. Computamos sin parent style (background-image no es
    // heredable, así que el value es independiente del padre). Las
    // URLs descargadas también caen en la cache global.
    prefetch_background_image_urls(&dom.document(), styles, base.as_ref());
    let mut counters: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    let mut root = build_node(&body, styles, base.as_ref(), None, &mut counters)
        .unwrap_or_else(empty_root);
    let mut forms: Vec<FormInfo> = Vec::new();
    // Pre-walk del DOM para coleccionar `<form>` (orden DFS) con sus
    // attributes resueltos contra base. La asignación de form_idx por
    // input se hace en un post-pass sobre el box tree con el mismo
    // criterio DFS — ambos walks coinciden porque el box tree refleja
    // el DOM (sólo dropea text-whitespace inter-block; los <form> son
    // block-level y nunca se descartan).
    collect_forms_dom(&body, base.as_ref(), &mut forms);
    let mut form_stack: Vec<usize> = Vec::new();
    let mut form_cursor: usize = 0;
    assign_form_idx(&mut root, &mut form_stack, &mut form_cursor);
    // Identidad estable por nodo (1..N en DFS pre-orden). El chrome la usa
    // para llevar estado por-nodo (tween de `transition` en hover) keyeado
    // por id, sin contar índices en walks paralelos frágiles.
    let mut node_cursor: u32 = 1;
    assign_node_ids(&mut root, &mut node_cursor);
    BoxTree { root, forms, styles: styles.clone() }
}

/// Post-pass: numera cada nodo del árbol en orden DFS pre-orden empezando
/// en `*next`. Determinista y estable mientras la estructura del árbol no
/// cambie — exactamente la garantía que necesita el estado de hover del
/// chrome (keyeado por `node_id`).
pub(crate) fn assign_node_ids(node: &mut BoxNode, next: &mut u32) {
    node.node_id = *next;
    *next += 1;
    for child in &mut node.children {
        assign_node_ids(child, next);
    }
}

pub(crate) fn collect_forms_dom(node: &Handle, base: Option<&url::Url>, out: &mut Vec<FormInfo>) {
    if let markup5ever_rcdom::NodeData::Element { .. } = &node.data {
        if dom::element_name(node).as_deref() == Some("form") {
            let action = dom::attr(node, "action").and_then(|a| resolve_href(base, &a));
            let method = dom::attr(node, "method")
                .map(|m| {
                    if m.eq_ignore_ascii_case("post") {
                        FormMethod::Post
                    } else {
                        FormMethod::Get
                    }
                })
                .unwrap_or(FormMethod::Get);
            out.push(FormInfo { action, method });
        }
    }
    for c in node.children.borrow().iter() {
        collect_forms_dom(c, base, out);
    }
}

pub(crate) fn assign_form_idx(b: &mut BoxNode, stack: &mut Vec<usize>, cursor: &mut usize) {
    let is_form = b.tag.as_deref() == Some("form");
    if is_form {
        stack.push(*cursor);
        *cursor += 1;
    }
    if b.input_kind.is_some() || b.select.is_some() {
        b.form_idx = stack.last().copied();
    }
    for c in &mut b.children {
        assign_form_idx(c, stack, cursor);
    }
    if is_form {
        stack.pop();
    }
}

