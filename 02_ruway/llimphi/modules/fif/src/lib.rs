//! `llimphi-module-fif` — find-in-files reutilizable (estilo JetBrains).
//!
//! Módulo Llimphi siguiendo el contrato canónico:
//!
//! - [`FifState`] — estado opaco que el host embebe en su Model.
//! - [`FifMsg`] — vocabulario interno del módulo; el host lo wrapea
//!   dentro de su propio Msg (típicamente `AppMsg::Fif(FifMsg)`).
//! - [`FifAction`] — efecto que el módulo le pide al host después de
//!   procesar un mensaje (cerrar, abrir un archivo en una posición,
//!   actualizar el status bar). El módulo NO ejecuta side effects; los
//!   delega vía Action.
//! - [`apply`] — `(state, msg, paths) → action`. Pure (excepto la
//!   búsqueda, que lee del FS — eso pasa cuando `Submit` ejecuta search).
//! - [`on_key`] — `(state, event) → Option<Msg>`. El host la llama
//!   cuando el panel está abierto para rutear teclas.
//! - [`open_shortcut`] — chequea si un KeyEvent es el atajo de apertura
//!   recomendado (Ctrl+Shift+F). El host puede usarlo o definir el suyo.
//! - [`view`] — render parametrizado sobre el `HostMsg` del que llama,
//!   vía un callback `to_host: Fn(FifMsg) -> HostMsg`.
//!
//! ## Cómo lo enchufa una app
//!
//! ```ignore
//! // En el Model de la app:
//! struct AppModel {
//!     all_files: Vec<PathBuf>,
//!     fif: Option<FifState>,
//!     // … resto del estado …
//! }
//!
//! // En el Msg de la app:
//! enum AppMsg {
//!     Fif(llimphi_module_fif::FifMsg),
//!     // …
//! }
//!
//! // En update(model, msg):
//! AppMsg::Fif(fm) => {
//!     let mut m = model;
//!     let action = match m.fif.as_mut() {
//!         Some(s) => llimphi_module_fif::apply(s, fm, &m.all_files),
//!         None => llimphi_module_fif::FifAction::None,
//!     };
//!     match action {
//!         FifAction::Close => m.fif = None,
//!         FifAction::OpenAt { path, line, col } => {
//!             m.fif = None;
//!             m = open_path_in_app(m, path, line, col);
//!         }
//!         FifAction::Searched { matches, elapsed, query } => {
//!             m.status = format!("«{query}» · {matches} matches · {:.0} ms",
//!                                elapsed.as_secs_f64() * 1000.0);
//!         }
//!         FifAction::None => {}
//!     }
//!     m
//! }
//!
//! // En on_key(model, event):
//! //   1. Si el módulo está abierto, routea ahí primero:
//! if let Some(state) = model.fif.as_ref() {
//!     if let Some(fm) = llimphi_module_fif::on_key(state, event) {
//!         return Some(AppMsg::Fif(fm));
//!     }
//! }
//! //   2. Si no, chequea el atajo de apertura:
//! if llimphi_module_fif::open_shortcut(event) {
//!     return Some(AppMsg::Fif(FifMsg::Open));
//! }
//!
//! // En view(model):
//! if let Some(state) = model.fif.as_ref() {
//!     let panel = llimphi_module_fif::view(
//!         state,
//!         &model.all_files,
//!         &model.root,
//!         &FifPalette::from_theme(&theme),
//!         AppMsg::Fif,
//!     );
//!     // … añadir `panel` al árbol de View …
//! }
//! ```
//!
//! ## Por qué Action en lugar de un trait `FifHost`
//!
//! El módulo no toma `&mut Host` porque acoplar el módulo a un trait
//! arrastra problemas de ownership/lifetimes en el loop tipo Elm que usa
//! Llimphi (Model se mueve por value en update). Devolver una [`FifAction`]
//! deja al host libre de aplicar el efecto donde y como quiera, y mantiene
//! al módulo libre de cualquier conocimiento sobre el host.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

/// Caps razonables para que un workspace grande no funda el UI.
pub const MAX_RESULTS: usize = 1000;
pub const MAX_FILE_SIZE: u64 = 2_000_000;
pub const SNIPPET_MAX_CHARS: usize = 160;
pub const MIN_QUERY_LEN: usize = 2;

const BAR_H: f32 = 280.0;
const ROW_H: f32 = 20.0;
const MAX_VISIBLE: usize = 11;

/// Un match individual.
#[derive(Debug, Clone)]
pub struct FifMatch {
    /// Índice dentro del slice de paths que el host pasa a [`apply`] y
    /// [`view`]. Convención: el host no debe reordenar/mutar el slice
    /// entre frames mientras el módulo esté abierto.
    pub file_idx: usize,
    /// 0-based.
    pub line: usize,
    /// 0-based, en chars (no bytes).
    pub col: usize,
    /// Línea matcheada trimmed-left y truncada a [`SNIPPET_MAX_CHARS`].
    pub snippet: String,
}

