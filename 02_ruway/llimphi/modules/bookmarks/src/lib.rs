//! llimphi-module-bookmarks - marcadores per-file persistentes en sesion.
//!
//! El usuario marca lineas con Ctrl+Alt+B y luego salta con
//! Ctrl+Alt+N / Ctrl+Alt+P. Ctrl+Shift+B abre un overlay con la
//! lista filtrable.
//!
//! Los marks son tuplas (PathBuf, line). Viven en memoria del
//! proceso; el host puede serializar marks si quiere persistir.
//!
//! Sigue el contrato Llimphi de docs/MODULES.md.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

/// Capabilities que aporta este modulo al host.
pub const CAPABILITIES: &[&str] = &["editor.bookmarks"];

pub const MAX_RESULTS: usize = 500;

const PANEL_H: f32 = 320.0;
const ROW_H: f32 = 20.0;
const MAX_VISIBLE: usize = 12;

/// Sub-state del overlay tipo lista (input + results + selected).
/// None cuando no hay panel abierto.
pub struct BookmarksOverlay {
    pub input: TextInputState,
    /// Indices a state.marks rankeados por fuzzy match. Cap MAX_RESULTS.
    pub results: Vec<usize>,
    pub selected: usize,
}

impl BookmarksOverlay {
    pub fn new() -> Self {
        Self { input: TextInputState::new(), results: Vec::new(), selected: 0 }
    }
}

/// Estado interno. Persiste durante toda la sesion (no es Option en
/// el host como otros modulos): los marks viven siempre, el overlay si
/// es opcional. Hace de mini-registro de waypoints del usuario.
pub struct BookmarksState {
    /// Marks en orden de creacion. Cada uno es (path, line).
    /// Toggle quita uno existente o agrega uno nuevo al final.
    pub marks: Vec<(PathBuf, usize)>,
    /// Overlay-list abierto cuando Some.
    pub overlay: Option<BookmarksOverlay>,
}

impl Default for BookmarksState {
    fn default() -> Self { Self::new() }
}

impl BookmarksState {
    pub fn new() -> Self {
        Self { marks: Vec::new(), overlay: None }
    }

    /// True si existe un mark con la misma (path, line).
    pub fn contains(&self, path: &Path, line: usize) -> bool {
        self.marks.iter().any(|(p, l)| p == path && *l == line)
    }

    /// Toggle: si ya existe lo remueve; si no, lo agrega al final.
    /// Devuelve true si quedo agregado.
    pub fn toggle(&mut self, path: PathBuf, line: usize) -> bool {
        if let Some(idx) = self.marks.iter().position(|(p, l)| p == &path && *l == line) {
            self.marks.remove(idx);
            false
        } else {
            self.marks.push((path, line));
            true
        }
    }
}

/// Vocabulario interno. El host lo wrapea en su Msg.
#[derive(Debug, Clone)]
pub enum BookmarksMsg {
    /// Toggle del mark en (path, line). El host emite esto cuando
    /// detecta el shortcut (Ctrl+Alt+B) y conoce la posicion del caret.
    ToggleAt { path: PathBuf, line: usize },
    /// Saltar al proximo mark cronologicamente despues de
    /// (current_path, current_line). Si no hay marks, no-op.
    JumpNext { current_path: PathBuf, current_line: usize },
    /// Saltar al previo. Misma semantica reversa.
    JumpPrev { current_path: PathBuf, current_line: usize },
    /// Abrir el overlay-list.
    OpenList,
    /// Cerrar el overlay.
    CloseList,
    /// Teclas para el input del overlay.
    ListKey(KeyEvent),
    /// Navegacion en la lista del overlay.
    ListNav(i32),
    /// Enter: salta al mark seleccionado.
    ListApply,
    /// Limpia todos los marks.
    ClearAll,
}

/// Efecto solicitado al host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookmarksAction {
    None,
    /// El host deberia cerrar el overlay (limpiar la sub-state).
    Close,
    /// El host deberia abrir ese path (si no esta abierto) y
    /// posicionar el caret. Cierra el overlay automaticamente cuando
    /// llega vinculado a ListApply.
    JumpTo { path: PathBuf, line: usize },
    /// Mensaje informativo para la status bar (eg toggle feedback).
    SetStatus(String),
}

