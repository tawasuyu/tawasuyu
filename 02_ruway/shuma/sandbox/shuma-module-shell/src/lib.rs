//! `shuma-module-shell` — el shell interactivo como módulo enchufable.
//!
//! El REPL del shell (input + runs + historial + monitores de procesos)
//! deja de vivir hardcodeado en `shuma-shell-llimphi` y pasa a ser un
//! módulo más, igual que matilda o un futuro `launcher`. El chasis lo
//! enlista en su `Registry` y el shumarc decide si activarlo como tab
//! principal o como drawer desplegable.
//!
//! Este crate trae **el placeholder** del módulo (vista mínima + Msg
//! vacío + `contributions()` con cero monitores y cero shortcuts).
//! La migración real del REPL desde `shuma-shell` (GPUI, 3.7k LOC)
//! llega en su propio bloque — al portarse, este módulo gana state
//! sin que el chasis se entere.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::Theme;
use shuma_module::{ModuleContributions, Source};

/// `id` canónico del módulo. El shumarc lo referencia para activarlo.
pub const ID: &str = "shell";

/// Estado del módulo. En el placeholder es vacío; cuando se porte el
/// REPL real, aquí vivirá la línea actual, el historial, los runs, etc.
/// El `Source` lo guardamos desde el día uno: un shell puede ser local
/// (sesión local de procesos) o remoto (REPL contra un servidor SSH).
#[derive(Debug, Clone)]
pub struct State {
    pub source: Source,
}

impl State {
    pub fn new(source: Source) -> Self {
        Self { source }
    }
}

/// Mensajes del módulo. Vacío por ahora — el chasis los enruta sin
/// interpretarlos. Cuando se migre el REPL, aparecerán `InputChanged`,
/// `Submit`, `Tick`, etc., y el chasis los seguirá enrutando con el
/// mismo `lift`.
#[derive(Debug, Clone)]
pub enum Msg {}

/// Transición. Como `Msg` está vacío, este enum nunca tiene match
/// posible; el `_` cubre cualquier futuro Msg sin romper compilación
/// del chasis.
pub fn update(state: State, _msg: Msg) -> State {
    state
}

/// Vista del tab del shell. Recibe `lift: Fn(Msg) -> HostMsg` para que
/// el módulo pueda emitir Msgs que el chasis enruta al `update` del
/// host. Como Msg está vacío, el placeholder no usa `lift` todavía —
/// la firma queda lista para cuando llegue el REPL real.
pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    _lift: impl Fn(Msg) -> HostMsg + 'static,
) -> View<HostMsg> {
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        format!("Shell · {} · placeholder", state.source.label()),
        16.0,
        theme.fg_text,
        Alignment::Start,
    );

    let body = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        "El REPL Llimphi llega cuando se migre la versión GPUI (3.726 LOC).\n\n\
         Lo que vive aquí: input con resaltado + autocomplete, grid de runs,\n\
         historial durable, monitores de procesos. El estado por sesión va\n\
         en `State`; el chasis sólo enruta Msgs vía `lift`."
            .to_string(),
        12.0,
        theme.fg_muted,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![header, body])
}

/// Contribuciones declarativas (monitores + shortcuts). El placeholder
/// no aporta ninguna; cuando llegue el REPL real, los monitores de
/// procesos y shortcuts (`:kill`, `:fg`, `:bg`) entrarán por aquí.
pub fn contributions(_state: &State) -> ModuleContributions {
    ModuleContributions::empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "shell");
    }

    #[test]
    fn placeholder_state_constructs() {
        let s = State::new(Source::Local);
        assert_eq!(s.source, Source::Local);
    }

    #[test]
    fn contributions_are_empty() {
        let s = State::new(Source::Local);
        let c = contributions(&s);
        assert!(c.monitors.is_empty());
        assert!(c.shortcuts.is_empty());
    }
}
