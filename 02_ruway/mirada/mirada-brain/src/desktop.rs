//! El [`Desktop`] — el estado del escritorio y el bucle `evento → comandos`.

use std::collections::HashMap;

use mirada_layout::{LayoutParams, Rect, WindowId, Workspace};
use mirada_protocol::{placements, BodyEvent, BrainCommand, OutputId};

use crate::action::{DesktopAction, WORKSPACE_COUNT};
use crate::keymap::Keymap;
use crate::rules::Rules;

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
    /// Atajos globales → acción. Configurable, recargable en caliente.
    keymap: Keymap,
    /// Reglas de ventana — escritorio/flotante por `app_id`/título.
    rules: Rules,
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
        Self::with_keymap(Keymap::default())
    }

    /// Como [`Desktop::new`], pero con un keymap dado — el que la app
    /// cargó del archivo de configuración del usuario.
    pub fn with_keymap(keymap: Keymap) -> Self {
        let workspaces = (0..WORKSPACE_COUNT)
            .map(|_| Workspace::new(LayoutParams::default()))
            .collect();
        Self {
            outputs: Vec::new(),
            workspaces,
            active: 0,
            windows: HashMap::new(),
            keymap,
            rules: Rules::default(),
        }
    }

    /// Reemplaza las reglas de ventana. Se aplican a las ventanas que se
    /// abran a partir de ahora; las ya abiertas no se tocan.
    pub fn set_rules(&mut self, rules: Rules) {
        self.rules = rules;
    }

    /// El comando que registra los atajos globales en el Cuerpo. La app
    /// lo envía al conectar, y de nuevo tras cada recarga del keymap.
    pub fn grab_keys(&self) -> BrainCommand {
        BrainCommand::GrabKeys(self.keymap.grab_list())
    }

    /// Reemplaza el keymap en caliente. Devuelve el [`BrainCommand`] que
    /// el dueño debe enviar al Cuerpo para reajustar qué teclas intercepta.
    pub fn set_keymap(&mut self, keymap: Keymap) -> BrainCommand {
        self.keymap = keymap;
        self.grab_keys()
    }

    /// El keymap vigente — para un HUD o un editor visual de atajos.
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
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
                // Las reglas pueden mandarla a otro escritorio o hacerla flotar.
                let outcome = self.rules.resolve(&app_id, &title);
                self.windows.insert(id, WindowInfo { app_id, title });
                let ws = outcome
                    .workspace
                    .filter(|&n| n < self.workspaces.len())
                    .unwrap_or(self.active);
                self.workspaces[ws].add(id);
                if outcome.floating {
                    let rect = self
                        .screen()
                        .map(centered_float_rect)
                        .unwrap_or_else(|| Rect::new(100, 100, 800, 600));
                    self.workspaces[ws].set_floating(id, Some(rect));
                }
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
            BodyEvent::Keybind(key) => match self.keymap.lookup(&key) {
                Some(action) => self.apply(action),
                None => Vec::new(),
            },
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
            DesktopAction::FocusWindow(id) => {
                // En el escritorio activo basta enfocar; si la ventana
                // está en otro, saltamos a ese escritorio.
                if self.workspaces[self.active].focus_window(id) {
                    return self.relayout();
                }
                for n in 0..self.workspaces.len() {
                    if n != self.active && self.workspaces[n].focus_window(id) {
                        self.active = n;
                        return self.relayout();
                    }
                }
                Vec::new()
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
            DesktopAction::ToggleFloat => {
                let Some(id) = self.workspaces[self.active].focused() else {
                    return Vec::new();
                };
                let screen = self.screen();
                let ws = &mut self.workspaces[self.active];
                if ws.is_floating(id) {
                    ws.set_floating(id, None);
                } else {
                    let rect = screen
                        .map(centered_float_rect)
                        .unwrap_or_else(|| Rect::new(100, 100, 800, 600));
                    ws.set_floating(id, Some(rect));
                }
                self.relayout()
            }
            DesktopAction::CycleLayout => {
                let next = self.workspaces[self.active].params().mode.next();
                self.workspaces[self.active].set_mode(next);
                self.relayout()
            }
            DesktopAction::SetLayout(mode) => {
                self.workspaces[self.active].set_mode(mode);
                self.relayout()
            }
            DesktopAction::GrowMaster => self.nudge_master(0.05),
            DesktopAction::ShrinkMaster => self.nudge_master(-0.05),
            DesktopAction::IncMaster => self.nudge_master_count(1),
            DesktopAction::DecMaster => self.nudge_master_count(-1),
            DesktopAction::PromoteToMaster => {
                self.workspaces[self.active].promote_focused();
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

    /// Ajusta la fracción del área maestra del escritorio activo (la usan
    /// `MasterStack` y `CenteredMaster`), acotada a `0.05..=0.95`.
    fn nudge_master(&mut self, delta: f32) -> Vec<BrainCommand> {
        let ws = &mut self.workspaces[self.active];
        let ratio = (ws.params().master_ratio + delta).clamp(0.05, 0.95);
        ws.set_master_ratio(ratio);
        self.relayout()
    }

    /// Ajusta `nmaster` del escritorio activo, acotado a `1..=9`.
    fn nudge_master_count(&mut self, delta: i32) -> Vec<BrainCommand> {
        let ws = &mut self.workspaces[self.active];
        let n = (ws.params().master_count as i32 + delta).clamp(1, 9) as usize;
        ws.set_master_count(n);
        self.relayout()
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

    /// Una vista de todas las ventanas conocidas, en todos los
    /// escritorios — la base de `mirada-ctl windows` y de una taskbar.
    pub fn window_lines(&self) -> Vec<crate::ctl::WindowLine> {
        let mut lines = Vec::new();
        for (n, ws) in self.workspaces.iter().enumerate() {
            let ws_focus = ws.focused();
            for &id in ws.windows() {
                let info = self.windows.get(&id);
                lines.push(crate::ctl::WindowLine {
                    id,
                    app_id: info.map(|i| i.app_id.clone()).unwrap_or_default(),
                    title: info.map(|i| i.title.clone()).unwrap_or_default(),
                    workspace: n + 1,
                    focused: n == self.active && ws_focus == Some(id),
                });
            }
        }
        lines
    }
}

/// El rectángulo flotante por defecto: 60 % de la pantalla, centrado.
fn centered_float_rect(screen: Rect) -> Rect {
    let w = screen.w * 3 / 5;
    let h = screen.h * 3 / 5;
    Rect::new(
        screen.x + (screen.w - w) / 2,
        screen.y + (screen.h - h) / 2,
        w,
        h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirada_layout::LayoutMode;

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
    fn set_keymap_swaps_the_bindings_and_regrabs() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        // El keymap por defecto no usa Alt.
        assert!(d.on_event(BodyEvent::Keybind("Alt+x".into())).is_empty());
        // Cargamos un keymap a medida; el comando devuelto re-registra grabs.
        let custom = crate::Keymap::from_ron(r#"( bindings: { "Alt+x": "focus-prev" } )"#).unwrap();
        match d.set_keymap(custom) {
            BrainCommand::GrabKeys(keys) => assert_eq!(keys, vec!["Alt+x".to_string()]),
            other => panic!("se esperaba GrabKeys, no {other:?}"),
        }
        // Ahora «Alt+x» sí mueve el foco, y «Super+j» ya no.
        assert_eq!(d.focused_window(), Some(3));
        d.on_event(BodyEvent::Keybind("Alt+x".into()));
        assert_eq!(d.focused_window(), Some(2));
        assert!(d.on_event(BodyEvent::Keybind("Super+j".into())).is_empty());
    }

    #[test]
    fn focus_window_addresses_a_specific_window() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        assert_eq!(d.focused_window(), Some(3));
        d.apply(DesktopAction::FocusWindow(1));
        assert_eq!(d.focused_window(), Some(1));
    }

    #[test]
    fn focus_window_jumps_to_the_workspace_that_holds_it() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2); // enfocada
        // Manda la 2 al escritorio 3; seguimos en el 1.
        d.on_event(BodyEvent::Keybind("Super+Shift+3".into()));
        assert_eq!(d.active_index(), 0);
        // Enfocar la 2 nos lleva a su escritorio.
        d.apply(DesktopAction::FocusWindow(2));
        assert_eq!(d.active_index(), 2);
        assert_eq!(d.focused_window(), Some(2));
    }

    #[test]
    fn window_lines_cover_every_window_with_its_workspace() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2);
        d.on_event(BodyEvent::Keybind("Super+Shift+3".into())); // la 2 al esc. 3
        let lines = d.window_lines();
        assert_eq!(lines.len(), 2);
        let w1 = lines.iter().find(|l| l.id == 1).unwrap();
        let w2 = lines.iter().find(|l| l.id == 2).unwrap();
        assert_eq!(w1.workspace, 1);
        assert_eq!(w2.workspace, 3);
        // La 1 quedó enfocada en el escritorio activo (el 1).
        assert!(w1.focused);
        assert!(!w2.focused);
    }

    #[test]
    fn toggle_float_marks_the_focused_window_and_floats_it_last() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2); // enfocada
        let cmds = d.apply(DesktopAction::ToggleFloat);
        let p = places(&cmds);
        assert!(p.iter().find(|x| x.id == 2).unwrap().floating);
        // La flotante va al final de la lista — orden de pintado.
        assert_eq!(p.last().unwrap().id, 2);
        // Alternar de nuevo la devuelve al teselado.
        let cmds = d.apply(DesktopAction::ToggleFloat);
        assert!(!places(&cmds).iter().find(|x| x.id == 2).unwrap().floating);
    }

    #[test]
    fn a_rule_sends_a_new_window_to_its_workspace() {
        let mut d = desktop_with_screen();
        d.set_rules(Rules::from_ron(r#"( rules: [ (app_id: "app2", workspace: 3) ] )"#).unwrap());
        open(&mut d, 1); // app1 → sin regla → escritorio activo (1)
        open(&mut d, 2); // app2 → regla → escritorio 3
        assert_eq!(d.workspace_loads()[0], 1);
        assert_eq!(d.workspace_loads()[2], 1);
    }

    #[test]
    fn a_rule_can_open_a_window_floating() {
        let mut d = desktop_with_screen();
        d.set_rules(Rules::from_ron(r#"( rules: [ (app_id: "app1", floating: true) ] )"#).unwrap());
        let cmds = open(&mut d, 1);
        assert!(places(&cmds).iter().find(|p| p.id == 1).unwrap().floating);
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
    fn cycle_layout_walks_every_mode_and_returns() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        let start = d.active_workspace().params().mode;
        for _ in 0..LayoutMode::ALL.len() {
            let before = d.active_workspace().params().mode;
            d.on_event(BodyEvent::Keybind("Super+space".into()));
            assert_eq!(d.active_workspace().params().mode, before.next());
        }
        // Una vuelta completa devuelve al modo inicial.
        assert_eq!(d.active_workspace().params().mode, start);
    }

    #[test]
    fn grow_and_shrink_master_adjust_the_ratio() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        let r0 = d.active_workspace().params().master_ratio;
        d.apply(DesktopAction::GrowMaster);
        assert!(d.active_workspace().params().master_ratio > r0);
        d.apply(DesktopAction::ShrinkMaster);
        assert!((d.active_workspace().params().master_ratio - r0).abs() < 1e-6);
    }

    #[test]
    fn inc_and_dec_master_adjust_nmaster() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        assert_eq!(d.active_workspace().params().master_count, 1);
        d.apply(DesktopAction::IncMaster);
        assert_eq!(d.active_workspace().params().master_count, 2);
        d.apply(DesktopAction::DecMaster);
        d.apply(DesktopAction::DecMaster); // no baja de 1
        assert_eq!(d.active_workspace().params().master_count, 1);
    }

    #[test]
    fn promote_to_master_brings_the_focused_window_to_the_front() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        d.apply(DesktopAction::FocusWindow(3));
        d.apply(DesktopAction::PromoteToMaster);
        assert_eq!(d.active_workspace().windows()[0], 3);
        assert_eq!(d.focused_window(), Some(3));
    }

    #[test]
    fn master_ratio_stays_within_bounds() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        for _ in 0..50 {
            d.apply(DesktopAction::GrowMaster);
        }
        assert!(d.active_workspace().params().master_ratio <= 0.95);
        for _ in 0..50 {
            d.apply(DesktopAction::ShrinkMaster);
        }
        assert!(d.active_workspace().params().master_ratio >= 0.05);
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
