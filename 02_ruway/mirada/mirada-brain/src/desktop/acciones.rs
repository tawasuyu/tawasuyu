//! Aplicación de acciones de escritorio y helpers de layout/navegación.

use mirada_layout::{Rect, WindowId};
use mirada_protocol::{placements, BrainCommand};

use crate::action::{DesktopAction, Direction};
use crate::config::DROPTERM_APP_ID;

use super::estado::Desktop;
use super::geometria::{
    cascaded_float_rect, centered_float_rect, dropdown_rect, nearest_in_direction,
};

impl Desktop {
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
                // No está en ningún escritorio: quizás está MINIMIZADA (en el
                // scratchpad/especial). La rescatamos al escritorio activo y la
                // enfocamos — así el taskicon "des-minimiza". Sin esto, una
                // ventana minimizada quedaba irrecuperable (ni taskicon ni
                // alt-tab la veían).
                if self.windows.contains_key(&id)
                    && self.specials.values().any(|b| b.contains(&id))
                {
                    self.forget_special_window(id);
                    self.workspaces[active].add(id); // `add` la enfoca
                    return self.relayout();
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
            DesktopAction::CloseWindow(id) => {
                // Cierre por id (clic derecho del taskbar): sólo si la ventana
                // existe —en algún escritorio o en el scratchpad—. El estado se
                // actualiza al recibir el `WindowClosed`, igual que CloseFocused.
                let existe = self.windows.contains_key(&id);
                if existe {
                    vec![BrainCommand::Close(id)]
                } else {
                    Vec::new()
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
                    // Escalonar sobre los flotantes ya presentes: no apilarlos
                    // todos en el mismo centro.
                    let n = ws
                        .windows()
                        .iter()
                        .filter(|&&w| w != id && ws.is_floating(w))
                        .count();
                    let rect = screen
                        .map(|s| cascaded_float_rect(s, n))
                        .unwrap_or_else(|| Rect::new(100, 100, 800, 600));
                    ws.set_floating(id, Some(rect));
                }
                self.relayout()
            }
            DesktopAction::ToggleTiling => {
                let screen = self.screen();
                let ws = &mut self.workspaces[active];
                // Las teseladas: las no flotantes que sí están en el orden.
                let tiled: Vec<_> = ws
                    .windows()
                    .iter()
                    .copied()
                    .filter(|&id| !ws.is_floating(id))
                    .collect();
                if tiled.is_empty() {
                    // Todas flotando ya: devolverlas al teselado.
                    let floating: Vec<_> = ws.windows().to_vec();
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
            DesktopAction::ToggleMaximize => {
                let Some(id) = self.workspaces[active].focused() else {
                    return Vec::new();
                };
                let work = self.outputs.get(self.focused_output).map(|o| o.work_rect());
                let ws = &mut self.workspaces[active];
                // "Maximizada" = flotando ocupando toda el área de trabajo.
                // Conserva la barra de título (no es pantalla completa), así el
                // mismo botón la restaura y no se "apropia" del escritorio.
                let maximizada = ws
                    .floating_rect(id)
                    .zip(work)
                    .is_some_and(|(r, w)| r == w);
                if maximizada {
                    ws.set_floating(id, None); // restaurar al teselado
                } else if let Some(w) = work {
                    ws.set_floating(id, Some(w));
                }
                self.relayout()
            }
            // El scratchpad clásico es el escritorio especial por defecto ("").
            DesktopAction::SendToScratchpad => self.stash_focused_to_special(""),
            DesktopAction::ToggleScratchpad => self.toggle_special(""),
            DesktopAction::MoveToSpecialWorkspace(name) => {
                self.stash_focused_to_special(&name)
            }
            DesktopAction::ToggleSpecialWorkspace(name) => self.toggle_special(&name),
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
                            let bucket = self.specials.entry(String::new()).or_default();
                            if !bucket.contains(&id) {
                                bucket.push(id);
                            }
                        } else {
                            for ws in &mut self.workspaces {
                                ws.remove(id);
                            }
                            self.forget_special_window(id);
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
                let stack: Vec<_> = ws.view_leaves().into_iter().skip(nmaster).collect();
                ws.group(&stack);
                self.relayout()
            }
            DesktopAction::GroupConstellation => {
                let ws = &self.workspaces[active];
                let Some(focused) = ws.focused() else {
                    return Vec::new();
                };
                // La constelación de la enfocada, restringida a las hojas sueltas
                // del nivel en vista (lo que `group` puede plegar). Con una sola
                // ventana, `group` no hace nada.
                let members = self.activity.constellation_of(focused, &ws.view_leaves());
                self.workspaces[active].group(&members);
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
            DesktopAction::FocusConstellationNext => self.focus_constellation(true),
            DesktopAction::FocusConstellationPrev => self.focus_constellation(false),
            DesktopAction::SwitchWorkspace(n) => {
                if n < self.workspaces.len() && n != active {
                    self.show_workspace(n);
                    self.relayout()
                } else {
                    Vec::new()
                }
            }
            DesktopAction::WorkspaceNext | DesktopAction::WorkspacePrev => {
                // Win+Tab: salta al siguiente escritorio **ocupado** (con wrap),
                // no a los vacíos — vagar por escritorios vacíos invisibles no
                // sirve; la convención es ciclar entre los activos. Si ningún
                // otro está ocupado, no hace nada. El modo de transición
                // (`Config::workspace_switch_mode`) gobernará la animación; hoy
                // sólo `Direct` está cableado → salto seco.
                let n = self.workspaces.len();
                if n <= 1 {
                    return Vec::new();
                }
                let forward = matches!(action, DesktopAction::WorkspaceNext);
                let mut idx = active;
                for _ in 0..n {
                    idx = if forward {
                        (idx + 1) % n
                    } else {
                        (idx + n - 1) % n
                    };
                    if idx == active {
                        break;
                    }
                    if self.workspaces[idx].len() > 0 {
                        self.show_workspace(idx);
                        return self.relayout();
                    }
                }
                Vec::new()
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
            DesktopAction::Lock => vec![BrainCommand::Lock],
            DesktopAction::Logout => vec![BrainCommand::Logout],
        }
    }

    // --- Helpers privados de navegación y layout ---

    /// Hace que la salida enfocada muestre el escritorio `n`. Un escritorio
    /// vive en un solo monitor a la vez: si **otra** salida ya muestra `n`,
    /// no se lo robamos (eso arrastraba sus ventanas de un monitor a otro y
    /// confundía — «cambié de escritorio y aparecieron ventanas de otro»);
    /// en su lugar movemos el **foco** a esa salida. Si no lo muestra nadie,
    /// la salida enfocada pasa a mostrarlo.
    pub(super) fn show_workspace(&mut self, n: usize) {
        if n >= self.workspaces.len() || self.focused_output >= self.outputs.len() {
            return;
        }
        let current = self.outputs[self.focused_output].workspace;
        if current == n {
            return;
        }
        if let Some(other) = self.outputs.iter().position(|o| o.workspace == n) {
            self.focused_output = other;
            return;
        }
        self.outputs[self.focused_output].workspace = n;
    }

    // --- Escritorios especiales (estilo Hyprland) ---

    /// Las ventanas de un escritorio especial (vacío si no existe).
    fn special_windows(&self, name: &str) -> Vec<WindowId> {
        self.specials.get(name).cloned().unwrap_or_default()
    }

    /// Olvida una ventana de TODOS los especiales (al cerrarse, o al volver a
    /// un escritorio normal). Limpia los buckets que queden vacíos.
    pub(super) fn forget_special_window(&mut self, id: WindowId) {
        for bucket in self.specials.values_mut() {
            bucket.retain(|&w| w != id);
        }
        self.specials.retain(|_, v| !v.is_empty());
    }

    /// Manda la ventana enfocada del escritorio activo a un especial `name`
    /// (`""` = scratchpad): la saca de todo escritorio normal y la aparta.
    pub(super) fn stash_focused_to_special(&mut self, name: &str) -> Vec<BrainCommand> {
        let active = self.active_index();
        let Some(id) = self.workspaces[active].focused() else {
            return Vec::new();
        };
        for ws in &mut self.workspaces {
            ws.remove(id);
        }
        let bucket = self.specials.entry(name.to_string()).or_default();
        if !bucket.contains(&id) {
            bucket.push(id);
        }
        self.relayout()
    }

    /// Muestra/oculta TODAS las ventanas de un especial como overlay flotante
    /// sobre el escritorio activo. Si alguna está a la vista, las guarda todas;
    /// si no, trae las apartadas (en cascada para que no se tapen).
    pub(super) fn toggle_special(&mut self, name: &str) -> Vec<BrainCommand> {
        let active = self.active_index();
        let members = self.special_windows(name);
        if members.is_empty() {
            return Vec::new();
        }
        let shown: Vec<WindowId> = members
            .iter()
            .copied()
            .filter(|id| self.workspaces[active].windows().contains(id))
            .collect();
        if !shown.is_empty() {
            // A la vista en el activo → guardar: salen y vuelven a quedar apartadas.
            for id in shown {
                self.workspaces[active].remove(id);
            }
        } else {
            // No están en el activo → invocarlas flotando, en cascada. Si alguna
            // está visible en OTRO escritorio, la traemos (sigue al puntero, como
            // el scratchpad clásico): la sacamos de donde esté y la ponemos acá.
            let base = self
                .screen()
                .map(centered_float_rect)
                .unwrap_or_else(|| Rect::new(100, 100, 800, 600));
            for (i, id) in members.into_iter().enumerate() {
                for ws in &mut self.workspaces {
                    ws.remove(id);
                }
                let off = (i as i32) * 32;
                let rect = Rect::new(base.x + off, base.y + off, base.w, base.h);
                self.workspaces[active].add(id);
                self.workspaces[active].set_floating(id, Some(rect));
            }
        }
        self.relayout()
    }

    /// Recoloca las salidas en su orden de aparición según la **dirección** de
    /// la config (horizontal por defecto, o vertical). Es sólo el arreglo
    /// provisional: el Cuerpo —que conoce nombres y `order`— reafirma el origen
    /// real de cada monitor con [`BodyEvent::OutputMoved`], que manda. Sin un
    /// backend que lo corrija (tests, simulación), este arreglo es el efectivo.
    pub(super) fn reflow_outputs(&mut self) {
        let vertical = matches!(
            self.config.output_disposition(),
            mirada_layout::Disposicion::Vertical
        );
        let mut avance = 0;
        for o in &mut self.outputs {
            if vertical {
                o.rect.x = 0;
                o.rect.y = avance;
                avance += o.rect.h;
            } else {
                o.rect.x = avance;
                o.rect.y = 0;
                avance += o.rect.w;
            }
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
    fn output_in_direction(&self, dir: Direction) -> Option<mirada_protocol::OutputId> {
        if self.outputs.len() < 2 {
            return None;
        }
        let cur = self.outputs.get(self.focused_output)?;
        let cands: Vec<(mirada_protocol::OutputId, Rect)> =
            self.outputs.iter().map(|o| (o.id, o.rect)).collect();
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
        let tiled: Vec<_> = layout
            .into_iter()
            .filter(|(id, _)| !self.workspaces[active].is_floating(*id))
            .collect();
        match nearest_in_direction(from, &tiled, focused, dir) {
            Some(target) if self.workspaces[active].swap(focused, target) => self.relayout(),
            _ => Vec::new(),
        }
    }

    /// Salta el foco a la constelación vecina (la siguiente con `forward`, la
    /// anterior si no) del escritorio activo: el "alt-tab" por familia de
    /// actividad. Enfoca el primer miembro de la constelación destino. No hace
    /// nada si hay menos de dos constelaciones (nada entre lo que saltar).
    fn focus_constellation(&mut self, forward: bool) -> Vec<BrainCommand> {
        let active = self.active_index();
        let Some(focused) = self.workspaces[active].focused() else {
            return Vec::new();
        };
        let ids: Vec<_> = self.workspaces[active].windows().to_vec();
        let consts = self.activity.constellations(&ids);
        if consts.len() < 2 {
            return Vec::new();
        }
        let cur = consts.iter().position(|c| c.contains(&focused)).unwrap_or(0);
        let n = consts.len();
        let next = if forward { (cur + 1) % n } else { (cur + n - 1) % n };
        match consts[next].first() {
            Some(&target) => {
                self.workspaces[active].focus_window(target);
                self.relayout()
            }
            None => Vec::new(),
        }
    }

    /// Recalcula la geometría de **todas** las salidas y la empaqueta en
    /// un único [`BrainCommand::Place`]. Sin salidas, no hay nada que
    /// colocar.
    pub(super) fn relayout(&self) -> Vec<BrainCommand> {
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
        // Throttle de fondo: las ventanas visibles pero sin foco (y teseladas:
        // ni flotantes ni fullscreen, que suelen ser el vídeo/PiP activo) pintan
        // a 1 de cada N vblanks, en vez de quemar GPU a 60 Hz detrás del foco.
        // Apagado por defecto (divisor 1). Las dormidas ya tienen el frame
        // cortado del todo, así que no se tocan.
        let bg_divisor = self.config.background_frame_divisor.max(1);
        if bg_divisor > 1 {
            for p in &mut all {
                if p.visible && !p.focused && !p.floating && !p.fullscreen && !p.suspended {
                    p.frame_divisor = bg_divisor;
                }
            }
        }
        vec![BrainCommand::Place(all)]
    }
}
