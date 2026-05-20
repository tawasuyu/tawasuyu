//! Acciones de escritorio y su mapa de teclas por defecto.
//!
//! Una [`DesktopAction`] es una orden de alto nivel del usuario, ya
//! desligada de la tecla concreta: el [`Desktop`](crate::Desktop) las
//! aplica sin saber qué combinación las disparó.

use mirada_layout::LayoutMode;

/// Número de escritorios virtuales que mantiene el `Desktop`.
pub const WORKSPACE_COUNT: usize = 9;

/// Una orden de escritorio de alto nivel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopAction {
    /// Mueve el foco a la ventana siguiente del escritorio activo.
    FocusNext,
    /// Mueve el foco a la ventana anterior.
    FocusPrev,
    /// Adelanta la ventana enfocada en el orden de teselado.
    MoveForward,
    /// Atrasa la ventana enfocada en el orden de teselado.
    MoveBackward,
    /// Cierra la ventana enfocada (cierre ordenado).
    CloseFocused,
    /// Pasa al siguiente modo de teselado.
    CycleLayout,
    /// Fija un modo de teselado concreto.
    SetLayout(LayoutMode),
    /// Activa el escritorio virtual `n` (índice 0-based).
    SwitchWorkspace(usize),
    /// Manda la ventana enfocada al escritorio virtual `n`.
    SendToWorkspace(usize),
    /// Apaga el compositor.
    Quit,
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
        ("Super+space".into(), DesktopAction::CycleLayout),
        ("Super+t".into(), DesktopAction::SetLayout(LayoutMode::MasterStack)),
        ("Super+m".into(), DesktopAction::SetLayout(LayoutMode::Monocle)),
        ("Super+g".into(), DesktopAction::SetLayout(LayoutMode::Grid)),
        ("Super+c".into(), DesktopAction::SetLayout(LayoutMode::Columns)),
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
}
