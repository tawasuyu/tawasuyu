//! Certificación headless del bucle Tier 3 — sin GPU ni ventana.
//!
//! Carga el `.wasm` del contador (compilado a wasm32, versionado en assets/),
//! lo corre vía [`WasmGuest`] y verifica el viaje redondo completo: el guest
//! pinta un `WireNode`, el host lo lee, un evento cruza a `wasm_dispatch` con su
//! `EventId` + `EventPayload`, el `Model` del guest muta y la nueva vista lo
//! refleja. También materializa el `WireNode` a un `View<RunnerMsg>` real.

use llimphi_wasm_runner::{wire_to_view, EventPayload, RunnerMsg, WasmGuest};

const COUNTER_WASM: &[u8] = include_bytes!("../assets/counter.wasm");

// El +1 es el primer control que registra el guest ⇒ EventId 0; reset ⇒ 1.
const EV_INCREMENT: u32 = 0;
const EV_RESET: u32 = 1;

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
    let mut guest = WasmGuest::load(COUNTER_WASM, 0).expect("carga el guest counter");

    // Estado inicial.
    assert_eq!(number(&guest), "0");

    // Los botones cargan los EventId que el guest asignó.
    let buttons = &guest.view().children[1].children;
    assert_eq!(buttons.len(), 2, "fila con +1 y reset");
    assert_eq!(buttons[0].on_click, Some(EV_INCREMENT));
    assert_eq!(buttons[1].on_click, Some(EV_RESET));

    // Un click en +1 cruza a wasm_dispatch y muta el Model del guest.
    guest
        .dispatch(EV_INCREMENT, EventPayload::Click)
        .expect("dispatch increment");
    assert_eq!(number(&guest), "1");

    guest
        .dispatch(EV_INCREMENT, EventPayload::Click)
        .expect("dispatch increment");
    assert_eq!(number(&guest), "2");

    // Reset vuelve a 0.
    guest
        .dispatch(EV_RESET, EventPayload::Click)
        .expect("dispatch reset");
    assert_eq!(number(&guest), "0");
}

#[test]
fn materializa_view_real() {
    let guest = WasmGuest::load(COUNTER_WASM, 0).expect("carga el guest counter");

    // El WireNode se convierte en un View<RunnerMsg> Llimphi de verdad: si el
    // mapeo explota (estilos, colores, handlers), esto trap-ea.
    let view: llimphi_ui::View<RunnerMsg> = wire_to_view(guest.view(), None, None);

    // Raíz: columna con relleno y dos hijos (número + fila de botones).
    assert_eq!(view.children.len(), 2);

    // El botón +1 quedó con on_click = RunnerMsg::Event(0, Click).
    let row = &view.children[1];
    let inc = &row.children[0];
    match inc.on_click.as_ref().expect("el botón +1 tiene on_click") {
        RunnerMsg::Event(id, EventPayload::Click) => assert_eq!(*id, EV_INCREMENT),
        other => panic!("on_click inesperado: {other:?}"),
    }
    assert!(inc.fill.is_some(), "el botón conserva su relleno");
}
