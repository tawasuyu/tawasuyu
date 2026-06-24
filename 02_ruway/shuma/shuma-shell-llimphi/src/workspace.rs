//! `workspace` — el layout tipo **zellij** del canvas de una sesión.
//!
//! Una sesión ya no hospeda *un* shell: hospeda un [`Workspace`] con varias
//! **tabs**, cada una con un árbol **tiling** de paneles (BSP, vía
//! `llimphi-widget-panes`) y una capa de paneles **pseudo-flotantes** que se
//! superponen al tiling. Cada panel —tiled o flotante— es una `Instance` de
//! `shuma-module-shell` viva e independiente (cada una con su PTY, su cwd, su
//! historial).
//!
//! ## Modelo
//!
//! - [`Workspace`] tiene `tabs: Vec<WsTab>` y la tab activa.
//! - [`WsTab`] tiene:
//!   - `panes: HashMap<PaneId, Instance>` — **todos** los paneles de la tab
//!     (tiled + flotantes), por id. Única fuente de verdad del contenido.
//!   - `layout: Layout` — el árbol BSP, que sólo referencia ids **tiled**.
//!   - `floating: Vec<FloatPane>` — geometría (id + rect en px) de los ids que
//!     están en la capa flotante (NO viven en `layout`).
//!   - `focused: PaneId` — el panel con foco (puede ser tiled o flotante); sus
//!     teclas las recibe el chasis.
//!   - `show_floating` — si la capa flotante se pinta arriba del tiling.
//!
//! **Invariante:** siempre hay ≥1 tab, cada tab tiene ≥1 panel, y `focused`
//! existe en `panes`. Las operaciones lo mantienen. `Session::shell()` devuelve
//! `panes[focused]` de la tab activa — por eso el resto del chasis (teclado,
//! cwd, input hospedado…) sigue operando sobre "el shell" sin enterarse de que
//! hay tiling.
//!
//! Las ops que **crean** un panel reciben la `Instance` ya construida desde el
//! caller (`update.rs`), porque armar un shell necesita el `Source`/nombre de
//! la sesión — detalle que este módulo no conoce a propósito.

use std::collections::HashMap;

use llimphi_widget_panes::{Axis, Layout, PaneId, Side};

use crate::types::Instance;

/// Geometría de un panel flotante, en px relativos al canvas.
#[derive(Debug, Clone)]
pub(crate) struct FloatPane {
    pub id: PaneId,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Una tab del workspace: un árbol tiling + su capa flotante.
pub(crate) struct WsTab {
    pub name: String,
    pub layout: Layout,
    pub focused: PaneId,
    pub panes: HashMap<PaneId, Instance>,
    pub floating: Vec<FloatPane>,
    pub show_floating: bool,
}

impl WsTab {
    fn single(name: String, id: PaneId, inst: Instance) -> Self {
        let mut panes = HashMap::new();
        panes.insert(id, inst);
        Self {
            name,
            layout: Layout::single(id),
            focused: id,
            panes,
            floating: Vec::new(),
            show_floating: false,
        }
    }

    /// `true` si `id` está en la capa flotante.
    pub fn is_floating(&self, id: PaneId) -> bool {
        self.floating.iter().any(|f| f.id == id)
    }

    /// Reasigna el foco a un panel tiled válido (el primero del árbol).
    fn refocus_tiled(&mut self) {
        self.focused = self.layout.first_leaf();
    }

    /// Estado de actividad agregado de la tab para el aviso visual: claude tiene
    /// prioridad, luego movimiento (algún panel corriendo), si no quieto. Mira
    /// **todos** los paneles, así una tab inactiva igual avisa que algo pasa.
    pub fn activity(&self) -> shuma_module_shell::Activity {
        use shuma_module_shell::Activity;
        let mut acc = Activity::Idle;
        for inst in self.panes.values() {
            if let crate::types::ModuleState::Shell(s) = &inst.state {
                match s.activity() {
                    Activity::Claude => return Activity::Claude,
                    Activity::Busy => acc = Activity::Busy,
                    Activity::Idle => {}
                }
            }
        }
        acc
    }
}

/// El layout completo de una sesión.
pub(crate) struct Workspace {
    pub tabs: Vec<WsTab>,
    pub active_tab: usize,
    /// Contador monótono de ids de panel (único por workspace).
    pub next_id: PaneId,
}

impl Workspace {
    /// Workspace de un solo panel (el shell inicial de la sesión).
    pub fn single(inst: Instance) -> Self {
        Self {
            tabs: vec![WsTab::single("1".to_string(), 0, inst)],
            active_tab: 0,
            next_id: 1,
        }
    }

