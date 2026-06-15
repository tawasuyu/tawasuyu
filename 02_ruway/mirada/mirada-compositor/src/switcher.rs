//! Switcher visual (Alt-Tab para ventanas, Win-Tab para escritorios). Muestra
//! un overlay con la lista mientras se mantiene el modificador: cada Tab
//! adelanta la selección, soltar el modificador confirma, Esc cancela.
//!
//! - **Windows** (Alt+Tab): lista las ventanas; confirma enfocando. Visible
//!   también en tiling, donde el cambio de foco no se nota (no se solapan).
//! - **Workspaces** (Win+Tab): lista sólo los escritorios **ocupados** (vagar
//!   por vacíos invisibles no sirve); confirma saltando a ese escritorio.
//!
//! El estado vive en [`App`]; el dibujo en `drm_backend::render::emit_switcher`.

use crate::App;

/// Qué cicla el switcher: ventanas (Alt+Tab) o escritorios (Win+Tab).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwitcherKind {
    Windows,
    Workspaces,
}

/// Una sesión de switcher en curso.
pub(crate) struct Switcher {
    pub(crate) kind: SwitcherKind,
    /// Ids a confirmar: id de ventana (Windows) o índice de escritorio
    /// (Workspaces, 0-based), según [`kind`](Self::kind).
    pub(crate) order: Vec<u64>,
    /// Etiqueta a pintar por cada entrada (alineada con `order`).
    pub(crate) labels: Vec<String>,
    /// Índice seleccionado.
    pub(crate) sel: usize,
}

/// El modificador que, al soltarse, confirma este switcher.
impl SwitcherKind {
    pub(crate) fn modifier_held(self, mods: &smithay::input::keyboard::ModifiersState) -> bool {
        match self {
            SwitcherKind::Windows => mods.alt,
            SwitcherKind::Workspaces => mods.logo,
        }
    }
}

/// Abre el switcher del `kind` pedido (si no había, o si había de otro kind) o
/// adelanta/retrocede la selección. La primera pulsación va a la **siguiente**
/// entrada a la actual, como cualquier alt-tab.
pub(crate) fn advance(app: &mut App, kind: SwitcherKind, forward: bool) {
    // Si ya hay uno del MISMO kind, sólo movemos la selección.
    if let Some(sw) = &mut app.switcher {
        if sw.kind == kind {
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
    }
    app.switcher = match kind {
        SwitcherKind::Windows => build_windows(app, forward),
        SwitcherKind::Workspaces => build_workspaces(app, forward),
    };
}

fn build_windows(app: &App, forward: bool) -> Option<Switcher> {
    let mut order = Vec::new();
    let mut labels = Vec::new();
    for w in app.windows.iter().filter(|w| !w.is_shell) {
        order.push(w.id);
        labels.push(if w.title.trim().is_empty() {
            format!("ventana {}", w.id)
        } else {
            w.title.clone()
        });
    }
    if order.is_empty() {
        return None;
    }
    let focused = app
        .windows
        .iter()
        .find(|w| w.focused && !w.is_shell)
        .map(|w| w.id);
    let cur = focused
        .and_then(|fid| order.iter().position(|&i| i == fid))
        .unwrap_or(0);
    let sel = step(cur, order.len(), forward);
    Some(Switcher {
        kind: SwitcherKind::Windows,
        order,
        labels,
        sel,
    })
}

fn build_workspaces(app: &App, forward: bool) -> Option<Switcher> {
    let (active, loads) = app.workspace_overview()?;
    // Sólo escritorios ocupados (los vacíos no se listan).
    let occ: Vec<usize> = (0..loads.len()).filter(|&i| loads[i] > 0).collect();
    if occ.is_empty() {
        return None;
    }
    let order: Vec<u64> = occ.iter().map(|&i| i as u64).collect();
    let labels: Vec<String> = occ
        .iter()
        .map(|&i| {
            let n = loads[i];
            format!(
                "Escritorio {} · {} ventana{}",
                i + 1,
                n,
                if n == 1 { "" } else { "s" }
            )
        })
        .collect();
    let cur = occ.iter().position(|&i| i == active);
    let sel = match cur {
        Some(p) => step(p, occ.len(), forward),
        None => 0,
    };
    Some(Switcher {
        kind: SwitcherKind::Workspaces,
        order,
        labels,
        sel,
    })
}

fn step(cur: usize, n: usize, forward: bool) -> usize {
    if forward {
        (cur + 1) % n
    } else {
        (cur + n - 1) % n
    }
}

/// Confirma la selección y cierra el switcher: enfoca la ventana o salta al
/// escritorio elegido, según el kind.
pub(crate) fn commit(app: &mut App) {
    let Some(sw) = app.switcher.take() else {
        return;
    };
    let Some(&id) = sw.order.get(sw.sel) else {
        return;
    };
    match sw.kind {
        SwitcherKind::Windows => app.activar_ventana(id),
        SwitcherKind::Workspaces => app.cambiar_workspace(id as usize),
    }
}

/// Cierra el switcher sin actuar (Esc).
pub(crate) fn cancel(app: &mut App) {
    app.switcher = None;
}
