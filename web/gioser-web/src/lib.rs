//! Landing gioser — escritorio web tipo Llimphi.
//!
//! La portada presenta un plano cartesiano con los 4 cuadrantes
//! (00_unanchay, 01_yachay, 02_ruway, 03_ukupacha) y un menú
//! lateral con la documentación del sistema. Cada click abre una
//! ventana flotante que muestra el markdown renderizado con
//! `pluma-md-reader-web`. Las ventanas se manejan en JS (drag,
//! minimizar, cerrar) y se reflejan en la taskbar inferior.
//!
//! Rust/WASM expone dos entradas al host JS:
//!
//! * `cargar_md(container, url, theme)` — fetch + parse markdown
//!   y lo inyecta en el contenedor pasado por id.
//! * `mount_panel(container)` — monta el panel de control del
//!   escritorio (apariencia, idioma, demos, monitor, módulos,
//!   acerca). Reutiliza el sistema de ventanas del host.

mod panel;

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

/// Monta el panel de control del escritorio en el contenedor
/// `container_id`. Lee preferencias persistidas de `localStorage`,
/// las aplica al documento (variante del theme, acento, densidad,
/// idioma, módulos visibles), engancha controles y arranca el
/// refresco del monitor.
#[wasm_bindgen]
pub fn mount_panel(container_id: String) -> Result<(), JsValue> {
    let doc = web_sys::window()
        .ok_or_else(|| JsValue::from_str("no window"))?
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let cuerpo: HtmlElement = doc
        .get_element_by_id(&container_id)
        .ok_or_else(|| JsValue::from_str("contenedor no encontrado"))?
        .dyn_into()?;
    panel::Panel::new(cuerpo).mount()
}

/// Aplica al documento las preferencias persistidas (variante,
/// acento, densidad, módulos ocultos) sin montar la UI del panel.
/// Se llama al cargar la página para que el escritorio respete
/// inmediatamente lo que el usuario configuró antes.
#[wasm_bindgen]
pub fn aplicar_preferencias() -> Result<(), JsValue> {
    panel::aplicar_preferencias_iniciales()
}
