//! Certifica los eventos con payload (Text/Toggle/Value/Select) headless, con el
//! guest del formulario, más la edición value-driven y el render de los widgets.

use llimphi_ui::{Key, NamedKey};
use llimphi_wasm_runner::{edit_value, EventPayload, RunnerMsg, WasmGuest};

const FORM_WASM: &[u8] = include_bytes!("../assets/form.wasm");

// Orden de controles en Form::view (= orden de registro = EventId):
// text_input(0), checkbox(1), slider(2), select(3), button(4).
const EV_NOMBRE: u32 = 0;
const EV_MAYUS: u32 = 1;
const EV_EDAD: u32 = 2;
const EV_COLOR: u32 = 3;
const EV_LIMPIAR: u32 = 4;

// col = [titulo, campo, check, edad_row, color, saludo, detalle, limpiar].
fn campo(g: &WasmGuest) -> String {
    g.view().children[1].input.as_ref().unwrap().value.clone()
}
fn saludo(g: &WasmGuest) -> String {
    g.view().children[5].text.as_ref().unwrap().content.clone()
}
fn detalle(g: &WasmGuest) -> String {
    g.view().children[6].text.as_ref().unwrap().content.clone()
}

#[test]
fn form_text_toggle_y_click() {
    let mut g = WasmGuest::load(FORM_WASM, 0).expect("carga el form");
    assert_eq!(saludo(&g), "Hola, …");
    assert_eq!(campo(&g), "");

    g.dispatch(EV_NOMBRE, EventPayload::Text("ana".into())).unwrap();
    assert_eq!(campo(&g), "ana");
    assert_eq!(saludo(&g), "Hola, ana!");

    g.dispatch(EV_MAYUS, EventPayload::Toggle(true)).unwrap();
    assert_eq!(saludo(&g), "HOLA, ANA!");

    g.dispatch(EV_LIMPIAR, EventPayload::Click).unwrap();
    assert_eq!(campo(&g), "");
    assert_eq!(saludo(&g), "Hola, …");
}

#[test]
fn form_slider_y_dropdown() {
    let mut g = WasmGuest::load(FORM_WASM, 0).expect("carga el form");
    // Defaults: edad 30, color 1 (verde).
    assert!(detalle(&g).contains("Edad: 30"), "{}", detalle(&g));
    assert!(detalle(&g).contains("Color: verde"), "{}", detalle(&g));

    // Evento Value: el guest reconstruye Msg::SetEdad(75.0).
    g.dispatch(EV_EDAD, EventPayload::Value(75.0)).unwrap();
    assert!(detalle(&g).contains("Edad: 75"), "{}", detalle(&g));

    // Evento Select: Msg::SetColor(0) ⇒ "rojo".
    g.dispatch(EV_COLOR, EventPayload::Select(0)).unwrap();
    assert!(detalle(&g).contains("Color: rojo"), "{}", detalle(&g));
}

#[test]
fn dropdown_abre_y_lista_opciones() {
    let mut g = WasmGuest::load(FORM_WASM, 0).expect("carga el form");
    // Cerrado: el select (children[4]) materializa sólo su header.
    assert_eq!(g.render().children[4].children.len(), 1, "cerrado: sólo header");
    // Abrir: header + las 3 opciones.
    g.apply(&RunnerMsg::ToggleSelect(EV_COLOR)).unwrap();
    assert_eq!(
        g.render().children[4].children.len(),
        1 + 3,
        "abierto: header + 3 opciones"
    );
    // Cualquier evento cierra el dropdown.
    g.apply(&RunnerMsg::Event(EV_COLOR, EventPayload::Select(2))).unwrap();
    assert_eq!(g.render().children[4].children.len(), 1, "elegir cierra");
}

#[test]
fn slider_se_materializa_con_barra() {
    let g = WasmGuest::load(FORM_WASM, 0).expect("carga el form");
    let v = g.render();
    // edad_row = children[3]; el slider es su 2º hijo.
    let slider = &v.children[3].children[1];
    assert!(slider.on_click_at.is_some(), "el slider responde a click posicional");
    assert_eq!(slider.children.len(), 1, "track con barra de relleno");
}

#[test]
fn input_se_materializa_focusable() {
    let g = WasmGuest::load(FORM_WASM, 0).expect("carga el form");
    assert!(g.render().children[1].focusable.is_some(), "el input es focusable");
}

#[test]
fn edit_value_anexa_y_borra() {
    assert_eq!(
        edit_value("an", &Key::Character("a".into()), Some("a")),
        Some("ana".into())
    );
    assert_eq!(
        edit_value("ana", &Key::Named(NamedKey::Backspace), None),
        Some("an".into())
    );
    assert_eq!(edit_value("x", &Key::Named(NamedKey::Enter), None), None);
}