    fn fresh_id(&mut self) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ─── Lectura ────────────────────────────────────────────────────

    pub fn tab(&self) -> &WsTab {
        // `active_tab` se mantiene en rango; clamp defensivo igual.
        let i = self.active_tab.min(self.tabs.len().saturating_sub(1));
        &self.tabs[i]
    }

    pub fn tab_mut(&mut self) -> &mut WsTab {
        let i = self.active_tab.min(self.tabs.len().saturating_sub(1));
        &mut self.tabs[i]
    }

    /// El panel con foco de la tab activa (infalible por invariante).
    pub fn focused_instance(&self) -> &Instance {
        let t = self.tab();
        t.panes
            .get(&t.focused)
            .or_else(|| t.panes.values().next())
            .expect("workspace: toda tab tiene ≥1 panel")
    }

    pub fn focused_instance_mut(&mut self) -> &mut Instance {
        let t = self.tab_mut();
        if t.panes.contains_key(&t.focused) {
            t.panes.get_mut(&t.focused).unwrap()
        } else {
            t.panes.values_mut().next().expect("workspace: toda tab tiene ≥1 panel")
        }
    }

    /// Instancia de un panel concreto de la tab activa.
    pub fn pane(&self, id: PaneId) -> Option<&Instance> {
        self.tab().panes.get(&id)
    }

    pub fn pane_mut(&mut self, id: PaneId) -> Option<&mut Instance> {
        self.tab_mut().panes.get_mut(&id)
    }

    /// Visita **toda** instancia de panel de **todas** las tabs (para drenar
    /// el output streamed: los paneles de fondo también producen salida).
    pub fn for_each_pane_mut(&mut self, mut f: impl FnMut(&mut Instance)) {
        for t in &mut self.tabs {
            for inst in t.panes.values_mut() {
                f(inst);
            }
        }
    }

    // ─── Tiling ─────────────────────────────────────────────────────

    /// Parte el panel con foco en dos. El panel nuevo (`inst`) toma el foco.
    /// `axis` `Horizontal` = lado a lado; `Vertical` = apilado.
    pub fn split(&mut self, axis: Axis, inst: Instance) {
        let id = self.fresh_id();
        let t = self.tab_mut();
        // Sólo se parte un panel tiled. Si el foco está en un flotante,
        // partimos el primer tiled del árbol (comportamiento simple y
        // predecible para el MVP).
        let target = if t.is_floating(t.focused) {
            t.layout.first_leaf()
        } else {
            t.focused
        };
        if t.layout.split(target, id, axis) {
            t.panes.insert(id, inst);
            t.focused = id;
        }
    }

    /// Pone el foco en `id` (si existe en la tab activa).
    pub fn focus(&mut self, id: PaneId) {
        let t = self.tab_mut();
        if t.panes.contains_key(&id) {
            t.focused = id;
            // Enfocar un flotante lo trae al frente y enciende la capa.
            if let Some(pos) = t.floating.iter().position(|f| f.id == id) {
                let f = t.floating.remove(pos);
                t.floating.push(f);
                t.show_floating = true;
            }
        }
    }

    /// Mueve el foco al siguiente / anterior panel tiled (ciclo).
    pub fn cycle_focus(&mut self, forward: bool) {
        let t = self.tab_mut();
        let ids = t.layout.leaves();
        if ids.is_empty() {
            return;
        }
        let cur = ids.iter().position(|x| *x == t.focused);
        let n = ids.len();
        let next = match cur {
            Some(i) if forward => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
            None => 0,
        };
        t.focused = ids[next];
    }

