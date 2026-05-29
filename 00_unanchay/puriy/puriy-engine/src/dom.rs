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

    /// Busca el primer `<base href="...">` del `<head>`. Su valor
    /// override la base URL del documento para todos los hrefs
    /// relativos. Si no aparece o está vacío, devuelve `None` y el
    /// caller usa la URL del documento como base.
    pub fn base_href(&self) -> Option<String> {
        let mut found: Option<String> = None;
        walk(&self.document(), &mut |node| {
            if found.is_some() {
                return;
            }
            if let NodeData::Element { name, .. } = &node.data {
                if name.local.as_ref() == "base" {
                    if let Some(href) = attr(node, "href") {
                        let trimmed = href.trim();
                        if !trimmed.is_empty() {
                            found = Some(trimmed.to_string());
                        }
                    }
                }
            }
        });
        found
    }

    /// Busca el primer `<meta http-equiv="refresh" content="N;url=...">`
    /// y extrae `(delay_secs, target_url_opcional)`. Si la URL es None,
    /// el refresh recarga la página actual. Si no hay meta refresh,
    /// devuelve `None`.
    pub fn meta_refresh(&self) -> Option<MetaRefresh> {
        let mut found: Option<MetaRefresh> = None;
        walk(&self.document(), &mut |node| {
            if found.is_some() {
                return;
            }
            if let NodeData::Element { name, .. } = &node.data {
                if name.local.as_ref() != "meta" {
                    return;
                }
                let http_equiv = attr(node, "http-equiv").unwrap_or_default();
                if !http_equiv.eq_ignore_ascii_case("refresh") {
                    return;
                }
                let content = attr(node, "content").unwrap_or_default();
                if let Some(mr) = parse_meta_refresh_content(&content) {
                    found = Some(mr);
                }
            }
        });
        found
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

    /// Itera in-order todos los `<script>` del documento y devuelve sus
    /// metadatos. Cada script lleva, o bien un `src` externo (a bajar),
    /// o bien el body inline; nunca ambos a la vez (HTML spec: si hay
    /// `src`, el contenido inline se ignora).
    ///
    /// Scripts con `type` no-JS (`application/json`, `module-shim`,
    /// `text/template`, etc.) se devuelven igual — el caller decide qué
    /// hacer. `type="module"` y `type="text/javascript"` son JS estándar;
    /// el resto el runtime los puede saltear.
    ///
    /// **Fase 7.0**: el caller (`puriy-js::JsRuntime`) todavía es un
    /// stub y no ejecuta nada. Esto sólo expone el cableado para que
    /// Fase 7.1 enchufe el runtime real sin tocar el DOM.
    pub fn collect_scripts(&self) -> Vec<ScriptInfo> {
        let mut out = Vec::new();
        walk(&self.document(), &mut |node| {
            if let NodeData::Element { name, .. } = &node.data {
                if name.local.as_ref() != "script" {
                    return;
                }
                let src = attr(node, "src").and_then(|v| {
                    let t = v.trim().to_string();
                    if t.is_empty() {
                        None
                    } else {
                        Some(t)
                    }
                });
                let inline = if src.is_some() {
                    None
                } else {
                    let body = collect_text(node);
                    if body.is_empty() {
                        None
                    } else {
                        Some(body)
                    }
                };
                let type_attr = attr(node, "type").and_then(|v| {
                    let t = v.trim().to_string();
                    if t.is_empty() {
                        None
                    } else {
                        Some(t)
                    }
                });
                let is_module = type_attr
                    .as_deref()
                    .map(|t| t.eq_ignore_ascii_case("module"))
                    .unwrap_or(false);
                let defer = attr(node, "defer").is_some();
                let async_ = attr(node, "async").is_some();
                // Si no hay ni src ni inline, no aporta nada — lo
                // dropeamos para que el caller no itere sobre vacíos.
                if src.is_none() && inline.is_none() {
                    return;
                }
                out.push(ScriptInfo {
                    src,
                    inline,
                    type_attr,
                    is_module,
                    defer,
                    async_,
                });
            }
        });
        out
    }
}

