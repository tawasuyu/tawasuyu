//! Landing gioser — sobria.
//!
//! Plano cartesiano SVG estático (en index.html) + visor markdown WASM.
//! Click en un dominio → abre md/<dom>.md con pluma-md-reader-web.

use pluma_md_reader_web::Reader;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlElement;

#[wasm_bindgen]
pub fn abrir_md(dominio: String) -> Result<(), JsValue> {
    let doc = web_sys::window()
        .ok_or_else(|| JsValue::from_str("no window"))?
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    if let Some(titulo) = doc.get_element_by_id("lector-titulo") {
        titulo.set_text_content(Some(&format!("md/{dominio}.md")));
    }
    if let Some(lector) = doc.get_element_by_id("lector") {
        lector.remove_attribute("hidden").ok();
    }

    let cuerpo: HtmlElement = doc
        .get_element_by_id("lector-cuerpo")
        .ok_or_else(|| JsValue::from_str("no #lector-cuerpo"))?
        .dyn_into()?;

    let reader = Reader::new(cuerpo);
    let url = format!("md/{dominio}.md");
    spawn_local(async move {
        let _ = reader.open_url(&url, "gioser").await;
    });
    Ok(())
}
