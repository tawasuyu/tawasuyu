//! `llimphi-module-fif` — find-in-files reutilizable (estilo JetBrains).
//!
//! Módulo Llimphi con dos vistas independientes:
//!
//! - [`view_dialog`] — popup compacto (header + input) que el host pinta
//!   como overlay modal centrado. Sólo visible cuando
//!   [`FifState::dialog_open`] es `true`.
//! - [`view_results_bar`] — barra inferior persistente con la lista de
//!   matches. El host la pinta como tool window al pie (estilo JetBrains
//!   "Find" tool window). Sobrevive al cierre del dialog: el user puede
//!   Esc-cerrar el popup y seguir clickeando los resultados.
//!
//! El flujo típico es: `Ctrl+Shift+F` abre el dialog → tipear → Enter
//! ejecuta `search` → resultados aparecen en la barra inferior → Esc
//! cierra el popup pero la barra queda → click en una fila abre el
//! archivo. Re-disparar `Ctrl+Shift+F` reabre el popup conservando los
//! últimos resultados.
//!
//! ## Cómo lo enchufa una app
//!
//! ```ignore
//! struct AppModel {
//!     all_files: Vec<PathBuf>,
//!     fif: Option<FifState>,
//!     // …
//! }
//!
//! enum AppMsg { Fif(llimphi_module_fif::FifMsg), … }
//!
//! // En update(model, msg):
//! AppMsg::Fif(fm) => {
//!     // Lazy-init en Open:
//!     if matches!(fm, FifMsg::Open) && model.fif.is_none() {
//!         model.fif = Some(FifState::new());
//!     } else if matches!(fm, FifMsg::Open) {
//!         model.fif.as_mut().unwrap().dialog_open = true;
//!     }
//!     let action = match model.fif.as_mut() {
//!         Some(s) => llimphi_module_fif::apply(s, fm, &model.all_files),
//!         None => FifAction::None,
//!     };
//!     match action {
//!         FifAction::None => {}
//!         FifAction::CloseDialog => {
//!             if let Some(s) = model.fif.as_mut() { s.dialog_open = false; }
//!         }
//!         FifAction::CloseAll => model.fif = None,
//!         FifAction::Searched { .. } => { /* actualizar status bar */ }
//!         FifAction::OpenAt { path, line, col } => {
//!             if let Some(s) = model.fif.as_mut() { s.dialog_open = false; }
//!             open_path_in_app(path, line, col);
//!         }
//!     }
//! }
//!
//! // En on_key(model, event): solo rutea cuando el dialog está visible.
//! if let Some(state) = model.fif.as_ref() {
//!     if let Some(fm) = llimphi_module_fif::on_key(state, event) {
//!         return Some(AppMsg::Fif(fm));
//!     }
//! }
//! if llimphi_module_fif::open_shortcut(event) {
//!     return Some(AppMsg::Fif(FifMsg::Open));
//! }
//!
//! // En view(model):
//! //   - dialog como overlay arriba del editor:
//! if let Some(s) = model.fif.as_ref().filter(|s| s.dialog_open) {
//!     overlay_children.push(view_dialog(s, &palette, AppMsg::Fif));
//! }
//! //   - barra de resultados como panel inferior persistente:
//! if let Some(s) = model.fif.as_ref().filter(|s| !s.results.is_empty()) {
//!     bottom_panels.push(view_results_bar(
//!         s, &model.all_files, &model.root, &palette, AppMsg::Fif,
//!     ));
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
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

/// Capabilities que este módulo aporta al host. Convención del protocolo
/// Brahman Card aplicada a módulos compile-time: el host (cuando construye
/// su [`card_core::Card`]) puede agregar esto a `provides` para anunciar
/// — vía broker — que su instancia ofrece find-in-files al ecosistema.
pub const CAPABILITIES: &[&str] = &["editor.find-in-files"];

/// Caps razonables para que un workspace grande no funda el UI.
pub const MAX_RESULTS: usize = 1000;
pub const MAX_FILE_SIZE: u64 = 2_000_000;
pub const SNIPPET_MAX_CHARS: usize = 160;
pub const MIN_QUERY_LEN: usize = 2;

const DIALOG_W: f32 = 560.0;
const DIALOG_H: f32 = 116.0;
const BAR_H: f32 = 220.0;
const ROW_H: f32 = 20.0;
const MAX_VISIBLE: usize = 9;

/// Qué input tiene el foco dentro del dialog. `Tab` alterna.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FifFocus {
    Search,
    Replace,
}

