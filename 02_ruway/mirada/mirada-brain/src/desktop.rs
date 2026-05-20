//! El [`Desktop`] — el estado del escritorio y el bucle `evento → comandos`.

use std::collections::HashMap;

use mirada_layout::{LayoutMode, LayoutParams, Rect, WindowId, Workspace};
use mirada_protocol::{placements, BodyEvent, BrainCommand, OutputId};

use crate::action::{default_keymap, DesktopAction, WORKSPACE_COUNT};

/// Lo que el Cerebro sabe de una ventana: su identidad de aplicación.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
}

/// El estado completo del escritorio.
///
/// Mantiene las salidas físicas, [`WORKSPACE_COUNT`] escritorios
/// virtuales, el registro de ventanas y el mapa de atajos. El único
/// punto de entrada es [`Desktop::on_event`]: traga un [`BodyEvent`],
/// muta el estado y devuelve los [`BrainCommand`]s a enviar al Cuerpo.
///
/// Limitación de v1: el teselado se calcula sobre la salida primaria
/// (la primera conectada). El multi-monitor real llegará después.
pub struct Desktop {
    /// Salidas físicas, en fila horizontal y en orden de aparición.
    outputs: Vec<(OutputId, Rect)>,
    /// Escritorios virtuales — `WORKSPACE_COUNT` fijos.
    workspaces: Vec<Workspace>,
    /// Índice del escritorio activo.
    active: usize,
    /// Identidad de cada ventana conocida.
    windows: HashMap<WindowId, WindowInfo>,
    /// Atajos globales → acción.
    keymap: Vec<(String, DesktopAction)>,
}

impl Default for Desktop {
    fn default() -> Self {
        Self::new()
    }
}

impl Desktop {
    /// Escritorio recién arrancado: sin salidas ni ventanas, con los
    /// escritorios virtuales vacíos y el mapa de teclas por defecto.
    pub fn new() -> Self {
        let workspaces = (0..WORKSPACE_COUNT)
            .map(|_| Workspace::new(LayoutParams::default()))
            .collect();
        Self {
            outputs: Vec::new(),
            workspaces,
            active: 0,
            windows: HashMap::new(),
            keymap: default_keymap(),
        }
    }

    /// El comando que registra los atajos globales en el Cuerpo. La app
    /// GPUI lo envía una vez, al conectar.
    pub fn grab_keys(&self) -> BrainCommand {
        BrainCommand::GrabKeys(self.keymap.iter().map(|(k, _)| k.clone()).collect())
    }

    /// Geometría de la salida primaria, si hay alguna conectada.
    pub fn screen(&self) -> Option<Rect> {
        self.outputs.first().map(|(_, r)| *r)
    }

