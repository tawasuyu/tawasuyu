//! Landing gioser — escritorio web tipo Llimphi.
//!
//! La portada presenta un plano cartesiano con los 4 cuadrantes
//! (00_unanchay, 01_yachay, 02_ruway, 03_ukupacha) y un menú
//! lateral con la documentación del sistema. Cada click abre una
//! ventana flotante que muestra el markdown renderizado con
//! `pluma-md-reader-web`. Las ventanas se manejan en JS (drag,
//! minimizar, cerrar) y se reflejan en la taskbar inferior.
//!
//! El único trabajo que hace Rust/WASM acá es: fetch + parse MD →
//! HTML temático, inyectado en el contenedor pasado por id.

use pluma_md_reader_web::Reader;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlElement;

/// Carga un markdown remoto y lo inyecta en el elemento con id
/// `container_id`. El `theme` se propaga al CSS via
/// `data-pluma-theme=…` para que el host pueda customizar colores.
#[wasm_bindgen]
pub fn cargar_md(container_id: String, url: String, theme: String) -> Result<(), JsValue> {
    let doc = web_sys::window()
        .ok_or_else(|| JsValue::from_str("no window"))?
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let cuerpo: HtmlElement = doc
        .get_element_by_id(&container_id)
        .ok_or_else(|| JsValue::from_str("contenedor no encontrado"))?
        .dyn_into()?;

    let reader = Reader::new(cuerpo);
    spawn_local(async move {
        let _ = reader.open_url(&url, &theme).await;
    });
    Ok(())
}