/// Estado interno del módulo.
pub struct FifState {
    pub input: TextInputState,
    pub results: Vec<FifMatch>,
    pub selected: usize,
    /// Última query realmente ejecutada (puede diferir del input si el
    /// user siguió tipeando sin re-Enter).
    pub last_query: String,
}

impl Default for FifState {
    fn default() -> Self {
        Self::new()
    }
}

impl FifState {
    pub fn new() -> Self {
        Self {
            input: TextInputState::new(),
            results: Vec::new(),
            selected: 0,
            last_query: String::new(),
        }
    }
}

/// Vocabulario interno. El host lo wrapea en su propio Msg.
#[derive(Clone)]
pub enum FifMsg {
    /// Apertura del panel (lo dispatcha el host tras detectar el atajo).
    /// El módulo no maneja Open él mismo — sólo existe para que el host
    /// tenga un símbolo conveniente para dispatchar.
    Open,
    /// El user pidió cerrar (Esc).
    Close,
    /// Tecla rumbo al input.
    KeyInput(KeyEvent),
    /// Navegación dentro de la lista de resultados.
    Nav(i32),
    /// Enter: la primera vez ejecuta search; subsiguientes abren el
    /// match seleccionado.
    Submit,
}

/// Efecto solicitado al host. El módulo nunca toca el FS ni el resto del
/// modelo de la app — devuelve el deseo, el host elige cómo lo aplica.
#[derive(Debug, Clone)]
pub enum FifAction {
    None,
    /// El host debería remover el state del modelo.
    Close,
    /// Tras un Submit que ejecutó search.
    Searched { matches: usize, elapsed: Duration, query: String },
    /// El host debería abrir `path` y posicionar el caret en `(line, col)`.
    /// El módulo NO se cierra automáticamente: el host decide si ocultar
    /// el panel tras abrir el match.
    OpenAt { path: PathBuf, line: usize, col: usize },
}

/// Aplica un mensaje al estado y retorna el efecto que el host debe ejecutar.
///
/// `paths` es la lista canónica de archivos sobre la que buscar. El host
/// la pasa por referencia; cuando Submit dispara una búsqueda, este
/// vector se itera y se leen los archivos (skip binarios y >MAX_FILE_SIZE).
pub fn apply(state: &mut FifState, msg: FifMsg, paths: &[PathBuf]) -> FifAction {
    match msg {
        FifMsg::Open => FifAction::None,
        FifMsg::Close => FifAction::Close,
        FifMsg::KeyInput(ev) => {
            state.input.apply_key(&ev);
            FifAction::None
        }
        FifMsg::Nav(d) => {
            let n = state.results.len() as i32;
            if n > 0 {
                state.selected = (state.selected as i32 + d).rem_euclid(n) as usize;
            }
            FifAction::None
        }
        FifMsg::Submit => {
            let query = state.input.text();
            let needs_search = query != state.last_query || state.results.is_empty();
            if needs_search {
                if query.len() < MIN_QUERY_LEN {
                    return FifAction::None;
                }
                let started = std::time::Instant::now();
                let results = search(paths, &query);
                let elapsed = started.elapsed();
                let n = results.len();
                state.results = results;
                state.selected = 0;
                state.last_query = query.clone();
                FifAction::Searched { matches: n, elapsed, query }
            } else {
                let Some(fm) = state.results.get(state.selected).cloned() else {
                    return FifAction::None;
                };
                let Some(path) = paths.get(fm.file_idx).cloned() else {
                    return FifAction::None;
                };
                FifAction::OpenAt { path, line: fm.line, col: fm.col }
            }
        }
    }
}

/// Routing de teclas cuando el panel está abierto. Devuelve `Some(msg)`
/// si el evento es Pressed; `None` para releases u otros eventos que el
/// host puede ignorar.
pub fn on_key(_state: &FifState, event: &KeyEvent) -> Option<FifMsg> {
    if event.state != KeyState::Pressed {
        return None;
    }
    Some(match &event.key {
        Key::Named(NamedKey::Escape) => FifMsg::Close,
        Key::Named(NamedKey::Enter) => FifMsg::Submit,
        Key::Named(NamedKey::ArrowDown) => FifMsg::Nav(1),
        Key::Named(NamedKey::ArrowUp) => FifMsg::Nav(-1),
        _ => FifMsg::KeyInput(event.clone()),
    })
}

/// Chequea si el evento es el atajo recomendado: **Ctrl+Shift+F**. El
/// host puede ignorar esto y definir su propio binding.
pub fn open_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && event.modifiers.shift
        && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("f"))
}

