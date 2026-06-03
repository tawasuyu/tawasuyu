//! Widgets builtin. Cada uno es un struct con `Widget` + un constructor
//! `from_spec(&WidgetSpec [, &BuildCtx])`. El factory [`build`] mapea
//! `kind` → trait-object.

pub mod brightness;
pub mod clipboard;
pub mod clock;
pub mod cpu;
pub mod placeholder;
pub mod quake;
pub mod ram;
pub mod shuma_bar;
pub mod system_tray;
pub mod volume;

use crate::config::WidgetSpec;
use crate::widget::Widget;
use clock::TzMode;

/// Contexto inmutable que el factory pasa a los widgets que dependen de
/// settings transversales (timezone, locale, etc.). Crece cuando hace
/// falta sin cambiar la firma de los widgets que no lo usan.
#[derive(Debug, Clone, Copy)]
pub struct BuildCtx {
    pub tz: TzMode,
}

/// Despacha un `WidgetSpec` a su widget concreto. Kinds desconocidos caen
/// a un placeholder que dice `?<kind>` — visible pero sin romper la barra.
pub fn build(spec: &WidgetSpec, ctx: &BuildCtx) -> Box<dyn Widget> {
    match spec.kind.as_str() {
        "clock" => Box::new(clock::Clock::from_spec(spec, ctx.tz)),
        "ram_meter" => Box::new(ram::RamMeter::from_spec(spec)),
        "cpu_meter" => Box::new(cpu::CpuMeter::from_spec(spec)),
        "brightness" => Box::new(brightness::Brightness::from_spec(spec)),
        "volume" => Box::new(volume::Volume::from_spec(spec)),
        "clipboard" => Box::new(clipboard::Clipboard::from_spec(spec)),
        "quake_input" => Box::new(quake::QuakeInput::from_spec(spec)),
        "shuma_bar" => Box::new(shuma_bar::ShumaBar::from_spec(spec)),
        "system_tray" => Box::new(system_tray::SystemTray::from_spec(spec)),
        other => Box::new(placeholder::Placeholder::new(format!("?{other}"))),
    }
}
