//! Pluma — parser markdown agnóstico, listo para envolver en cualquier viewer.
//!
//! Es deliberadamente delgado: wrappea `pulldown-cmark` (todas las
//! extensiones GFM habilitadas) y emite HTML envuelto en `<div class="pluma-doc">`
//! con un `data-pluma-theme="…"` para que el CSS del viewer aplique colores
//! por tema sin necesidad de re-renderear.
//!
//! No tiene deps de web/DOM/wasm: corre igual en server, terminal, WASM o
//! tests. Si necesitás emitir Markdown-AST en lugar de HTML, usá la API
//! `events()` y construí tu propio renderer.

pub mod import;
pub use import::{parse_md, DocumentoImportado};

use pulldown_cmark::{html, Event, Options, Parser};

/// Opciones por default — GFM completo: tables, footnotes, tasklists, strikethrough,
/// smart punctuation, heading anchors.
pub fn default_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_HEADING_ATTRIBUTES
}

/// Markdown → HTML "crudo" (sin wrapper de tema).
pub fn to_html(md: &str) -> String {
    let mut out = String::with_capacity(md.len() * 2);
    let parser = Parser::new_ext(md, default_options());
    html::push_html(&mut out, parser);
    out
}

/// Markdown → HTML envuelto en `<div class="pluma-doc" data-pluma-theme="…">`.
/// El `theme` es un string opaco (ej. "aire", "fuego") que el CSS del viewer
/// matchea via `[data-pluma-theme="aire"]`.
pub fn to_themed_html(md: &str, theme: &str) -> String {
    let body = to_html(md);
    let safe_theme: String = theme
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    format!(
        r#"<article class="pluma-doc" data-pluma-theme="{theme}">{body}</article>"#,
        theme = safe_theme,
        body = body
    )
}

/// Devuelve un iterador de eventos pulldown-cmark (AST stream).
/// Útil si querés renderear a algo distinto que HTML.
pub fn events(md: &str) -> impl Iterator<Item = Event<'_>> {
    Parser::new_ext(md, default_options())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_h1() {
        let html = to_html("# Hola");
        assert!(html.contains("<h1>Hola</h1>"), "got {}", html);
    }

    #[test]
    fn renders_list() {
        let html = to_html("- a\n- b\n");
        assert!(html.contains("<li>a</li>"));
        assert!(html.contains("<li>b</li>"));
    }

    #[test]
    fn themed_wrapper_sanitizes_theme_name() {
        let html = to_themed_html("# x", "aire<script>");
        assert!(html.contains(r#"data-pluma-theme="airescript""#));
    }

    #[test]
    fn renders_code_fence() {
        let html = to_html("```rust\nfn main(){}\n```");
        assert!(html.contains("<pre><code") && html.contains("fn main"));
    }

    #[test]
    fn renders_table_gfm() {
        let md = "| a | b |\n|---|---|\n| 1 | 2 |\n";
        let html = to_html(md);
        assert!(html.contains("<table>"), "got {}", html);
    }
}
