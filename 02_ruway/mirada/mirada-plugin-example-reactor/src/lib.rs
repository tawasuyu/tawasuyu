//! Un plugin reactor de ejemplo, con tres comportamientos que ejercitan cuatro
//! capacidades gateadas:
//!
//! - **Terminal** (`CAP_KEYS` + `CAP_SPAWN`): registra `Super+a` y, al pulsarlo,
//!   lanza una terminal.
//! - **Realce por foco** (`CAP_EFFECTS`): la ventana enfocada queda opaca y con
//!   sombra; las demás, a media luz y sin sombra — el efecto Tier-2 clásico de
//!   "inactive window dimming". Sigue el foco por click / entrada del puntero.
//! - **Auto-teselado** (`CAP_ACTIONS`): cuando el escritorio se llena
//!   (≥ [`CROWD`] ventanas) pide `layout:monocle` para despejar; con menos,
//!   vuelve a `layout:master-stack`. Es el ejemplo de un reactor que **maneja
//!   ventanas** pidiéndole acciones al `Desktop` autoritativo, no sólo
//!   observándolas — el patrón de los plugins de awesome/qtile.
//!
//! Si el manifest no concediera alguna capacidad, el símbolo del host no se
//! registra y el módulo ni instancia — la frontera es física.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use mirada_plugin_sdk::{export_reactor_plugin, BodyEvent, Ctx, ReactorPlugin, WindowEffects, WindowId};

/// Opacidad de las ventanas sin foco (≈ 70 %).
const DIM: u8 = 180;
const FULL: u8 = 255;

/// A partir de cuántas ventanas el escritorio se considera «lleno» y se pasa a
/// monocle para despejar. Por debajo, master-stack.
const CROWD: usize = 3;

#[derive(Default)]
struct Reactor {
    windows: Vec<WindowId>,
    focused: Option<WindowId>,
}

impl Reactor {
    /// Reaplica los efectos: la enfocada plena + sombra, el resto atenuado.
    fn redim(&self, ctx: &mut Ctx) {
        for &w in &self.windows {
            let foco = Some(w) == self.focused;
            ctx.set_effects(
                w,
                WindowEffects { opacity: if foco { FULL } else { DIM }, shadow: foco },
            );
        }
    }

    /// Pide el teselado acorde a la cantidad de ventanas: monocle si está lleno,
    /// master-stack si no. `SetLayout` al modo ya activo es inocuo, así que se
    /// puede pedir en cada apertura/cierre sin llevar la cuenta del modo previo.
    fn auto_layout(&self, ctx: &mut Ctx) {
        let action = if self.windows.len() >= CROWD {
            "layout:monocle"
        } else {
            "layout:master-stack"
        };
        ctx.act(action);
    }
}

impl ReactorPlugin for Reactor {
    fn on_event(&mut self, event: BodyEvent, ctx: &mut Ctx) {
        // Registro idempotente del atajo (el host deduplica la unión).
        ctx.grab_keys(&["Super+a"]);

        match event {
            BodyEvent::Keybind(k) if k.as_str() == "Super+a" => ctx.spawn("foot"),

            BodyEvent::WindowOpened { id, .. } => {
                if !self.windows.contains(&id) {
                    self.windows.push(id);
                }
                self.focused = Some(id);
                self.redim(ctx);
                self.auto_layout(ctx);
            }
            BodyEvent::WindowClosed { id } => {
                self.windows.retain(|&w| w != id);
                if self.focused == Some(id) {
                    self.focused = None;
                }
                self.redim(ctx);
                self.auto_layout(ctx);
            }
            // El foco sigue al click y a la entrada del puntero.
            BodyEvent::Clicked { id } | BodyEvent::PointerEntered { id } => {
                if self.focused != Some(id) {
                    self.focused = Some(id);
                    self.redim(ctx);
                }
            }
            _ => {}
        }
    }
}

export_reactor_plugin!(Reactor::default());
