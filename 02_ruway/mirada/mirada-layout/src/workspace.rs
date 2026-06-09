//! `Workspace` — un conjunto de ventanas, su foco y su modo de teselado.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
// El macro `vec!` sólo lo usan los tests de este módulo.
#[cfg(test)]
use alloc::vec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::geometry::Rect;
use crate::layout::{tile, LayoutMode, LayoutParams};
use crate::tree::{LayoutNode, SpaceNode};

/// Identificador de una ventana (una superficie Wayland).
pub type WindowId = u64;

/// Un escritorio: ventanas en orden de teselado + la enfocada + el modo.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Workspace {
    /// Ventanas en orden de teselado (la 0 es la maestra en `MasterStack`).
    windows: Vec<WindowId>,
    /// Índice de la ventana enfocada en `windows`.
    focus: usize,
    params: LayoutParams,
    /// Ventanas flotantes y su rectángulo: salen del teselado y se pintan
    /// encima. Las que no están aquí se teselan normalmente.
    floating: BTreeMap<WindowId, Rect>,
    /// La ventana en pantalla completa, si hay alguna: cubre toda la
    /// salida y oculta al resto.
    fullscreen: Option<WindowId>,
    /// Agrupación opcional en sub-espacios anidados (árbol fractal). `None` =
    /// teselado plano: el caso por defecto y el de todo el código existente.
    /// Es una capa de *arreglo* sobre `windows`, que sigue siendo la membresía
    /// autoritativa: [`layout`](Workspace::layout) la reconcilia con `windows`
    /// al resolver (añade las teseladas ausentes, poda las que ya no están). No
    /// se persiste: se rehace a voluntad.
    #[cfg_attr(feature = "serde", serde(skip))]
    grouping: Option<SpaceNode>,
    /// Camino de zoom dentro de `grouping`: índices de los sub-espacios en los
    /// que se ha "entrado". Vacío = se ve el espacio entero. Implica
    /// `grouping.is_some()`.
    #[cfg_attr(feature = "serde", serde(skip))]
    view_path: Vec<usize>,
}

