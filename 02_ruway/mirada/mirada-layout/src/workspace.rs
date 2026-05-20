//! `Workspace` — un conjunto de ventanas, su foco y su modo de teselado.

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;
use crate::layout::{tile, LayoutMode, LayoutParams};

/// Identificador de una ventana (una superficie Wayland).
pub type WindowId = u64;

/// Un escritorio: ventanas en orden de teselado + la enfocada + el modo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    /// Ventanas en orden de teselado (la 0 es la maestra en `MasterStack`).
    windows: Vec<WindowId>,
    /// Índice de la ventana enfocada en `windows`.
    focus: usize,
    params: LayoutParams,
}

impl Workspace {
    /// Escritorio vacío con los parámetros dados.
    pub fn new(params: LayoutParams) -> Self {
        Self { windows: Vec::new(), focus: 0, params }
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Ventanas en orden de teselado.
    pub fn windows(&self) -> &[WindowId] {
        &self.windows
    }

    pub fn params(&self) -> &LayoutParams {
        &self.params
    }

    /// Cambia el modo de teselado.
    pub fn set_mode(&mut self, mode: LayoutMode) {
        self.params.mode = mode;
    }

    /// Ajusta la fracción de la ventana maestra.
    pub fn set_master_ratio(&mut self, ratio: f32) {
        self.params.master_ratio = ratio;
    }

    /// Añade una ventana y la enfoca. Si ya estaba, sólo la enfoca.
    pub fn add(&mut self, window: WindowId) {
        if let Some(i) = self.windows.iter().position(|&w| w == window) {
            self.focus = i;
        } else {
            self.windows.push(window);
            self.focus = self.windows.len() - 1;
        }
    }

    /// Quita una ventana. `false` si no estaba. El foco se reajusta para
    /// seguir apuntando a una ventana válida.
    pub fn remove(&mut self, window: WindowId) -> bool {
        let Some(i) = self.windows.iter().position(|&w| w == window) else {
            return false;
        };
        self.windows.remove(i);
        if i < self.focus {
            self.focus -= 1;
        }
        if self.focus >= self.windows.len() {
            self.focus = self.windows.len().saturating_sub(1);
        }
        true
    }

    /// Ventana enfocada, o `None` si el escritorio está vacío.
    pub fn focused(&self) -> Option<WindowId> {
        self.windows.get(self.focus).copied()
    }

    /// Mueve el foco a la ventana siguiente (cíclico).
    pub fn focus_next(&mut self) {
        if !self.windows.is_empty() {
            self.focus = (self.focus + 1) % self.windows.len();
        }
    }

    /// Mueve el foco a la ventana anterior (cíclico).
    pub fn focus_prev(&mut self) {
        if !self.windows.is_empty() {
            self.focus = (self.focus + self.windows.len() - 1) % self.windows.len();
        }
    }

    /// Enfoca una ventana por id. `false` si no está en el escritorio.
    pub fn focus_window(&mut self, window: WindowId) -> bool {
        match self.windows.iter().position(|&w| w == window) {
            Some(i) => {
                self.focus = i;
                true
            }
            None => false,
        }
    }

    /// Intercambia la ventana enfocada con la siguiente en el orden de
    /// teselado; el foco la acompaña. No hace nada si ya es la última.
    pub fn move_focused_forward(&mut self) {
        if self.focus + 1 < self.windows.len() {
            self.windows.swap(self.focus, self.focus + 1);
            self.focus += 1;
        }
    }

    /// Intercambia la ventana enfocada con la anterior. No hace nada si
    /// ya es la primera.
    pub fn move_focused_backward(&mut self) {
        if self.focus > 0 && !self.windows.is_empty() {
            self.windows.swap(self.focus, self.focus - 1);
            self.focus -= 1;
        }
    }

    /// Resuelve la geometría: el rectángulo de cada ventana dentro de
    /// `screen`, en orden de teselado.
    pub fn layout(&self, screen: Rect) -> Vec<(WindowId, Rect)> {
        let rects = tile(screen, self.windows.len(), &self.params);
        self.windows.iter().copied().zip(rects).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> Workspace {
        Workspace::new(LayoutParams::default())
    }

    #[test]
    fn add_focuses_the_new_window() {
        let mut w = ws();
        w.add(10);
        w.add(20);
        assert_eq!(w.focused(), Some(20));
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn adding_an_existing_window_just_focuses_it() {
        let mut w = ws();
        w.add(10);
        w.add(20);
        w.add(10);
        assert_eq!(w.focused(), Some(10));
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn focus_cycles_both_ways() {
        let mut w = ws();
        for id in [1, 2, 3] {
            w.add(id);
        }
        assert_eq!(w.focused(), Some(3));
        w.focus_next();
        assert_eq!(w.focused(), Some(1)); // dio la vuelta
        w.focus_prev();
        assert_eq!(w.focused(), Some(3));
    }

    #[test]
    fn remove_keeps_focus_valid() {
        let mut w = ws();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.focus_window(2);
        w.remove(2);
        // El foco se mantiene dentro de rango.
        assert!(w.focused().is_some());
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn remove_before_focus_shifts_it() {
        let mut w = ws();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.focus_window(3); // focus = 2
        w.remove(1); // quita una anterior
        assert_eq!(w.focused(), Some(3)); // sigue enfocada la 3
    }

    #[test]
    fn remove_last_window_empties_workspace() {
        let mut w = ws();
        w.add(7);
        assert!(w.remove(7));
        assert!(w.is_empty());
        assert_eq!(w.focused(), None);
    }

    #[test]
    fn move_focused_reorders_tiling() {
        let mut w = ws();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.focus_window(1); // primera
        w.move_focused_forward();
        assert_eq!(w.windows(), &[2, 1, 3]);
        assert_eq!(w.focused(), Some(1)); // el foco la acompañó
        w.move_focused_backward();
        assert_eq!(w.windows(), &[1, 2, 3]);
    }

    #[test]
    fn layout_pairs_each_window_with_a_rect() {
        let mut w = ws();
        for id in [100, 200, 300] {
            w.add(id);
        }
        let screen = Rect::new(0, 0, 1920, 1080);
        let placed = w.layout(screen);
        assert_eq!(placed.len(), 3);
        let ids: Vec<_> = placed.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![100, 200, 300]);
    }

    #[test]
    fn empty_workspace_lays_out_nothing() {
        assert!(ws().layout(Rect::new(0, 0, 800, 600)).is_empty());
    }
}