/// Un match individual.
#[derive(Debug, Clone)]
pub struct FifMatch {
    /// Índice dentro del slice de paths que el host pasa a [`apply`] y
    /// las vistas. Convención: el host no debe reordenar/mutar el slice
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
    /// Texto de reemplazo. Si vacío, `ReplaceAll` borra los matches.
    pub replace: TextInputState,
    pub focus: FifFocus,
    pub results: Vec<FifMatch>,
    pub selected: usize,
    /// Última query realmente ejecutada (puede diferir del input si el
    /// user siguió tipeando sin re-Enter).
    pub last_query: String,
    /// `true` cuando el popup modal está visible. La barra de resultados
    /// se pinta independientemente de esto: sobrevive al cierre del popup.
    pub dialog_open: bool,
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
            replace: TextInputState::new(),
            focus: FifFocus::Search,
            results: Vec::new(),
            selected: 0,
            last_query: String::new(),
            dialog_open: true,
        }
    }
}

/// Vocabulario interno. El host lo wrapea en su propio Msg.
#[derive(Clone)]
pub enum FifMsg {
    /// El host detectó el atajo de apertura (o un comando). Lazy-init del
    /// state lo hace el host; `apply` sólo marca `dialog_open = true`.
    Open,
    /// El user pidió cerrar el popup (Esc). Los resultados quedan en la
    /// barra inferior.
    CloseDialog,
    /// Cerrar todo: el host debería tirar el `FifState` completo.
    CloseAll,
    /// Tecla rumbo al input.
    KeyInput(KeyEvent),
    /// Navegación dentro de la lista de resultados.
    Nav(i32),
    /// Enter: la primera vez ejecuta search; subsiguientes abren el
    /// match seleccionado.
    Submit,
    /// Click en una fila de la barra inferior: selecciona y abre.
    ActivateAt(usize),
    /// Alterna el foco entre los inputs search ↔ replace (Tab).
    ToggleFocus,
    /// Reemplaza el texto matcheado por `replace.text()` en todos los
    /// matches actuales. Idempotente: re-leer el archivo, sustituir
    /// case-insensitive por la query, escribir. Vacía `results` para
    /// forzar nueva búsqueda si el user quiere ver el estado posterior.
    ReplaceAll,
}

/// Efecto solicitado al host. El módulo nunca toca el FS ni el resto del
/// modelo de la app — devuelve el deseo, el host elige cómo lo aplica.
#[derive(Debug, Clone)]
pub enum FifAction {
    None,
    /// El host debería marcar `state.dialog_open = false` y dejar el
    /// resto del state intacto (resultados visibles en la barra).
    CloseDialog,
    /// El host debería remover el state del modelo entero.
    CloseAll,
    /// Tras un Submit que ejecutó search.
    Searched { matches: usize, elapsed: Duration, query: String },
    /// El host debería abrir `path` y posicionar el caret en `(line, col)`.
    /// El módulo NO se cierra automáticamente: el host decide si ocultar
    /// el dialog tras abrir el match.
    OpenAt { path: PathBuf, line: usize, col: usize },
    /// Tras `ReplaceAll`: cuántos archivos tocados, cuántos matches
    /// sustituidos, cuántos fallaron. El host debería refrescar buffers
    /// abiertos (recargar de disco si no-dirty) y mostrar status.
    Replaced {
        files_changed: usize,
        replacements: usize,
        failures: usize,
        query: String,
        replacement: String,
    },
}

