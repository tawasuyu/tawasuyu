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
pub use dom::DomTree;
pub use fetch::{fetch, FetchError};
pub use style::{
    AlignItems, AlignSelf, BoxShadow, BoxSizing, ComputedStyle, FlexDirection, FlexWrap, FontStyle,
    GradientStop, GridTrackSize, JustifyContent, LengthVal, LinearGradient, Outline, Overflow,
    PointerEvents, Position, Sides, StyleEngine, TextAlign, TextDecorationLine, TextShadow,
    TextTransform, Transform, VerticalAlign, Viewport, Visibility, WhiteSpace, DEFAULT_VIEWPORT,
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
    pub fn load(&self, url: &str) -> Result<Document, EngineError> {
        let parsed = url::Url::parse(url).map_err(|e| EngineError::Url(e.to_string()))?;
        let html = fetch(&parsed)?;
        Ok(self.load_html(parsed.as_str(), &html))
    }

    /// POST con body `application/x-www-form-urlencoded`. Mismo pipeline
    /// que `load` después del fetch.
    pub fn load_post(&self, url: &str, body: &str) -> Result<Document, EngineError> {
        let parsed = url::Url::parse(url).map_err(|e| EngineError::Url(e.to_string()))?;
        let html = fetch::post_form(parsed.as_str(), body)?;
        Ok(self.load_html(parsed.as_str(), &html))
    }

    /// Variante para tests / data URLs: parsea HTML ya en memoria.
    pub fn load_html(&self, url: &str, html: &str) -> Document {
        let dom = DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let box_tree = boxes::build(&dom, &styles, url);
        let title = dom.title().unwrap_or_default();
        Document {
            url: url.to_string(),
            title,
            source: html.to_string(),
            dom,
            box_tree,
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
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("URL inválida: {0}")]
    Url(String),
    #[error(transparent)]
    Fetch(#[from] FetchError),
}
