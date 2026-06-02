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

/// Una salida física y el escritorio virtual que muestra ahora mismo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    pub id: OutputId,
    /// Rectángulo en el espacio global — las salidas van en fila horizontal.
    pub rect: Rect,
    /// Zonas exclusivas reservadas por el marco (`pata`/shell), en px desde
    /// cada borde: `(top, bottom, left, right)`. El teselado las esquiva.
    pub reserved: (i32, i32, i32, i32),
    /// Índice del escritorio que esta salida muestra.
    pub workspace: usize,
}

impl Output {
    /// El área teselable: el rect global menos las zonas reservadas. Es lo que
    /// se le pasa al motor de layout, así que las barras de cualquier borde
    /// quedan libres de ventanas.
    pub fn work_rect(&self) -> Rect {
        let (top, bottom, left, right) = self.reserved;
        Rect::new(
            self.rect.x + left,
            self.rect.y + top,
            (self.rect.w - left - right).max(1),
            (self.rect.h - top - bottom).max(1),
        )
    }
}

/// El estado completo del escritorio.
///
/// Mantiene las salidas físicas, [`WORKSPACE_COUNT`] escritorios
/// virtuales, el registro de ventanas, el keymap y las reglas. El único
/// punto de entrada es [`Desktop::on_event`]: traga un [`BodyEvent`],
/// muta el estado y devuelve los [`BrainCommand`]s a enviar al Cuerpo.
///
/// **Multi-monitor**: cada salida muestra un escritorio distinto; el
/// teselado se calcula para todas y el `Place` resultante las cubre. Un
/// escritorio se ve en una salida como mucho — pedir uno que ya muestra
/// otra salida las intercambia.
pub struct Desktop {
    /// Salidas físicas, en fila horizontal y en orden de aparición.
    outputs: Vec<Output>,
    /// Escritorios virtuales — `WORKSPACE_COUNT` fijos.
    workspaces: Vec<Workspace>,
    /// Índice (en `outputs`) de la salida con el foco.
    focused_output: usize,
    /// Identidad de cada ventana conocida.
    windows: HashMap<WindowId, WindowInfo>,
    /// Atajos globales → acción. Configurable, recargable en caliente.
    keymap: Keymap,
    /// Reglas de ventana — escritorio/flotante por `app_id`/título.
    rules: Rules,
    /// Ventanas del scratchpad: se invocan flotando y se ocultan a
    /// voluntad; mientras están guardadas no viven en ningún escritorio.
    scratchpad: Vec<WindowId>,
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
            focused_output: 0,
            windows: HashMap::new(),
            keymap,
            rules: Rules::default(),
            scratchpad: Vec::new(),
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

    /// Geometría de la salida enfocada, si hay alguna conectada.
    pub fn screen(&self) -> Option<Rect> {
        self.outputs.get(self.focused_output).map(|o| o.rect)
    }

