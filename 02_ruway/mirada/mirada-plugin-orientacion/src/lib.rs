//! Plugin reactor **orientación adaptativa**: ajusta el modo de teselado al
//! aspecto del monitor. Cuando una salida aparece o cambia de tamaño, si es más
//! alta que ancha (monitor en vertical) pide `layout:rows` (ventanas apiladas);
//! si es apaisada, `layout:columns` (ventanas lado a lado). Así un monitor
//! pivotado a retrato no queda con un teselado pensado para horizontal.
//!
//! Sin config. Usa sólo `CAP_ACTIONS`. Nota: `layout:` fija el modo **global**
//! del Desktop; en multi-monitor gana la última salida que disparó el evento —
//! suficiente para el caso típico (un monitor, o uno que rota).

#![no_std]

extern crate alloc;

use mirada_plugin_sdk::{export_reactor_plugin, BodyEvent, Ctx, ReactorPlugin};

#[derive(Default)]
struct Orientacion;

/// Pide el modo según el aspecto; ignora dimensiones degeneradas (0).
fn aplicar(width: i32, height: i32, ctx: &mut Ctx) {
    if width <= 0 || height <= 0 {
        return;
    }
    if height > width {
        ctx.act("layout:rows");
    } else {
        ctx.act("layout:columns");
    }
}

impl ReactorPlugin for Orientacion {
    fn on_event(&mut self, event: BodyEvent, ctx: &mut Ctx) {
        match event {
            BodyEvent::OutputAdded { width, height, .. }
            | BodyEvent::OutputResized { width, height, .. } => aplicar(width, height, ctx),
            _ => {}
        }
    }
}

export_reactor_plugin!(Orientacion::default());
