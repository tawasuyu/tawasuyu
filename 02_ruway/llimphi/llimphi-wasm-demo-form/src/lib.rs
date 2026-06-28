//! Formulario WASM Tier 3: texto + checkbox + slider + dropdown + saludo vivo.
//!
//! Ejercita todos los eventos con payload: `Text` (nombre), `Toggle`
//! (mayúsculas), `Value` (slider edad), `Select` (dropdown color) y `Click`
//! (limpiar). El estado vive en el guest; el host sólo pinta y notifica.

use llimphi_wasm_app_sdk::{
    col, export_wasm_app, row, text, Align, Dim, Justify, TextAlign, Ui, WasmApp, WireNode,
};

const COLORES: [&str; 3] = ["rojo", "verde", "azul"];

#[derive(Clone)]
pub enum Msg {
    SetNombre(String),
    SetMayus(bool),
    SetEdad(f32),
    SetColor(u32),
    Limpiar,
}

pub struct Form {
    nombre: String,
    mayus: bool,
    edad: f32,
    color: u32,
}

impl Default for Form {
    fn default() -> Self {
        Form {
            nombre: String::new(),
            mayus: false,
            edad: 30.0,
            color: 1,
        }
    }
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
            Msg::SetEdad(v) => self.edad = v,
            Msg::SetColor(i) => self.color = i,
            Msg::Limpiar => {
                self.nombre.clear();
                self.mayus = false;
                self.edad = 30.0;
                self.color = 1;
            }
        }
    }

    fn view(&self, ui: &mut Ui<Msg>) -> WireNode {
        let titulo = text("Saludo", 32.0, [230, 240, 250, 255]);

        // EventId 0: campo de nombre.
        let campo = ui
            .text_input(self.nombre.clone(), "tu nombre…", Msg::SetNombre)
            .fill([28, 34, 44, 255])
            .radius(8.0)
            .padding(10.0, 12.0, 10.0, 12.0)
            .width(Dim::Pct(1.0))
            .height(Dim::Px(44.0));

        // EventId 1: checkbox.
        let check = row(vec![
            ui.checkbox(self.mayus, Msg::SetMayus),
            text("MAYÚSCULAS", 20.0, [200, 210, 225, 255]).align(Align::Center),
        ])
        .gap(10.0)
        .align(Align::Center);

        // EventId 2: slider de edad (0..100).
        let edad_row = row(vec![
            text(format!("Edad {}", self.edad as i32), 20.0, [200, 210, 225, 255])
                .width(Dim::Px(90.0)),
            ui.slider(self.edad, 0.0, 100.0, Msg::SetEdad)
                .grow(1.0)
                .height(Dim::Px(20.0)),
        ])
        .gap(12.0)
        .align(Align::Center);

        // EventId 3: dropdown de color.
        let color = ui
            .select(
                COLORES.iter().map(|s| s.to_string()).collect(),
                self.color,
                Msg::SetColor,
            )
            .width(Dim::Pct(1.0));

        let saludo = text(self.saludo(), 28.0, [120, 220, 170, 255]);
        let detalle = text(
            format!(
                "Edad: {} · Color: {}",
                self.edad as i32,
                COLORES[self.color as usize % COLORES.len()]
            ),
            18.0,
            [150, 160, 180, 255],
        )
        .grow(1.0);

        // EventId 4: botón limpiar.
        let limpiar = ui
            .button("limpiar", 20.0, [30, 10, 10, 255], Msg::Limpiar)
            .text_align(TextAlign::Center)
            .fill([200, 120, 90, 255])
            .radius(8.0)
            .size(Dim::Px(120.0), Dim::Px(40.0))
            .align(Align::Center)
            .justify(Justify::Center);

        col(vec![titulo, campo, check, edad_row, color, saludo, detalle, limpiar])
            .gap(16.0)
            .pad(28.0)
            .fill([18, 22, 30, 255])
            .width(Dim::Pct(1.0))
            .height(Dim::Pct(1.0))
    }
}

export_wasm_app!(Form);
