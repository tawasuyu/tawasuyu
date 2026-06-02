//! `puriy-engine` — bridge a Servo.
//!
//! Fase 2: embebe los crates publicados de Servo (`html5ever` +
//! `markup5ever_rcdom` para DOM, `cssparser` para inline styles) más
//! `ureq` para net síncrono. La salida es un [`Document`] con árbol de
//! [`BoxNode`] listo para que `llimphi-raster` lo pinte.
//!
//! Anti-objetivo: no embebemos el JS engine ni Stylo entero ni
//! webrender; eso queda para fases posteriores cuando Llimphi lo
//! demande. Acá sólo cargamos HTML, lo parseamos, computamos un box
//! tree elemental y devolvemos.
//!
//! ```no_run
//! use puriy_engine::Engine;
//!
//! let engine = Engine::new();
//! let doc = engine.load("https://example.com").unwrap();
//! println!("título: {}", doc.title);
//! ```

#![forbid(unsafe_code)]

pub mod anim;
pub mod boxes;
pub mod cache;
pub mod cookies;
pub mod dom;
pub mod fetch;
pub mod scripts;
pub mod sse;
pub mod style;

use thiserror::Error;

pub use boxes::{
    synthesize_box_node, BoxNode, BoxTree, Color, Display, FormInfo, FormMethod, InputKind,
    PathCmd, SelectInfo, SelectOption, SvgPrim, SvgScene,
};
pub use dom::{DomTree, MetaRefresh, ScriptInfo};
pub use fetch::{fetch, fetch_full, FetchError, FetchResponse};
pub use style::{
    AlignItems, AlignSelf, AnimationBinding, AnimationDirection, AnimationFillMode,
    AnimationIterations, BoxShadow, BoxSizing, ComputedStyle, ContentItem, EasingFunction,
    FlexDirection, FlexWrap, FontStyle, GradientStop, GridTrackSize, JustifyContent, KeyframeStep,
    Keyframes, LengthVal, LinearGradient, Outline, Overflow, PointerEvents, Position,
    PseudoElement, Sides, StyleEngine, TextAlign, TextDecorationLine, TextShadow, TextTransform,
    Transform, TransitionBinding, VerticalAlign, Viewport, Visibility, WhiteSpace, DEFAULT_VIEWPORT,
    evaluate_media_query,
};

/// Pipeline completo del navegador. Sin estado mutable — cada `load`
/// devuelve un [`Document`] independiente.
pub struct Engine {
    /// Viewport contra el que se evalúan los `@media` del documento. El chrome
    /// lo setea con el tamaño/DPR real de la ventana (`with_viewport`); por
    /// defecto es `DEFAULT_VIEWPORT` (1280×800 @1.0) para tests y carga headless.
    viewport: Viewport,
}

impl Engine {
    pub fn new() -> Self {
        Self { viewport: DEFAULT_VIEWPORT }
    }

    /// Fija el viewport real (ancho/alto en px + DPR) para que los `@media`
    /// del documento se resuelvan contra la ventana de verdad. El chrome lo
    /// llama antes de cargar con el tamaño actual. Builder: `Engine::new().with_viewport(vp)`.
    pub fn with_viewport(mut self, viewport: Viewport) -> Self {
        self.viewport = viewport;
        self
    }

    /// Carga una URL y produce un documento listo para render.
    ///
    /// Pipeline: `fetch` → `parse_html` → `parse_styles` → `build_box_tree`.
    /// La URL en el `Document` puede diferir de la solicitada si el server
    /// redirigió (3xx) — usamos la final para resolver hrefs relativos y
    /// la barra del chrome la muestra como URL canónica de la pestaña.
    pub fn load(&self, url: &str) -> Result<Document, EngineError> {
        self.load_with_referer(url, None)
    }

    /// Como `load` pero envía `Referer:` con la URL fuente. El chrome lo
    /// pasa al navegar desde un link clickeado (anti-fugas: aceptamos
    /// sólo http/https como referer, y strippeamos fragment).
    pub fn load_with_referer(
        &self,
        url: &str,
        referer: Option<&str>,
    ) -> Result<Document, EngineError> {
        let parsed = url::Url::parse(url).map_err(|e| EngineError::Url(e.to_string()))?;
        let (html, final_url) = fetch::fetch_with_referer(&parsed, referer)?;
        let mut doc = self.load_html(&final_url, &html);
        scripts::fetch_externals(&mut doc.scripts, &doc.url);
        Ok(doc)
    }

    /// POST con body `application/x-www-form-urlencoded`. Mismo pipeline
    /// que `load` después del fetch.
    pub fn load_post(&self, url: &str, body: &str) -> Result<Document, EngineError> {
        self.load_post_with_referer(url, body, None)
    }

