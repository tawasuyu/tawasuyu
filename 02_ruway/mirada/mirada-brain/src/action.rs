//! Acciones de escritorio y su mapa de teclas por defecto.
//!
//! Una [`DesktopAction`] es una orden de alto nivel del usuario, ya
//! desligada de la tecla concreta: el [`Desktop`](crate::Desktop) las
//! aplica sin saber qué combinación las disparó.
//!
//! Cada acción tiene una **forma textual** estable ([`Display`] /
//! [`FromStr`]) — `"focus-next"`, `"layout:grid"`, `"workspace:3"` — que
//! es el vocabulario del keymap configurable en RON (ver [`crate::keymap`]).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use mirada_layout::{LayoutMode, WindowId};

/// Número de escritorios virtuales que mantiene el `Desktop`.
pub const WORKSPACE_COUNT: usize = 9;

/// Una orden de escritorio de alto nivel.
///
/// Es serializable (`postcard`) para viajar por el API de control
/// ([`crate::ctl`]) y tiene una forma textual estable ([`Display`] /
/// [`FromStr`]) para el keymap y `mirada-ctl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesktopAction {
    /// Mueve el foco a la ventana siguiente del escritorio activo.
    FocusNext,
    /// Mueve el foco a la ventana anterior.
    FocusPrev,
    /// Enfoca una ventana concreta por su id; si está en otro escritorio,
    /// salta a él. Para clics de taskbar o `mirada-ctl focus-window`.
    FocusWindow(WindowId),
    /// Adelanta la ventana enfocada en el orden de teselado.
    MoveForward,
    /// Atrasa la ventana enfocada en el orden de teselado.
    MoveBackward,
    /// Cierra la ventana enfocada (cierre ordenado).
    CloseFocused,
    /// Alterna entre flotante y teselada la ventana enfocada.
    ToggleFloat,
    /// Alterna pantalla completa en la ventana enfocada.
    ToggleFullscreen,
    /// Pasa al siguiente modo de teselado.
    CycleLayout,
    /// Fija un modo de teselado concreto.
    SetLayout(LayoutMode),
    /// Agranda el área de la ventana maestra (`MasterStack`/`CenteredMaster`).
    GrowMaster,
    /// Encoge el área de la ventana maestra.
    ShrinkMaster,
    /// Mete una ventana más en el área maestra (`nmaster`).
    IncMaster,
    /// Saca una ventana del área maestra.
    DecMaster,
    /// Lleva la ventana enfocada al puesto maestro (orden de teselado).
    PromoteToMaster,
    /// Activa el escritorio virtual `n` (índice 0-based).
    SwitchWorkspace(usize),
    /// Manda la ventana enfocada al escritorio virtual `n`.
    SendToWorkspace(usize),
    /// Mueve el foco a la siguiente salida (monitor).
    FocusOutputNext,
    /// Apaga el compositor.
    Quit,
}

/// El nombre RON-seguro de un modo de teselado (sin guiones problemáticos
/// para identificadores: aquí van como valor de cadena, no de enum).
fn layout_slug(mode: LayoutMode) -> &'static str {
    match mode {
        LayoutMode::MasterStack => "master-stack",
        LayoutMode::Monocle => "monocle",
        LayoutMode::Grid => "grid",
        LayoutMode::Columns => "columns",
        LayoutMode::Rows => "rows",
        LayoutMode::CenteredMaster => "centered-master",
        LayoutMode::Spiral => "spiral",
    }
}

/// Modo de teselado desde su `slug`.
fn layout_from_slug(slug: &str) -> Option<LayoutMode> {
    Some(match slug {
        "master-stack" => LayoutMode::MasterStack,
        "monocle" => LayoutMode::Monocle,
        "grid" => LayoutMode::Grid,
        "columns" => LayoutMode::Columns,
        "rows" => LayoutMode::Rows,
        "centered-master" => LayoutMode::CenteredMaster,
        "spiral" => LayoutMode::Spiral,
        _ => return None,
    })
}