/// Aplica un mensaje al estado.
pub fn apply(state: &mut BookmarksState, msg: BookmarksMsg) -> BookmarksAction {
    match msg {
        BookmarksMsg::ToggleAt { path, line } => {
            let added = state.toggle(path.clone(), line);
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
            let msg = if added {
                format!("bookmark agregado en {} linea {}", name, line + 1)
            } else {
                format!("bookmark removido de {} linea {}", name, line + 1)
            };
            BookmarksAction::SetStatus(msg)
        }
        BookmarksMsg::JumpNext { current_path, current_line } => {
            match next_after(state, &current_path, current_line) {
                Some((p, l)) => BookmarksAction::JumpTo { path: p, line: l },
                None => BookmarksAction::SetStatus("sin bookmarks".into()),
            }
        }
        BookmarksMsg::JumpPrev { current_path, current_line } => {
            match prev_before(state, &current_path, current_line) {
                Some((p, l)) => BookmarksAction::JumpTo { path: p, line: l },
                None => BookmarksAction::SetStatus("sin bookmarks".into()),
            }
        }
        BookmarksMsg::OpenList => BookmarksAction::None,
        BookmarksMsg::CloseList => BookmarksAction::Close,
        BookmarksMsg::ListKey(ev) => {
            if let Some(ov) = state.overlay.as_mut() {
                ov.input.apply_key(&ev);
                refilter_overlay(state);
            }
            BookmarksAction::None
        }
        BookmarksMsg::ListNav(d) => {
            if let Some(ov) = state.overlay.as_mut() {
                let n = ov.results.len() as i32;
                if n > 0 {
                    ov.selected = (ov.selected as i32 + d).rem_euclid(n) as usize;
                }
            }
            BookmarksAction::None
        }
        BookmarksMsg::ListApply => {
            let Some(ov) = state.overlay.as_ref() else { return BookmarksAction::None };
            let Some(&idx) = ov.results.get(ov.selected) else { return BookmarksAction::None };
            let Some((p, l)) = state.marks.get(idx).cloned() else { return BookmarksAction::None };
            BookmarksAction::JumpTo { path: p, line: l }
        }
        BookmarksMsg::ClearAll => {
            let n = state.marks.len();
            state.marks.clear();
            if let Some(ov) = state.overlay.as_mut() {
                ov.results.clear();
                ov.selected = 0;
            }
            BookmarksAction::SetStatus(format!("bookmarks limpios ({} removidos)", n))
        }
    }
}

/// Devuelve el mark inmediatamente posterior a (path, line) en orden
/// de marks. Wraparound al final.
fn next_after(state: &BookmarksState, path: &Path, line: usize) -> Option<(PathBuf, usize)> {
    if state.marks.is_empty() { return None; }
    let n = state.marks.len();
    let cur_idx = state.marks.iter().position(|(p, l)| p == path && *l == line);
    let start = match cur_idx {
        Some(i) => (i + 1) % n,
        None => 0,
    };
    Some(state.marks[start].clone())
}

/// Devuelve el mark inmediatamente previo. Wraparound al inicio.
fn prev_before(state: &BookmarksState, path: &Path, line: usize) -> Option<(PathBuf, usize)> {
    if state.marks.is_empty() { return None; }
    let n = state.marks.len();
    let cur_idx = state.marks.iter().position(|(p, l)| p == path && *l == line);
    let start = match cur_idx {
        Some(i) if i > 0 => i - 1,
        Some(_) => n - 1,
        None => n - 1,
    };
    Some(state.marks[start].clone())
}

/// Routing de teclas cuando el overlay esta abierto.
pub fn on_key(state: &BookmarksState, event: &KeyEvent) -> Option<BookmarksMsg> {
    state.overlay.as_ref()?;
    if event.state != KeyState::Pressed { return None; }
    Some(match &event.key {
        Key::Named(NamedKey::Escape) => BookmarksMsg::CloseList,
        Key::Named(NamedKey::Enter) => BookmarksMsg::ListApply,
        Key::Named(NamedKey::ArrowDown) => BookmarksMsg::ListNav(1),
        Key::Named(NamedKey::ArrowUp) => BookmarksMsg::ListNav(-1),
        _ => BookmarksMsg::ListKey(event.clone()),
    })
}