    /// POST con `Referer:` opcional.
    pub fn load_post_with_referer(
        &self,
        url: &str,
        body: &str,
        referer: Option<&str>,
    ) -> Result<Document, EngineError> {
        let parsed = url::Url::parse(url).map_err(|e| EngineError::Url(e.to_string()))?;
        let (html, final_url) =
            fetch::post_form_with_referer(parsed.as_str(), body, referer)?;
        let mut doc = self.load_html(&final_url, &html);
        scripts::fetch_externals(&mut doc.scripts, &doc.url);
        Ok(doc)
    }

    /// Variante para tests / data URLs: parsea HTML ya en memoria.
    pub fn load_html(&self, url: &str, html: &str) -> Document {
        // Instala el viewport real para toda la carga: resuelve `vw/vh/vmin/vmax`
        // (en hojas y en `style="…"` inline parseado por `boxes::build`) contra
        // el tamaño de ventana actual, no contra el viewport por defecto.
        let _vp_scope = crate::style::ViewportScope::new(self.viewport);
        let dom = DomTree::parse(html);
        // Resuelve las hojas de estilo en orden de documento: `<style>` inline
        // se usa tal cual; `<link rel="stylesheet">` se baja (http/file/data:)
        // contra la base. Una hoja externa que falle se saltea (queda sin sus
        // reglas, como un browser tras un 404 de CSS). Hojas relativas con base
        // no-http (`about:test` en tests) no resuelven → sin red.
        // Base para resolver hrefs relativos de hojas/`@import`: honra
        // `<base href>` igual que `boxes::build` (consistencia con `<img>`/`<a>`).
        let doc_base = url::Url::parse(url).ok();
        let base = dom
            .base_href()
            .as_deref()
            .and_then(|href| {
                url::Url::parse(href)
                    .ok()
                    .or_else(|| doc_base.as_ref().and_then(|b| b.join(href).ok()))
            })
            .or(doc_base);
        // El atributo `media` del `<link>`/`<style>` gatea la hoja: una que no
        // matchea el viewport (`media="print"` en pantalla, `media="(max-width:
        // 600px)"` en ventana ancha) se descarta entera, sin bajarla.
        let media_ok = |media: &Option<String>| -> bool {
            media
                .as_deref()
                .map(|q| evaluate_media_query(q, self.viewport))
                .unwrap_or(true)
        };
        let mut sheets: Vec<String> = Vec::new();
        for src in dom.collect_style_sources() {
            match src {
                dom::StyleSource::Inline { css, media } => {
                    if media_ok(&media) {
                        // base del `<style>` inline = el documento.
                        expand_sheet(&css, base.as_ref(), self.viewport, 0, &mut sheets);
                    }
                }
                dom::StyleSource::External { href, media } => {
                    if !media_ok(&media) {
                        continue;
                    }
                    if let Some(abs) = resolve_resource_url(base.as_ref(), &href) {
                        if let Ok(bytes) = fetch::fetch_bytes(&abs) {
                            let css = String::from_utf8_lossy(&bytes).into_owned();
                            // base de la hoja externa = su propia URL (los
                            // `@import`/`url()` relativos cuelgan de ella).
                            let sheet_base = url::Url::parse(&abs).ok();
                            expand_sheet(&css, sheet_base.as_ref(), self.viewport, 0, &mut sheets);
                        }
                    }
                }
            }
        }
        let styles = StyleEngine::from_sheets_with_viewport(&sheets, self.viewport);
        let box_tree = boxes::build(&dom, &styles, url);
        let title = dom.title().unwrap_or_default();
        let meta_refresh = dom.meta_refresh();
        let scripts = dom.collect_scripts();
        Document {
            url: url.to_string(),
            title,
            source: html.to_string(),
            dom,
            box_tree,
            meta_refresh,
            scripts,
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// Resuelve el `href` de un recurso (hoja de estilo externa) a una URL
/// bajable. A diferencia de la navegación (`<a href>`, que bloquea `data:`),
/// un recurso SÍ puede venir como `data:`/`file:`. Las relativas se unen a la
/// base; las absolutas se aceptan sólo para http/https/file/data:. `None` si
/// no resuelve (base no-hierárquica, scheme no soportado, href vacío).
fn resolve_resource_url(base: Option<&url::Url>, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    if fetch::is_data_url(href) {
        return Some(href.to_string());
    }
    if let Ok(abs) = url::Url::parse(href) {
        return match abs.scheme() {
            "http" | "https" | "file" => Some(abs.into()),
            _ => None,
        };
    }
    base.and_then(|b| b.join(href).ok()).map(|u| u.to_string())
}

/// Profundidad máxima de anidamiento de `@import` — corta ciclos
/// (`a.css` importa `b.css` que reimporta `a.css`) y cadenas patológicas.
const MAX_IMPORT_DEPTH: u8 = 8;

/// Expande una hoja de estilo siguiendo sus reglas `@import` y empuja el
/// resultado aplanado a `out` en orden de cascada: cada hoja importada va
/// ANTES del cuerpo que la importó (lo exige la spec — `@import` precede a las
/// reglas propias). Los `@import` se resuelven contra `base` (la URL de ESTA
/// hoja, no la del documento) y se bajan vía `fetch_bytes` (http/file/data:).
/// Un `@import ... <media query>` que no matchea el viewport se saltea sin
/// bajarlo. `@import` sólo se reconoce en la cabecera (antes de la primera `{`),
/// que es donde la spec lo permite — así no confundimos un `@import` literal
/// dentro de un valor/selector.
fn expand_sheet(css: &str, base: Option<&url::Url>, vp: Viewport, depth: u8, out: &mut Vec<String>) {
    if depth >= MAX_IMPORT_DEPTH {
        out.push(css.to_string());
        return;
    }
    let head_end = css.find('{').unwrap_or(css.len());
    let head_lower = css[..head_end].to_ascii_lowercase();
    // (rango en `css`, url, media opcional) de cada @import de la cabecera.
    let mut imports: Vec<(usize, usize, String, Option<String>)> = Vec::new();
    let mut search = 0;
    while let Some(rel) = head_lower[search..].find("@import") {
        let at = search + rel;
        let Some(semi_rel) = css[at..].find(';') else { break };
        let semi = at + semi_rel;
        if let Some((url, media)) = parse_import_rule(&css[at + "@import".len()..semi]) {
            imports.push((at, semi + 1, url, media));
        }
        search = semi + 1;
    }
    if imports.is_empty() {
        out.push(css.to_string());
        return;
    }
    // Cuerpo sin las líneas `@import` (parse_stylesheet no las necesita).
    let mut body = String::new();
    let mut cursor = 0;
    for (s, e, _, _) in &imports {
        body.push_str(&css[cursor..*s]);
        cursor = *e;
    }
    body.push_str(&css[cursor..]);
    // Hojas importadas PRIMERO (menor prioridad en la cascada).
    for (_, _, url, media) in imports {
        if let Some(m) = &media {
            if !evaluate_media_query(m, vp) {
                continue;
            }
        }
        if let Some(abs) = resolve_resource_url(base, &url) {
            if let Ok(bytes) = fetch::fetch_bytes(&abs) {
                let imported = String::from_utf8_lossy(&bytes).into_owned();
                let imported_base = url::Url::parse(&abs).ok();
                expand_sheet(&imported, imported_base.as_ref(), vp, depth + 1, out);
            }
        }
    }
    out.push(body);
}

/// Parsea el cuerpo de un `@import` (el texto entre `@import` y `;`). Acepta
/// `url("x")`, `url(x)` y la forma con string suelto `"x"`/`'x'`, seguidos de
/// una media query opcional. Devuelve `(url, media)` o `None` si no parsea.
fn parse_import_rule(rule: &str) -> Option<(String, Option<String>)> {
    let s = rule.trim();
    let (url, rest) = if s.len() >= 4 && s[..4].eq_ignore_ascii_case("url(") {
        let after = &s[4..];
        let end = after.find(')')?;
        let inner = after[..end].trim().trim_matches(|c| c == '"' || c == '\'');
        (inner.to_string(), after[end + 1..].trim())
    } else if let Some(first) = s.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let after = &s[1..];
        let end = after.find(first)?;
        (after[..end].to_string(), after[end + 1..].trim())
    } else {
        return None;
    };
    if url.is_empty() {
        return None;
    }
    let media = (!rest.is_empty()).then(|| rest.to_string());
    Some((url, media))
}

/// Documento web parseado y layouted (en forma de box tree).
pub struct Document {
    pub url: String,
    pub title: String,
    /// HTML crudo del documento (idéntico al que se le pasó al parser).
    /// Útil para "ver código fuente" (Ctrl+U) y debug.
    pub source: String,
    pub dom: DomTree,
    pub box_tree: BoxTree,
    /// Si el `<head>` lleva un `<meta http-equiv="refresh">`, contiene
    /// el delay y URL destino. El chrome lo programa con un sleep en un
    /// worker thread y dispatcha `Msg::Navigate` cuando vence.
    pub meta_refresh: Option<MetaRefresh>,
    /// `<script>` recolectados en orden DOM. Fase 7.0: el chrome todavía
    /// no los ejecuta (`puriy-js::JsRuntime` es un stub). Fase 7.1
    /// enchufa el runtime real y arranca a procesarlos.
    pub scripts: Vec<ScriptInfo>,
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("URL inválida: {0}")]
    Url(String),
    #[error(transparent)]
    Fetch(#[from] FetchError),
}
