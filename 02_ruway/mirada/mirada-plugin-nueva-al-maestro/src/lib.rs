//! Plugin reactor **nueva al maestro**: cada ventana que se abre se promueve al
//! área maestra, dejándola arriba de la pila — el clásico «new window on top» de
//! dwm. Útil si querés que lo último que abrís sea siempre lo prominente, sin
//! tener que pulsar el atajo de promoción a mano.
//!
//! Sin config. Usa sólo `CAP_ACTIONS`: emite `promote-to-master` (la ventana
//! recién abierta tiene el foco, y la acción promueve a la enfocada). El Desktop
//! arbitra; el plugin no mueve píxeles.

#![no_std]

extern crate alloc;

use mirada_plugin_sdk::{export_reactor_plugin, BodyEvent, Ctx, ReactorPlugin};

#[derive(Default)]
struct NuevaAlMaestro;

impl ReactorPlugin for NuevaAlMaestro {
    fn on_event(&mut self, event: BodyEvent, ctx: &mut Ctx) {
        if let BodyEvent::WindowOpened { .. } = event {
            ctx.act("promote-to-master");
        }
    }
}

export_reactor_plugin!(NuevaAlMaestro::default());
