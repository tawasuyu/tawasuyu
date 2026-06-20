//! Manejador de eventos del Cuerpo → [`Desktop::on_event`].

use mirada_layout::Rect;
use mirada_protocol::{BodyEvent, BrainCommand};

use crate::config::DROPTERM_APP_ID;

use super::estado::Desktop;
use super::geometria::{centered_float_rect, dropdown_rect};
use super::tipos::WindowInfo;

impl Desktop {
    /// Procesa un evento del Cuerpo: muta el estado y devuelve los
    /// comandos a enviar de vuelta.
    pub fn on_event(&mut self, event: BodyEvent) -> Vec<BrainCommand> {
        match event {
            // El salto de escritorio (Win+Tab en modo enlazado) lo aplica
            // mirada-app en su `feed` antes de delegar acá; el Desktop no lo ve.
            BodyEvent::SwitchWorkspace(_) => Vec::new(),
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
                self.outputs.push(super::tipos::Output {
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
                    let mut rect = self
                        .screen()
                        .map(centered_float_rect)
                        .unwrap_or_else(|| Rect::new(100, 100, 800, 600));
                    // Regla con `size`: respeta el tamaño pedido, centrado en pantalla.
                    if let Some((w, h)) = outcome.size {
                        let (w, h) = (w.max(1), h.max(1));
                        if let Some(s) = self.screen() {
                            rect = Rect::new(
                                s.x + (s.w - w).max(0) / 2,
                                s.y + (s.h - h).max(0) / 2,
                                w,
                                h,
                            );
                        } else {
                            rect = Rect::new(rect.x, rect.y, w, h);
                        }
                    }
                    self.workspaces[ws].set_floating(id, Some(rect));
                }
                // Regla `fullscreen`: abre la ventana a pantalla completa.
                if outcome.fullscreen {
                    self.workspaces[ws].set_fullscreen(Some(id));
                }
                // Si este escritorio tenía una agrupación guardada esperando a sus
                // apps, quizás esta ventana completa el cuadro.
                self.try_restore_grouping(ws);
                self.relayout()
            }
            BodyEvent::WindowLineage { id, pid, ancestors } => {
                // Sólo contabilidad para las constelaciones: no cambia geometría.
                self.activity.record(id, pid, ancestors);
                Vec::new()
            }
            BodyEvent::WindowClosed { id } => {
                self.windows.remove(&id);
                self.activity.forget(id);
                self.forget_special_window(id);
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
                use crate::action::DesktopAction;
                self.apply(DesktopAction::FocusWindow(id))
            }
            BodyEvent::WindowDragged { id, x, y } => {
                // Arrastre soltado sobre el mosaico. Dos casos:
                //  · teselada → la intercambia con la tesela bajo el puntero.
                //  · flotante → **vuelve al mosaico** en el lugar de esa tesela
                //    (el fix de «si muevo una ventana no vuelve al tile jamás»).
                //    Soltada sobre vacío, sigue flotando (no se toca).
                let active = self.active_index();
                if !self.workspaces[active].windows().contains(&id) {
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
                if self.workspaces[active].is_floating(id) {
                    match target {
                        Some(t) => {
                            self.workspaces[active].set_floating(id, None);
                            self.workspaces[active].swap(id, t);
                            self.relayout()
                        }
                        None => Vec::new(),
                    }
                } else {
                    match target {
                        Some(t) if self.workspaces[active].swap(id, t) => self.relayout(),
                        _ => Vec::new(),
                    }
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
}