impl fmt::Display for DesktopAction {
    /// La forma textual estable de la acción — el vocabulario del keymap.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DesktopAction::FocusNext => f.write_str("focus-next"),
            DesktopAction::FocusPrev => f.write_str("focus-prev"),
            DesktopAction::FocusWindow(id) => write!(f, "focus-window:{id}"),
            DesktopAction::MoveForward => f.write_str("move-forward"),
            DesktopAction::MoveBackward => f.write_str("move-backward"),
            DesktopAction::CloseFocused => f.write_str("close-focused"),
            DesktopAction::ToggleFloat => f.write_str("toggle-float"),
            DesktopAction::ToggleFullscreen => f.write_str("toggle-fullscreen"),
            DesktopAction::CycleLayout => f.write_str("cycle-layout"),
            DesktopAction::SetLayout(m) => write!(f, "layout:{}", layout_slug(*m)),
            DesktopAction::GrowMaster => f.write_str("grow-master"),
            DesktopAction::ShrinkMaster => f.write_str("shrink-master"),
            DesktopAction::IncMaster => f.write_str("inc-master"),
            DesktopAction::DecMaster => f.write_str("dec-master"),
            DesktopAction::PromoteToMaster => f.write_str("promote-to-master"),
            // Los escritorios se numeran 1-based de cara al usuario.
            DesktopAction::SwitchWorkspace(n) => write!(f, "workspace:{}", n + 1),
            DesktopAction::SendToWorkspace(n) => write!(f, "send-to-workspace:{}", n + 1),
            DesktopAction::FocusOutputNext => f.write_str("focus-output-next"),
            DesktopAction::Quit => f.write_str("quit"),
        }
    }
}

impl FromStr for DesktopAction {
    /// Mensaje de error ya formateado, listo para mostrar al usuario.
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        Ok(match s {
            "focus-next" => Self::FocusNext,
            "focus-prev" => Self::FocusPrev,
            "move-forward" => Self::MoveForward,
            "move-backward" => Self::MoveBackward,
            "close-focused" => Self::CloseFocused,
            "toggle-float" => Self::ToggleFloat,
            "toggle-fullscreen" => Self::ToggleFullscreen,
            "cycle-layout" => Self::CycleLayout,
            "grow-master" => Self::GrowMaster,
            "shrink-master" => Self::ShrinkMaster,
            "inc-master" => Self::IncMaster,
            "dec-master" => Self::DecMaster,
            "promote-to-master" => Self::PromoteToMaster,
            "focus-output-next" => Self::FocusOutputNext,
            "quit" => Self::Quit,
            _ => {
                if let Some(slug) = s.strip_prefix("layout:") {
                    Self::SetLayout(
                        layout_from_slug(slug)
                            .ok_or_else(|| format!("modo de teselado desconocido: '{slug}'"))?,
                    )
                } else if let Some(id) = s.strip_prefix("focus-window:") {
                    Self::FocusWindow(
                        id.trim()
                            .parse()
                            .map_err(|_| format!("id de ventana inválido: '{id}'"))?,
                    )
                } else if let Some(n) = s.strip_prefix("send-to-workspace:") {
                    Self::SendToWorkspace(parse_workspace(n)?)
                } else if let Some(n) = s.strip_prefix("workspace:") {
                    Self::SwitchWorkspace(parse_workspace(n)?)
                } else {
                    return Err(format!("acción desconocida: '{s}'"));
                }
            }
        })
    }
}

/// Parsea el número de escritorio del keymap (1-based) a índice (0-based),
/// acotado a [`WORKSPACE_COUNT`].
fn parse_workspace(s: &str) -> Result<usize, String> {
    let n: usize = s
        .trim()
        .parse()
        .map_err(|_| format!("número de escritorio inválido: '{s}'"))?;
    if (1..=WORKSPACE_COUNT).contains(&n) {
        Ok(n - 1)
    } else {
        Err(format!("escritorio fuera de rango (1..={WORKSPACE_COUNT}): {n}"))
    }
}

