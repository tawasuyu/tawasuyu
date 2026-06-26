//! Persistencia del escritorio: instantánea y restauración de sesión.

use mirada_layout::{LayoutNode, SpaceNode, WindowId};

use crate::session::{DesktopState, NodeShape, SpaceShape, SESSION_VERSION};

use super::estado::Desktop;

impl Desktop {
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
            groupings: self.grouping_shapes(),
        }
    }

    /// Proyecta la agrupación de cada escritorio agrupado a su **forma** anclada
    /// por `app_id` (para persistirla; los `WindowId` son efímeros). Salta el
    /// escritorio entero si alguna hoja no se puede resolver a un `app_id` no
    /// vacío —mejor no persistir una forma que no se podrá reconstruir fielmente.
    fn grouping_shapes(&self) -> Vec<(usize, SpaceShape)> {
        let mut out = Vec::new();
        for (n, ws) in self.workspaces.iter().enumerate() {
            if let Some(node) = ws.grouping() {
                if let Some(shape) = self.space_to_shape(node) {
                    out.push((n, shape));
                }
            }
        }
        out
    }

    /// Un [`SpaceNode`] (hojas = `WindowId`) → [`SpaceShape`] (hojas = `app_id`).
    /// `None` si alguna ventana es desconocida o no tiene `app_id`.
    fn space_to_shape(&self, node: &SpaceNode) -> Option<SpaceShape> {
        let mut children = Vec::with_capacity(node.children.len());
        for child in &node.children {
            children.push(match child {
                LayoutNode::Leaf(id) => {
                    let app_id = self.windows.get(id).map(|i| i.app_id.as_str())?;
                    if app_id.is_empty() {
                        return None;
                    }
                    NodeShape::Leaf(app_id.to_string())
                }
                LayoutNode::Space(s) => NodeShape::Space(self.space_to_shape(s)?),
            });
        }
        Some(SpaceShape { params: node.params, children })
    }

    /// Deriva el mapa `app_id`→escritorio de las ventanas vivas, para
    /// persistirlo en la sesión: cada ventana de cada escritorio aporta el
    /// hogar de su app. Orden estable (BTreeMap) y, si una app está en varios
    /// escritorios, gana el de índice mayor.
    fn window_homes(&self) -> Vec<(String, usize)> {
        let mut homes: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
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

    /// Intenta **rematerializar** la agrupación pendiente del escritorio `n`
    /// (restaurada de una sesión): si cada hoja `app_id` de la forma encuentra una
    /// ventana viva distinta en ese escritorio, reconstruye el árbol con los
    /// `WindowId` actuales y lo instala; si falta alguna app, no hace nada y queda
    /// pendiente para cuando reabra. Las flotantes no cuentan (no se agrupan).
    pub(super) fn try_restore_grouping(&mut self, n: usize) {
        let Some(shape) = self.restored_groupings.get(&n).cloned() else {
            return;
        };
        let ws = &self.workspaces[n];
        let mut pool: Vec<(WindowId, String)> = ws
            .windows()
            .iter()
            .filter(|&&id| !ws.is_floating(id))
            .filter_map(|&id| self.windows.get(&id).map(|i| (id, i.app_id.clone())))
            .collect();
        if let Some(node) = Self::shape_to_space(&shape, &mut pool) {
            self.workspaces[n].set_grouping(Some(node));
            self.restored_groupings.remove(&n);
        }
    }

    /// Una [`SpaceShape`] (hojas = `app_id`) → [`SpaceNode`] (hojas = `WindowId`),
    /// consumiendo del `pool` una ventana distinta por hoja, en orden de árbol.
    /// `None` si alguna hoja no encuentra ventana —entonces no se materializa nada.
    fn shape_to_space(shape: &SpaceShape, pool: &mut Vec<(WindowId, String)>) -> Option<SpaceNode> {
        let mut children = Vec::with_capacity(shape.children.len());
        for child in &shape.children {
            children.push(match child {
                NodeShape::Leaf(app_id) => {
                    let pos = pool.iter().position(|(_, a)| a == app_id)?;
                    LayoutNode::Leaf(pool.remove(pos).0)
                }
                NodeShape::Space(s) => {
                    LayoutNode::Space(Box::new(Self::shape_to_space(s, pool)?))
                }
            });
        }
        Some(SpaceNode { params: shape.params, children })
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
        self.restored_groupings = state
            .groupings
            .iter()
            .filter(|(n, _)| *n < self.workspaces.len())
            .cloned()
            .collect();
    }

    /// Aplica el mapa salida→escritorio restaurado a las salidas **ya
    /// conectadas** y devuelve los comandos del re-teselado.
    ///
    /// [`restore`](Desktop::restore) deja el mapa *pendiente* porque al arrancar
    /// aún no hay salidas (se aplica en cada [`OutputAdded`](mirada_protocol::BodyEvent::OutputAdded)).
    /// En un **cambio de sesión en vivo** (FUS) las salidas no se reconectan, así
    /// que el mapa quedaría colgado: este método lo consume contra las salidas
    /// presentes —por orden de aparición, como `OutputAdded`— para que cada
    /// monitor recupere el escritorio que mostraba esa sesión. No-op si no hay
    /// mapa pendiente o no hay salidas.
    pub fn apply_restored_output_workspaces(&mut self) -> Vec<crate::BrainCommand> {
        if self.pending_output_workspaces.is_empty() || self.outputs.is_empty() {
            return Vec::new();
        }
        let ws_count = self.workspaces.len();
        let mut taken: Vec<usize> = Vec::new();
        for (i, o) in self.outputs.iter_mut().enumerate() {
            if let Some(ws) = self
                .pending_output_workspaces
                .get(i)
                .copied()
                .filter(|&n| n < ws_count && !taken.contains(&n))
            {
                o.workspace = ws;
            }
            taken.push(o.workspace);
        }
        self.pending_output_workspaces.clear();
        self.reflow_outputs();
        self.relayout()
    }
}
