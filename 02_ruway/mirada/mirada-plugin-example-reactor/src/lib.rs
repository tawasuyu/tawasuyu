//! Un plugin reactor de ejemplo. Registra el atajo `Super+a` (el host lo **une**
//! a los del `Desktop`) y, al pulsarlo, lanza una terminal. Ejercita dos
//! capacidades gateadas: `CAP_KEYS` (registrar el atajo) y `CAP_SPAWN` (lanzar).
//!
//! Si el manifest no concediera alguna, el símbolo del host correspondiente no
//! se registra y el módulo ni instancia — la frontera es física.

#![no_std]

extern crate alloc;

use mirada_plugin_sdk::{export_reactor_plugin, BodyEvent, Ctx, ReactorPlugin};

#[derive(Default)]
struct Terminalazo;

impl ReactorPlugin for Terminalazo {
    fn on_event(&mut self, event: BodyEvent, ctx: &mut Ctx) {
        // Idempotente: el host deduplica la unión, así que registrar en cada
        // evento sólo «cuesta» la primera vez (cuando la unión cambia).
        ctx.grab_keys(&["Super+a"]);

        if let BodyEvent::Keybind(k) = &event {
            if k.as_str() == "Super+a" {
                ctx.spawn("foot");
            }
        }
    }
}

export_reactor_plugin!(Terminalazo);