/// Mapa de teclas por defecto, estilo *tiling WM* (modificador `Super`).
///
/// Las cadenas deben coincidir literalmente con las que el Cuerpo emite
/// en [`BodyEvent::Keybind`](mirada_protocol::BodyEvent::Keybind); son
/// también las que se registran con
/// [`BrainCommand::GrabKeys`](mirada_protocol::BrainCommand::GrabKeys).
pub fn default_keymap() -> Vec<(String, DesktopAction)> {
    let mut map = vec![
        ("Super+j".into(), DesktopAction::FocusNext),
        ("Super+k".into(), DesktopAction::FocusPrev),
        ("Super+Shift+j".into(), DesktopAction::MoveForward),
        ("Super+Shift+k".into(), DesktopAction::MoveBackward),
        ("Super+q".into(), DesktopAction::CloseFocused),
        ("Super+f".into(), DesktopAction::ToggleFloat),
        ("Super+Shift+f".into(), DesktopAction::ToggleFullscreen),
        ("Super+space".into(), DesktopAction::CycleLayout),
        ("Super+t".into(), DesktopAction::SetLayout(LayoutMode::MasterStack)),
        ("Super+m".into(), DesktopAction::SetLayout(LayoutMode::Monocle)),
        ("Super+g".into(), DesktopAction::SetLayout(LayoutMode::Grid)),
        ("Super+c".into(), DesktopAction::SetLayout(LayoutMode::Columns)),
        ("Super+r".into(), DesktopAction::SetLayout(LayoutMode::Rows)),
        ("Super+d".into(), DesktopAction::SetLayout(LayoutMode::CenteredMaster)),
        ("Super+s".into(), DesktopAction::SetLayout(LayoutMode::Spiral)),
        ("Super+h".into(), DesktopAction::ShrinkMaster),
        ("Super+l".into(), DesktopAction::GrowMaster),
        ("Super+o".into(), DesktopAction::FocusOutputNext),
        ("Super+Return".into(), DesktopAction::PromoteToMaster),
        ("Super+,".into(), DesktopAction::IncMaster),
        ("Super+.".into(), DesktopAction::DecMaster),
        ("Super+Shift+e".into(), DesktopAction::Quit),
    ];
    // Un escritorio por dígito: `Super+1`..`Super+9` lo activan,
    // `Super+Shift+1`.. mandan la ventana enfocada allí.
    for n in 0..WORKSPACE_COUNT {
        map.push((format!("Super+{}", n + 1), DesktopAction::SwitchWorkspace(n)));
        map.push((
            format!("Super+Shift+{}", n + 1),
            DesktopAction::SendToWorkspace(n),
        ));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_has_no_duplicate_bindings() {
        let map = default_keymap();
        let mut keys: Vec<_> = map.iter().map(|(k, _)| k.clone()).collect();
        keys.sort();
        let unique = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), unique, "hay un atajo repetido");
    }

    #[test]
    fn keymap_covers_every_virtual_workspace() {
        let map = default_keymap();
        for n in 0..WORKSPACE_COUNT {
            assert!(map
                .iter()
                .any(|(_, a)| *a == DesktopAction::SwitchWorkspace(n)));
            assert!(map
                .iter()
                .any(|(_, a)| *a == DesktopAction::SendToWorkspace(n)));
        }
    }

    #[test]
    fn every_default_action_round_trips_through_its_text_form() {
        for (_, action) in default_keymap() {
            let text = action.to_string();
            let back: DesktopAction = text.parse().unwrap();
            assert_eq!(action, back, "no redondea: {text}");
        }
    }

    #[test]
    fn every_layout_mode_round_trips() {
        for mode in LayoutMode::ALL {
            let a = DesktopAction::SetLayout(mode);
            assert_eq!(a, a.to_string().parse().unwrap());
        }
    }

    #[test]
    fn workspace_actions_are_one_based_in_text() {
        assert_eq!(DesktopAction::SwitchWorkspace(0).to_string(), "workspace:1");
        assert_eq!(
            "workspace:1".parse::<DesktopAction>().unwrap(),
            DesktopAction::SwitchWorkspace(0)
        );
        assert_eq!(
            "send-to-workspace:9".parse::<DesktopAction>().unwrap(),
            DesktopAction::SendToWorkspace(8)
        );
    }

    #[test]
    fn out_of_range_or_unknown_actions_are_rejected() {
        assert!("workspace:0".parse::<DesktopAction>().is_err());
        assert!("workspace:99".parse::<DesktopAction>().is_err());
        assert!("layout:fractal".parse::<DesktopAction>().is_err());
        assert!("focus-window:abc".parse::<DesktopAction>().is_err());
        assert!("teleport".parse::<DesktopAction>().is_err());
    }

    #[test]
    fn focus_window_round_trips_with_its_id() {
        let a = DesktopAction::FocusWindow(42);
        assert_eq!(a.to_string(), "focus-window:42");
        assert_eq!("focus-window:42".parse::<DesktopAction>().unwrap(), a);
    }
}
