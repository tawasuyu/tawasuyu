//! Widgets builtin. Cada uno es un struct con `Widget` + un constructor
//! `from_spec(&WidgetSpec)`. El factory [`build`] mapea `kind` → trait-object.

pub mod clock;
pub mod cpu;
pub mod placeholder;
pub mod quake;
pub mod ram;

use crate::config::WidgetSpec;
use crate::widget::Widget;

/// Despacha un `WidgetSpec` a su widget concreto. Kinds desconocidos caen
/// a un placeholder que dice `?<kind>` — visible pero sin romper la barra.
pub fn build(spec: &WidgetSpec) -> Box<dyn Widget> {
    match spec.kind.as_str() {
        "clock" => Box::new(clock::Clock::from_spec(spec)),
        "ram_meter" => Box::new(ram::RamMeter::from_spec(spec)),
        "cpu_meter" => Box::new(cpu::CpuMeter::from_spec(spec)),
        "quake_input" => Box::new(quake::QuakeInput::from_spec(spec)),
        other => Box::new(placeholder::Placeholder::new(format!("?{other}"))),
    }
}
