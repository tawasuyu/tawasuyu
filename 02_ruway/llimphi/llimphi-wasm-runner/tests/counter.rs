//! Certificación headless del bucle Tier 3 — sin GPU ni ventana.
//!
//! Carga el `.wasm` del contador (compilado a wasm32, versionado en assets/),
//! lo corre vía [`WasmGuest`] y verifica el viaje redondo completo: el guest
//! pinta un `WireNode`, el host lo lee, un evento cruza a `wasm_dispatch`, el
//! `Model` del guest muta y la nueva vista lo refleja. También materializa el
//! `WireNode` a un `View<RunnerMsg>` real y confirma que el árbol Llimphi tiene
//! la forma esperada (número + dos botones con su Msg).

use llimphi_wasm_runner::{wire_to_view, RunnerMsg, WasmGuest};

const COUNTER_WASM: &[u8] = include_bytes!("../assets/counter.wasm");

// Encoding postcard de los Msg del guest (enum unit-variants → índice varint).
// Increment es la variante 0, Reset la 1. Lo verificamos abajo contra el
// `on_click` que el propio guest serializó, así no es un número mágico ciego.
const MSG_INCREMENT: &[u8] = &[0];
const MSG_RESET: &[u8] = &[1];

/// El texto del número es el del primer hijo del árbol raíz (col → [número, botones]).
fn number(guest: &WasmGuest) -> String {
    guest.view().children[0]
        .text
        .as_ref()
        .expect("el nodo número tiene texto")
        .content
        .clone()
}

#[test]
fn counter_round_trip() {
    let mut guest = WasmGuest::load(COUNTER_WASM).expect("carga el guest counter");

    // Estado inicial.
    assert_eq!(number(&guest), "0");

    // El guest serializó sus propios Msg en los botones. Confirmamos que
    // nuestros bytes de evento coinciden con lo que él emite — el contrato es
    // simétrico, no adivinado.
    let buttons = &guest.view().children[1].children;
    assert_eq!(buttons.len(), 2, "fila con +1 y reset");
    assert_eq!(buttons[0].on_click.as_deref(), Some(MSG_INCREMENT));
    assert_eq!(buttons[1].on_click.as_deref(), Some(MSG_RESET));

    // Un click en +1 cruza a wasm_dispatch y muta el Model del guest.
    guest.dispatch(MSG_INCREMENT).expect("dispatch increment");
    assert_eq!(number(&guest), "1");

    guest.dispatch(MSG_INCREMENT).expect("dispatch increment");
    assert_eq!(number(&guest), "2");

    // Reset vuelve a 0.
    guest.dispatch(MSG_RESET).expect("dispatch reset");
    assert_eq!(number(&guest), "0");
}

#[test]
fn materializa_view_real() {
    let guest = WasmGuest::load(COUNTER_WASM).expect("carga el guest counter");

    // El WireNode se convierte en un View<RunnerMsg> Llimphi de verdad: si el
    // mapeo explota (estilos, colores, handlers), esto trap-ea.
    let view: llimphi_ui::View<RunnerMsg> = wire_to_view(guest.view());

    // Raíz: columna con relleno y dos hijos (número + fila de botones).
    assert_eq!(view.children.len(), 2);

    // El botón +1 quedó con su on_click = RunnerMsg::Guest(bytes de Increment).
    let row = &view.children[1];
    let inc = &row.children[0];
    match inc.on_click.as_ref().expect("el botón +1 tiene on_click") {
        RunnerMsg::Guest(bytes) => assert_eq!(bytes.as_slice(), MSG_INCREMENT),
    }
    // Y arrastró su fill (botón verde) al View real.
    assert!(inc.fill.is_some(), "el botón conserva su relleno");
}
