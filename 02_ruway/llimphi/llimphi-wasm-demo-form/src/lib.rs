//! Formulario WASM Tier 3: campo de texto + checkbox + saludo vivo.
//!
//! Ejercita los eventos con payload: `Text` (lo tecleado en el input) y `Toggle`
//! (el checkbox), además de `Click` (limpiar). El `value` del input es la fuente
//! de verdad del guest — el host no guarda buffer de edición.

use llimphi_wasm_app_sdk::{
    col, export_wasm_app, row, text, Align, Dim, Justify, TextAlign, Ui, WasmApp, WireNode,
};

#[derive(Clone)]
pub enum Msg {
    SetNombre(String),
    SetMayus(bool),
    Limpiar,
}

#[derive(Default)]
pub struct Form {
    nombre: String,
    mayus: bool,
}

impl Form {
    fn saludo(&self) -> String {
        if self.nombre.is_empty() {
            return "Hola, …".into();
        }
        let s = format!("Hola, {}!", self.nombre);
        if self.mayus {
            s.to_uppercase()
        } else {
            s
        }
    }
}

impl WasmApp for Form {
    type Msg = Msg;

    fn init() -> Self {
        Form::default()
    }

    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::SetNombre(s) => self.nombre = s,
            Msg::SetMayus(b) => self.mayus = b,
            Msg::Limpiar => {
                self.nombre.clear();
                self.mayus = false;
            }
        }
    }

    fn view(&self, ui: &mut Ui<Msg>) -> WireNode {
        let titulo = text("Saludo", 32.0, [230, 240, 250, 255]);

        // EventId 0: el campo de nombre.
        let campo = ui
            .text_input(self.nombre.clone(), "tu nombre…", Msg::SetNombre)
            .fill([28, 34, 44, 255])
            .radius(8.0)
            .padding(10.0, 12.0, 10.0, 12.0)
            .width(Dim::Pct(1.0))
            .height(Dim::Px(44.0));

        // EventId 1: el checkbox; EventId 2: el botón limpiar.
        let check = row(vec![
            ui.checkbox(self.mayus, Msg::SetMayus),
            text("MAYÚSCULAS", 20.0, [200, 210, 225, 255]).align(Align::Center),
        ])
        .gap(10.0)
        .align(Align::Center);

        let saludo = text(self.saludo(), 28.0, [120, 220, 170, 255]).grow(1.0);

        let limpiar = ui
            .button("limpiar", 20.0, [30, 10, 10, 255], Msg::Limpiar)
            .text_align(TextAlign::Center)
            .fill([200, 120, 90, 255])
            .radius(8.0)
            .size(Dim::Px(120.0), Dim::Px(40.0))
            .align(Align::Center)
            .justify(Justify::Center);

        col(vec![titulo, campo, check, saludo, limpiar])
            .gap(18.0)
            .pad(28.0)
            .fill([18, 22, 30, 255])
            .width(Dim::Pct(1.0))
            .height(Dim::Pct(1.0))
    }
}

export_wasm_app!(Form);