impl Workspace {
    /// Escritorio vacío con los parámetros dados.
    pub fn new(params: LayoutParams) -> Self {
        Self {
            windows: Vec::new(),
            focus: 0,
            params,
            floating: BTreeMap::new(),
            fullscreen: None,
            grouping: None,
            view_path: Vec::new(),
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

    /// Reemplaza todos los parámetros del teselado de una vez — lo usa la
    /// config del usuario al fijar gap/ratio/nmaster/modo iniciales.
    pub fn set_params(&mut self, params: LayoutParams) {
        self.params = params;
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
        if self.fullscreen == Some(window) {
            self.fullscreen = None;
        }
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

    /// El rectángulo flotante de una ventana, si flota — para moverla o
    /// redimensionarla por teclado.
    pub fn floating_rect(&self, window: WindowId) -> Option<Rect> {
        self.floating.get(&window).copied()
    }

    /// La ventana en pantalla completa de este escritorio, si hay alguna.
    pub fn fullscreen(&self) -> Option<WindowId> {
        self.fullscreen
    }

    /// Pone (o quita, con `None`) la ventana en pantalla completa.
    pub fn set_fullscreen(&mut self, window: Option<WindowId>) {
        self.fullscreen = window;
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

    /// Intercambia dos ventanas en el orden de teselado, dejando el foco en
    /// `a`. No hace nada si alguna no está en el escritorio o son la misma.
    /// Lo usa el arrastre interactivo de ventanas teseladas (swap-on-drag).
    pub fn swap(&mut self, a: WindowId, b: WindowId) -> bool {
        if a == b {
            return false;
        }
        let (Some(ia), Some(ib)) = (
            self.windows.iter().position(|&w| w == a),
            self.windows.iter().position(|&w| w == b),
        ) else {
            return false;
        };
        self.windows.swap(ia, ib);
        self.focus = ib;
        true
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
    ///
    /// Con agrupación activa ([`group`](Workspace::group)) reparte a través del
    /// árbol fractal: resuelve el sub-espacio en el que se ha hecho zoom
    /// ([`zoom_in`](Workspace::zoom_in)) a pantalla completa, dejando fuera al
    /// resto. Las flotantes siguen yendo al final, siempre encima.
    pub fn layout(&self, screen: Rect) -> Vec<(WindowId, Rect)> {
        // Camino plano (por defecto): byte-idéntico al de siempre.
        if self.grouping.is_none() && self.view_path.is_empty() {
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
            return out;
        }
        // Camino agrupado: resolver el sub-espacio en vista a pantalla completa.
        let root = self.root_tree();
        let view = node_at_path(&root, &self.view_path).unwrap_or(&root);
        let mut out = view.resolve(screen);
        // Las flotantes nunca entran al árbol: van al final, encima de todo.
        for &id in &self.windows {
            if let Some(&rect) = self.floating.get(&id) {
                out.push((id, rect));
            }
        }
        out
    }

    /// Las ventanas **dormidas** tras la capa de zoom actual: las teseladas que
    /// existen pero quedan fuera del sub-espacio en vista, con el rectángulo que
    /// tendrían en el nivel superior (su "hogar", al que vuelven al salir del
    /// zoom). Vacío sin zoom activo — en el nivel superior se ve todo, nada
    /// duerme. Es disjunto de [`layout`](Workspace::layout): éste devuelve lo
    /// que se ve, aquél lo que se aparta. El Cuerpo las oculta **y** les suspende
    /// los frame callbacks (quedan inertes en vez de seguir pintando a ciegas).
    pub fn dormant(&self, screen: Rect) -> Vec<(WindowId, Rect)> {
        if self.grouping.is_none() && self.view_path.is_empty() {
            return Vec::new();
        }
        let root = self.root_tree();
        let view = node_at_path(&root, &self.view_path).unwrap_or(&root);
        let in_view = collect_leaves(view);
        root.resolve(screen)
            .into_iter()
            .filter(|(id, _)| !in_view.contains(id) && !self.floating.contains_key(id))
            .collect()
    }

    /// `true` si el escritorio está agrupado en sub-espacios.
    pub fn is_grouped(&self) -> bool {
        self.grouping.is_some()
    }

    /// El árbol de agrupación actual, si lo hay — para **persistir su forma**
    /// (el dueño la proyecta a `app_id` porque los [`WindowId`] son efímeros).
    pub fn grouping(&self) -> Option<&SpaceNode> {
        self.grouping.as_ref()
    }

    /// Fija (o quita, con `None`) el árbol de agrupación directamente — para
    /// **restaurar** una agrupación guardada, ya reconstruida con los `WindowId`
    /// vivos. Resetea el zoom al nivel superior. El `layout` reconcilia el árbol
    /// con `windows` igual que con [`group`](Workspace::group), así que sobra/
    /// falta una ventana no rompe nada.
    pub fn set_grouping(&mut self, grouping: Option<SpaceNode>) {
        self.grouping = grouping;
        self.view_path.clear();
    }

    /// Profundidad de zoom actual: `0` = se ve el espacio entero.
    pub fn zoom_depth(&self) -> usize {
        self.view_path.len()
    }

    /// Las ventanas que son hijas-hoja **directas** del sub-espacio en vista —
    /// las "sueltas" del nivel actual, sin contar las ya plegadas en
    /// sub-espacios— en orden de teselado. Sin zoom son las teseladas del nivel
    /// superior. Es lo que [`group`](Workspace::group) puede plegar y lo que la
    /// pila de `GroupStack` mira para anidar dentro del nivel actual.
    pub fn view_leaves(&self) -> Vec<WindowId> {
        let root = self.root_tree();
        let view = node_at_path(&root, &self.view_path).unwrap_or(&root);
        view.children
            .iter()
            .filter_map(|c| match c {
                LayoutNode::Leaf(id) => Some(*id),
                LayoutNode::Space(_) => None,
            })
            .collect()
    }

    /// Pliega las ventanas dadas en un nuevo sub-espacio **dentro del nivel en
    /// vista** (con los parámetros de teselado actuales): para ese nivel ocupa un
    /// solo hueco del teselado, y dentro reparte su propio espacio. Sólo pliega
    /// las que sean hojas directas del nivel actual ([`view_leaves`](Workspace::view_leaves));
    /// no hace nada con menos de dos. Operar sobre la vista —no siempre sobre la
    /// raíz— es lo que hace el árbol genuinamente fractal: agrupar dentro de un
    /// grupo, a profundidad arbitraria.
    pub fn group(&mut self, ids: &[WindowId]) {
        let mut root = self.root_tree();
        {
            let Some(view) = node_at_path_mut(&mut root, &self.view_path) else {
                return;
            };
            // Sólo hojas directas del nivel en vista: no se agrupa a través de
            // niveles ni se mete una hoja ya anidada.
            let direct: Vec<WindowId> = view
                .children
                .iter()
                .filter_map(|c| match c {
                    LayoutNode::Leaf(id) => Some(*id),
                    LayoutNode::Space(_) => None,
                })
                .collect();
            let members: Vec<WindowId> = ids
                .iter()
                .copied()
                .filter(|id| direct.contains(id) && !self.floating.contains_key(id))
                .collect();
            if members.len() < 2 {
                return;
            }
            // Saca las hojas de los miembros de este nivel y mételas en un nuevo
            // sub-espacio al final.
            view.children
                .retain(|n| !matches!(n, LayoutNode::Leaf(id) if members.contains(id)));
            let sub = SpaceNode {
                params: self.params,
                children: members.iter().map(|&id| LayoutNode::Leaf(id)).collect(),
            };
            view.children.push(LayoutNode::Space(Box::new(sub)));
        }
        self.grouping = Some(root);
    }

    /// Deshace toda la agrupación: vuelve al teselado plano y al nivel raíz.
    pub fn ungroup(&mut self) {
        self.grouping = None;
        self.view_path.clear();
    }

    /// Entra ("zoom in") en el sub-espacio del nivel de vista actual que
    /// contiene la ventana enfocada: ese sub-espacio pasa a ocupar toda la
    /// pantalla. No hace nada si el foco no está dentro de ningún sub-espacio
    /// (no hay dónde entrar).
    pub fn zoom_in(&mut self) {
        let Some(focused) = self.focused() else {
            return;
        };
        let root = self.root_tree();
        let Some(view) = node_at_path(&root, &self.view_path) else {
            return;
        };
        for (i, child) in view.children.iter().enumerate() {
            if let LayoutNode::Space(s) = child {
                if collect_leaves(s).contains(&focused) {
                    self.view_path.push(i);
                    return;
                }
            }
        }
    }

    /// Sale ("zoom out") un nivel hacia el espacio contenedor.
    pub fn zoom_out(&mut self) {
        self.view_path.pop();
    }

    /// El árbol de nivel superior, reconciliado con `windows`: parte de la
    /// agrupación actual (o de un árbol plano si no hay), añade como hojas de
    /// nivel superior las ventanas teseladas que no aparezcan ya en el árbol y
    /// poda las hojas de ventanas que ya no existen o que ahora flotan. El nivel
    /// superior usa siempre los parámetros de teselado del escritorio.
    fn root_tree(&self) -> SpaceNode {
        let mut root = self
            .grouping
            .clone()
            .unwrap_or_else(|| SpaceNode::new(self.params));
        root.params = self.params;
        prune(&mut root, &self.windows, &self.floating);
        let present = collect_leaves(&root);
        for &id in &self.windows {
            if !self.floating.contains_key(&id) && !present.contains(&id) {
                root.children.push(LayoutNode::Leaf(id));
            }
        }
        root
    }
}

/// Todas las hojas (ids de ventana) de un sub-espacio, recursivamente.
fn collect_leaves(node: &SpaceNode) -> Vec<WindowId> {
    let mut out = Vec::new();
    for c in &node.children {
        match c {
            LayoutNode::Leaf(id) => out.push(*id),
            LayoutNode::Space(s) => out.extend(collect_leaves(s)),
        }
    }
    out
}

/// Poda del árbol las hojas de ventanas ausentes (o que ahora flotan) y los
/// sub-espacios que quedan vacíos tras la poda.
fn prune(node: &mut SpaceNode, windows: &[WindowId], floating: &BTreeMap<WindowId, Rect>) {
    node.children.retain_mut(|c| match c {
        LayoutNode::Leaf(id) => windows.contains(id) && !floating.contains_key(id),
        LayoutNode::Space(s) => {
            prune(s, windows, floating);
            !s.children.is_empty()
        }
    });
}

/// Navega un camino de índices de sub-espacio desde la raíz. `None` si el
/// camino se sale del árbol o atraviesa una hoja.
fn node_at_path<'a>(root: &'a SpaceNode, path: &[usize]) -> Option<&'a SpaceNode> {
    let mut node = root;
    for &i in path {
        match node.children.get(i) {
            Some(LayoutNode::Space(s)) => node = s,
            _ => return None,
        }
    }
    Some(node)
}

/// Igual que [`node_at_path`] pero da acceso mutable — para plegar un grupo
/// dentro del sub-espacio en vista sin tocar el resto del árbol.
fn node_at_path_mut<'a>(root: &'a mut SpaceNode, path: &[usize]) -> Option<&'a mut SpaceNode> {
    let mut node = root;
    for &i in path {
        match node.children.get_mut(i) {
            Some(LayoutNode::Space(s)) => node = s,
            _ => return None,
        }
    }
    Some(node)
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
    fn swap_exchanges_two_windows_and_focuses_the_first() {
        let mut w = ws();
        for id in [1, 2, 3] {
            w.add(id);
        }
        assert!(w.swap(1, 3));
        assert_eq!(w.windows(), &[3, 2, 1]);
        // El foco queda en la primera del par (la arrastrada).
        assert_eq!(w.focused(), Some(1));
        // Swap con la misma, o con una ausente, no hace nada.
        assert!(!w.swap(2, 2));
        assert!(!w.swap(2, 99));
        assert_eq!(w.windows(), &[3, 2, 1]);
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

    // --- Agrupación en sub-espacios + zoom (árbol fractal) ---------------

    fn cols() -> Workspace {
        Workspace::new(LayoutParams { mode: LayoutMode::Columns, gap: 0, ..LayoutParams::default() })
    }

    #[test]
    fn grouping_needs_at_least_two_real_windows() {
        let mut w = cols();
        w.add(1);
        w.group(&[1]); // una sola → no agrupa
        assert!(!w.is_grouped());
        w.group(&[1, 99]); // 99 no existe → sigue siendo una válida
        assert!(!w.is_grouped());
    }

    #[test]
    fn a_group_occupies_a_single_slot_of_the_top_level() {
        // Tres ventanas en columnas; agrupo la 2 y la 3 en un sub-espacio.
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]);
        assert!(w.is_grouped());
        // El nivel superior tiene dos huecos: la 1 y el grupo {2,3}.
        let placed = w.layout(Rect::new(0, 0, 1200, 600));
        assert_eq!(placed.len(), 3);
        let r1 = placed.iter().find(|(id, _)| *id == 1).unwrap().1;
        let r2 = placed.iter().find(|(id, _)| *id == 2).unwrap().1;
        let r3 = placed.iter().find(|(id, _)| *id == 3).unwrap().1;
        // La 1 ocupa la columna izquierda entera; 2 y 3 parten la derecha.
        assert_eq!(r1, Rect::new(0, 0, 600, 600));
        assert_eq!(r2, Rect::new(600, 0, 300, 600));
        assert_eq!(r3, Rect::new(900, 0, 300, 600));
    }