/// Metadatos de un `<script>` extraído del DOM. La ejecución es
/// responsabilidad del caller (típicamente el chrome, vía `puriy-js`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInfo {
    /// URL externa (resolver contra base antes de fetchear). `None` si
    /// el script es inline.
    pub src: Option<String>,
    /// Body inline. `None` si tiene `src` (HTML spec ignora el inline en
    /// ese caso).
    pub inline: Option<String>,
    /// Valor del atributo `type` literal — útil para distinguir module
    /// JS de templates / json embebidos.
    pub type_attr: Option<String>,
    /// `true` cuando `type="module"`. El runtime de Fase 7.x es clásico
    /// (no module loader) — los módulos se saltean por ahora.
    pub is_module: bool,
    /// `defer`: ejecución pospuesta a `DOMContentLoaded`.
    pub defer: bool,
    /// `async`: ejecución asíncrona apenas el script termine de bajar.
    pub async_: bool,
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

/// Resultado de `<meta http-equiv="refresh" content="N;url=...">`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaRefresh {
    /// Segundos a esperar antes de la navegación. `0` = inmediato.
    pub delay_secs: u32,
    /// URL destino (relativa a la página actual — el chrome la resuelve
    /// contra `Document.url`). `None` = recargar la página actual.
    pub url: Option<String>,
}

