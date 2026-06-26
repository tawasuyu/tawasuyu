//! `agenda_demo` — raymi corriendo sobre un `MockBackend` sembrado, sin red.
//!
//! Calendario (grilla del mes con eventos coloreados + agenda del día) y
//! Contactos (lista buscable + ficha). Atajos: ←/→ cambian de mes · `t` hoy ·
//! `g` calendario · `k` contactos · rueda cambia de mes.
//!
//! Corre con: `cargo run -p raymi-llimphi --example agenda_demo --release`.

use std::time::{SystemTime, UNIX_EPOCH};

use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, KeyEvent, Modifiers, View, WheelDelta};

use raymi_llimphi::{Model, Msg};

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "raymi"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 720)
    }

    fn init(_handle: &Handle<Msg>) -> Model {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        Model::new(Box::new(raymi_llimphi::demo::backend(now)), Theme::dark())
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        raymi_llimphi::update(model, msg, handle)
    }

    fn view(model: &Model) -> View<Msg> {
        raymi_llimphi::view(model)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        raymi_llimphi::view_overlay(model)
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        raymi_llimphi::on_key(model, event)
    }

    fn on_wheel(model: &Model, delta: WheelDelta, cursor: (f32, f32), mods: Modifiers) -> Option<Msg> {
        raymi_llimphi::on_wheel(model, delta, cursor, mods)
    }

    fn on_resize(model: &Model, w: u32, h: u32) -> Option<Msg> {
        raymi_llimphi::on_resize(model, w, h)
    }
}

fn main() {
    rimay_localize::init();
    let _ = rimay_localize::set_locale(&wawa_config::WawaConfig::load().lang);
    llimphi_ui::run::<Demo>();
}