/// Aplica un mensaje al estado y retorna el efecto que el host debe ejecutar.
///
/// `paths` es la lista canónica de archivos sobre la que buscar. El host
/// la pasa por referencia; cuando Submit dispara una búsqueda, este
/// vector se itera y se leen los archivos (skip binarios y >MAX_FILE_SIZE).
pub fn apply(state: &mut FifState, msg: FifMsg, paths: &[PathBuf]) -> FifAction {
    match msg {
        FifMsg::Open => {
            state.dialog_open = true;
            FifAction::None
        }
        FifMsg::CloseDialog => FifAction::CloseDialog,
        FifMsg::CloseAll => FifAction::CloseAll,
        FifMsg::KeyInput(ev) => {
            let _ = match state.focus {
                FifFocus::Search => state.input.apply_key(&ev),
                FifFocus::Replace => state.replace.apply_key(&ev),
            };
            FifAction::None
        }
        FifMsg::ToggleFocus => {
            state.focus = match state.focus {
                FifFocus::Search => FifFocus::Replace,
                FifFocus::Replace => FifFocus::Search,
            };
            FifAction::None
        }
        FifMsg::ReplaceAll => {
            let query = state.last_query.clone();
            if query.is_empty() || state.results.is_empty() {
                return FifAction::None;
            }
            let replacement = state.replace.text();
            let (files_changed, replacements, failures) =
                replace_all(paths, &state.results, &query, &replacement);
            // Invalidamos resultados: las posiciones (line, col) ya no
            // necesariamente apuntan al mismo texto. El user puede re-Enter.
            state.results.clear();
            state.selected = 0;
            FifAction::Replaced {
                files_changed,
                replacements,
                failures,
                query,
                replacement,
            }
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
        FifMsg::ActivateAt(idx) => {
            if idx >= state.results.len() {
                return FifAction::None;
            }
            state.selected = idx;
            let fm = state.results[idx].clone();
            let Some(path) = paths.get(fm.file_idx).cloned() else {
                return FifAction::None;
            };
            FifAction::OpenAt { path, line: fm.line, col: fm.col }
        }
    }
}

/// Routing de teclas cuando el dialog está abierto. Si el popup está
/// cerrado, devuelve `None` y el host puede seguir routeando al editor.
pub fn on_key(state: &FifState, event: &KeyEvent) -> Option<FifMsg> {
    if !state.dialog_open {
        return None;
    }
    if event.state != KeyState::Pressed {
        return None;
    }
    Some(match &event.key {
        Key::Named(NamedKey::Escape) => FifMsg::CloseDialog,
        Key::Named(NamedKey::Enter) => FifMsg::Submit,
        Key::Named(NamedKey::Tab) => FifMsg::ToggleFocus,
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
    pub border: Color,
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
            border: t.border,
            theme: t.clone(),
        }
    }
}

/// Popup modal compacto: header + input. Sin lista de resultados — esa
/// vive en [`view_results_bar`]. El host lo pinta como overlay centrado.
///
/// El `View` devuelto tiene tamaño fijo ([`DIALOG_W`] × [`DIALOG_H`]). Si
/// el host quiere centrarlo, debe envolverlo en un container con
/// `JustifyContent::Center`/`AlignItems::Center` o usar el slot de overlay.
pub fn view_dialog<HostMsg, F>(
    state: &FifState,
    palette: &FifPalette,
    to_host: F,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
    F: Fn(FifMsg) -> HostMsg + Copy + 'static,
{
    let dirty_query = state.input.text() != state.last_query;
    let header = if state.last_query.is_empty() {
        "find in files · Enter busca · Esc cierra".to_string()
    } else if state.results.is_empty() {
        format!("«{}» · sin matches · Esc cierra", state.last_query)
    } else {
        let staleness = if dirty_query { " · Enter re-busca" } else { "" };
        format!(
            "«{}» · {} matches · ↓↑ navega · Enter abre{staleness} · Esc cierra",
            state.last_query,
            state.results.len(),
        )
    };

    let header_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
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
    let search_focus = state.focus == FifFocus::Search;
    let search_view = labelled_input(
        "buscar",
        &state.input,
        "buscar en archivos…",
        search_focus,
        palette,
        &tp,
        to_host(FifMsg::Open),
    );
    let replace_view = labelled_input(
        "reemplazar",
        &state.replace,
        "(vacío para borrar)",
        !search_focus,
        palette,
        &tp,
        to_host(FifMsg::Open),
    );

    let replace_btn = View::new(Style {
        size: Size { width: length(118.0_f32), height: length(20.0_f32) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_header)
    .radius(3.0)
    .text_aligned(
        "reemplazar todo".to_string(),
        10.0,
        palette.fg_muted,
        Alignment::Center,
    )
    .on_click(to_host(FifMsg::ReplaceAll));

    let hint = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: length(20.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned("Tab alterna campos".to_string(), 9.0, palette.fg_muted, Alignment::Start);

    let actions = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
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
    .fill(palette.bg_panel)
    .children(vec![hint, replace_btn]);

    // Wrapper exterior: tamaño fijo del dialog + borde sutil.
    let dialog = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(DIALOG_W), height: length(DIALOG_H) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .radius(6.0)
    .children(vec![header_view, search_view, replace_view, actions]);

    // Container que centra el dialog horizontalmente — el host pone esto
    // como overlay arriba del editor; un click en zona vacía no hace nada
    // (no cerramos por click-outside, sería sorpresivo si el user está
    // ojeando resultados en la barra).
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(DIALOG_H + 16.0) },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(12.0_f32),
            bottom: length(4.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Start),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![dialog])
}

/// Barra inferior persistente con los matches. Filas clickeables (click
/// → [`FifMsg::ActivateAt`]). El host la pinta como tool window al pie
/// del editor, hermana del terminal/output (estilo JetBrains).
///
/// Si no hay resultados, devuelve una barra mínima con un mensaje — el
/// host puede usar `state.results.is_empty()` para no renderizarla.
pub fn view_results_bar<HostMsg, F>(
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
    let header_text = if state.results.is_empty() {
        format!("find · «{}» · sin matches", state.last_query)
    } else {
        format!(
            "find · «{}» · {} / {} matches · click abre · Ctrl+Shift+F reabre",
            state.last_query,
            state.selected + 1,
            state.results.len(),
        )
    };

    let close_btn = View::new(Style {
        size: Size { width: length(54.0_f32), height: length(18.0_f32) },
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
    .text_aligned("cerrar ✕".to_string(), 10.0, palette.fg_muted, Alignment::Center)
    .on_click(to_host(FifMsg::CloseAll));

    let header_label = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: length(20.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let header_bar = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_header)
    .children(vec![header_label, close_btn]);

    let visible_start = state
        .selected
        .saturating_sub(MAX_VISIBLE.saturating_sub(1));
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
                    left: length(12.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                flex_shrink: 0.0,
                ..Default::default()
            })
            .fill(bg)
            .text_aligned(label, 11.0, fg, Alignment::Start)
            .on_click(to_host(FifMsg::ActivateAt(i))),
        );
    }

    let mut children: Vec<View<HostMsg>> = Vec::with_capacity(1 + rows.len());
    children.push(header_bar);
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

/// Reemplazo case-insensitive sobre los archivos involucrados en
/// `results`. Devuelve `(files_changed, replacements, failures)`.
/// Lee cada archivo una sola vez, sustituye todas las apariciones de
/// `query` por `replacement` (case-insensitive, preservando el resto), y
/// escribe sólo si hubo cambios. No toca buffers en memoria del host —
/// el host es responsable de recargar tabs si quiere ver los cambios.
pub fn replace_all(
    paths: &[PathBuf],
    results: &[FifMatch],
    query: &str,
    replacement: &str,
) -> (usize, usize, usize) {
    if query.is_empty() {
        return (0, 0, 0);
    }
    let mut touched: std::collections::BTreeSet<usize> =
        std::collections::BTreeSet::new();
    for fm in results {
        touched.insert(fm.file_idx);
    }
    let mut files_changed = 0usize;
    let mut total_replacements = 0usize;
    let mut failures = 0usize;
    let q_lc = query.to_lowercase();
    for idx in touched {
        let Some(path) = paths.get(idx) else { continue };
        let Ok(content) = std::fs::read_to_string(path) else {
            failures += 1;
            continue;
        };
        let (new_content, n) = ci_replace_all(&content, query, &q_lc, replacement);
        if n == 0 {
            continue;
        }
        if std::fs::write(path, new_content).is_err() {
            failures += 1;
            continue;
        }
        files_changed += 1;
        total_replacements += n;
    }
    (files_changed, total_replacements, failures)
}

/// Reemplazo case-insensitive preservando los bytes no-matchados.
fn ci_replace_all(haystack: &str, _needle: &str, needle_lc: &str, repl: &str) -> (String, usize) {
    let hay_lc = haystack.to_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut count = 0usize;
    let mut i = 0usize;
    while i <= hay_lc.len() {
        if let Some(pos) = hay_lc[i..].find(needle_lc) {
            let abs = i + pos;
            out.push_str(&haystack[i..abs]);
            out.push_str(repl);
            i = abs + needle_lc.len();
            count += 1;
        } else {
            out.push_str(&haystack[i..]);
            break;
        }
    }
    (out, count)
}

// ---------------------------------------------------------------------
// Helpers internos
// ---------------------------------------------------------------------

/// Pinta un input con etiqueta a la izquierda; cuando `focus` es true,
/// el fondo se realza para que el user vea dónde está tipeando.
fn labelled_input<HostMsg>(
    label: &str,
    state: &TextInputState,
    placeholder: &str,
    focus: bool,
    palette: &FifPalette,
    tp: &TextInputPalette,
    fallback_msg: HostMsg,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
{
    let bg = if focus { palette.bg_selected } else { palette.bg_panel };
    let label_view = View::new(Style {
        size: Size { width: length(82.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(4.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(label.to_string(), 10.0, palette.fg_muted, Alignment::Start);

    let input_view = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(4.0_f32),
            right: length(10.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(vec![text_input_view(
        state,
        placeholder,
        focus,
        tp,
        fallback_msg,
    )]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(bg)
    .children(vec![label_view, input_view])
}

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}
