//! `shuma-module-shell` — REPL del shell como módulo enchufable.
//!
//! Núcleo del shell interactivo: cwd + input + ejecución por `shuma-exec`
//! con salida en streaming + buffer de output acotado. Builtins: `cd`,
//! `pwd`, `clear`, `exit` (no-op — el chasis maneja la salida).
//!
//! Diseño del tab:
//!
//! ```text
//!  Shell · local · cwd: /home/usuario
//!  ┌──────────────────────────────────────────────────────────┐
//!  │ $ ls                                                     │
//!  │ Cargo.toml                                               │
//!  │ src                                                      │
//!  │ ...                                                      │
//!  │ ✔ exit 0                                                 │
//!  └──────────────────────────────────────────────────────────┘
//!  ┌──────────────────────────────────────────────────────────┐
//!  │ $ █                                                      │
//!  └──────────────────────────────────────────────────────────┘
//! ```
//!
//! **Ejecución no bloqueante.** Cada submisión lanza `shuma_exec::run`
//! que vuelve de inmediato; el `RunHandle` se guarda en el state. El
//! chasis manda `Msg::Tick` periódicamente y el módulo drena
//! `try_events()` sin bloquear la UI. `sleep 5`, `top` y demás dejan
//! de congelar el shell. Mientras hay un run vivo, Enter encola la
//! nueva línea — el siguiente comando arranca al cerrar el actual.

#![forbid(unsafe_code)]

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use shuma_exec::{CommandSpec, Exec, Killer, RunEvent, RunHandle, StageSpec};
use shuma_intent::SessionGraph;
use shuma_line::{LineState, TokenKind};
use shuma_module::{ModuleContributions, ShortcutSpec, Source};
use shuma_remote_exec::RemoteRunHandle;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// `id` canónico del módulo. El shumarc lo referencia para activarlo.
pub const ID: &str = "shell";

/// Tope de líneas guardadas en el buffer de output activo. El scrollback
/// real vive en `State.surf_history` (con spill a disco configurable);
/// este buffer es el que alimenta `body_lines_for_block` y los detectores
/// (`sections::detect_sections`). 500 cortaba `ls -alR` antes de que el
/// detector viera el primer header de directorio. 50k cubre comandos
/// gordos sin pasarse en RAM (~10 MB para líneas de 200 bytes promedio).
pub const MAX_OUTPUT_LINES: usize = 50_000;

// ── Submódulos de tipos, mensajes y fuentes ──────────────────────────────────
mod types;
pub use types::*;

mod msg;
pub use msg::*;

mod shell_source;
pub use shell_source::*;

mod history_helpers;
pub use history_helpers::*;

// ── Submódulos de UI y lógica ────────────────────────────────────────────────
mod mouse_xterm;
pub mod sections;
mod update;
mod view;

pub use mouse_xterm::{XBtn, XPhase};
pub use update::*;
pub use view::*;

/// Arma el `Scrollback` persistente desde la config: cap en MiB +
/// (opcional) spill a un archivo en `$XDG_RUNTIME_DIR/shuma-<pid>.spill`
/// (o el path explícito de la config). Errores al armar el spill se
/// degradan a "sin spill" (el history funciona igual, sólo pierde el
/// archivo de archive).
fn build_surf_history(config: &shuma_config::Config) -> llimphi_widget_terminal::Scrollback {
    let limit_bytes = config.scrollback.limit_mb.saturating_mul(1024 * 1024);
    let mut sb = llimphi_widget_terminal::Scrollback::new(limit_bytes);
    if config.scrollback.spill {
        let path = if !config.scrollback.spill_path.is_empty() {
            PathBuf::from(&config.scrollback.spill_path)
        } else {
            let dir = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir());
            dir.join(format!("shuma-{}.spill", std::process::id()))
        };
        if let Ok(spill) = llimphi_widget_terminal::SpillStore::create(&path) {
            sb.enable_spill(spill);
        }
        // Sin spill si falló crear el archivo — no es fatal, el shell sigue.
    }
    sb
}

/// Refresca el cache de líneas spilled visibles si `spilled_count` cambió
/// desde el último refresh. Lee las últimas [`MAX_SPILLED_VISIBLE`] líneas
/// del archive vía `Scrollback::read_spilled`. Si el read falla por I/O,
/// la entrada queda como `<I/O error>` (no propaga el error — el view
/// sigue mostrando el resto). Sincrono: el costo del refresh es N reads
/// del archivo, una sola vez por cambio de spill (no por frame).
pub(crate) fn refresh_surf_spilled_visible(
    history: &Arc<Mutex<llimphi_widget_terminal::Scrollback>>,
    cache: &Arc<Mutex<SurfSpilledCache>>,
) {
    // Snapshot del estado del history sin retener el lock durante el I/O.
    let (spilled_count, hist_clone) = {
        let Ok(h) = history.lock() else { return };
        (h.spilled_count(), h.clone())
    };
    {
        let Ok(c) = cache.lock() else { return };
        if c.cached_at == spilled_count {
            return; // no hubo append al spill desde el último refresh
        }
    }
    // Refresh: leer las últimas N spilled.
    let n = spilled_count.min(MAX_SPILLED_VISIBLE);
    let first_id = (spilled_count - n) as u64;
    let mut lines = Vec::with_capacity(n);
    for i in 0..n {
        let id = first_id + i as u64;
        match hist_clone.read_spilled(id) {
            Ok(Some(text)) => lines.push(text),
            Ok(None) => lines.push(String::new()),
            Err(_) => lines.push("<I/O error reading spill>".into()),
        }
    }
    if let Ok(mut c) = cache.lock() {
        c.lines = lines;
        c.first_id = first_id;
        c.cached_at = spilled_count;
    }
}

/// Appendea el texto de `line` a la `Scrollback` persistente sólo si es una
/// línea de **body** (no Prompt, no salida de etapa intermedia, no notice
/// de cierre `✔/✘/⏹`). Espeja el filtro de `body_lines_for_block` para
/// que el history acumule sólo lo que el view ve como cuerpo. Errores del
/// lock se ignoran (poison defensivo).
fn push_to_surf_history(
    history: &Arc<Mutex<llimphi_widget_terminal::Scrollback>>,
    line: &OutputLine,
) {
    if line.kind == OutputKind::Prompt {
        return;
    }
    if line.stage.is_some() {
        return;
    }
    if view::is_status_line(&line.text) {
        return;
    }
    if let Ok(mut h) = history.lock() {
        h.push_line(&line.text);
    }
}

pub fn contributions(_state: &State) -> ModuleContributions {
    ModuleContributions {
        monitors: vec![],
        shortcuts: vec![
            ShortcutSpec::module_action("Clear", "shell.clear")
                .with_hint("Vacía el buffer de output"),
            ShortcutSpec::module_action("Cancel", "shell.cancel")
                .with_hint("SIGTERM al comando actual"),
        ],
    }
}

#[cfg(test)]
mod tests;
