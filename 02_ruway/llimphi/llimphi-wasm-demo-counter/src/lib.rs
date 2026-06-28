//! Contador Elm canónico de Llimphi, como app WASM Tier 3.
//!
//! Es el gemelo de `llimphi-ui/examples/counter.rs`, pero el `Model`, el
//! `update` y el `view` corren del lado guest (este `.wasm`); el host
//! (`llimphi-wasm-runner`) materializa el `WireNode` en un `View` real y rebota
//! los clicks a `update`. Compilar con `scripts/build-wasm-demo.sh`.

use llimphi_wasm_app_sdk::{
    button, col, export_wasm_app, row, text, Align, Dim, Justify, TextAlign, WasmApp, WireNode,
};
use serde::{Deserialize, Serialize};

/// Mensajes del contador. `pub` + (De)serialize: el host versiona el mismo
/// enum para encodear eventos en sus tests.
#[derive(Serialize, Deserialize)]
pub enum Msg {
    Increment,
    Reset,
}

pub struct Counter {
    n: u32,
}

impl WasmApp for Counter {
    type Msg = Msg;

    fn init() -> Self {
        Counter { n: 0 }
    }

    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Increment => self.n = self.n.saturating_add(1),
            Msg::Reset => self.n = 0,
        }
    }

    fn view(&self) -> WireNode {
        // Número grande, centrado, ocupando el espacio sobrante.
        let number = text(self.n.to_string(), 160.0, [230, 240, 250, 255])
            .text_align(TextAlign::Center)
            .grow(1.0)
            .width(Dim::Pct(1.0))
            .align(Align::Center)
            .justify(Justify::Center);

        let inc = button("+1", 28.0, [10, 30, 20, 255], &Msg::Increment)
            .text_align(TextAlign::Center)
            .fill([60, 200, 130, 255])
            .radius(12.0)
            .size(Dim::Px(160.0), Dim::Px(56.0))
            .align(Align::Center)
            .justify(Justify::Center);

        let reset = button("reset", 22.0, [30, 10, 10, 255], &Msg::Reset)
            .text_align(TextAlign::Center)
            .fill([220, 80, 80, 255])
            .radius(12.0)
            .size(Dim::Px(120.0), Dim::Px(56.0))
            .align(Align::Center)
            .justify(Justify::Center);

        let buttons = row(vec![inc, reset])
            .gap(16.0)
            .justify(Justify::Center)
            .width(Dim::Pct(1.0))
            .height(Dim::Px(56.0));

        col(vec![number, buttons])
            .gap(24.0)
            .pad(32.0)
            .fill([20, 24, 32, 255])
            .width(Dim::Pct(1.0))
            .height(Dim::Pct(1.0))
    }
}

export_wasm_app!(Counter);
