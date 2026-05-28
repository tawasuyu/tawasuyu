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

pub mod boxes;
pub mod cache;
pub mod cookies;
pub mod dom;
pub mod fetch;
pub mod style;

use thiserror::Error;

pub use boxes::{
    BoxNode, BoxTree, Color, Display, FormInfo, FormMethod, InputKind, PathCmd, SelectInfo,
    SelectOption, SvgPrim, SvgScene,
};
pub use dom::{DomTree, MetaRefresh};
pub use fetch::{fetch, FetchError};
pub use style::{
    AlignItems, AlignSelf, BoxShadow, BoxSizing, ComputedStyle, ContentItem, FlexDirection,
    FlexWrap, FontStyle, GradientStop, GridTrackSize, JustifyContent, LengthVal, LinearGradient,
    Outline, Overflow, PointerEvents, Position, PseudoElement, Sides, StyleEngine, TextAlign,
    TextDecorationLine, TextShadow, TextTransform, Transform, VerticalAlign, Viewport, Visibility,
    WhiteSpace, DEFAULT_VIEWPORT,
};

/// Pipeline completo del navegador. Sin estado mutable — cada `load`
/// devuelve un [`Document`] independiente.
pub struct Engine;

impl Engine {
    pub fn new() -> Self {
        Self
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
        Ok(self.load_html(&final_url, &html))
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
        Ok(self.load_html(&final_url, &html))
    }

    /// Variante para tests / data URLs: parsea HTML ya en memoria.
    pub fn load_html(&self, url: &str, html: &str) -> Document {
        let dom = DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let box_tree = boxes::build(&dom, &styles, url);
        let title = dom.title().unwrap_or_default();
        let meta_refresh = dom.meta_refresh();
        Document {
            url: url.to_string(),
            title,
            source: html.to_string(),
            dom,
            box_tree,
            meta_refresh,
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
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
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("URL inválida: {0}")]
    Url(String),
    #[error(transparent)]
    Fetch(#[from] FetchError),
}
