//! Switcher visual de ventanas (Alt-Tab). A diferencia del keybind
//! `FocusNext` —que cambia el foco al instante, invisible en tiling porque las
//! teseladas no se solapan— este muestra un overlay con la lista de ventanas
//! mientras se mantiene Alt apretado: cada Alt+Tab adelanta la selección,
//! soltar Alt confirma (enfoca la elegida), Esc cancela.
//!
//! El estado vive en [`App`] (lo maneja el backend de input); el dibujo del
//! overlay vive en `drm_backend::render` (`emit_switcher`), con el mismo text
//! rendering que el HUD y el menú raíz.

use crate::App;

/// Una sesión de Alt-Tab en curso.
pub(crate) struct Switcher {
    /// Ids de ventana candidatas, en orden estable (orden de aparición).
    pub(crate) order: Vec<u64>,
    /// Índice seleccionado dentro de [`order`](Self::order).
    pub(crate) sel: usize,
}

/// Abre el switcher (si no estaba) o adelanta/retrocede la selección. La
/// primera pulsación selecciona la **siguiente** ventana a la enfocada, como
/// cualquier alt-tab. `forward=false` retrocede (Shift+Alt+Tab).
pub(crate) fn advance(app: &mut App, forward: bool) {
    if let Some(sw) = &mut app.switcher {
        if sw.order.is_empty() {
            return;
        }
        let n = sw.order.len();
        sw.sel = if forward {
            (sw.sel + 1) % n
        } else {
            (sw.sel + n - 1) % n
        };
        return;
    }
    let order: Vec<u64> = app
        .windows
        .iter()
        .filter(|w| !w.is_shell)
        .map(|w| w.id)
        .collect();
    if order.is_empty() {
        return;
    }
    // Arranca en la siguiente a la enfocada (o en la primera si no hay foco).
    let focused = app
        .windows
        .iter()
        .find(|w| w.focused && !w.is_shell)
        .map(|w| w.id);
    let cur = focused
        .and_then(|fid| order.iter().position(|&i| i == fid))
        .unwrap_or(0);
    let n = order.len();
    let sel = if forward {
        (cur + 1) % n
    } else {
        (cur + n - 1) % n
    };
    app.switcher = Some(Switcher { order, sel });
}

/// Confirma la selección: enfoca la ventana elegida y cierra el switcher.
pub(crate) fn commit(app: &mut App) {
    if let Some(sw) = app.switcher.take() {
        if let Some(&id) = sw.order.get(sw.sel) {
            app.activar_ventana(id);
        }
    }
}

/// Cierra el switcher sin cambiar el foco (Esc).
pub(crate) fn cancel(app: &mut App) {
    app.switcher = None;
}

/// Título a mostrar para una ventana: su título, o el `app_id`, o un
/// genérico. Lo usa el render del overlay.
pub(crate) fn etiqueta(app: &App, id: u64) -> String {
    app.windows
        .iter()
        .find(|w| w.id == id)
        .map(|w| {
            if w.title.trim().is_empty() {
                format!("ventana {id}")
            } else {
                w.title.clone()
            }
        })
        .unwrap_or_else(|| format!("ventana {id}"))
}
