// a11y_rt.rs — Integración con AccessKit en tiempo de ejecución.
// Empuja el árbol de accesibilidad al SO y atiende las acciones del lector
// de pantalla (focus, click). Separado para no contaminar el flujo de input
// con detalles de la API de accesskit_winit.

use super::super::*;
use super::push_a11y_tree;

impl<A: App> Runtime<A> {
    /// Recibe un `accesskit_winit::Event` (ruteado vía `EventLoopProxy` como
    /// `UserEvent::A11y(...)`) y reacciona:
    /// - `InitialTreeRequested`: el lector pidió el árbol inicial → empujamos
    ///   uno desde `last_render` si lo hay, o pedimos un redraw que lo creará.
    /// - `ActionRequested(req)`: el lector quiere ejecutar una acción sobre un
    ///   `NodeId`. v1 soporta `Action::Focus` (mueve `state.focused` + dispara
    ///   `App::on_focus`) y `Action::Click` (ejecuta el `on_click` del nodo).
    /// - `AccessibilityDeactivated`: nada que hacer; el siguiente paint dejará
    ///   de construir trees (el `update_if_active` se autoinhibe).
    pub(super) fn handle_a11y_event(&mut self, ev: accesskit_winit::Event) {
        use accesskit_winit::WindowEvent as AkWinEvent;
        let Some(state) = self.state.as_mut() else { return };
        match ev.window_event {
            AkWinEvent::InitialTreeRequested => {
                // Si ya pintamos un frame, ese mounted sirve para el árbol
                // inicial. Si no, forzamos un redraw — el path normal llamará
                // a `push_a11y_tree::<A>` al final.
                if state.last_render.is_some() {
                    push_a11y_tree::<A>(state);
                } else {
                    state.window.request_redraw();
                }
            }
            AkWinEvent::ActionRequested(req) => {
                let Some(idx) = crate::a11y::mounted_idx_for(req.target_node) else {
                    return;
                };
                let Some(cache) = state.last_render.as_ref() else {
                    return;
                };
                let Some(node) = cache.mounted.nodes.get(idx) else {
                    return;
                };
                match req.action {
                    accesskit::Action::Focus => {
                        // Si el nodo es focusable, movemos el foco a su id
                        // opaco; si no, lo limpiamos. La app recibe la
                        // transición vía `App::on_focus`.
                        let new_focus = node.focusable;
                        state.focused = new_focus;
                        let model = state.model.as_ref().expect("model");
                        if let Some(msg) = A::on_focus(model, new_focus) {
                            let m = state.model.take().expect("model");
                            state.model = Some(A::update(m, msg, &self.handle));
                        }
                        state.last_render = None;
                        state.window.request_redraw();
                    }
                    accesskit::Action::Click => {
                        // Sólo soportamos `on_click` (Msg directo) en v1. Los
                        // handlers `*_at` necesitan una posición sintética
                        // coherente que no tenemos — los ignoramos.
                        if let Some(msg) = node.on_click.clone() {
                            let m = state.model.take().expect("model");
                            state.model = Some(A::update(m, msg, &self.handle));
                            state.last_render = None;
                            state.window.request_redraw();
                        }
                    }
                    _ => {
                        // Otras acciones (Expand/Collapse/Increment/Decrement/
                        // SetValue/ScrollIntoView/etc.) se sumarán cuando un
                        // widget concreto lo pida — el modelo `SemanticsSpec`
                        // ya tiene los flags relevantes; solo falta cablear el
                        // efecto inverso (acción → mutación de Model).
                    }
                }
            }
            AkWinEvent::AccessibilityDeactivated => {}
        }
    }
}
