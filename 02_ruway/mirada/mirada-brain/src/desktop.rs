//! El [`Desktop`] — el estado del escritorio y el bucle `evento → comandos`.

use std::collections::HashMap;

use mirada_layout::{LayoutParams, Rect, WindowId, Workspace};
use mirada_protocol::{placements, BodyEvent, BrainCommand, OutputId, WindowPlacement};

use crate::action::{Direction, DesktopAction, WORKSPACE_COUNT};
use crate::config::Config;
use crate::keymap::Keymap;
use crate::rules::Rules;
use crate::session::{DesktopState, SESSION_VERSION};

pub use crate::config::DROPTERM_APP_ID;

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
    /// Config general del WM — dropterm, parámetros del teselado, foco.
    config: Config,
    /// Ventanas del scratchpad: se invocan flotando y se ocultan a
    /// voluntad; mientras están guardadas no viven en ningún escritorio.
    scratchpad: Vec<WindowId>,
    /// Mapa salida→escritorio pendiente de aplicar, restaurado de una sesión
    /// guardada: al restaurar en el arranque aún no hay salidas conectadas, así
    /// que se aplica a medida que aparecen (por orden), en `OutputAdded`.
    pending_output_workspaces: Vec<usize>,
    /// `app_id` → escritorio donde vivía, restaurado de una sesión guardada.
    /// Cuando una ventana de esa app **reaparece**, vuelve a ese escritorio; la
    /// entrada se consume (se quita) en el primer acierto, así que sólo
    /// restaura la primera ventana de cada app y no fija las posteriores.
    restored_homes: HashMap<String, usize>,
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
            config: Config::default(),
            scratchpad: Vec::new(),
            pending_output_workspaces: Vec::new(),
            restored_homes: HashMap::new(),
        }
    }

    /// Reemplaza las reglas de ventana. Se aplican a las ventanas que se
    /// abran a partir de ahora; las ya abiertas no se tocan.
    pub fn set_rules(&mut self, rules: Rules) {
        self.rules = rules;
    }

    /// Aplica la config general del WM. Los parámetros de teselado
    /// (modo/gap/ratio/nmaster) se siembran en **todos** los escritorios;
    /// el resto (dropterm, foco-sigue-ratón) se consulta cuando hace falta.
    /// Pensado para llamarse una vez al arrancar, antes de conectar salidas.
    pub fn set_config(&mut self, config: Config) {
        let params = config.layout_params();
        for ws in &mut self.workspaces {
            ws.set_params(params);
        }
        self.config = config;
    }

    /// La config general vigente — para un HUD o un editor de ajustes.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Captura la **forma** persistible del escritorio: los parámetros de
    /// teselado de cada escritorio virtual, qué escritorio mostraba cada salida
    /// y cuál tenía el foco. **No** incluye las ventanas vivas — sus ids son
    /// efímeros (los clientes se reconectan con otros), así que sobrevive la
    /// forma del escritorio, no la geometría por-ventana. Es la cara
    /// serializable de [`session`](crate::session).
    pub fn snapshot(&self) -> DesktopState {
        DesktopState {
            version: SESSION_VERSION,
            workspaces: self.workspaces.iter().map(|w| *w.params()).collect(),
            output_workspaces: self.outputs.iter().map(|o| o.workspace).collect(),
            focused_output: self.focused_output,
            window_homes: self.window_homes(),
        }
    }

    /// Deriva el mapa `app_id`→escritorio de las ventanas vivas, para
    /// persistirlo en la sesión: cada ventana de cada escritorio aporta el
    /// hogar de su app. Orden estable (BTreeMap) y, si una app está en varios
    /// escritorios, gana el de índice mayor.
    fn window_homes(&self) -> Vec<(String, usize)> {
        let mut homes: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for (n, ws) in self.workspaces.iter().enumerate() {
            for &id in ws.windows() {
                if let Some(info) = self.windows.get(&id) {
                    if !info.app_id.is_empty() {
                        homes.insert(info.app_id.clone(), n);
                    }
                }
            }
        }
        homes.into_iter().collect()
    }

    /// Restaura un estado guardado por [`snapshot`](Desktop::snapshot):
    /// re-aplica los parámetros de teselado a cada escritorio y deja el mapa
    /// salida→escritorio en pendiente, para aplicarlo a medida que las salidas
    /// se reconectan (al restaurar en el arranque aún no hay ninguna).
    ///
    /// Debe llamarse **después** de [`set_config`](Desktop::set_config): la
    /// sesión guardada manda sobre los parámetros que la config siembra.
    pub fn restore(&mut self, state: &DesktopState) {
        for (ws, params) in self.workspaces.iter_mut().zip(&state.workspaces) {
            ws.set_params(*params);
        }
        self.pending_output_workspaces = state.output_workspaces.clone();
        self.focused_output = state.focused_output;
        self.restored_homes = state.window_homes.iter().cloned().collect();
    }

    /// Recarga la config en caliente: re-siembra los parámetros de teselado
    /// (el archivo manda — un cambio de gap/modo/ratio se ve al guardar,
    /// aunque pise un layout cambiado a mano) y devuelve el comando que
    /// re-envía la decoración al Cuerpo. dropterm/foco se leen en vivo.
    pub fn reload_config(&mut self, config: Config) -> Vec<BrainCommand> {
        self.set_config(config);
        vec![self.decorations()]
    }

    /// El comando que registra los atajos globales en el Cuerpo. La app
    /// lo envía al conectar, y de nuevo tras cada recarga del keymap.
    pub fn grab_keys(&self) -> BrainCommand {
        BrainCommand::GrabKeys(self.keymap.grab_list())
    }

    /// El comando que fija la decoración de ventana (marco, …) en el
    /// Cuerpo, según la config. La app lo envía al arrancar (junto a
    /// [`grab_keys`](Desktop::grab_keys)) y tras recargar la config.
    pub fn decorations(&self) -> BrainCommand {
        BrainCommand::SetDecorations(self.config.decorations())
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
                let taken: Vec<usize> = self.outputs.iter().map(|o| o.workspace).collect();
                // Si hay una sesión restaurada, esta salida —por su orden de
                // aparición— recupera el escritorio que mostraba; si no (o si ya
                // lo muestra otra), cae al primero libre.
                let appearing = self.outputs.len();
                let workspace = self
                    .pending_output_workspaces
                    .get(appearing)
                    .copied()
                    .filter(|&n| n < self.workspaces.len() && !taken.contains(&n))
                    .unwrap_or_else(|| {
                        (0..self.workspaces.len())
                            .find(|n| !taken.contains(n))
                            .unwrap_or(0)
                    });
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
                // Si ninguna regla fija escritorio y una sesión restaurada
                // recuerda dónde vivía esta app, vuelve ahí (consumido una vez).
                let home = self
                    .restored_homes
                    .remove(&app_id)
                    .filter(|&n| n < self.workspaces.len());
                self.windows.insert(id, WindowInfo { app_id, title });
                let ws = outcome
                    .workspace
                    .filter(|&n| n < self.workspaces.len())
                    .or(home)
                    .unwrap_or(self.active_index());
                self.workspaces[ws].add(id);
                if is_dropterm {
                    let pct = self.config.dropterm_height_pct();
                    let rect = self
                        .screen()
                        .map(|s| dropdown_rect(s, pct))
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
                // Foco al pasar el puntero, si la config lo habilita y la
                // ventana está en el escritorio activo.
                if !self.config.focus_follows_mouse {
                    return Vec::new();
                }
                let active = self.active_index();
                if self.workspaces[active].focus_window(id) {
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            BodyEvent::Clicked { id } => {
                // Foco-al-click: enfoca la ventana clickeada, esté donde
                // esté, sin depender del foco-sigue-ratón. El z-order
                // (levantar la flotante clickeada) lo resuelve el Cuerpo al
                // pintar la enfocada encima.
                self.apply(DesktopAction::FocusWindow(id))
            }
            BodyEvent::WindowDragged { id, x, y } => {
                // Arrastre de una ventana teselada: la intercambia con la
                // teselada que haya bajo el puntero. Una flotante no entra
                // aquí (usa WindowFloatTo) — si llega, la ignoramos.
                let active = self.active_index();
                if self.workspaces[active].is_floating(id)
                    || !self.workspaces[active].windows().contains(&id)
                {
                    return Vec::new();
                }
                let Some(o) = self.outputs.get(self.focused_output).copied() else {
                    return Vec::new();
                };
                let target = self.workspaces[active]
                    .layout(o.work_rect())
                    .into_iter()
                    .find(|(wid, rect)| {
                        *wid != id
                            && !self.workspaces[active].is_floating(*wid)
                            && rect.contains(x, y)
                    })
                    .map(|(wid, _)| wid);
                match target {
                    Some(t) if self.workspaces[active].swap(id, t) => self.relayout(),
                    _ => Vec::new(),
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
            DesktopAction::FocusDir(dir) => self.focus_in_direction(dir),
            DesktopAction::MoveDir(dir) => self.move_in_direction(dir),
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
            DesktopAction::ToggleTiling => {
                let screen = self.screen();
                let ws = &mut self.workspaces[active];
                // Las teseladas: las no flotantes que sí están en el orden.
                let tiled: Vec<WindowId> = ws
                    .windows()
                    .iter()
                    .copied()
                    .filter(|&id| !ws.is_floating(id))
                    .collect();
                if tiled.is_empty() {
                    // Todas flotando ya: devolverlas al teselado.
                    let floating: Vec<WindowId> = ws.windows().to_vec();
                    for id in floating {
                        ws.set_floating(id, None);
                    }
                } else if let Some(base) = screen {
                    // Hacerlas flotar todas, en cascada desde una base.
                    let w = base.w * 3 / 5;
                    let h = base.h * 3 / 5;
                    for (i, id) in tiled.into_iter().enumerate() {
                        let off = (i as i32) * 32;
                        let rect = Rect::new(
                            base.x + (base.w - w) / 2 + off,
                            base.y + (base.h - h) / 2 + off,
                            w,
                            h,
                        );
                        ws.set_floating(id, Some(rect));
                    }
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
                            let pct = self.config.dropterm_height_pct();
                            let rect = self
                                .screen()
                                .map(|s| dropdown_rect(s, pct))
                                .unwrap_or_else(|| Rect::new(100, 100, 800, 600));
                            self.workspaces[active].add(id);
                            self.workspaces[active].set_floating(id, Some(rect));
                        }
                        self.relayout()
                    }
                    // Aún no existe: la creamos perezosamente con el comando
                    // de la config. Al abrirse, `WindowOpened` la reconoce
                    // (por su `app_id`) y la baja flotando+enfocada.
                    None => vec![BrainCommand::Spawn(self.config.dropterm_cmd.clone())],
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
            DesktopAction::GrowMaster => self.nudge_master(self.config.master_step()),
            DesktopAction::ShrinkMaster => self.nudge_master(-self.config.master_step()),
            DesktopAction::IncMaster => self.nudge_master_count(1),
            DesktopAction::DecMaster => self.nudge_master_count(-1),
            DesktopAction::PromoteToMaster => {
                self.workspaces[active].promote_focused();
                self.relayout()
            }
            DesktopAction::SwapMaster => {
                let ws = &mut self.workspaces[active];
                let Some(focused) = ws.focused() else {
                    return Vec::new();
                };
                let Some(&master) = ws.windows().first() else {
                    return Vec::new();
                };
                // Sólo intercambia esas dos; el resto del orden no se mueve.
                if focused != master && ws.swap(focused, master) {
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            DesktopAction::GroupStack => {
                let ws = &mut self.workspaces[active];
                let nmaster = ws.params().master_count;
                // La pila del **nivel en vista**: sus hojas sueltas menos el área
                // maestra. Con zoom activo pliega dentro del sub-espacio actual
                // (anidamiento), no en la raíz.
                let stack: Vec<WindowId> = ws.view_leaves().into_iter().skip(nmaster).collect();
                ws.group(&stack);
                self.relayout()
            }
            DesktopAction::Ungroup => {
                self.workspaces[active].ungroup();
                self.relayout()
            }
            DesktopAction::ZoomIn => {
                self.workspaces[active].zoom_in();
                self.relayout()
            }
            DesktopAction::ZoomOut => {
                self.workspaces[active].zoom_out();
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
            DesktopAction::MoveToWorkspace(n) => {
                if n >= self.workspaces.len() || n == active {
                    return Vec::new();
                }
                match self.workspaces[active].focused() {
                    Some(id) => {
                        self.workspaces[active].remove(id);
                        self.workspaces[n].add(id);
                        // …y salta con ella al escritorio destino.
                        self.show_workspace(n);
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
            DesktopAction::FocusOutputDir(dir) => {
                let Some(oid) = self.output_in_direction(dir) else {
                    return Vec::new();
                };
                match self.outputs.iter().position(|o| o.id == oid) {
                    Some(i) => {
                        self.focused_output = i;
                        self.relayout()
                    }
                    None => Vec::new(),
                }
            }
            DesktopAction::SendToOutputDir(dir) => {
                let Some(id) = self.workspaces[active].focused() else {
                    return Vec::new();
                };
                let Some(oid) = self.output_in_direction(dir) else {
                    return Vec::new();
                };
                let Some(ti) = self.outputs.iter().position(|o| o.id == oid) else {
                    return Vec::new();
                };
                let target_ws = self.outputs[ti].workspace;
                self.workspaces[active].remove(id);
                self.workspaces[target_ws].add(id);
                self.relayout()
            }
            DesktopAction::ResizeFloatDir(dir) => self.resize_float(dir),
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

    /// Mueve el foco a la ventana más cercana en una dirección cardinal,
    /// según la geometría real del escritorio activo (no el orden de
    /// teselado). Sin ventana enfocada, sin salida, o sin candidata en esa
    /// dirección, no hace nada.
    fn focus_in_direction(&mut self, dir: Direction) -> Vec<BrainCommand> {
        let active = self.active_index();
        let Some(focused) = self.workspaces[active].focused() else {
            return Vec::new();
        };
        let Some(o) = self.outputs.get(self.focused_output).copied() else {
            return Vec::new();
        };
        let layout = self.workspaces[active].layout(o.work_rect());
        let Some(from) = layout.iter().find(|(id, _)| *id == focused).map(|(_, r)| *r) else {
            return Vec::new();
        };
        match nearest_in_direction(from, &layout, focused, dir) {
            Some(target) if self.workspaces[active].focus_window(target) => self.relayout(),
            _ => Vec::new(),
        }
    }

    /// La salida (monitor) vecina en una dirección cardinal, por geometría.
    /// `None` si hay menos de dos salidas o ninguna en esa dirección.
    fn output_in_direction(&self, dir: Direction) -> Option<OutputId> {
        if self.outputs.len() < 2 {
            return None;
        }
        let cur = self.outputs.get(self.focused_output)?;
        let cands: Vec<(OutputId, Rect)> = self.outputs.iter().map(|o| (o.id, o.rect)).collect();
        nearest_in_direction(cur.rect, &cands, cur.id, dir)
    }

    /// Redimensiona la ventana flotante enfocada hacia una dirección, por
    /// `float_step` px (acotada a un mínimo). No hace nada sobre teseladas.
    fn resize_float(&mut self, dir: Direction) -> Vec<BrainCommand> {
        const MIN: i32 = 80;
        let active = self.active_index();
        let Some(id) = self.workspaces[active].focused() else {
            return Vec::new();
        };
        let Some(mut rect) = self.workspaces[active].floating_rect(id) else {
            return Vec::new(); // no flota
        };
        let step = self.config.float_step();
        match dir {
            Direction::Right => rect.w = (rect.w + step).max(MIN),
            Direction::Left => rect.w = (rect.w - step).max(MIN),
            Direction::Down => rect.h = (rect.h + step).max(MIN),
            Direction::Up => rect.h = (rect.h - step).max(MIN),
        }
        self.workspaces[active].set_floating(id, Some(rect));
        self.relayout()
    }

    /// Intercambia la ventana enfocada con su vecina **teselada** más
    /// cercana en una dirección — mover la ventana por geometría. Una
    /// ventana **flotante** se desplaza `float_step` px en esa dirección.
    /// Sin vecina (teselada) en esa dirección, no hace nada. El foco
    /// acompaña a la ventana movida.
    fn move_in_direction(&mut self, dir: Direction) -> Vec<BrainCommand> {
        let active = self.active_index();
        let Some(focused) = self.workspaces[active].focused() else {
            return Vec::new();
        };
        // Una flotante se mueve nudgeando su posición, no intercambiando.
        if let Some(mut rect) = self.workspaces[active].floating_rect(focused) {
            let step = self.config.float_step();
            match dir {
                Direction::Left => rect.x -= step,
                Direction::Right => rect.x += step,
                Direction::Up => rect.y -= step,
                Direction::Down => rect.y += step,
            }
            self.workspaces[active].set_floating(focused, Some(rect));
            return self.relayout();
        }
        let Some(o) = self.outputs.get(self.focused_output).copied() else {
            return Vec::new();
        };
        let layout = self.workspaces[active].layout(o.work_rect());
        let Some(from) = layout.iter().find(|(id, _)| *id == focused).map(|(_, r)| *r) else {
            return Vec::new();
        };
        // Sólo teseladas son candidatas a intercambio.
        let tiled: Vec<(WindowId, Rect)> = layout
            .into_iter()
            .filter(|(id, _)| !self.workspaces[active].is_floating(*id))
            .collect();
        match nearest_in_direction(from, &tiled, focused, dir) {
            Some(target) if self.workspaces[active].swap(focused, target) => self.relayout(),
            _ => Vec::new(),
        }
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

    /// La geometría teselada de **cada** escritorio, calculada contra `rect`
    /// (normalmente el [`work_rect`](Output::work_rect) de la salida primaria),
    /// para pintar miniaturas sin cambiar de escritorio. Es lo que consume la
    /// **vista espacial** (el "Prezi" de mirada): un mosaico por escritorio con
    /// sus ventanas a escala. Cada `Vec` respeta el modo de teselado propio de
    /// su escritorio y marca el foco de ese escritorio. `out[i]` = escritorio
    /// `i` (0-based, casa con [`workspace_loads`](Desktop::workspace_loads)).
    pub fn workspace_layouts(&self, rect: Rect) -> Vec<Vec<WindowPlacement>> {
        self.workspaces
            .iter()
            .map(|ws| placements(ws, rect))
            .collect()
    }

    /// El rectángulo de referencia para la vista espacial: el área teselable de
    /// la salida enfocada, o el rect dado por defecto si no hay salidas (modo
    /// simulación). Da la relación de aspecto correcta a las miniaturas.
    pub fn overview_rect(&self, fallback: Rect) -> Rect {
        self.outputs
            .get(self.focused_output)
            .map(Output::work_rect)
            .unwrap_or(fallback)
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

/// El elemento de `candidates` (ventana o salida) más cercano a `from` en
/// la dirección `dir`, excluyendo a `self_id`. Pura — la base del foco
/// espacial entre ventanas y entre monitores.
///
/// Criterio (estilo i3/sway): sólo cuentan los candidatos cuyo centro cae
/// en el semiplano de esa dirección respecto al centro de `from`; entre
/// ellos gana el de menor distancia en el eje principal, penalizando el
/// desvío en el eje perpendicular (`×2`) para preferir el que está
/// «enfrente». Empates: el id menor, para ser determinista.
fn nearest_in_direction<T: Copy + Ord>(
    from: Rect,
    candidates: &[(T, Rect)],
    self_id: T,
    dir: Direction,
) -> Option<T> {
    let center = |r: &Rect| (r.x + r.w / 2, r.y + r.h / 2);
    let (fx, fy) = center(&from);
    let mut best: Option<(i64, T)> = None;
    for (id, rect) in candidates {
        if *id == self_id {
            continue;
        }
        let (cx, cy) = center(rect);
        let (dx, dy) = ((cx - fx) as i64, (cy - fy) as i64);
        // ¿Está en el semiplano de la dirección? (`primary` > 0) y, si sí,
        // el coste = primary + 2·|perpendicular|.
        let (primary, perp) = match dir {
            Direction::Left => (-dx, dy),
            Direction::Right => (dx, dy),
            Direction::Up => (-dy, dx),
            Direction::Down => (dy, dx),
        };
        if primary <= 0 {
            continue;
        }
        let cost = primary + 2 * perp.abs();
        let better = match best {
            None => true,
            Some((c, bid)) => cost < c || (cost == c && *id < bid),
        };
        if better {
            best = Some((cost, *id));
        }
    }
    best.map(|(_, id)| id)
}

/// El rectángulo de la terminal dropdown: anclada arriba, a todo el ancho,
/// `pct` % del alto — el gesto «quake» de bajar desde el borde superior.
/// El porcentaje sale de la config ([`Config::dropterm_height_pct`]).
fn dropdown_rect(screen: Rect, pct: i32) -> Rect {
    Rect::new(screen.x, screen.y, screen.w, (screen.h * pct / 100).max(1))
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
    fn swap_master_exchanges_only_the_focused_and_the_master() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3, 4] {
            open(&mut d, id);
        }
        d.apply(DesktopAction::FocusWindow(3));
        d.apply(DesktopAction::SwapMaster);
        // 3 pasa al puesto maestro, 1 a donde estaba 3; el resto intacto.
        assert_eq!(d.active_workspace().windows(), &[3, 2, 1, 4]);
        assert_eq!(d.focused_window(), Some(3));
        // A diferencia de promote-to-master, que rota: promover la 4…
        d.apply(DesktopAction::FocusWindow(4));
        d.apply(DesktopAction::PromoteToMaster);
        assert_eq!(d.active_workspace().windows(), &[4, 3, 2, 1]);
    }

    #[test]
    fn move_to_workspace_sends_the_window_and_follows_it() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2); // enfocada
        d.apply(DesktopAction::MoveToWorkspace(2)); // índice 2 = escritorio 3
        // La 2 viajó y el foco saltó con ella.
        assert_eq!(d.active_index(), 2);
        assert_eq!(d.focused_window(), Some(2));
        assert_eq!(d.workspace_loads()[2], 1);
        // El escritorio original conserva sólo la 1.
        assert_eq!(d.workspace_loads()[0], 1);
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
        // Sin terminal dropdown todavía: el toggle la crea con el comando
        // de la config (por defecto, kitty con el app_id de la dropterm).
        let cmds = d.apply(DesktopAction::ToggleDropterm);
        let cmd = crate::config::Config::default().dropterm_cmd;
        assert_eq!(cmds, vec![BrainCommand::Spawn(cmd)]);
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
    fn nearest_in_direction_picks_the_window_in_front() {
        let from = Rect::new(0, 0, 100, 100); // centro (50,50)
        let cands = vec![
            (1, Rect::new(0, 0, 100, 100)),   // la propia
            (2, Rect::new(200, 0, 100, 100)), // a la derecha, enfrente
            (3, Rect::new(200, 400, 100, 100)), // a la derecha pero muy abajo
            (4, Rect::new(-200, 0, 100, 100)), // a la izquierda
        ];
        assert_eq!(nearest_in_direction(from, &cands, 1, Direction::Right), Some(2));
        assert_eq!(nearest_in_direction(from, &cands, 1, Direction::Left), Some(4));
        // Hacia arriba no hay nada (todas a la misma altura o abajo).
        assert_eq!(nearest_in_direction(from, &cands, 1, Direction::Up), None);
    }

    #[test]
    fn focus_dir_moves_focus_spatially_in_columns() {
        let mut d = desktop_with_screen();
        // Tres columnas: la 1 a la izquierda, la 3 a la derecha.
        d.apply(DesktopAction::SetLayout(LayoutMode::Columns));
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        // La última abierta (3) queda enfocada, en la columna derecha.
        assert_eq!(d.focused_window(), Some(3));
        // Foco a la izquierda → la del medio, luego la primera.
        d.apply(DesktopAction::FocusDir(Direction::Left));
        assert_eq!(d.focused_window(), Some(2));
        d.apply(DesktopAction::FocusDir(Direction::Left));
        assert_eq!(d.focused_window(), Some(1));
        // Más a la izquierda no hay nada: el foco no se mueve.
        d.apply(DesktopAction::FocusDir(Direction::Left));
        assert_eq!(d.focused_window(), Some(1));
        // Y de vuelta a la derecha.
        d.apply(DesktopAction::FocusDir(Direction::Right));
        assert_eq!(d.focused_window(), Some(2));
    }

    #[test]
    fn move_dir_swaps_the_focused_tile_with_its_neighbor() {
        let mut d = desktop_with_screen();
        d.apply(DesktopAction::SetLayout(LayoutMode::Columns));
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        assert_eq!(d.active_workspace().windows(), &[1, 2, 3]);
        assert_eq!(d.focused_window(), Some(3)); // columna derecha
        // Mover a la izquierda intercambia la 3 con su vecina (la 2).
        d.apply(DesktopAction::MoveDir(Direction::Left));
        assert_eq!(d.active_workspace().windows(), &[1, 3, 2]);
        assert_eq!(d.focused_window(), Some(3)); // el foco acompaña a la movida
        // Y a la derecha la devuelve.
        d.apply(DesktopAction::MoveDir(Direction::Right));
        assert_eq!(d.active_workspace().windows(), &[1, 2, 3]);
        assert_eq!(d.focused_window(), Some(3));
    }

    #[test]
    fn move_dir_nudges_a_floating_window() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2); // enfocada → flotar
        d.apply(DesktopAction::ToggleFloat);
        let r0 = d.active_workspace().floating_rect(2).unwrap();
        d.apply(DesktopAction::MoveDir(Direction::Right));
        let r1 = d.active_workspace().floating_rect(2).unwrap();
        // Se desplazó float_step px a la derecha, sin cambiar tamaño.
        assert_eq!(r1.x, r0.x + d.config().float_step());
        assert_eq!((r1.w, r1.h), (r0.w, r0.h));
    }

    #[test]
    fn resize_float_grows_and_shrinks_the_focused_floating_window() {
        let mut d = desktop_with_screen();
        open(&mut d, 1); // enfocada → flotar
        d.apply(DesktopAction::ToggleFloat);
        let r0 = d.active_workspace().floating_rect(1).unwrap();
        let step = d.config().float_step();
        d.apply(DesktopAction::ResizeFloatDir(Direction::Right));
        assert_eq!(d.active_workspace().floating_rect(1).unwrap().w, r0.w + step);
        d.apply(DesktopAction::ResizeFloatDir(Direction::Down));
        assert_eq!(d.active_workspace().floating_rect(1).unwrap().h, r0.h + step);
        // Sobre una teselada no hace nada.
        open(&mut d, 2); // teselada, enfocada
        assert!(d.apply(DesktopAction::ResizeFloatDir(Direction::Right)).is_empty());
    }

    #[test]
    fn focus_and_send_to_output_dir_cross_monitors() {
        let mut d = desktop_with_two_outputs(); // salida 0 a la izq, 1 a la der
        open(&mut d, 1); // en la salida 0 (ws 0)
        assert_eq!(d.active_index(), 0);
        // Foco a la salida de la derecha → su escritorio (ws 1).
        d.apply(DesktopAction::FocusOutputDir(Direction::Right));
        assert_eq!(d.active_index(), 1);
        // Volver a la izquierda.
        d.apply(DesktopAction::FocusOutputDir(Direction::Left));
        assert_eq!(d.active_index(), 0);
        // Mandar la ventana 1 a la salida derecha → viaja al ws 1.
        d.apply(DesktopAction::SendToOutputDir(Direction::Right));
        assert_eq!(d.workspace_loads()[0], 0);
        assert_eq!(d.workspace_loads()[1], 1);
    }

    #[test]
    fn master_step_from_config_drives_grow_master() {
        use crate::config::Config;
        let mut d = desktop_with_screen();
        d.set_config(Config::from_ron("( master_step: 0.1 )").unwrap());
        open(&mut d, 1);
        let r0 = d.active_workspace().params().master_ratio;
        d.apply(DesktopAction::GrowMaster);
        assert!((d.active_workspace().params().master_ratio - (r0 + 0.1)).abs() < 1e-6);
    }

    #[test]
    fn clicked_focuses_even_with_focus_follows_mouse_off() {
        use crate::config::Config;
        let mut d = desktop_with_screen();
        d.set_config(Config::from_ron("( focus_follows_mouse: false )").unwrap());
        open(&mut d, 1);
        open(&mut d, 2); // enfocada
        // El hover ya no enfoca…
        d.on_event(BodyEvent::PointerEntered { id: 1 });
        assert_eq!(d.focused_window(), Some(2));
        // …pero el click sí.
        d.on_event(BodyEvent::Clicked { id: 1 });
        assert_eq!(d.focused_window(), Some(1));
    }

    #[test]
    fn clicked_jumps_to_the_workspace_holding_the_window() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2);
        d.on_event(BodyEvent::Keybind("Super+Shift+3".into())); // la 2 al esc. 3
        assert_eq!(d.active_index(), 0);
        d.on_event(BodyEvent::Clicked { id: 2 });
        assert_eq!(d.active_index(), 2);
        assert_eq!(d.focused_window(), Some(2));
    }

    #[test]
    fn dragging_a_tiled_window_swaps_with_the_window_under_the_pointer() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        assert_eq!(d.active_workspace().windows(), &[1, 2, 3]);
        // El centro de la tesela de la 3.
        let o = d.outputs()[d.focused_output()];
        let layout = d.active_workspace().layout(o.work_rect());
        let r3 = layout.iter().find(|(id, _)| *id == 3).unwrap().1;
        let (cx, cy) = (r3.x + r3.w / 2, r3.y + r3.h / 2);
        // Arrastrar la 1 sobre la 3 las intercambia, el foco sigue a la 1.
        d.on_event(BodyEvent::WindowDragged { id: 1, x: cx, y: cy });
        assert_eq!(d.active_workspace().windows(), &[3, 2, 1]);
        assert_eq!(d.focused_window(), Some(1));
    }

    #[test]
    fn window_dragged_ignores_a_floating_window() {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2); // enfocada → la flotamos
        d.apply(DesktopAction::ToggleFloat);
        let before = d.active_workspace().windows().to_vec();
        let cmds = d.on_event(BodyEvent::WindowDragged { id: 2, x: 10, y: 10 });
        assert!(cmds.is_empty());
        assert_eq!(d.active_workspace().windows(), &before[..]);
    }

    #[test]
    fn toggle_tiling_floats_all_then_restores() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        assert!([1, 2, 3]
            .iter()
            .all(|&id| !d.active_workspace().is_floating(id)));
        // Todo flota.
        d.apply(DesktopAction::ToggleTiling);
        assert!([1, 2, 3]
            .iter()
            .all(|&id| d.active_workspace().is_floating(id)));
        // Y vuelve al teselado.
        d.apply(DesktopAction::ToggleTiling);
        assert!([1, 2, 3]
            .iter()
            .all(|&id| !d.active_workspace().is_floating(id)));
    }

    #[test]
    fn reload_config_reseeds_params_and_re_sends_decorations() {
        use crate::config::Config;
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        let cfg = Config::from_ron("( gap: 30, border_width: 5 )").unwrap();
        let cmds = d.reload_config(cfg);
        // El gap nuevo se sembró (el archivo manda).
        assert_eq!(d.active_workspace().params().gap, 30);
        // Y se devuelve un SetDecorations con el marco nuevo.
        match cmds.as_slice() {
            [BrainCommand::SetDecorations(dec)] => assert_eq!(dec.border_width, 5),
            other => panic!("se esperaba un SetDecorations, no {other:?}"),
        }
    }

    #[test]
    fn reload_config_preserves_open_windows_and_focus() {
        use crate::config::Config;
        let mut d = desktop_with_screen();
        for id in [1, 2, 3] {
            open(&mut d, id);
        }
        d.apply(DesktopAction::FocusWindow(2));
        d.reload_config(Config::from_ron("( gap: 14 )").unwrap());
        // Las ventanas siguen ahí, el foco intacto — recargar no las pierde.
        assert_eq!(d.active_workspace().windows(), &[1, 2, 3]);
        assert_eq!(d.focused_window(), Some(2));
        assert_eq!(d.active_workspace().params().gap, 14);
    }

    #[test]
    fn reload_config_applies_focus_follows_mouse_live() {
        use crate::config::Config;
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        open(&mut d, 2); // enfocada
        // Por defecto el hover enfoca.
        d.on_event(BodyEvent::PointerEntered { id: 1 });
        assert_eq!(d.focused_window(), Some(1));
        // Recargar con foco-sigue-ratón apagado lo desactiva en vivo.
        d.reload_config(Config::from_ron("( focus_follows_mouse: false )").unwrap());
        d.on_event(BodyEvent::PointerEntered { id: 2 });
        assert_eq!(d.focused_window(), Some(1)); // el hover ya no mueve el foco
    }

    #[test]
    fn reload_config_applies_the_dropterm_command_live() {
        use crate::config::Config;
        let mut d = desktop_with_screen();
        d.reload_config(
            Config::from_ron("( dropterm_cmd: \"foot --app-id mirada.dropterm\" )").unwrap(),
        );
        let cmds = d.apply(DesktopAction::ToggleDropterm);
        assert_eq!(
            cmds,
            vec![BrainCommand::Spawn("foot --app-id mirada.dropterm".into())]
        );
    }

    #[test]
    fn set_config_seeds_the_layout_params_of_every_workspace() {
        use crate::config::Config;
        let mut d = Desktop::new();
        let cfg = Config::from_ron(r#"( gap: 20, master_ratio: 0.4, layout: "grid" )"#).unwrap();
        d.set_config(cfg);
        d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
        let p = d.active_workspace().params();
        assert_eq!(p.gap, 20);
        assert_eq!(p.mode, LayoutMode::Grid);
        assert!((p.master_ratio - 0.4).abs() < 1e-6);
    }

    #[test]
    fn focus_follows_mouse_can_be_disabled_by_config() {
        use crate::config::Config;
        let mut d = desktop_with_screen();
        d.set_config(Config::from_ron("( focus_follows_mouse: false )").unwrap());
        open(&mut d, 1);
        open(&mut d, 2); // enfocada
        // Con el foco-sigue-ratón apagado, pasar el puntero no cambia el foco.
        d.on_event(BodyEvent::PointerEntered { id: 1 });
        assert_eq!(d.focused_window(), Some(2));
    }

    #[test]
    fn config_sets_the_dropterm_command_and_height() {
        use crate::config::Config;
        let mut d = desktop_with_screen(); // 1920×1080
        d.set_config(
            Config::from_ron("( dropterm_cmd: \"foot --app-id mirada.dropterm\", dropterm_height_pct: 30 )")
                .unwrap(),
        );
        // El spawn perezoso usa el comando de la config.
        let cmds = d.apply(DesktopAction::ToggleDropterm);
        assert_eq!(
            cmds,
            vec![BrainCommand::Spawn("foot --app-id mirada.dropterm".into())]
        );
        // Y al abrirse, baja al 30 % del alto.
        let cmds = d.on_event(BodyEvent::WindowOpened {
            id: 9,
            app_id: super::DROPTERM_APP_ID.into(),
            title: "t".into(),
        });
        let p = places(&cmds).iter().find(|x| x.id == 9).unwrap();
        assert_eq!(p.rect.h, 1080 * 30 / 100);
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

    // --- Persistencia de sesión (snapshot/restore) ----------------------

    #[test]
    fn snapshot_captures_per_workspace_modes_and_the_output_map() {
        let mut d = desktop_with_screen();
        // Cambia el modo del escritorio activo y manda otra salida a otro.
        d.apply(DesktopAction::SetLayout(LayoutMode::Grid));
        let snap = d.snapshot();
        assert_eq!(snap.version, crate::session::SESSION_VERSION);
        assert_eq!(snap.workspaces.len(), WORKSPACE_COUNT);
        assert_eq!(snap.workspaces[0].mode, LayoutMode::Grid);
        // Una salida conectada, mostrando el escritorio 0.
        assert_eq!(snap.output_workspaces, vec![0]);
    }

    #[test]
    fn restore_reapplies_layout_params_to_each_workspace() {
        let snap = {
            let mut d = desktop_with_screen();
            d.apply(DesktopAction::SetLayout(LayoutMode::Spiral));
            d.apply(DesktopAction::IncMaster); // master_count 1 → 2
            d.snapshot()
        };
        // Un escritorio nuevo, sin salidas todavía.
        let mut d = Desktop::new();
        d.restore(&snap);
        // Los params del escritorio 0 se recuperaron.
        assert_eq!(d.workspaces[0].params().mode, LayoutMode::Spiral);
        assert_eq!(d.workspaces[0].params().master_count, 2);
    }

    #[test]
    fn restore_places_each_output_on_its_remembered_workspace() {
        // Sesión: dos salidas, la primera mostraba el escritorio 4, la segunda
        // el 2.
        let snap = DesktopState {
            version: crate::session::SESSION_VERSION,
            workspaces: vec![LayoutParams::default(); WORKSPACE_COUNT],
            output_workspaces: vec![4, 2],
            focused_output: 1,
            window_homes: Vec::new(),
        };
        let mut d = Desktop::new();
        d.restore(&snap);
        // Las salidas aparecen en orden y recuperan su escritorio.
        d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
        d.on_event(BodyEvent::OutputAdded { id: 1, width: 1920, height: 1080 });
        assert_eq!(d.outputs()[0].workspace, 4);
        assert_eq!(d.outputs()[1].workspace, 2);
        assert_eq!(d.focused_output(), 1);
        assert_eq!(d.active_index(), 2); // la salida enfocada (1) muestra el 2
    }

    #[test]
    fn restore_with_a_conflicting_map_falls_back_to_a_free_workspace() {
        // Ambas salidas pretenden el mismo escritorio: la segunda no puede.
        let snap = DesktopState {
            version: crate::session::SESSION_VERSION,
            workspaces: vec![LayoutParams::default(); WORKSPACE_COUNT],
            output_workspaces: vec![3, 3],
            focused_output: 0,
            window_homes: Vec::new(),
        };
        let mut d = Desktop::new();
        d.restore(&snap);
        d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
        d.on_event(BodyEvent::OutputAdded { id: 1, width: 1920, height: 1080 });
        assert_eq!(d.outputs()[0].workspace, 3);
        // La segunda cayó al primer escritorio libre (no el 3, ya tomado).
        assert_ne!(d.outputs()[1].workspace, 3);
    }

    #[test]
    fn snapshot_remembers_which_workspace_each_app_lived_on() {
        let mut d = desktop_with_screen();
        open(&mut d, 1); // app1 nace en el escritorio 0…
        d.apply(DesktopAction::SendToWorkspace(2)); // …y se va al índice 2
        assert!(d.snapshot().window_homes.contains(&("app1".to_string(), 2)));
    }

    #[test]
    fn a_reopened_window_returns_to_its_remembered_workspace() {
        let snap = {
            let mut d = desktop_with_screen();
            open(&mut d, 1);
            d.apply(DesktopAction::SendToWorkspace(2));
            d.snapshot()
        };
        let mut d = desktop_with_screen();
        d.restore(&snap);
        // app1 reaparece: vuelve al escritorio índice 2, no al activo (0).
        d.on_event(BodyEvent::WindowOpened { id: 1, app_id: "app1".into(), title: "x".into() });
        assert_eq!(d.workspace_loads()[2], 1);
        assert_eq!(d.workspace_loads()[0], 0);
    }

    #[test]
    fn a_rule_beats_a_session_home() {
        let snap = {
            let mut d = desktop_with_screen();
            open(&mut d, 1);
            d.apply(DesktopAction::SendToWorkspace(2)); // hogar = índice 2
            d.snapshot()
        };
        let mut d = desktop_with_screen();
        // La regla manda app1 al escritorio 5 (índice 4) — pisa el hogar.
        d.set_rules(Rules::from_ron(r#"( rules: [ (app_id: "app1", workspace: 5) ] )"#).unwrap());
        d.restore(&snap);
        d.on_event(BodyEvent::WindowOpened { id: 1, app_id: "app1".into(), title: "x".into() });
        assert_eq!(d.workspace_loads()[4], 1); // donde dice la regla
        assert_eq!(d.workspace_loads()[2], 0); // no en el hogar de la sesión
    }

    #[test]
    fn a_session_home_is_consumed_after_the_first_window() {
        let snap = {
            let mut d = desktop_with_screen();
            open(&mut d, 1);
            d.apply(DesktopAction::SendToWorkspace(2));
            d.snapshot()
        };
        let mut d = desktop_with_screen();
        d.restore(&snap);
        // Primera ventana de app1 → vuelve al hogar (índice 2).
        d.on_event(BodyEvent::WindowOpened { id: 1, app_id: "app1".into(), title: "x".into() });
        d.on_event(BodyEvent::WindowClosed { id: 1 });
        // Segunda ventana de app1 → el hogar ya se consumió: va al activo (0).
        d.on_event(BodyEvent::WindowOpened { id: 2, app_id: "app1".into(), title: "y".into() });
        assert_eq!(d.workspace_loads()[0], 1);
        assert_eq!(d.workspace_loads()[2], 0);
    }

    #[test]
    fn group_stack_then_zoom_makes_the_stack_absorb_the_screen() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3, 4] {
            open(&mut d, id); // MasterStack, nmaster=1 → maestra 1, pila 2/3/4
        }
        // Pliega la pila (2,3,4) en un sub-espacio.
        d.apply(DesktopAction::GroupStack);
        assert!(d.active_workspace().is_grouped());
        // Con el foco en la pila, entrar: sólo se ven 2,3,4.
        d.apply(DesktopAction::FocusWindow(3));
        let cmds = d.apply(DesktopAction::ZoomIn);
        let p = places(&cmds);
        // Las visibles son la pila; la maestra 1 queda fuera del zoom.
        let visibles: Vec<_> = p.iter().filter(|p| p.visible).map(|p| p.id).collect();
        assert_eq!(visibles.len(), 3);
        assert!(visibles.contains(&2) && visibles.contains(&3) && visibles.contains(&4));
        // Pero la 1 no se omite: se lista dormida (suspended) para cortarle los
        // frames, no oculta a ciegas por ausencia.
        let one = p.iter().find(|p| p.id == 1).unwrap();
        assert!(one.suspended && !one.visible);
        // Salir y deshacer: vuelven las cuatro.
        d.apply(DesktopAction::ZoomOut);
        let cmds = d.apply(DesktopAction::Ungroup);
        assert_eq!(places(&cmds).len(), 4);
        assert!(!d.active_workspace().is_grouped());
    }

    #[test]
    fn group_stack_nests_inside_the_current_zoom_level() {
        let mut d = desktop_with_screen();
        for id in [1, 2, 3, 4] {
            open(&mut d, id); // MasterStack nmaster=1 → maestra 1, pila 2/3/4
        }
        // Nivel 1: plegar la pila (2,3,4) y entrar.
        d.apply(DesktopAction::GroupStack);
        d.apply(DesktopAction::FocusWindow(3));
        d.apply(DesktopAction::ZoomIn);
        assert_eq!(d.active_workspace().zoom_depth(), 1);
        // Nivel 2: dentro del grupo, plegar SU pila (3,4) — la maestra del nivel
        // es la 2 — y entrar otra vez.
        d.apply(DesktopAction::GroupStack);
        d.apply(DesktopAction::FocusWindow(4));
        let cmds = d.apply(DesktopAction::ZoomIn);
        assert_eq!(d.active_workspace().zoom_depth(), 2);
        // En el nivel más profundo sólo se ven 3 y 4; 1 y 2 duermen.
        let p = places(&cmds);
        let visibles: Vec<_> = p.iter().filter(|p| p.visible).map(|p| p.id).collect();
        assert_eq!(visibles.len(), 2);
        assert!(visibles.contains(&3) && visibles.contains(&4));
        for id in [1, 2] {
            assert!(p.iter().find(|p| p.id == id).unwrap().suspended);
        }
    }

    #[test]
    fn a_snapshot_round_trips_through_restore() {
        let mut d = desktop_with_screen();
        d.apply(DesktopAction::SetLayout(LayoutMode::CenteredMaster));
        let snap = d.snapshot();
        let mut d2 = Desktop::new();
        d2.restore(&snap);
        assert_eq!(d2.snapshot().workspaces, snap.workspaces);
    }
}
