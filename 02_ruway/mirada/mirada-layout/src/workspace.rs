//! `Workspace` — un conjunto de ventanas, su foco y su modo de teselado.

use std::collections::BTreeMap;

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
    /// Ventanas flotantes y su rectángulo: salen del teselado y se pintan
    /// encima. Las que no están aquí se teselan normalmente.
    floating: BTreeMap<WindowId, Rect>,
}

impl Workspace {
    /// Escritorio vacío con los parámetros dados.
    pub fn new(params: LayoutParams) -> Self {
        Self {
            windows: Vec::new(),
            focus: 0,
            params,
            floating: BTreeMap::new(),
        }
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

    /// Ajusta cuántas ventanas van en el área maestra (`nmaster`).
    pub fn set_master_count(&mut self, count: usize) {
        self.params.master_count = count;
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
        self.floating.remove(&window);
        if i < self.focus {
            self.focus -= 1;
        }
        if self.focus >= self.windows.len() {
            self.focus = self.windows.len().saturating_sub(1);
        }
        true
    }

    /// Marca una ventana como flotante en `rect`, o la devuelve al
    /// teselado con `None`. La ventana sigue en el orden de foco.
    pub fn set_floating(&mut self, window: WindowId, rect: Option<Rect>) {
        match rect {
            Some(r) => {
                self.floating.insert(window, r);
            }
            None => {
                self.floating.remove(&window);
            }
        }
    }

    /// `true` si la ventana está flotando.
    pub fn is_floating(&self, window: WindowId) -> bool {
        self.floating.contains_key(&window)
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

    /// Lleva la ventana enfocada al primer puesto del orden de teselado
    /// (la posición maestra); el foco la acompaña. No hace nada si ya es
    /// la primera o el escritorio está vacío.
    pub fn promote_focused(&mut self) {
        if self.focus > 0 && self.focus < self.windows.len() {
            let w = self.windows.remove(self.focus);
            self.windows.insert(0, w);
            self.focus = 0;
        }
    }

    /// Resuelve la geometría: el rectángulo de cada ventana dentro de
    /// `screen`. Primero las teseladas en orden de teselado, luego las
    /// flotantes con su propio rectángulo — éstas van al final para que
    /// el Cuerpo las pinte encima.
    pub fn layout(&self, screen: Rect) -> Vec<(WindowId, Rect)> {
        let tiled: Vec<WindowId> = self
            .windows
            .iter()
            .copied()
            .filter(|id| !self.floating.contains_key(id))
            .collect();
        let rects = tile(screen, tiled.len(), &self.params);
        let mut out: Vec<(WindowId, Rect)> = tiled.into_iter().zip(rects).collect();
        for &id in &self.windows {
            if let Some(&rect) = self.floating.get(&id) {
                out.push((id, rect));
            }
        }
        out
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
    fn promote_brings_the_focused_window_to_the_front() {
        let mut w = ws();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.focus_window(3);
        w.promote_focused();
        assert_eq!(w.windows(), &[3, 1, 2]);
        assert_eq!(w.focused(), Some(3));
        // Promover la que ya es maestra no hace nada.
        w.promote_focused();
        assert_eq!(w.windows(), &[3, 1, 2]);
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

    #[test]
    fn a_floating_window_keeps_its_rect_and_goes_last() {
        let mut w = ws();
        for id in [1, 2, 3] {
            w.add(id);
        }
        let float_rect = Rect::new(50, 50, 400, 300);
        w.set_floating(2, Some(float_rect));
        assert!(w.is_floating(2));
        let placed = w.layout(Rect::new(0, 0, 1920, 1080));
        assert_eq!(placed.len(), 3);
        // La flotante va al final, con su rectángulo intacto.
        assert_eq!(placed[2], (2, float_rect));
        let ids: Vec<_> = placed.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![1, 3, 2]);
        // Devolverla al teselado.
        w.set_floating(2, None);
        assert!(!w.is_floating(2));
        assert_eq!(w.layout(Rect::new(0, 0, 1920, 1080)).len(), 3);
    }

    #[test]
    fn removing_a_window_clears_its_floating_state() {
        let mut w = ws();
        w.add(1);
        w.set_floating(1, Some(Rect::new(0, 0, 100, 100)));
        w.remove(1);
        w.add(1); // mismo id, ventana nueva: ya no flota
        assert!(!w.is_floating(1));
    }
}
