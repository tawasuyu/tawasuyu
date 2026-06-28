//! Corre la app WASM Tier 3 `counter` dentro de una ventana Llimphi real.
//!
//! El `.wasm` está embebido (compilado por `scripts/build-wasm-demo.sh`). El
//! host no sabe nada del contador: sólo pide `wasm_view`, materializa el
//! `WireNode` y rebota los clicks. Toda la lógica vive del lado guest.
//!
//! `cargo run -p llimphi-wasm-runner --example run_counter --release`

use llimphi_ui::{App, Handle, View};
use llimphi_wasm_runner::{RunnerMsg, WasmGuest};

const COUNTER_WASM: &[u8] = include_bytes!("../assets/counter.wasm");

struct Host;

impl App for Host {
    type Model = WasmGuest;
    type Msg = RunnerMsg;

    fn title() -> &'static str {
        "llimphi · wasm runner — counter"
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        WasmGuest::load(COUNTER_WASM).expect("cargar counter.wasm")
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        if let Err(e) = model.apply(&msg) {
            eprintln!("wasm dispatch: {e}");
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        model.render()
    }
}

fn main() {
    llimphi_ui::run::<Host>();
}