    /// Procesa un evento del Cuerpo: muta el estado y devuelve los
    /// comandos a enviar de vuelta.
    pub fn on_event(&mut self, event: BodyEvent) -> Vec<BrainCommand> {
        match event {
            BodyEvent::OutputAdded { id, width, height } => {
                // La salida nueva muestra el primer escritorio que no
                // muestre ya otra salida.
                let taken: Vec<usize> = self.outputs.iter().map(|o| o.workspace).collect();
                let workspace = (0..self.workspaces.len())
                    .find(|n| !taken.contains(n))
                    .unwrap_or(0);
                self.outputs.push(Output {
                    id,
                    rect: Rect::new(0, 0, width, height),
                    reserved: (0, 0, 0, 0),
                    workspace,
                });
                self.reflow_outputs();
                self.relayout()
            }
            BodyEvent::OutputRemoved { id } => {
                self.outputs.retain(|o| o.id != id);
                if self.focused_output >= self.outputs.len() {
                    self.focused_output = self.outputs.len().saturating_sub(1);
                }
                self.reflow_outputs();
                self.relayout()
            }
            BodyEvent::OutputResized { id, width, height } => {
                // Sólo cambia el área útil; el escritorio que muestra la
                // salida se conserva.
                if let Some(o) = self.outputs.iter_mut().find(|o| o.id == id) {
                    o.rect.w = width;
                    o.rect.h = height;
                    self.reflow_outputs();
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            BodyEvent::OutputReserved {
                id,
                top,
                bottom,
                left,
                right,
            } => {
                // El marco reservó/liberó franjas: cambia el área teselable
                // sin tocar el tamaño físico ni el escritorio que muestra.
                if let Some(o) = self.outputs.iter_mut().find(|o| o.id == id) {
                    o.reserved = (top.max(0), bottom.max(0), left.max(0), right.max(0));
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            BodyEvent::WindowOpened { id, app_id, title } => {
                // La terminal dropdown se reconoce por su `app_id`: nace
                // flotando anclada arriba y enfocada, lista para escribir.
                let is_dropterm = app_id == DROPTERM_APP_ID;
                // Las reglas pueden mandarla a otro escritorio o hacerla flotar.
                let outcome = self.rules.resolve(&app_id, &title);
                self.windows.insert(id, WindowInfo { app_id, title });
                let ws = outcome
                    .workspace
                    .filter(|&n| n < self.workspaces.len())
                    .unwrap_or(self.active_index());
                self.workspaces[ws].add(id);
                if is_dropterm {
                    let rect = self
                        .screen()
                        .map(dropdown_rect)
                        .unwrap_or_else(|| Rect::new(100, 100, 800, 600));
                    self.workspaces[ws].set_floating(id, Some(rect));
                } else if outcome.floating {
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
                self.scratchpad.retain(|&w| w != id);
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
                let active = self.active_index();
                if self.workspaces[active].focus_window(id) {
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            BodyEvent::Keybind(key) => match self.keymap.lookup(&key) {
                Some(action) => self.apply(action),
                None => Vec::new(),
            },
            BodyEvent::FullscreenRequest { id, fullscreen } => {
                // El cliente (un reproductor, un juego) pidió pantalla
                // completa: la fijamos en el escritorio que tiene la ventana.
                let mut changed = false;
                for ws in &mut self.workspaces {
                    if ws.windows().contains(&id) {
                        if fullscreen {
                            ws.set_fullscreen(Some(id));
                        } else if ws.fullscreen() == Some(id) {
                            ws.set_fullscreen(None);
                        }
                        changed = true;
                        break;
                    }
                }
                if changed {
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            BodyEvent::WindowFloatTo { id, rect } => {
                // Arrastre interactivo: la ventana pasa a flotar en el
                // rectángulo dado, en el escritorio donde viva.
                let mut changed = false;
                for ws in &mut self.workspaces {
                    if ws.windows().contains(&id) {
                        ws.set_floating(id, Some(rect));
                        changed = true;
                        break;
                    }
                }
                if changed {
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Aplica una acción de escritorio directamente (sin pasar por una
    /// tecla). Útil para disparar acciones desde un HUD.
    pub fn apply(&mut self, action: DesktopAction) -> Vec<BrainCommand> {
        let active = self.active_index();
        match action {
            DesktopAction::FocusNext => {
                self.workspaces[active].focus_next();
                self.relayout()
            }
            DesktopAction::FocusPrev => {
                self.workspaces[active].focus_prev();
                self.relayout()
            }
            DesktopAction::FocusWindow(id) => {
                // En el escritorio activo basta enfocar; si la ventana
                // está en otro, lo traemos a la salida enfocada.
                if self.workspaces[active].focus_window(id) {
                    return self.relayout();
                }
                for n in 0..self.workspaces.len() {
                    if n != active && self.workspaces[n].focus_window(id) {
                        self.show_workspace(n);
                        return self.relayout();
                    }
                }
                Vec::new()
            }
            DesktopAction::MoveForward => {
                self.workspaces[active].move_focused_forward();
                self.relayout()
            }
            DesktopAction::MoveBackward => {
                self.workspaces[active].move_focused_backward();
                self.relayout()
            }
            DesktopAction::CloseFocused => {
                // Pedimos el cierre; el estado se actualiza al recibir el
                // `WindowClosed` de vuelta, no antes.
                match self.workspaces[active].focused() {
                    Some(id) => vec![BrainCommand::Close(id)],
                    None => Vec::new(),
                }
            }
            DesktopAction::ToggleFloat => {
                let Some(id) = self.workspaces[active].focused() else {
                    return Vec::new();
                };
                let screen = self.screen();
                let ws = &mut self.workspaces[active];
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
            DesktopAction::ToggleFullscreen => {
                let Some(id) = self.workspaces[active].focused() else {
                    return Vec::new();
                };
                let ws = &mut self.workspaces[active];
                if ws.fullscreen() == Some(id) {
                    ws.set_fullscreen(None);
                } else {
                    ws.set_fullscreen(Some(id));
                }
                self.relayout()
            }
            DesktopAction::SendToScratchpad => {
                let Some(id) = self.workspaces[active].focused() else {
                    return Vec::new();
                };
                for ws in &mut self.workspaces {
                    ws.remove(id);
                }
                if !self.scratchpad.contains(&id) {
                    self.scratchpad.push(id);
                }
                self.relayout()
            }
            DesktopAction::ToggleScratchpad => {
                // ¿Hay alguna ventana del scratchpad en el escritorio activo?
                let shown: Vec<WindowId> = self.workspaces[active]
                    .windows()
                    .iter()
                    .copied()
                    .filter(|id| self.scratchpad.contains(id))
                    .collect();
                if !shown.is_empty() {
                    for id in shown {
                        self.workspaces[active].remove(id);
                    }
                    self.relayout()
                } else if let Some(&id) = self.scratchpad.first() {
                    // La traemos de donde esté y la mostramos flotando.
                    for ws in &mut self.workspaces {
                        ws.remove(id);
                    }
                    let rect = self
                        .screen()
                        .map(centered_float_rect)
                        .unwrap_or_else(|| Rect::new(100, 100, 800, 600));
                    self.workspaces[active].add(id);
                    self.workspaces[active].set_floating(id, Some(rect));
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            DesktopAction::ToggleDropterm => {
                // Buscamos la terminal dropdown por su marca de `app_id`.
                let existing = self
                    .windows
                    .iter()
                    .find(|(_, info)| info.app_id == DROPTERM_APP_ID)
                    .map(|(&id, _)| id);
                match existing {
                    // Ya existe: si está a la vista, la guardamos; si está
                    // guardada, la bajamos flotando y enfocada.
                    Some(id) => {
                        if self.workspaces[active].windows().contains(&id) {
                            for ws in &mut self.workspaces {
                                ws.remove(id);
                            }
                            if !self.scratchpad.contains(&id) {
                                self.scratchpad.push(id);
                            }
                        } else {
                            for ws in &mut self.workspaces {
                                ws.remove(id);
                            }
                            self.scratchpad.retain(|&w| w != id);
                            let rect = self
                                .screen()
                                .map(dropdown_rect)
                                .unwrap_or_else(|| Rect::new(100, 100, 800, 600));
                            self.workspaces[active].add(id);
                            self.workspaces[active].set_floating(id, Some(rect));
                        }
                        self.relayout()
                    }
                    // Aún no existe: la creamos perezosamente. Al abrirse,
                    // `WindowOpened` la reconoce y la baja flotando+enfocada.
                    None => vec![BrainCommand::Spawn(DROPTERM_CMD.into())],
                }
            }
            DesktopAction::CycleLayout => {
                let next = self.workspaces[active].params().mode.next();
                self.workspaces[active].set_mode(next);
                self.relayout()
            }
            DesktopAction::SetLayout(mode) => {
                self.workspaces[active].set_mode(mode);
                self.relayout()
            }
            DesktopAction::GrowMaster => self.nudge_master(0.05),
            DesktopAction::ShrinkMaster => self.nudge_master(-0.05),
            DesktopAction::IncMaster => self.nudge_master_count(1),
            DesktopAction::DecMaster => self.nudge_master_count(-1),
            DesktopAction::PromoteToMaster => {
                self.workspaces[active].promote_focused();
                self.relayout()
            }
            DesktopAction::SwitchWorkspace(n) => {
                if n < self.workspaces.len() && n != active {
                    self.show_workspace(n);
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            DesktopAction::SendToWorkspace(n) => {
                if n >= self.workspaces.len() || n == active {
                    return Vec::new();
                }
                match self.workspaces[active].focused() {
                    Some(id) => {
                        self.workspaces[active].remove(id);
                        self.workspaces[n].add(id);
                        self.relayout()
                    }
                    None => Vec::new(),
                }
            }
            DesktopAction::FocusOutputNext => {
                if self.outputs.len() > 1 {
                    self.focused_output = (self.focused_output + 1) % self.outputs.len();
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            DesktopAction::Spawn(cmd) => vec![BrainCommand::Spawn(cmd)],
            DesktopAction::Quit => vec![BrainCommand::Shutdown],
        }
    }

    /// El índice del escritorio activo — el que muestra la salida
    /// enfocada. `0` si todavía no hay ninguna salida.
    pub fn active_index(&self) -> usize {
        self.outputs
            .get(self.focused_output)
            .map(|o| o.workspace)
            .unwrap_or(0)
    }

    /// Hace que la salida enfocada muestre el escritorio `n`. Si otra
    /// salida ya lo mostraba, intercambian — así ningún escritorio se
    /// ve en dos sitios a la vez.
    fn show_workspace(&mut self, n: usize) {
        if n >= self.workspaces.len() || self.focused_output >= self.outputs.len() {
            return;
        }
        let current = self.outputs[self.focused_output].workspace;
        if current == n {
            return;
        }
        if let Some(other) = self.outputs.iter().position(|o| o.workspace == n) {
            self.outputs[other].workspace = current;
        }
        self.outputs[self.focused_output].workspace = n;
    }

    /// Recoloca las salidas en fila horizontal, en su orden de aparición.
    fn reflow_outputs(&mut self) {
        let mut x = 0;
        for o in &mut self.outputs {
            o.rect.x = x;
            o.rect.y = 0;
            x += o.rect.w;
        }
    }

    /// Ajusta la fracción del área maestra del escritorio activo (la usan
    /// `MasterStack` y `CenteredMaster`), acotada a `0.05..=0.95`.
    fn nudge_master(&mut self, delta: f32) -> Vec<BrainCommand> {
        let active = self.active_index();
        let ws = &mut self.workspaces[active];
        let ratio = (ws.params().master_ratio + delta).clamp(0.05, 0.95);
        ws.set_master_ratio(ratio);
        self.relayout()
    }

    /// Ajusta `nmaster` del escritorio activo, acotado a `1..=9`.
    fn nudge_master_count(&mut self, delta: i32) -> Vec<BrainCommand> {
        let active = self.active_index();
        let ws = &mut self.workspaces[active];
        let n = (ws.params().master_count as i32 + delta).clamp(1, 9) as usize;
        ws.set_master_count(n);
        self.relayout()
    }

    /// Recalcula la geometría de **todas** las salidas y la empaqueta en
    /// un único [`BrainCommand::Place`]. Sin salidas, no hay nada que
    /// colocar.
    fn relayout(&self) -> Vec<BrainCommand> {
        if self.outputs.is_empty() {
            return Vec::new();
        }
        let mut all = Vec::new();
        for o in &self.outputs {
            // El teselado usa el área útil (rect menos zonas reservadas), así las
            // barras del marco en cualquier borde quedan libres de ventanas.
            all.extend(placements(&self.workspaces[o.workspace], o.work_rect()));
        }
        // El foco del teclado es único: sólo la ventana enfocada de la
        // salida enfocada. `placements` marca el foco por escritorio (lo
        // necesita para la visibilidad en `Monocle`); aquí lo unificamos.
        let global_focus = self.focused_window();
        for p in &mut all {
            p.focused = Some(p.id) == global_focus;
        }
        vec![BrainCommand::Place(all)]
    }

    // --- Accesores de sólo lectura, para el HUD de la app GPUI ---------

    /// El escritorio activo — el de la salida enfocada.
    pub fn active_workspace(&self) -> &Workspace {
        &self.workspaces[self.active_index()]
    }

    /// Las salidas conectadas, en orden, con el escritorio que muestran.
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// Índice (en [`outputs`](Desktop::outputs)) de la salida enfocada.
    pub fn focused_output(&self) -> usize {
        self.focused_output
    }

    /// Identidad de una ventana conocida.
    pub fn window_info(&self, id: WindowId) -> Option<&WindowInfo> {
        self.windows.get(&id)
    }

    /// La ventana con el foco del teclado: la enfocada del escritorio
    /// activo — o su ventana en pantalla completa, si la hay.
    pub fn focused_window(&self) -> Option<WindowId> {
        let ws = &self.workspaces[self.active_index()];
        ws.fullscreen().or_else(|| ws.focused())
    }

    /// Cuántas ventanas hay en cada escritorio virtual.
    pub fn workspace_loads(&self) -> Vec<usize> {
        self.workspaces.iter().map(Workspace::len).collect()
    }

    /// Una vista de todas las ventanas conocidas, en todos los
    /// escritorios — la base de `mirada-ctl windows` y de una taskbar.
    pub fn window_lines(&self) -> Vec<crate::ctl::WindowLine> {
        let active = self.active_index();
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
                    focused: n == active && ws_focus == Some(id),
                });
            }
        }
        // Ventanas guardadas en el scratchpad — en ningún escritorio.
        for &id in &self.scratchpad {
            let stashed = !self.workspaces.iter().any(|ws| ws.windows().contains(&id));
            if stashed {
                let info = self.windows.get(&id);
                lines.push(crate::ctl::WindowLine {
                    id,
                    app_id: info.map(|i| i.app_id.clone()).unwrap_or_default(),
                    title: info.map(|i| i.title.clone()).unwrap_or_default(),
                    workspace: 0, // 0 = guardada en el scratchpad
                    focused: false,
                });
            }
        }
        lines
    }
}

/// `app_id` con el que se marca y reconoce la terminal dropdown (quake).
/// La crea `ToggleDropterm` con [`DROPTERM_CMD`] y se la reconoce al abrir.
pub const DROPTERM_APP_ID: &str = "mirada.dropterm";

/// Comando que lanza la terminal dropdown. `kitty --class` fija el `app_id`
/// en Wayland, que es como la reconocemos. Configurable en rondas futuras.
const DROPTERM_CMD: &str = "kitty --class mirada.dropterm";

/// El rectángulo de la terminal dropdown: anclada arriba, a todo el ancho,
/// 45 % del alto — el gesto «quake» de bajar desde el borde superior.
fn dropdown_rect(screen: Rect) -> Rect {
    Rect::new(screen.x, screen.y, screen.w, (screen.h * 45 / 100).max(1))
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
    fn toggle_fullscreen_covers_the_screen_and_hides_the_rest() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        let cmds = d.apply(DesktopAction::ToggleFullscreen); // sobre la 3
        let p = places(&cmds);
        let fs = p.iter().find(|x| x.id == 3).unwrap();
        assert!(fs.fullscreen && fs.visible);
        assert_eq!(fs.rect, d.screen().unwrap());
        assert!(p.iter().filter(|x| x.id != 3).all(|x| !x.visible));
        // Alternar de nuevo restaura el teselado: las tres visibles.
        let cmds = d.apply(DesktopAction::ToggleFullscreen);
        assert_eq!(places(&cmds).iter().filter(|x| x.visible).count(), 3);
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
    fn dragging_floats_a_window_at_the_given_rect() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2);
        assert!(!d.active_workspace().is_floating(2));
        let target = Rect::new(300, 200, 640, 480);
        let cmds = d.on_event(BodyEvent::WindowFloatTo { id: 2, rect: target });
        // La 2 ahora flota exactamente en el rectángulo pedido.
        assert!(d.active_workspace().is_floating(2));
        let p = places(&cmds).iter().find(|p| p.id == 2).unwrap();
        assert!(p.floating);
        assert_eq!(p.rect, target);
    }

    #[test]
    fn resizing_an_output_retiles_without_losing_the_workspace() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        d.on_event(BodyEvent::Keybind("Super+2".into())); // escritorio activo → 2
        assert_eq!(d.active_index(), 1);
        let cmds = d.on_event(BodyEvent::OutputResized {
            id: 0,
            width: 1920,
            height: 1040,
        });
        // A diferencia de quitar y volver a añadir la salida, el
        // escritorio activo se conserva.
        assert_eq!(d.active_index(), 1);
        assert!(matches!(cmds.as_slice(), [BrainCommand::Place(_)]));
    }

    #[test]
    fn reservar_franja_desplaza_y_encoge_el_teselado() {
        let mut d = desktop_with_screen(); // 1920×1080
        open(&mut d, 1);
        // Una sola ventana ocupa toda el área útil (smart gaps).
        let cmds = open(&mut d, 1); // re-relayout
        let p0 = places(&cmds)[0].rect;
        assert_eq!(p0, Rect::new(0, 0, 1920, 1080));

        // Reserva 40px arriba: la ventana arranca en y=40 y pierde 40 de alto.
        let cmds = d.on_event(BodyEvent::OutputReserved {
            id: 0,
            top: 40,
            bottom: 0,
            left: 0,
            right: 0,
        });
        let p = places(&cmds)[0].rect;
        assert_eq!(p, Rect::new(0, 40, 1920, 1040));

        // Reserva izquierda en vez de arriba: desplaza en x y encoge el ancho.
        let cmds = d.on_event(BodyEvent::OutputReserved {
            id: 0,
            top: 0,
            bottom: 0,
            left: 48,
            right: 0,
        });
        let p = places(&cmds)[0].rect;
        assert_eq!(p, Rect::new(48, 0, 1872, 1080));

        // Liberar (cero en los cuatro) restaura el monitor entero.
        let cmds = d.on_event(BodyEvent::OutputReserved {
            id: 0,
            top: 0,
            bottom: 0,
            left: 0,
            right: 0,
        });
        assert_eq!(places(&cmds)[0].rect, Rect::new(0, 0, 1920, 1080));
    }

    #[test]
    fn a_spawn_keybind_becomes_a_spawn_command() {
        let mut d = desktop_with_screen();
        let cmds = d.on_event(BodyEvent::Keybind("Super+Shift+Return".into()));
        assert_eq!(cmds, vec![BrainCommand::Spawn("foot".into())]);
    }

    #[test]
    fn dragging_an_unknown_window_does_nothing() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        let cmds = d.on_event(BodyEvent::WindowFloatTo {
            id: 99,
            rect: Rect::new(0, 0, 100, 100),
        });
        assert!(cmds.is_empty());
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

    // --- Multi-monitor -------------------------------------------------

    /// Un escritorio con dos salidas 1920×1080.
    fn desktop_with_two_outputs() -> Desktop {
        let mut d = Desktop::new();
        d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
        d.on_event(BodyEvent::OutputAdded { id: 1, width: 1920, height: 1080 });
        d
    }

    #[test]
    fn outputs_lay_side_by_side() {
        let mut d = Desktop::new();
        d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
        d.on_event(BodyEvent::OutputAdded { id: 1, width: 2560, height: 1440 });
        assert_eq!(d.outputs().len(), 2);
        // La segunda salida arranca donde acaba la primera.
        assert_eq!(d.outputs()[1].rect.x, 1920);
    }

    #[test]
    fn each_output_shows_a_distinct_workspace() {
        let d = desktop_with_two_outputs();
        assert_eq!(d.outputs()[0].workspace, 0);
        assert_eq!(d.outputs()[1].workspace, 1);
    }

    #[test]
    fn switching_to_a_workspace_shown_on_another_output_swaps_them() {
        let mut d = desktop_with_two_outputs();
        // La salida enfocada (0, ws 0) pide el ws 1, que muestra la 1 → swap.
        d.apply(DesktopAction::SwitchWorkspace(1));
        assert_eq!(d.outputs()[0].workspace, 1);
        assert_eq!(d.outputs()[1].workspace, 0);
    }

    #[test]
    fn focus_output_next_moves_the_focus_between_outputs() {
        let mut d = desktop_with_two_outputs();
        assert_eq!(d.active_index(), 0); // salida 0 → ws 0
        d.apply(DesktopAction::FocusOutputNext);
        assert_eq!(d.active_index(), 1); // salida 1 → ws 1
        d.apply(DesktopAction::FocusOutputNext); // envuelve
        assert_eq!(d.active_index(), 0);
    }

    #[test]
    fn relayout_places_windows_on_every_output() {
        let mut d = Desktop::new();
        d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
        d.on_event(BodyEvent::OutputAdded { id: 1, width: 1280, height: 720 });
        open(&mut d, 1); // en la salida 0 (ws 0)
        d.apply(DesktopAction::FocusOutputNext);
        let cmds = open(&mut d, 2); // en la salida 1 (ws 1)
        let p = places(&cmds);
        assert_eq!(p.len(), 2);
        // Cada ventana cae en el rectángulo de su salida.
        assert_eq!(p.iter().find(|x| x.id == 1).unwrap().rect.x, 0);
        assert_eq!(p.iter().find(|x| x.id == 2).unwrap().rect.x, 1920);
    }

    #[test]
    fn keyboard_focus_is_unique_across_outputs() {
        let mut d = desktop_with_two_outputs();
        open(&mut d, 1);
        d.apply(DesktopAction::FocusOutputNext);
        let cmds = open(&mut d, 2);
        // Sólo una ventana con foco de teclado en todo el Place.
        assert_eq!(places(&cmds).iter().filter(|p| p.focused).count(), 1);
    }

    #[test]
    fn removing_an_output_keeps_its_windows_in_their_workspace() {
        let mut d = desktop_with_two_outputs();
        d.apply(DesktopAction::FocusOutputNext); // foco en la salida 1 (ws 1)
        open(&mut d, 1); // en ws 1
        d.on_event(BodyEvent::OutputRemoved { id: 1 });
        // La ventana sigue registrada, en el ws 1.
        assert!(d.window_info(1).is_some());
        assert_eq!(d.workspace_loads()[1], 1);
        assert_eq!(d.outputs().len(), 1);
    }

    // --- Scratchpad ----------------------------------------------------

    #[test]
    fn send_to_scratchpad_hides_the_focused_window() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2); // enfocada
        d.apply(DesktopAction::SendToScratchpad);
        assert_eq!(d.workspace_loads()[0], 1); // sólo queda la 1
        assert!(d.window_info(2).is_some()); // sigue registrada
    }

    #[test]
    fn toggle_scratchpad_shows_then_hides_the_stashed_window() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2);
        d.apply(DesktopAction::SendToScratchpad); // guarda la 2
        assert_eq!(d.workspace_loads()[0], 1);
        // Toggle la invoca, flotando.
        let cmds = d.apply(DesktopAction::ToggleScratchpad);
        assert!(places(&cmds).iter().find(|x| x.id == 2).unwrap().floating);
        assert_eq!(d.workspace_loads()[0], 2);
        // Toggle de nuevo la oculta.
        d.apply(DesktopAction::ToggleScratchpad);
        assert_eq!(d.workspace_loads()[0], 1);
    }

    #[test]
    fn a_scratchpad_window_follows_you_across_workspaces() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        d.apply(DesktopAction::SendToScratchpad);
        d.apply(DesktopAction::ToggleScratchpad); // mostrada en el escritorio 1
        assert_eq!(d.workspace_loads()[0], 1);
        d.apply(DesktopAction::SwitchWorkspace(1)); // al escritorio 2
        d.apply(DesktopAction::ToggleScratchpad); // estaba en el 1 → la trae al 2
        assert_eq!(d.workspace_loads()[1], 1);
        assert_eq!(d.workspace_loads()[0], 0);
    }

    #[test]
    fn closing_a_stashed_window_drops_it_from_the_scratchpad() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        d.apply(DesktopAction::SendToScratchpad);
        d.on_event(BodyEvent::WindowClosed { id: 1 });
        // Ya no hay nada que invocar.
        assert!(d.apply(DesktopAction::ToggleScratchpad).is_empty());
    }

    // --- Terminal dropdown (quake) ------------------------------------

    #[test]
    fn dropterm_lazy_spawns_when_absent() {
        let mut d = desktop_with_screen();
        // Sin terminal dropdown todavía: el toggle la crea.
        let cmds = d.apply(DesktopAction::ToggleDropterm);
        assert_eq!(cmds, vec![BrainCommand::Spawn(super::DROPTERM_CMD.into())]);
    }

    #[test]
    fn dropterm_opens_floating_top_anchored_and_focused() {
        let mut d = desktop_with_screen(); // 1920×1080
        let cmds = d.on_event(BodyEvent::WindowOpened {
            id: 5,
            app_id: super::DROPTERM_APP_ID.into(),
            title: "dropterm".into(),
        });
        let p = places(&cmds).iter().find(|x| x.id == 5).unwrap();
        assert!(p.floating);
        assert_eq!(p.rect.x, 0);
        assert_eq!(p.rect.w, 1920); // a todo el ancho
        assert!(p.rect.h < 1080); // anclada arriba, no a pantalla completa
        assert_eq!(d.focused_window(), Some(5));
    }

    #[test]
    fn dropterm_toggles_hide_then_show_keeping_focus() {
        let mut d = desktop_with_screen();
        // Ya abierta (spawn + WindowOpened).
        d.on_event(BodyEvent::WindowOpened {
            id: 5,
            app_id: super::DROPTERM_APP_ID.into(),
            title: "t".into(),
        });
        assert_eq!(d.workspace_loads()[0], 1);
        // Toggle la guarda.
        d.apply(DesktopAction::ToggleDropterm);
        assert_eq!(d.workspace_loads()[0], 0);
        assert!(d.window_info(5).is_some()); // sigue registrada
        // Toggle la baja de nuevo, flotando y enfocada.
        let cmds = d.apply(DesktopAction::ToggleDropterm);
        assert_eq!(d.workspace_loads()[0], 1);
        assert!(places(&cmds).iter().find(|x| x.id == 5).unwrap().floating);
        assert_eq!(d.focused_window(), Some(5));
    }

    #[test]
    fn a_client_fullscreen_request_is_honoured() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2);
        let cmds = d.on_event(BodyEvent::FullscreenRequest { id: 1, fullscreen: true });
        assert!(places(&cmds).iter().find(|x| x.id == 1).unwrap().fullscreen);
        // El cliente la suelta.
        let cmds = d.on_event(BodyEvent::FullscreenRequest { id: 1, fullscreen: false });
        assert!(!places(&cmds).iter().find(|x| x.id == 1).unwrap().fullscreen);
    }

    #[test]
    fn a_fullscreen_request_for_an_unknown_window_does_nothing() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        assert!(d
            .on_event(BodyEvent::FullscreenRequest { id: 99, fullscreen: true })
            .is_empty());
    }

    #[test]
    fn window_lines_show_a_stashed_window_as_workspace_zero() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        d.apply(DesktopAction::SendToScratchpad);
        let line = d.window_lines().into_iter().find(|l| l.id == 1).unwrap();
        assert_eq!(line.workspace, 0);
    }
}