    #[test]
    fn zooming_into_a_group_makes_it_absorb_the_screen() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]);
        w.focus_window(2); // el foco vive dentro del grupo
        w.zoom_in();
        assert_eq!(w.zoom_depth(), 1);
        // Sólo se ven la 2 y la 3, repartiéndose toda la pantalla.
        let placed = w.layout(Rect::new(0, 0, 1200, 600));
        let ids: Vec<_> = placed.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![2, 3]);
        assert_eq!(placed[0].1, Rect::new(0, 0, 600, 600));
        assert_eq!(placed[1].1, Rect::new(600, 0, 600, 600));
        // Salir restaura la vista del nivel superior.
        w.zoom_out();
        assert_eq!(w.zoom_depth(), 0);
        assert_eq!(w.layout(Rect::new(0, 0, 1200, 600)).len(), 3);
    }

    #[test]
    fn zoom_in_does_nothing_when_focus_is_a_top_level_leaf() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]);
        w.focus_window(1); // la 1 es hoja de nivel superior, no un grupo
        w.zoom_in();
        assert_eq!(w.zoom_depth(), 0);
    }

    #[test]
    fn a_new_window_after_grouping_appears_at_the_top_level() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]);
        w.add(4); // nueva ventana: hoja de nivel superior
        let placed = w.layout(Rect::new(0, 0, 1200, 600));
        // Tres huecos arriba: 1, 4 y el grupo {2,3} → cuatro ventanas en total.
        assert_eq!(placed.len(), 4);
        assert!(placed.iter().any(|(id, _)| *id == 4));
    }

    #[test]
    fn closing_a_grouped_window_prunes_it_from_the_tree() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]);
        w.remove(3); // se va una del grupo
        let placed = w.layout(Rect::new(0, 0, 1200, 600));
        let ids: Vec<_> = placed.iter().map(|(id, _)| *id).collect();
        assert_eq!(placed.len(), 2);
        assert!(ids.contains(&1) && ids.contains(&2) && !ids.contains(&3));
    }

    #[test]
    fn ungroup_returns_to_the_flat_layout() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]);
        w.zoom_in();
        w.ungroup();
        assert!(!w.is_grouped());
        assert_eq!(w.zoom_depth(), 0);
        // Idéntico al teselado plano de tres columnas.
        let flat = {
            let mut f = cols();
            for id in [1, 2, 3] {
                f.add(id);
            }
            f.layout(Rect::new(0, 0, 1200, 600))
        };
        assert_eq!(w.layout(Rect::new(0, 0, 1200, 600)), flat);
    }

    #[test]
    fn grouping_can_be_read_and_re_set_reproducing_the_layout() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]);
        let screen = Rect::new(0, 0, 1200, 600);
        let grouped = w.layout(screen);
        // Leer el árbol y reinstalarlo en un escritorio fresco da el mismo layout.
        let tree = w.grouping().cloned().unwrap();
        let mut w2 = cols();
        for id in [1, 2, 3] {
            w2.add(id);
        }
        w2.set_grouping(Some(tree));
        assert!(w2.is_grouped());
        assert_eq!(w2.zoom_depth(), 0); // set_grouping resetea el zoom
        assert_eq!(w2.layout(screen), grouped);
        // Sin agrupación: grouping() es None.
        w2.set_grouping(None);
        assert!(w2.grouping().is_none());
    }

    #[test]
    fn nothing_is_dormant_without_zoom() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        // Plano: nada duerme.
        assert!(w.dormant(Rect::new(0, 0, 1200, 600)).is_empty());
        // Agrupado pero en el nivel superior: se ve todo, nada duerme.
        w.group(&[2, 3]);
        assert!(w.dormant(Rect::new(0, 0, 1200, 600)).is_empty());
    }

    #[test]
    fn zooming_in_makes_the_out_of_view_windows_dormant() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]);
        w.focus_window(2);
        w.zoom_in(); // entro al grupo {2,3}; la 1 queda fuera
        let screen = Rect::new(0, 0, 1200, 600);
        let dormant = w.dormant(screen);
        // Sólo la 1 duerme, con su rect del nivel superior (columna izquierda).
        assert_eq!(dormant, vec![(1, Rect::new(0, 0, 600, 600))]);
        // Es disjunto de lo que se ve: la vista trae 2 y 3, dormant trae 1.
        let shown: Vec<_> = w.layout(screen).iter().map(|(id, _)| *id).collect();
        assert_eq!(shown, vec![2, 3]);
        assert!(!shown.iter().any(|id| dormant.iter().any(|(d, _)| d == id)));
        // Al salir del zoom nadie duerme.
        w.zoom_out();
        assert!(w.dormant(screen).is_empty());
    }

    #[test]
    fn view_leaves_follows_the_zoom_into_a_nested_level() {
        let mut w = cols();
        for id in [1, 2, 3, 4] {
            w.add(id);
        }
        // Sin zoom: las cuatro son hojas del nivel superior.
        assert_eq!(w.view_leaves(), vec![1, 2, 3, 4]);
        w.group(&[2, 3, 4]);
        // Tras agrupar: arriba quedan la 1 (suelta) y el grupo {2,3,4}.
        assert_eq!(w.view_leaves(), vec![1]);
        w.focus_window(3);
        w.zoom_in();
        // Dentro del grupo, las hojas directas son 2,3,4.
        assert_eq!(w.view_leaves(), vec![2, 3, 4]);
    }

    #[test]
    fn grouping_nests_inside_the_current_view_to_arbitrary_depth() {
        let mut w = cols();
        for id in [1, 2, 3, 4] {
            w.add(id);
        }
        w.group(&[2, 3, 4]); // nivel 1: grupo {2,3,4}
        w.focus_window(3);
        w.zoom_in(); // entro al grupo
        assert_eq!(w.zoom_depth(), 1);
        // Pliego 3 y 4 DENTRO del grupo → un sub-sub-espacio.
        w.group(&[3, 4]);
        // En vista (nivel 1) ahora hay: la 2 suelta + el grupo {3,4}.
        assert_eq!(w.view_leaves(), vec![2]);
        w.focus_window(4);
        w.zoom_in(); // entro al grupo anidado {3,4}
        assert_eq!(w.zoom_depth(), 2);
        let screen = Rect::new(0, 0, 1200, 600);
        let ids: Vec<_> = w.layout(screen).iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![3, 4]); // sólo el nivel más profundo
        // La 1 y la 2 duermen (capas más superficiales fuera de vista).
        let dormant: Vec<_> = w.dormant(screen).iter().map(|(id, _)| *id).collect();
        assert!(dormant.contains(&1) && dormant.contains(&2));
        // Salir dos niveles vuelve al tope; deshacer aplana del todo.
        w.zoom_out();
        w.zoom_out();
        assert_eq!(w.zoom_depth(), 0);
        w.ungroup();
        assert!(!w.is_grouped());
    }

    #[test]
    fn group_only_folds_direct_leaves_of_the_view_not_across_levels() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]); // {2,3} ya anidado
        // Intentar agrupar la 1 (suelta) con la 2 (ya dentro del grupo): la 2 no
        // es hoja directa del tope → menos de dos miembros válidos → no hace nada.
        w.group(&[1, 2]);
        // El nivel superior sigue siendo {1} + grupo{2,3}: tres ventanas.
        let placed = w.layout(Rect::new(0, 0, 1200, 600));
        assert_eq!(placed.len(), 3);
        assert_eq!(w.view_leaves(), vec![1]);
    }

    #[test]
    fn a_floating_window_never_goes_dormant() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]);
        w.set_floating(1, Some(Rect::new(10, 10, 100, 100)));
        w.focus_window(2);
        w.zoom_in();
        // La 1 flota: se queda encima, no duerme.
        assert!(w.dormant(Rect::new(0, 0, 1200, 600)).is_empty());
    }

    #[test]
    fn a_floating_window_stays_on_top_even_when_grouped() {
        let mut w = cols();
        for id in [1, 2, 3] {
            w.add(id);
        }
        w.group(&[2, 3]);
        let fr = Rect::new(10, 10, 100, 100);
        w.set_floating(1, Some(fr));
        let placed = w.layout(Rect::new(0, 0, 1200, 600));
        // La 1 flota: va al final con su rect; 2 y 3 forman el árbol.
        assert_eq!(placed.last(), Some(&(1, fr)));
    }
}
