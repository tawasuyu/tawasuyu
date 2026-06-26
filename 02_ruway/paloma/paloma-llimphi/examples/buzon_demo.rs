//! `buzon_demo` — paloma corriendo sobre un `MockBackend` sembrado, sin red.
//!
//! Tres paneles reales (buzones · hilos · lectura) + redacción. Sirve para
//! ejercitar el frontend sin credenciales: un INBOX con varias conversaciones
//! hiladas (una con tres mensajes), no-leídos, y un `Sent` que se puebla al
//! enviar.
//!
//! Atajos: `c` redacta · `r` responde al hilo abierto · `F5` refresca ·
//! Tab cicla campos del compositor · Esc cierra · ⏎/botón envía.
//!
//! Corre con: `cargo run -p paloma-llimphi --example buzon_demo --release`.

use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, KeyEvent, Modifiers, View, WheelDelta};

use paloma_core::Address;
use paloma_llimphi::{Model, Msg};

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "paloma"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 720)
    }

    fn init(_handle: &Handle<Msg>) -> Model {
        let me = Address::named("Sergio", "sergio@jlsoltech.com");
        Model::new(Box::new(paloma_llimphi::demo::backend()), me, Theme::dark())
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        paloma_llimphi::update(model, msg, handle)
    }

    fn view(model: &Model) -> View<Msg> {
        paloma_llimphi::view(model)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        paloma_llimphi::view_overlay(model)
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        paloma_llimphi::on_key(model, event)
    }

    fn on_wheel(model: &Model, delta: WheelDelta, cursor: (f32, f32), mods: Modifiers) -> Option<Msg> {
        paloma_llimphi::on_wheel(model, delta, cursor, mods)
    }

    fn on_resize(model: &Model, w: u32, h: u32) -> Option<Msg> {
        paloma_llimphi::on_resize(model, w, h)
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