    /// Cierra el panel con foco. No-op si es el último panel de la última tab.
    /// Devuelve la `Instance` removida (el caller la deja caer / la usa para
    /// matar el PTY si hiciera falta).
    pub fn close_focused(&mut self) -> Option<Instance> {
        // ¿Es el último panel del workspace entero? Entonces no se cierra.
        let total: usize = self.tabs.iter().map(|t| t.panes.len()).sum();
        if total <= 1 {
            return None;
        }
        let i = self.active_tab.min(self.tabs.len().saturating_sub(1));
        let victim = self.tabs[i].focused;
        let was_floating = self.tabs[i].is_floating(victim);

        if was_floating {
            let t = &mut self.tabs[i];
            t.floating.retain(|f| f.id != victim);
            let inst = t.panes.remove(&victim);
            // Nuevo foco: otro flotante arriba, o un tiled.
            t.focused = t.floating.last().map(|f| f.id).unwrap_or_else(|| t.layout.first_leaf());
            return inst;
        }

        // Tiled: si la tab tiene un solo panel tiled (y nada flotante), cerrar
        // ese panel cierra la tab entera (si hay más de una tab).
        let only_tiled = self.tabs[i].layout.count();
        if only_tiled <= 1 {
            if self.tabs[i].floating.is_empty() {
                // Cerrar la tab (hay >1 porque total>1 y esta sólo tiene 1 panel).
                if self.tabs.len() > 1 {
                    let mut tab = self.tabs.remove(i);
                    self.active_tab = self.active_tab.min(self.tabs.len() - 1);
                    return tab.panes.drain().map(|(_, v)| v).next();
                }
                return None;
            } else {
                // Hay flotantes: el último tiled no se puede quitar del árbol
                // (Layout no soporta árbol vacío); pasamos el foco a un flotante.
                let t = &mut self.tabs[i];
                t.focused = t.floating.last().map(|f| f.id).unwrap();
                t.show_floating = true;
                return None;
            }
        }

        // Caso normal: sacar el panel del árbol y recolapsar.
        let t = &mut self.tabs[i];
        let layout = std::mem::replace(&mut t.layout, Layout::single(victim));
        let (new_layout, removed) = layout.without(victim);
        t.layout = new_layout;
        if removed {
            let inst = t.panes.remove(&victim);
            t.refocus_tiled();
            inst
        } else {
            None
        }
    }

    /// Ajusta el ratio del split direccionado por `path`.
    pub fn resize(&mut self, path: &[Side], delta: f32) {
        self.tab_mut().layout.resize(path, delta);
    }

    // ─── Tabs ───────────────────────────────────────────────────────

    /// Crea una tab nueva con un único panel (`inst`) y la activa.
    pub fn new_tab(&mut self, inst: Instance) {
        let id = self.fresh_id();
        let name = (self.tabs.len() + 1).to_string();
        self.tabs.push(WsTab::single(name, id, inst));
        self.active_tab = self.tabs.len() - 1;
    }