/// Parsea el atributo `content` de `<meta http-equiv="refresh">`. El
/// formato real (HTML spec, sección "Pragma directive: refresh") es
/// `<N>` o `<N>; url=<URL>` (con variantes en whitespace y comillas).
/// Devuelve `None` si no se encuentra un delay entero.
fn parse_meta_refresh_content(content: &str) -> Option<MetaRefresh> {
    let content = content.trim();
    let (delay_str, rest) = match content.find(|c: char| c == ';' || c == ',') {
        Some(i) => (&content[..i], Some(content[i + 1..].trim())),
        None => (content, None),
    };
    let delay_str = delay_str.trim();
    let delay_secs: u32 = delay_str
        .split('.')
        .next()
        .and_then(|d| d.parse::<u32>().ok())?;
    let url = rest.and_then(|r| {
        // Busca `url=...` (case-insensitive, opcionalmente con comillas).
        let lower = r.to_ascii_lowercase();
        let key = lower.find("url=")?;
        let after = r[key + 4..].trim();
        let after = after.trim_start_matches(['"', '\''].as_ref());
        let after = after.trim_end_matches(['"', '\''].as_ref());
        let after = after.trim();
        if after.is_empty() {
            None
        } else {
            Some(after.to_string())
        }
    });
    Some(MetaRefresh { delay_secs, url })
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

    #[test]
    fn base_href_extrae_del_head() {
        let html = r#"<html><head><base href="https://example.com/sub/"></head><body></body></html>"#;
        let dom = DomTree::parse(html);
        assert_eq!(dom.base_href().as_deref(), Some("https://example.com/sub/"));
    }

    #[test]
    fn base_href_ausente_o_vacio_devuelve_none() {
        let html = r#"<html><head><base href=""></head></html>"#;
        assert!(DomTree::parse(html).base_href().is_none());
        let html2 = r#"<html><head></head></html>"#;
        assert!(DomTree::parse(html2).base_href().is_none());
    }

    #[test]
    fn meta_refresh_extrae_delay_y_url() {
        let html = r#"<html><head><meta http-equiv="refresh" content="5; url=/next">
            </head><body>x</body></html>"#;
        let dom = DomTree::parse(html);
        let mr = dom.meta_refresh().expect("meta refresh debería estar");
        assert_eq!(mr.delay_secs, 5);
        assert_eq!(mr.url.as_deref(), Some("/next"));
    }

    #[test]
    fn meta_refresh_solo_delay() {
        let html = r#"<html><head><meta http-equiv="refresh" content="10">
            </head></html>"#;
        let dom = DomTree::parse(html);
        let mr = dom.meta_refresh().expect("delay-only debería parsear");
        assert_eq!(mr.delay_secs, 10);
        assert_eq!(mr.url, None);
    }

    #[test]
    fn meta_refresh_url_con_comillas() {
        let html = r#"<html><head><meta http-equiv="REFRESH" content='0;URL="https://example.com/x"'>
            </head></html>"#;
        let dom = DomTree::parse(html);
        let mr = dom.meta_refresh().expect("case-insensitive + comillas");
        assert_eq!(mr.delay_secs, 0);
        assert_eq!(mr.url.as_deref(), Some("https://example.com/x"));
    }

    #[test]
    fn meta_refresh_inexistente_devuelve_none() {
        let html = "<html><head></head><body></body></html>";
        let dom = DomTree::parse(html);
        assert!(dom.meta_refresh().is_none());
    }

    #[test]
    fn collect_scripts_detecta_inline() {
        let html = r#"<html><head><script>console.log("hola")</script></head><body></body></html>"#;
        let scripts = DomTree::parse(html).collect_scripts();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].src, None);
        assert_eq!(scripts[0].inline.as_deref(), Some("console.log(\"hola\")"));
        assert!(!scripts[0].is_module);
        assert!(!scripts[0].defer);
        assert!(!scripts[0].async_);
    }

    #[test]
    fn collect_scripts_detecta_src_externo() {
        let html = r#"<html><body><script src="/main.js" defer async></script></body></html>"#;
        let scripts = DomTree::parse(html).collect_scripts();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].src.as_deref(), Some("/main.js"));
        assert_eq!(scripts[0].inline, None);
        assert!(scripts[0].defer);
        assert!(scripts[0].async_);
    }

    #[test]
    fn collect_scripts_ignora_body_si_hay_src() {
        // Según HTML spec, si <script src=...> tiene contenido, se ignora.
        let html =
            r#"<html><body><script src="/x.js">esto no se ejecuta</script></body></html>"#;
        let scripts = DomTree::parse(html).collect_scripts();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].src.as_deref(), Some("/x.js"));
        assert_eq!(scripts[0].inline, None);
    }

    #[test]
    fn collect_scripts_marca_modulo_por_type() {
        let html = r#"<html><body>
            <script type="module">import x from "./x.js"</script>
            <script type="text/javascript">var a=1</script>
            <script type="application/json">{"k":"v"}</script>
        </body></html>"#;
        let scripts = DomTree::parse(html).collect_scripts();
        assert_eq!(scripts.len(), 3);
        assert!(scripts[0].is_module);
        assert!(!scripts[1].is_module);
        assert_eq!(scripts[2].type_attr.as_deref(), Some("application/json"));
        assert!(!scripts[2].is_module);
    }

    #[test]
    fn collect_scripts_dropea_vacios() {
        // <script></script> sin src ni body: no aporta nada, lo
        // saltamos para que el caller no itere sobre nada.
        let html = r#"<html><body><script></script><script src=""></script></body></html>"#;
        let scripts = DomTree::parse(html).collect_scripts();
        assert!(scripts.is_empty());
    }

    #[test]
    fn collect_scripts_preserva_orden_dom() {
        let html = r#"<html><body>
            <script>1</script>
            <p>x</p>
            <script>2</script>
            <div><script>3</script></div>
        </body></html>"#;
        let scripts = DomTree::parse(html).collect_scripts();
        assert_eq!(scripts.len(), 3);
        assert_eq!(scripts[0].inline.as_deref(), Some("1"));
        assert_eq!(scripts[1].inline.as_deref(), Some("2"));
        assert_eq!(scripts[2].inline.as_deref(), Some("3"));
    }
}
