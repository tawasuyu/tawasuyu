//! Pluma reader — visor de markdown elegante para WASM/web.
//!
//! Toma un `<div>` que actúa como contenedor y le inyecta el HTML
//! producido por `pluma-md`. El styling (fonts, colores, animaciones)
//! lo provee el CSS del host: este crate no inyecta estilos, sólo
//! marcado y `data-pluma-theme="…"` para que el CSS reaccione.
//!
//! Patrón de uso:
//!
//! ```ignore
//! let container = document.get_element_by_id("drawer-aire-content")?
//!     .dyn_into::<HtmlElement>()?;
//! let reader = Reader::new(container);
//! reader.show_loading();
//! wasm_bindgen_futures::spawn_local(async move {
//!     let _ = reader.open_url("./md/aire.md", "aire").await;
//! });
//! ```

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{HtmlElement, Response};

pub struct Reader {
    container: HtmlElement,
}

impl Reader {
    pub fn new(container: HtmlElement) -> Self {
        Self { container }
    }

    pub fn container(&self) -> &HtmlElement {
        &self.container
    }

    /// Inyecta un mensaje de carga mientras se resuelve `open_url`.
    pub fn show_loading(&self) {
        self.container.set_inner_html(
            r#"<div class="pluma-loading" aria-live="polite">…</div>"#,
        );
    }

    /// Inyecta un mensaje de error visible.
    pub fn show_error(&self, msg: &str) {
        let safe: String = msg.replace('<', "&lt;").replace('>', "&gt;");
        self.container.set_inner_html(&format!(
            r#"<div class="pluma-error">{}</div>"#,
            safe
        ));
    }

    /// Renderea un string markdown directamente, sin fetch.
    pub fn render_md(&self, md: &str, theme: &str) {
        let html = fana_md::to_themed_html(md, theme);
        self.container.set_inner_html(&html);
    }

    /// Inyecta HTML pre-renderizado (sin parsear). Útil si el caller ya
    /// hizo el parse en otro lado.
    pub fn render_html(&self, html: &str) {
        self.container.set_inner_html(html);
    }

    /// Limpia el contenedor.
    pub fn clear(&self) {
        self.container.set_inner_html("");
    }

    /// Fetcha la URL, parsea el markdown y lo renderea con el tema dado.
    /// El loader muestra un placeholder mientras la promesa está pendiente.
    pub async fn open_url(&self, url: &str, theme: &str) -> Result<(), JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        self.show_loading();
        let resp_value = JsFuture::from(window.fetch_with_str(url)).await?;
        let resp: Response = resp_value.dyn_into()?;
        if !resp.ok() {
            let err = format!("HTTP {} para {}", resp.status(), url);
            self.show_error(&err);
            return Err(JsValue::from_str(&err));
        }
        let text_value = JsFuture::from(resp.text()?).await?;
        let md = text_value.as_string().unwrap_or_default();
        self.render_md(&md, theme);
        Ok(())
    }
}