    /// Procesa un evento del Cuerpo: muta el estado y devuelve los
    /// comandos a enviar de vuelta.
    pub fn on_event(&mut self, event: BodyEvent) -> Vec<BrainCommand> {
        match event {
            BodyEvent::OutputAdded { id, width, height } => {
                // Las salidas se alinean en fila a la derecha de las previas.
                let x: i32 = self.outputs.iter().map(|(_, r)| r.w).sum();
                self.outputs.push((id, Rect::new(x, 0, width, height)));
                self.relayout()
            }
            BodyEvent::OutputRemoved { id } => {
                self.outputs.retain(|(o, _)| *o != id);
                self.relayout()
            }
            BodyEvent::WindowOpened { id, app_id, title } => {
                self.windows.insert(id, WindowInfo { app_id, title });
                self.workspaces[self.active].add(id);
                self.relayout()
            }
            BodyEvent::WindowClosed { id } => {
                self.windows.remove(&id);
                for ws in &mut self.workspaces {
                    ws.remove(id);
                }
                self.relayout()
            }
            BodyEvent::WindowRetitled { id, title } => {
                if let Some(info) = self.windows.get_mut(&id) {
                    info.title = title;
                }
                // Un cambio de título no altera la geometría.
                Vec::new()
            }
            BodyEvent::PointerEntered { id } => {
                // Foco al pasar el puntero, sólo si la ventana está en el
                // escritorio activo.
                if self.workspaces[self.active].focus_window(id) {
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            BodyEvent::Keybind(key) => {
                match self.keymap.iter().find(|(k, _)| *k == key).map(|(_, a)| *a) {
                    Some(action) => self.apply(action),
                    None => Vec::new(),
                }
            }
        }
    }

    /// Aplica una acción de escritorio directamente (sin pasar por una
    /// tecla). Útil para disparar acciones desde un HUD.
    pub fn apply(&mut self, action: DesktopAction) -> Vec<BrainCommand> {
        match action {
            DesktopAction::FocusNext => {
                self.workspaces[self.active].focus_next();
                self.relayout()
            }
            DesktopAction::FocusPrev => {
                self.workspaces[self.active].focus_prev();
                self.relayout()
            }
            DesktopAction::MoveForward => {
                self.workspaces[self.active].move_focused_forward();
                self.relayout()
            }
            DesktopAction::MoveBackward => {
                self.workspaces[self.active].move_focused_backward();
                self.relayout()
            }
            DesktopAction::CloseFocused => {
                // Pedimos el cierre; el estado se actualiza al recibir el
                // `WindowClosed` de vuelta, no antes.
                match self.workspaces[self.active].focused() {
                    Some(id) => vec![BrainCommand::Close(id)],
                    None => Vec::new(),
                }
            }
            DesktopAction::CycleLayout => {
                let next = cycle_mode(self.workspaces[self.active].params().mode);
                self.workspaces[self.active].set_mode(next);
                self.relayout()
            }
            DesktopAction::SetLayout(mode) => {
                self.workspaces[self.active].set_mode(mode);
                self.relayout()
            }
            DesktopAction::SwitchWorkspace(n) => {
                if n < self.workspaces.len() && n != self.active {
                    self.active = n;
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            DesktopAction::SendToWorkspace(n) => {
                if n >= self.workspaces.len() || n == self.active {
                    return Vec::new();
                }
                match self.workspaces[self.active].focused() {
                    Some(id) => {
                        self.workspaces[self.active].remove(id);
                        self.workspaces[n].add(id);
                        self.relayout()
                    }
                    None => Vec::new(),
                }
            }
            DesktopAction::Quit => vec![BrainCommand::Shutdown],
        }
    }

    /// Recalcula la geometría del escritorio activo y la empaqueta en un
    /// [`BrainCommand::Place`]. Sin salida conectada, no hay nada que
    /// colocar.
    fn relayout(&self) -> Vec<BrainCommand> {
        match self.screen() {
            Some(screen) => {
                vec![BrainCommand::Place(placements(
                    &self.workspaces[self.active],
                    screen,
                ))]
            }
            None => Vec::new(),
        }
    }

    // --- Accesores de sólo lectura, para el HUD de la app GPUI ---------

    /// Índice del escritorio activo.
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// El escritorio activo.
    pub fn active_workspace(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    /// Las salidas conectadas, en orden.
    pub fn outputs(&self) -> &[(OutputId, Rect)] {
        &self.outputs
    }

    /// Identidad de una ventana conocida.
    pub fn window_info(&self, id: WindowId) -> Option<&WindowInfo> {
        self.windows.get(&id)
    }

    /// La ventana enfocada en el escritorio activo.
    pub fn focused_window(&self) -> Option<WindowId> {
        self.workspaces[self.active].focused()
    }

    /// Cuántas ventanas hay en cada escritorio virtual.
    pub fn workspace_loads(&self) -> Vec<usize> {
        self.workspaces.iter().map(Workspace::len).collect()
    }
}

/// El siguiente modo en el ciclo de [`DesktopAction::CycleLayout`].
fn cycle_mode(mode: LayoutMode) -> LayoutMode {
    match mode {
        LayoutMode::MasterStack => LayoutMode::Monocle,
        LayoutMode::Monocle => LayoutMode::Grid,
        LayoutMode::Grid => LayoutMode::Columns,
        LayoutMode::Columns => LayoutMode::MasterStack,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un escritorio con una salida 1920×1080 ya conectada.
    fn desktop_with_screen() -> Desktop {
        let mut d = Desktop::new();
        d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
        d
    }

    fn open(d: &mut Desktop, id: WindowId) -> Vec<BrainCommand> {
        d.on_event(BodyEvent::WindowOpened {
            id,
            app_id: format!("app{id}"),
            title: format!("win {id}"),
        })
    }

    /// Extrae las colocaciones de un único `Place`.
    fn places(cmds: &[BrainCommand]) -> &[mirada_protocol::WindowPlacement] {
        match cmds {
            [BrainCommand::Place(p)] => p,
            other => panic!("se esperaba un solo Place, no {other:?}"),
        }
    }

    #[test]
    fn grab_keys_lists_the_whole_keymap() {
        let d = Desktop::new();
        match d.grab_keys() {
            BrainCommand::GrabKeys(keys) => {
                assert!(keys.contains(&"Super+j".to_string()));
                assert!(keys.contains(&"Super+Shift+e".to_string()));
            }
            other => panic!("se esperaba GrabKeys, no {other:?}"),
        }
    }

    #[test]
    fn without_a_screen_nothing_is_placed() {
        let mut d = Desktop::new();
        assert!(open(&mut d, 1).is_empty());
    }

    #[test]
    fn opening_a_window_places_it() {
        let mut d = desktop_with_screen();
        let cmds = open(&mut d, 1);
        assert_eq!(places(&cmds).len(), 1);
        assert_eq!(d.focused_window(), Some(1));
    }

    #[test]
    fn closing_a_window_removes_it_everywhere() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2);
        let cmds = d.on_event(BodyEvent::WindowClosed { id: 1 });
        assert_eq!(places(&cmds).len(), 1);
        assert!(d.window_info(1).is_none());
        assert_eq!(d.focused_window(), Some(2));
    }

    #[test]
    fn focus_keybind_cycles_within_the_active_workspace() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        assert_eq!(d.focused_window(), Some(3));
        d.on_event(BodyEvent::Keybind("Super+j".into())); // next, da la vuelta
        assert_eq!(d.focused_window(), Some(1));
        d.on_event(BodyEvent::Keybind("Super+k".into())); // prev
        assert_eq!(d.focused_window(), Some(3));
    }

    #[test]
    fn close_focused_keybind_asks_to_close_the_focused_window() {
        let mut d = desktop_with_screen();
        open(&mut d, 7);
        let cmds = d.on_event(BodyEvent::Keybind("Super+q".into()));
        assert_eq!(cmds, vec![BrainCommand::Close(7)]);
        // No se elimina hasta que el Cuerpo confirme con WindowClosed.
        assert!(d.window_info(7).is_some());
    }

    #[test]
    fn cycle_layout_walks_the_four_modes() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        assert_eq!(d.active_workspace().params().mode, LayoutMode::MasterStack);
        for expected in [
            LayoutMode::Monocle,
            LayoutMode::Grid,
            LayoutMode::Columns,
            LayoutMode::MasterStack,
        ] {
            d.on_event(BodyEvent::Keybind("Super+space".into()));
            assert_eq!(d.active_workspace().params().mode, expected);
        }
    }

    #[test]
    fn monocle_keybind_hides_all_but_the_focused_window() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        let cmds = d.on_event(BodyEvent::Keybind("Super+m".into()));
        let visible = places(&cmds).iter().filter(|p| p.visible).count();
        assert_eq!(visible, 1);
    }

    #[test]
    fn switching_workspace_changes_what_is_placed() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2);
        // Escritorio 2 (índice 1) está vacío.
        let cmds = d.on_event(BodyEvent::Keybind("Super+2".into()));
        assert!(places(&cmds).is_empty());
        assert_eq!(d.active_index(), 1);
        // Volver al 1 reaparece las dos ventanas.
        let cmds = d.on_event(BodyEvent::Keybind("Super+1".into()));
        assert_eq!(places(&cmds).len(), 2);
    }

    #[test]
    fn send_to_workspace_moves_the_focused_window() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2); // enfocada
        d.on_event(BodyEvent::Keybind("Super+Shift+3".into()));
        assert_eq!(d.workspace_loads()[0], 1); // sólo queda la 1
        assert_eq!(d.workspace_loads()[2], 1); // la 2 viajó al escritorio 3
        // La ventana 2 sigue registrada — sólo cambió de escritorio.
        assert!(d.window_info(2).is_some());
    }

    #[test]
    fn pointer_focuses_a_window_in_the_active_workspace() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2); // enfocada
        d.on_event(BodyEvent::PointerEntered { id: 1 });
        assert_eq!(d.focused_window(), Some(1));
    }

    #[test]
    fn retitling_updates_the_registry_without_relayout() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        let cmds = d.on_event(BodyEvent::WindowRetitled {
            id: 1,
            title: "nuevo".into(),
        });
        assert!(cmds.is_empty());
        assert_eq!(d.window_info(1).unwrap().title, "nuevo");
    }

    #[test]
    fn an_unknown_keybind_does_nothing() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        assert!(d.on_event(BodyEvent::Keybind("Super+F12".into())).is_empty());
    }

    #[test]
    fn quit_emits_a_shutdown() {
        let mut d = desktop_with_screen();
        assert_eq!(
            d.on_event(BodyEvent::Keybind("Super+Shift+e".into())),
            vec![BrainCommand::Shutdown]
        );
    }

    #[test]
    fn outputs_lay_side_by_side() {
        let mut d = Desktop::new();
        d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
        d.on_event(BodyEvent::OutputAdded { id: 1, width: 2560, height: 1440 });
        assert_eq!(d.outputs().len(), 2);
        // La segunda salida arranca donde acaba la primera.
        assert_eq!(d.outputs()[1].1.x, 1920);
        // El teselado sigue sobre la salida primaria.
        assert_eq!(d.screen().unwrap().w, 1920);
    }
}