/// Atajo de toggle: Ctrl+Alt+B.
pub fn toggle_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && event.modifiers.alt
        && !event.modifiers.shift
        && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("b"))
}

/// Atajo de open-list: Ctrl+Shift+B. Tambien sirve como toggle del
/// panel (cierra si ya estaba abierto). El host decide en base a su
/// state.
pub fn open_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && event.modifiers.shift
        && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("b"))
}

/// Atajo de next: Ctrl+Alt+N.
pub fn next_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && event.modifiers.alt
        && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("n"))
}

/// Atajo de prev: Ctrl+Alt+P.
pub fn prev_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && event.modifiers.alt
        && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("p"))
}

/// Recalcula overlay.results con fuzzy match contra path+line.
/// Query vacio = todos los marks en orden.
pub fn refilter_overlay(state: &mut BookmarksState) {
    let Some(ov) = state.overlay.as_mut() else { return; };
    let q = ov.input.text();
    if q.trim().is_empty() {
        ov.results = (0..state.marks.len().min(MAX_RESULTS)).collect();
        ov.selected = 0;
        return;
    }
    use nucleo_matcher::{pattern::{CaseMatching, Normalization, Pattern}, Config, Matcher, Utf32Str};
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pat = Pattern::parse(&q, CaseMatching::Smart, Normalization::Smart);
    let mut scored: Vec<(u32, usize)> = Vec::new();
    let mut buf = Vec::new();
    for (i, (p, l)) in state.marks.iter().enumerate() {
        let hay_str = format!("{} {}", p.display(), l + 1);
        buf.clear();
        let hay = Utf32Str::new(&hay_str, &mut buf);
        if let Some(score) = pat.score(hay, &mut matcher) {
            scored.push((score, i));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(MAX_RESULTS);
    ov.results = scored.into_iter().map(|(_, i)| i).collect();
    ov.selected = 0;
}

/// Paleta visual.
#[derive(Debug, Clone)]
pub struct BookmarksPalette {
    pub bg_panel: Color,
    pub bg_header: Color,
    pub bg_selected: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_accent: Color,
    theme: llimphi_theme::Theme,
}

impl BookmarksPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_panel,
            bg_header: t.bg_panel_alt,
            bg_selected: t.bg_selected,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_accent: t.accent,
            theme: t.clone(),
        }
    }
}

/// Render del overlay. Solo se llama cuando state.overlay es Some.
/// El host pasa root para mostrar paths relativos en la lista.
pub fn view<HostMsg, F>(
    state: &BookmarksState,
    root: &Path,
    palette: &BookmarksPalette,
    to_host: F,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
    F: Fn(BookmarksMsg) -> HostMsg + Copy + 'static,
{
    let ov = match state.overlay.as_ref() {
        Some(o) => o,
        None => return View::new(Style::default()),
    };
    let header = if state.marks.is_empty() {
        "bookmarks - sin marks - Ctrl+Alt+B agrega - Esc cierra".to_string()
    } else if ov.results.is_empty() {
        format!("bookmarks - sin matches - {} marks - Esc cierra", state.marks.len())
    } else {
        format!(
            "bookmarks - {} / {} - flechas navegan - Enter salta - Esc cierra",
            ov.selected + 1,
            ov.results.len(),
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
        &ov.input,
        "filtro: path o numero de linea",
        true,
        &tp,
        to_host(BookmarksMsg::OpenList),
    )]);

    let visible_start = ov.selected.saturating_sub(MAX_VISIBLE.saturating_sub(1));
    let visible_end = (visible_start + MAX_VISIBLE).min(ov.results.len());
    let mut rows: Vec<View<HostMsg>> = Vec::with_capacity(MAX_VISIBLE);
    for i in visible_start..visible_end {
        let Some(&idx) = ov.results.get(i) else { continue };
        let Some((p, line)) = state.marks.get(idx) else { continue };
        let rel: String = match p.strip_prefix(root) {
            Ok(r) => r.display().to_string(),
            Err(_) => p.display().to_string(),
        };
        let label = format!("{}  :  linea {}", rel, line + 1);
        let selected = i == ov.selected;
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
        size: Size { width: percent(1.0_f32), height: length(PANEL_H) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .children(children)
}