/// Paleta visual. Construible desde un [`llimphi_theme::Theme`].
#[derive(Debug, Clone)]
pub struct FifPalette {
    pub bg_panel: Color,
    pub bg_header: Color,
    pub bg_selected: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    /// Theme cacheado para reusar en `TextInputPalette::from_theme`.
    theme: llimphi_theme::Theme,
}

impl FifPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_panel,
            bg_header: t.bg_panel_alt,
            bg_selected: t.bg_selected,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            theme: t.clone(),
        }
    }
}

/// Render del panel. `to_host` mapea cada `FifMsg` interno al `Msg` de
/// la app. `paths` y `root` se usan para etiquetas (path relativo).
pub fn view<HostMsg, F>(
    state: &FifState,
    paths: &[PathBuf],
    root: &Path,
    palette: &FifPalette,
    to_host: F,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
    F: Fn(FifMsg) -> HostMsg + Copy + 'static,
{
    let dirty_query = state.input.text() != state.last_query;
    let header = if state.results.is_empty() && state.last_query.is_empty() {
        format!(
            "find-in-files · escribí + Enter · {} archivos · Esc cierra",
            paths.len(),
        )
    } else if state.results.is_empty() {
        format!(
            "find-in-files · «{}» · sin matches · Esc cierra",
            state.last_query,
        )
    } else {
        let staleness = if dirty_query { " · query cambió, Enter re-busca" } else { "" };
        format!(
            "find-in-files · «{}» · {} / {} matches · ↓↑ Enter abre{staleness} · Esc cierra",
            state.last_query,
            state.selected + 1,
            state.results.len(),
        )
    };

    let header_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_header)
    .text_aligned(header, 10.0, palette.fg_muted, Alignment::Start);

    let tp = TextInputPalette::from_theme(&palette.theme);
    let input_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .children(vec![text_input_view(
        &state.input,
        "buscar en archivos…",
        true,
        &tp,
        to_host(FifMsg::Open),
    )]);

    let visible_start = state.selected.saturating_sub(MAX_VISIBLE.saturating_sub(1));
    let visible_end = (visible_start + MAX_VISIBLE).min(state.results.len());
    let mut rows: Vec<View<HostMsg>> = Vec::with_capacity(MAX_VISIBLE);
    for i in visible_start..visible_end {
        let Some(fm) = state.results.get(i) else { continue };
        let Some(path) = paths.get(fm.file_idx) else { continue };
        let rel = relative_to(root, path);
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        let dir = rel.strip_suffix(name).unwrap_or("").trim_end_matches('/');
        let dir_label = if dir.is_empty() { String::new() } else { format!("  {dir}") };
        let label = format!("{name}:{}{dir_label}    {}", fm.line + 1, fm.snippet);
        let selected = i == state.selected;
        let bg = if selected { palette.bg_selected } else { palette.bg_panel };
        let fg = if selected { palette.fg_text } else { palette.fg_muted };
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(ROW_H) },
                padding: Rect {
                    left: length(10.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                flex_shrink: 0.0,
                ..Default::default()
            })
            .fill(bg)
            .text_aligned(label, 11.0, fg, Alignment::Start),
        );
    }

    let mut children: Vec<View<HostMsg>> = Vec::with_capacity(2 + rows.len());
    children.push(header_view);
    children.push(input_view);
    children.extend(rows);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(BAR_H) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .children(children)
}

/// Búsqueda substring case-insensitive. Pública para tests / hosts que
/// quieran disparar una búsqueda sin pasar por el state machine.
pub fn search(paths: &[PathBuf], query: &str) -> Vec<FifMatch> {
    let mut out: Vec<FifMatch> = Vec::new();
    let q_lc = query.to_lowercase();
    for (file_idx, path) in paths.iter().enumerate() {
        if out.len() >= MAX_RESULTS {
            break;
        }
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.len() > MAX_FILE_SIZE {
                continue;
            }
        }
        let Ok(content) = std::fs::read_to_string(path) else { continue };
        for (line_idx, line) in content.lines().enumerate() {
            if out.len() >= MAX_RESULTS {
                break;
            }
            let line_lc = line.to_ascii_lowercase();
            let Some(byte_off) = line_lc.find(&q_lc) else { continue };
            let col = line[..byte_off.min(line.len())].chars().count();
            let trimmed = line.trim_start();
            let snippet = if trimmed.chars().count() <= SNIPPET_MAX_CHARS {
                trimmed.to_string()
            } else {
                let cut: String = trimmed.chars().take(SNIPPET_MAX_CHARS - 1).collect();
                format!("{cut}…")
            };
            out.push(FifMatch { file_idx, line: line_idx, col, snippet });
        }
    }
    out
}

// ---------------------------------------------------------------------
// Helpers internos
// ---------------------------------------------------------------------

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}
