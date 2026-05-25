//! `shuma-module-commandbar` — barra inferior fija con input de doble modo.
//!
//! Vive en el slot [`Placement::BottomBar`] del chasis. La barra
//! presenta **un solo input** que cambia de contexto según `Mode`:
//!
//! - **Launcher** (default): el texto ingresado se busca contra el
//!   catálogo de apps y comandos rápidos (tipo dmenu / krunner).
//! - **Shell**: el texto se ejecuta como una línea de shell (REPL).
//!
//! La tecla de cambio de modo se configura en el shumarc; por
//! convención el chasis la cablea a `Ctrl+\`` o similar. Click en la
//! barra abre el drawer Quake (el chasis intercepta el `on_click`).
//!
//! Este crate sólo trae la **vista placeholder** del input + el
//! state mínimo (el texto actual + el modo). El cableado real a
//! `shuma-line`/`shuma-exec` (modo shell) y al run-dialog (modo
//! launcher) llega aparte — son módulos del REPL existente.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::Theme;
use shuma_module::{ModuleContributions, Placement};

/// `id` canónico del módulo.
pub const ID: &str = "command-bar";

/// `Placement` por defecto: barra inferior fija.
pub const DEFAULT_PLACEMENT: Placement = Placement::BottomBar;

/// Contexto del input. El chasis cablea la tecla de cambio (típicamente
/// `Ctrl+grave`); el módulo sólo expone el estado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Tipear busca apps + comandos rápidos (tipo dmenu).
    #[default]
    Launcher,
    /// Tipear ejecuta como línea de shell.
    Shell,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Launcher => "launcher",
            Mode::Shell => "shell",
        }
    }

    pub fn prompt(self) -> &'static str {
        match self {
            Mode::Launcher => "›",
            Mode::Shell => "$",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Mode::Launcher => Mode::Shell,
            Mode::Shell => Mode::Launcher,
        }
    }
}

/// Estado del módulo: texto actual del input + modo. El chasis no lo
/// interpreta; lo refresca con `Msg`.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub text: String,
    pub mode: Mode,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// El usuario tipeó (sustituye el texto entero — el placeholder no
    /// hace edición de carácter por carácter).
    TextChanged(String),
    /// El usuario togglea el modo (tecla de cambio).
    ToggleMode,
    /// El usuario presionó Enter — ejecutar el comando o lanzar la app.
    /// El chasis lo intercepta para enrutar a shell-exec o app-launch.
    Submit,
}

/// Mapea `action_id` a `Msg`. La command bar no expone shortcuts
/// hoy, así que cualquier `action_id` da `None`.
pub fn dispatch(_action_id: &str) -> Option<Msg> {
    None
}

pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::TextChanged(t) => s.text = t,
        Msg::ToggleMode => s.mode = s.mode.toggle(),
        // El Submit limpia el texto pero deja el modo intacto. La
        // ejecución/launch la hace el chasis interceptando el Msg
        // antes de delegar al update del módulo.
        Msg::Submit => s.text.clear(),
    }
    s
}

/// Renderiza la barra inferior. Click sobre la barra dispara
/// `Msg::ToggleMode` SOLO si el chasis no intercepta el click para
/// abrir el drawer; el chasis decide el orden. Placeholder visual.
pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + 'static + Clone,
) -> View<HostMsg> {
    let prompt = format!("{} ", state.mode.prompt());
    let text = if state.text.is_empty() {
        format!(
            "{}escribí — Enter ejecuta · Ctrl+` cambia a {}",
            prompt,
            state.mode.toggle().label()
        )
    } else {
        format!("{}{}", prompt, state.text)
    };

    let on_click_msg = lift(Msg::ToggleMode);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(
        text,
        12.0,
        if state.text.is_empty() {
            theme.fg_muted
        } else {
            theme.fg_text
        },
        Alignment::Start,
    )
    .on_click(on_click_msg)
}

/// Contribuciones: ningún monitor (la command bar es su propia barra,
/// no aporta al monitor stack del drawer) ni shortcuts (los que vivan
/// en el toolbar son del módulo `Main` activo, no de la command bar).
pub fn contributions(_state: &State) -> ModuleContributions {
    ModuleContributions::empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "command-bar");
    }

    #[test]
    fn default_placement_is_bottombar() {
        assert_eq!(DEFAULT_PLACEMENT, Placement::BottomBar);
    }

    #[test]
    fn default_mode_is_launcher() {
        assert_eq!(Mode::default(), Mode::Launcher);
        assert_eq!(Mode::default().prompt(), "›");
    }

    #[test]
    fn mode_toggle_round_trips() {
        assert_eq!(Mode::Launcher.toggle(), Mode::Shell);
        assert_eq!(Mode::Shell.toggle(), Mode::Launcher);
    }

    #[test]
    fn text_changed_updates_state() {
        let s = State::default();
        let s = update(s, Msg::TextChanged("ls -la".into()));
        assert_eq!(s.text, "ls -la");
    }

    #[test]
    fn submit_clears_text_but_keeps_mode() {
        let mut s = State::default();
        s.text = "ls".into();
        s.mode = Mode::Shell;
        let s = update(s, Msg::Submit);
        assert_eq!(s.text, "");
        assert_eq!(s.mode, Mode::Shell);
    }

    #[test]
    fn toggle_mode_flips_only_mode() {
        let mut s = State::default();
        s.text = "ls".into();
        let s = update(s, Msg::ToggleMode);
        assert_eq!(s.mode, Mode::Shell);
        assert_eq!(s.text, "ls"); // texto intacto
    }
}
