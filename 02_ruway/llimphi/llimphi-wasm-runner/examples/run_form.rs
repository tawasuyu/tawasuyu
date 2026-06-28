//! Corre la app WASM Tier 3 `form` (campo de texto + checkbox + saludo) en una
//! ventana Llimphi. Clickeá el campo para enfocarlo y tecleá: el host rutea las
//! teclas como eventos `Text` al guest, que es la fuente de verdad del texto.
//!
//! `cargo run -p llimphi-wasm-runner --example run_form --release`

use llimphi_ui::{App, Handle, KeyEvent, View};
use llimphi_wasm_runner::{RunnerMsg, WasmGuest};

const FORM_WASM: &[u8] = include_bytes!("../assets/form.wasm");

struct Host;

impl App for Host {
    type Model = WasmGuest;
    type Msg = RunnerMsg;

    fn title() -> &'static str {
        "llimphi · wasm runner — form"
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        WasmGuest::load(FORM_WASM, 0).expect("cargar form.wasm")
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

    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        model.key_to_msg(event)
    }

    fn on_focus(_model: &Self::Model, id: Option<u64>) -> Option<Self::Msg> {
        Some(WasmGuest::focus_msg(id))
    }
}

fn main() {
    llimphi_ui::run::<Host>();
}
