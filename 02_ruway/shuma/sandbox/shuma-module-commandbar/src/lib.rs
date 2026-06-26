//! `shuma-module-commandbar` — palette tipo Cmd-P en la barra inferior.
//!
//! Vive en el slot [`Placement::BottomBar`] del chasis. El usuario
//! tipea para buscar contra un **catálogo** de comandos (focus a un
//! tab, abrir el launcher, ejecutar una línea); los matches se
//! puntúan con fuzzy (`nucleo_matcher`) y se listan en un dropdown
//! sobre la barra. Up/Down navegan, Enter activa el seleccionado.
//!
//! El catálogo lo provee el chasis vía [`State::set_catalog`] —
//! típicamente son los `[apps]` del shumarc + los `[modules]`
//! activos + un par de built-ins (`> reload-config`, `> theme`, …).
//!
//! El modo dual (launcher/shell) se mantiene: en modo `Shell`, Enter
//! emite `Activated(Exec(text))` para que el chasis lo enrute al
//! módulo shell; en modo `Launcher`, Enter activa el match
//! seleccionado del dropdown.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_theme::Theme;
use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Matcher,
};
use shuma_module::{ModuleContributions, Placement};

/// `id` canónico del módulo.
pub const ID: &str = "command-bar";

/// `Placement` por defecto: barra inferior fija.
pub const DEFAULT_PLACEMENT: Placement = Placement::BottomBar;

/// Contexto del input. En `Launcher` se busca el catálogo;
/// en `Shell` se ejecuta como línea de shell (el chasis hace el ruteo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Launcher,
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

/// Una entrada del catálogo de comandos. El `kind` decide qué hace
/// el chasis al activarla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandEntry {
    /// Texto que se busca y se muestra.
    pub label: String,
    /// Categoría (App, Module, Builtin, …) — sólo para subtítulo.
    pub category: String,
    /// Acción al activarla.
    pub kind: CommandKind,
}

/// Acción asociada a una entry — opaca para el módulo, interpretada
/// por el chasis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandKind {
    /// Focar el tab del drawer cuyo `Kind::id()` es `target`.
    FocusTab(String),
    /// Lanzar una línea (spawn detached o exec en shell según el
    /// chasis decida — el módulo no sabe).
    Exec(String),
    /// Dispatch genérico (`open:files`, `theme:next`, etc.) —
    /// resuelto por la tabla de shortcuts del chasis.
    Action(String),
}

/// Estado del módulo.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub text: String,
    pub mode: Mode,
    /// Catálogo provisionado por el chasis (típicamente al arranque
    /// y cuando cambia el shumarc).
    pub catalog: Vec<CommandEntry>,
    /// Índice seleccionado dentro de la lista de matches actuales.
    pub selected: usize,
}

impl State {
    pub fn set_catalog(&mut self, catalog: Vec<CommandEntry>) {
        self.catalog = catalog;
        self.selected = 0;
    }

    /// Devuelve los índices de `catalog` que matchean `self.text`,
    /// ordenados por score descendente. Limita a `limit` resultados.
    pub fn matches(&self, limit: usize) -> Vec<usize> {
        if self.text.trim().is_empty() {
            // Sin query: todos los catalog en orden de declaración.
            return (0..self.catalog.len()).take(limit).collect();
        }
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
        let pattern = Pattern::parse(
            self.text.trim(),
            CaseMatching::Smart,
            Normalization::Smart,
        );
        let mut scored: Vec<(usize, u32)> = self
            .catalog
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                pattern
                    .score(
                        nucleo_matcher::Utf32String::from(e.label.as_str()).slice(..),
                        &mut matcher,
                    )
                    .map(|s| (i, s))
            })
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.into_iter().take(limit).map(|(i, _)| i).collect()
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Tecla recibida desde el chasis. Texto, Backspace, Up/Down y
    /// Enter se procesan acá.
    Key(KeyEvent),
    /// El usuario togglea el modo (Ctrl+grave o similar).
    ToggleMode,
    /// Click en una row del dropdown — el `usize` es el `catalog_idx`.
    ActivateAt(usize),
    /// Click sobre la barra (no en el dropdown). El chasis lo
    /// intercepta para abrir el drawer Quake; el módulo no lo procesa.
    BarClicked,
}