    pub fn switch_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.active_tab = i;
        }
    }

    /// Cierra la tab `i`. No-op si es la única. Devuelve sus instancias para
    /// que el caller las deje caer.
    pub fn close_tab(&mut self, i: usize) -> Vec<Instance> {
        if self.tabs.len() <= 1 || i >= self.tabs.len() {
            return Vec::new();
        }
        let tab = self.tabs.remove(i);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if self.active_tab > i {
            self.active_tab -= 1;
        }
        tab.panes.into_values().collect()
    }

    /// Cierra todas las tabs menos la `keep`, que pasa a ser la activa.
    /// Devuelve las instancias de las tabs cerradas para que el caller las
    /// deje caer. No-op (vector vacío) si `keep` no existe o es la única.
    pub fn close_others(&mut self, keep: usize) -> Vec<Instance> {
        if keep >= self.tabs.len() || self.tabs.len() <= 1 {
            return Vec::new();
        }
        let mut dropped = Vec::new();
        let kept = self.tabs.remove(keep);
        for tab in self.tabs.drain(..) {
            dropped.extend(tab.panes.into_values());
        }
        self.tabs.push(kept);
        self.active_tab = 0;
        dropped
    }

    // ─── Flotantes ──────────────────────────────────────────────────

    /// Agrega un panel flotante (`inst`), lo enfoca y enciende la capa. La
    /// geometría arranca en cascada según cuántos flotantes haya.
    pub fn new_float(&mut self, inst: Instance) {
        let id = self.fresh_id();
        let t = self.tab_mut();
        let n = t.floating.len() as f32;
        let geo = FloatPane {
            id,
            x: 120.0 + n * 28.0,
            y: 80.0 + n * 28.0,
            w: 620.0,
            h: 380.0,
        };
        t.panes.insert(id, inst);
        t.floating.push(geo);
        t.focused = id;
        t.show_floating = true;
    }

    /// Enciende/apaga la capa flotante. Al apagarla, si el foco estaba en un
    /// flotante, vuelve a un panel tiled. Al encenderla, enfoca el flotante de
    /// arriba (si hay).
    pub fn toggle_floating(&mut self) {
        let t = self.tab_mut();
        if t.floating.is_empty() {
            return;
        }
        t.show_floating = !t.show_floating;
        if t.show_floating {
            if let Some(top) = t.floating.last() {
                t.focused = top.id;
            }
        } else if t.is_floating(t.focused) {
            t.refocus_tiled();
        }
    }

    /// Desplaza un panel flotante por (dx, dy) px. Lo trae al frente.
    pub fn move_float(&mut self, id: PaneId, dx: f32, dy: f32) {
        let t = self.tab_mut();
        if let Some(pos) = t.floating.iter().position(|f| f.id == id) {
            let mut f = t.floating.remove(pos);
            f.x = (f.x + dx).max(0.0);
            f.y = (f.y + dy).max(0.0);
            t.floating.push(f);
            t.focused = id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Panel de prueba liviano (un canvas, sin shell/PTY/sled).
    fn pane() -> Instance {
        Instance::canvas("t".to_string())
    }

    fn ws() -> Workspace {
        Workspace::single(pane())
    }

    #[test]
    fn single_arranca_con_un_panel_una_tab() {
        let w = ws();
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.tab().panes.len(), 1);
        assert_eq!(w.tab().layout.count(), 1);
    }

    #[test]
    fn split_agrega_panel_y_enfoca_el_nuevo() {
        let mut w = ws();
        let antes = w.tab().focused;
        w.split(Axis::Horizontal, pane());
        assert_eq!(w.tab().panes.len(), 2);
        assert_eq!(w.tab().layout.count(), 2);
        assert_ne!(w.tab().focused, antes, "el panel nuevo toma el foco");
    }

    #[test]
    fn close_focused_recolapsa_y_no_borra_el_ultimo() {
        let mut w = ws();
        w.split(Axis::Vertical, pane());
        assert_eq!(w.tab().panes.len(), 2);
        assert!(w.close_focused().is_some());
        assert_eq!(w.tab().panes.len(), 1);
        // El último panel del último tab no se cierra.
        assert!(w.close_focused().is_none());
        assert_eq!(w.tab().panes.len(), 1);
    }

    #[test]
    fn cycle_focus_recorre_los_tiled() {
        let mut w = ws();
        w.split(Axis::Horizontal, pane());
        let a = w.tab().focused;
        w.cycle_focus(true);
        let b = w.tab().focused;
        assert_ne!(a, b);
        w.cycle_focus(true);
        assert_eq!(w.tab().focused, a, "dos paneles → vuelve al primero");
    }

    #[test]
    fn tabs_nuevo_activa_y_cierra() {
        let mut w = ws();
        w.new_tab(pane());
        assert_eq!(w.tabs.len(), 2);
        assert_eq!(w.active_tab, 1);
        // Cerrar la tab activa vuelve a una.
        let dropped = w.close_tab(1);
        assert_eq!(dropped.len(), 1);
        assert_eq!(w.tabs.len(), 1);
        // No se cierra la única tab.
        assert!(w.close_tab(0).is_empty());
    }

    #[test]
    fn floating_agrega_capa_enfoca_y_togglea() {
        let mut w = ws();
        w.new_float(pane());
        assert_eq!(w.tab().floating.len(), 1);
        assert!(w.tab().show_floating);
        let fid = w.tab().floating[0].id;
        assert_eq!(w.tab().focused, fid, "el flotante nuevo toma el foco");
        // Apagar la capa devuelve el foco a un panel tiled.
        w.toggle_floating();
        assert!(!w.tab().show_floating);
        assert!(!w.tab().is_floating(w.tab().focused));
    }

    #[test]
    fn move_float_acumula_delta() {
        let mut w = ws();
        w.new_float(pane());
        let fid = w.tab().floating[0].id;
        let (x0, y0) = (w.tab().floating[0].x, w.tab().floating[0].y);
        w.move_float(fid, 10.0, -5.0);
        let f = &w.tab().floating[0];
        assert!((f.x - (x0 + 10.0)).abs() < 1e-3);
        assert!((f.y - (y0 - 5.0)).abs() < 1e-3);
    }

    #[test]
    fn close_floating_quita_de_la_capa() {
        let mut w = ws();
        w.new_float(pane());
        assert_eq!(w.tab().floating.len(), 1);
        // El foco está en el flotante → close_focused lo quita.
        assert!(w.close_focused().is_some());
        assert_eq!(w.tab().floating.len(), 0);
        assert_eq!(w.tab().panes.len(), 1);
    }
}
