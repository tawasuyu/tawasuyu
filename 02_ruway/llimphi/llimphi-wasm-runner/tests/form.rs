//! Certifica los eventos con payload (Text/Toggle) headless, con el guest del
//! formulario, y la lógica de edición value-driven del host.

use llimphi_wasm_runner::{edit_value, EventPayload, WasmGuest};
use llimphi_ui::{Key, NamedKey};

const FORM_WASM: &[u8] = include_bytes!("../assets/form.wasm");

// Orden de controles en Form::view: text_input(0), checkbox(1), button(2).
const EV_NOMBRE: u32 = 0;
const EV_MAYUS: u32 = 1;
const EV_LIMPIAR: u32 = 2;

// col = [titulo, campo, check, saludo, limpiar].
fn saludo(g: &WasmGuest) -> String {
    g.view().children[3].text.as_ref().unwrap().content.clone()
}
fn campo(g: &WasmGuest) -> String {
    g.view().children[1].input.as_ref().unwrap().value.clone()
}

#[test]
fn form_text_toggle_y_click() {
    let mut g = WasmGuest::load(FORM_WASM, 0).expect("carga el form");
    assert_eq!(saludo(&g), "Hola, …");
    assert_eq!(campo(&g), "");

    // Evento Text: el guest reconstruye Msg::SetNombre(texto) y actualiza.
    g.dispatch(EV_NOMBRE, EventPayload::Text("ana".into())).unwrap();
    assert_eq!(campo(&g), "ana");
    assert_eq!(saludo(&g), "Hola, ana!");

    // Evento Toggle: Msg::SetMayus(true) ⇒ saludo en mayúsculas.
    g.dispatch(EV_MAYUS, EventPayload::Toggle(true)).unwrap();
    assert_eq!(saludo(&g), "HOLA, ANA!");

    // Evento Click: Msg::Limpiar.
    g.dispatch(EV_LIMPIAR, EventPayload::Click).unwrap();
    assert_eq!(campo(&g), "");
    assert_eq!(saludo(&g), "Hola, …");
}

#[test]
fn input_se_materializa_focusable() {
    let g = WasmGuest::load(FORM_WASM, 0).expect("carga el form");
    let view = g.render();
    // El campo (2º hijo) es un input: focusable.
    assert!(view.children[1].focusable.is_some(), "el input es focusable");
}

#[test]
fn edit_value_anexa_y_borra() {
    // Carácter se anexa.
    assert_eq!(
        edit_value("an", &Key::Character("a".into()), Some("a")),
        Some("ana".into())
    );
    // Backspace borra el último.
    assert_eq!(
        edit_value("ana", &Key::Named(NamedKey::Backspace), None),
        Some("an".into())
    );
    // Otras teclas no editan.
    assert_eq!(edit_value("x", &Key::Named(NamedKey::Enter), None), None);
}