/// El chasis observa este `Activated` después de llamar `update` y
/// dispatchea según el `CommandKind` (focar tab, lanzar binario,
/// etc.). El módulo deja `last_activated` lleno por un solo Tick.
#[derive(Debug, Clone, Default)]
pub struct Activation {
    pub kind: Option<CommandKind>,
}

pub fn dispatch(_action_id: &str) -> Option<Msg> {
    None
}

pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Key(ev) => {
            if ev.state != KeyState::Pressed {
                return s;
            }
            match &ev.key {
                Key::Named(NamedKey::Backspace) => {
                    s.text.pop();
                    s.selected = 0;
                }
                Key::Named(NamedKey::ArrowDown) => {
                    let max = s.matches(50).len();
                    if max > 0 && s.selected + 1 < max {
                        s.selected += 1;
                    }
                }
                Key::Named(NamedKey::ArrowUp) => {
                    s.selected = s.selected.saturating_sub(1);
                }
                Key::Named(NamedKey::Escape) => {
                    s.text.clear();
                    s.selected = 0;
                }
                Key::Named(NamedKey::Enter) => {
                    // Enter NO clear-ea acá — el chasis intercepta el
                    // submit y limpia tras procesar el Activation.
                }
                _ => {
                    if let Some(text) = &ev.text {
                        if !text.is_empty()
                            && !text.chars().any(|c| c.is_control())
                        {
                            s.text.push_str(text);
                            s.selected = 0;
                        }
                    }
                }
            }
        }
        Msg::ToggleMode => {
            s.mode = s.mode.toggle();
            s.selected = 0;
        }
        Msg::ActivateAt(idx) => {
            s.selected = idx;
        }
        Msg::BarClicked => {}
    }
    s
}

/// Devuelve la acción "activada" cuando el usuario presiona Enter o
/// hace click en una row del dropdown. El chasis la consume y limpia
/// el state vía `clear_after_activation`.
pub fn activation_for(state: &State, ev: &KeyEvent) -> Option<CommandKind> {
    if !matches!(ev.key, Key::Named(NamedKey::Enter)) {
        return None;
    }
    if ev.state != KeyState::Pressed {
        return None;
    }
    match state.mode {
        Mode::Shell => {
            // En modo shell, Enter ejecuta la línea tal cual.
            if state.text.trim().is_empty() {
                None
            } else {
                Some(CommandKind::Exec(state.text.clone()))
            }
        }
        Mode::Launcher => {
            // En modo launcher, Enter activa el match seleccionado.
            let matches = state.matches(50);
            matches
                .get(state.selected)
                .and_then(|i| state.catalog.get(*i))
                .map(|e| e.kind.clone())
        }
    }
}

/// Llamar después de procesar una activación: limpia el texto y
/// resetea el cursor. Mantiene el catalog y el mode.
pub fn clear_after_activation(state: State) -> State {
    let mut s = state;
    s.text.clear();
    s.selected = 0;
    s
}

pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + 'static + Clone,
) -> View<HostMsg> {
    let prompt = format!("{} ", state.mode.prompt());
    let placeholder = format!(
        "{}escribí — Enter ejecuta · Ctrl+` cambia a {}",
        prompt,
        state.mode.toggle().label()
    );
    let display_text = if state.text.is_empty() {
        placeholder
    } else {
        format!("{}{}", prompt, state.text)
    };

    let bar = View::new(Style {
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
        display_text,
        12.0,
        if state.text.is_empty() {
            theme.fg_muted
        } else {
            theme.fg_text
        },
        Alignment::Start,
    )
    .on_click(lift.clone()(Msg::BarClicked));

    // Dropdown sólo en modo Launcher con texto no vacío.
    if !matches!(state.mode, Mode::Launcher) || state.text.is_empty() {
        return bar;
    }
    let matches = state.matches(8);
    if matches.is_empty() {
        return bar;
    }

    let mut rows: Vec<View<HostMsg>> = Vec::with_capacity(matches.len());
    for (row_i, cat_i) in matches.iter().enumerate() {
        let Some(entry) = state.catalog.get(*cat_i) else {
            continue;
        };
        let is_selected = row_i == state.selected;
        let bg = if is_selected {
            theme.bg_selected
        } else {
            theme.bg_panel
        };
        let fg = if is_selected {
            theme.fg_text
        } else {
            theme.fg_muted
        };
        let label_row = format!("  {}   ({})", entry.label, entry.category);
        let idx = *cat_i;
        let lift_row = lift.clone();
        rows.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(bg)
            .text_aligned(label_row, 12.0, fg, Alignment::Start)
            .on_click(lift_row(Msg::ActivateAt(idx))),
        );
    }
    let dropdown = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(rows);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .children(vec![dropdown, bar])
}

pub fn contributions(_state: &State) -> ModuleContributions {
    ModuleContributions::empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_ui::Modifiers;

    fn ev(key: Key, text: Option<&str>) -> KeyEvent {
        KeyEvent {
            key,
            state: KeyState::Pressed,
            text: text.map(|s| s.to_string()),
            modifiers: Modifiers::default(),
            repeat: false,
        }
    }

    fn fixture_catalog() -> Vec<CommandEntry> {
        vec![
            CommandEntry {
                label: "Pluma editor".into(),
                category: "app".into(),
                kind: CommandKind::Exec("pluma-app-llimphi".into()),
            },
            CommandEntry {
                label: "Focus shell".into(),
                category: "module".into(),
                kind: CommandKind::FocusTab("shell".into()),
            },
            CommandEntry {
                label: "Focus matilda".into(),
                category: "module".into(),
                kind: CommandKind::FocusTab("matilda".into()),
            },
        ]
    }

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "command-bar");
    }

    #[test]
    fn default_placement_is_bottombar() {
        assert_eq!(DEFAULT_PLACEMENT, Placement::BottomBar);
    }

    #[test]
    fn typing_filters_catalog() {
        let mut s = State::default();
        s.set_catalog(fixture_catalog());
        // Tipear "shell" debería poner "Focus shell" arriba.
        for c in "shell".chars() {
            let mut buf = [0u8; 4];
            s = update(
                s,
                Msg::Key(ev(Key::Character(c.to_string().into()), Some(c.encode_utf8(&mut buf)))),
            );
        }
        let m = s.matches(5);
        assert!(!m.is_empty());
        assert_eq!(s.catalog[m[0]].label, "Focus shell");
    }

    #[test]
    fn arrow_down_moves_selection() {
        let mut s = State::default();
        s.set_catalog(fixture_catalog());
        // Sin filtro: 3 matches en orden de declaración.
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowDown), None)));
        assert_eq!(s.selected, 1);
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowDown), None)));
        assert_eq!(s.selected, 2);
        // No pasa el límite.
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowDown), None)));
        assert_eq!(s.selected, 2);
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowUp), None)));
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn enter_in_launcher_activates_selected() {
        let mut s = State::default();
        s.set_catalog(fixture_catalog());
        // selected = 0 → "Pluma editor"
        let enter = ev(Key::Named(NamedKey::Enter), None);
        let kind = activation_for(&s, &enter).expect("activación");
        assert!(matches!(kind, CommandKind::Exec(ref l) if l == "pluma-app-llimphi"));
    }

    #[test]
    fn enter_in_shell_returns_exec_with_text() {
        let mut s = State::default();
        s.mode = Mode::Shell;
        s.text = "ls -la".into();
        let enter = ev(Key::Named(NamedKey::Enter), None);
        let kind = activation_for(&s, &enter).expect("activación");
        assert!(matches!(kind, CommandKind::Exec(ref l) if l == "ls -la"));
    }

    #[test]
    fn escape_clears_text() {
        let mut s = State::default();
        s.text = "hola".into();
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Escape), None)));
        assert!(s.text.is_empty());
    }

    #[test]
    fn toggle_mode_flips_only_mode() {
        let mut s = State::default();
        s.text = "ls".into();
        let s = update(s, Msg::ToggleMode);
        assert_eq!(s.mode, Mode::Shell);
        assert_eq!(s.text, "ls");
    }

    #[test]
    fn clear_after_activation_resets_text_keeps_catalog() {
        let mut s = State::default();
        s.set_catalog(fixture_catalog());
        s.text = "hola".into();
        let s = clear_after_activation(s);
        assert!(s.text.is_empty());
        assert_eq!(s.catalog.len(), 3);
    }

    #[test]
    fn matches_returns_all_with_empty_query() {
        let mut s = State::default();
        s.set_catalog(fixture_catalog());
        let m = s.matches(10);
        assert_eq!(m.len(), 3);
    }
}
