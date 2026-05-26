//! `llimphi-module-diff-viewer` — visualización side-by-side de cambios.
//!
//! Equivalente al "Compare with Saved" de VS Code o el panel "Compare"
//! de JetBrains, pero como módulo Llimphi enchufable. El host le pasa
//! dos textos (`before`/`after`) y dos etiquetas (`"HEAD"`, `"Working
//! Tree"`, `"Buffer"` — lo que tenga sentido en su contexto), y el
//! módulo computa el diff line-based con [`similar`] y lo renderiza
//! en dos columnas con marcadores `+`/`-` y números de línea.
//!
//! El módulo no abre archivos, no llama a `git`, no toca disco. Toda
//! la fuente del diff la decide el host: puede comparar el disco vs
//! el buffer dirty, dos branches, dos snapshots de history, etc.
//!
//! Sigue el contrato Llimphi de `docs/MODULES.md`:
//! `State + Msg + Action + apply/on_key/open_shortcut/view + Palette`.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use similar::{ChangeTag, TextDiff};

/// Capabilities que aporta este módulo al host.
pub const CAPABILITIES: &[&str] = &["editor.diff-viewer"];

const HEADER_H: f32 = 18.0;
const ROW_H: f32 = 15.0;

/// Una línea del diff alineada para render side-by-side.
///
/// El render usa dos celdas por fila (izquierda = `before`, derecha =
/// `after`). En una línea `Equal`, ambas celdas tienen el mismo
/// contenido. En `Delete`, sólo la izquierda; en `Insert`, sólo la
/// derecha. La struct cumple las dos roles para simplificar el render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    pub kind: DiffKind,
    /// Contenido de la celda izquierda (Equal o Delete) o vacío.
    pub left: Option<DiffCell>,
    /// Contenido de la celda derecha (Equal o Insert) o vacío.
    pub right: Option<DiffCell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffCell {
    /// Número de línea 1-based en el lado correspondiente.
    pub line_no: usize,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Equal,
    Delete,
    Insert,
}

/// Estado del panel.
pub struct DiffState {
    pub before_label: String,
    pub after_label: String,
    pub rows: Vec<DiffRow>,
    pub scroll: usize,
    /// Conteo agregado para mostrar en el header (`+12 / -3` etc.).
    pub stats: DiffStats,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DiffStats {
    pub inserts: usize,
    pub deletes: usize,
    pub equals: usize,
}

impl DiffState {
    /// Construye el state computando el diff entre `before` y `after`.
    /// Líneas se separan por '\n'; el último '\n' se conserva como
    /// separador (no aparece como línea extra vacía).
    pub fn new(
        before_label: impl Into<String>,
        after_label: impl Into<String>,
        before: &str,
        after: &str,
    ) -> Self {
        let (rows, stats) = compute_rows(before, after);
        Self {
            before_label: before_label.into(),
            after_label: after_label.into(),
            rows,
            scroll: 0,
            stats,
        }
    }
}

/// Computa las filas alineadas a partir de los dos textos. La salida
/// preserva el orden lineal del archivo: bloques `Equal` mantienen las
/// líneas pareadas; un `Delete` que no tiene contraparte en el otro
/// lado aparece con `right = None`, y viceversa para `Insert`. No se
/// emparejan visualmente delete con insert — siguen la convención de
/// VS Code, que los muestra como líneas separadas.
pub fn compute_rows(before: &str, after: &str) -> (Vec<DiffRow>, DiffStats) {
    let diff = TextDiff::from_lines(before, after);
    let mut rows: Vec<DiffRow> = Vec::new();
    let mut stats = DiffStats::default();
    let mut left_no = 0usize;
    let mut right_no = 0usize;
    for change in diff.iter_all_changes() {
        let text = change.value().trim_end_matches('\n').to_string();
        match change.tag() {
            ChangeTag::Equal => {
                left_no += 1;
                right_no += 1;
                stats.equals += 1;
                rows.push(DiffRow {
                    kind: DiffKind::Equal,
                    left: Some(DiffCell { line_no: left_no, text: text.clone() }),
                    right: Some(DiffCell { line_no: right_no, text }),
                });
            }
            ChangeTag::Delete => {
                left_no += 1;
                stats.deletes += 1;
                rows.push(DiffRow {
                    kind: DiffKind::Delete,
                    left: Some(DiffCell { line_no: left_no, text }),
                    right: None,
                });
            }
            ChangeTag::Insert => {
                right_no += 1;
                stats.inserts += 1;
                rows.push(DiffRow {
                    kind: DiffKind::Insert,
                    left: None,
                    right: Some(DiffCell { line_no: right_no, text }),
                });
            }
        }
    }
    (rows, stats)
}

/// Vocabulario interno. El host lo wrapea en su Msg.
#[derive(Clone)]
pub enum DiffMsg {
    Open,
    Close,
    /// Scroll vertical en líneas (positivo = baja).
    Scroll(i32),
    /// Salta al próximo hunk (∆+/-) en dirección.
    NextHunk,
    PrevHunk,
}

/// Efecto solicitado al host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffAction {
    None,
    /// El host debería remover el state del modelo.
    Close,
}

pub fn apply(state: &mut DiffState, msg: DiffMsg, visible_rows: usize) -> DiffAction {
    match msg {
        DiffMsg::Open => DiffAction::None,
        DiffMsg::Close => DiffAction::Close,
        DiffMsg::Scroll(delta) => {
            scroll_by(state, delta, visible_rows);
            DiffAction::None
        }
        DiffMsg::NextHunk => {
            jump_to_hunk(state, true, visible_rows);
            DiffAction::None
        }
        DiffMsg::PrevHunk => {
            jump_to_hunk(state, false, visible_rows);
            DiffAction::None
        }
    }
}

fn scroll_by(state: &mut DiffState, delta: i32, visible_rows: usize) {
    let max_scroll = state.rows.len().saturating_sub(visible_rows);
    let new_scroll = (state.scroll as i64 + delta as i64).max(0) as usize;
    state.scroll = new_scroll.min(max_scroll);
}

/// Busca la próxima fila con `kind != Equal` en la dirección dada,
/// empezando justo después/antes del scroll actual. Si no hay más,
/// no-op.
fn jump_to_hunk(state: &mut DiffState, forward: bool, visible_rows: usize) {
    let start = state.scroll;
    let n = state.rows.len();
    let found = if forward {
        (start + 1..n).find(|&i| !matches!(state.rows[i].kind, DiffKind::Equal))
    } else {
        (0..start.min(n)).rev().find(|&i| !matches!(state.rows[i].kind, DiffKind::Equal))
    };
    if let Some(i) = found {
        let max_scroll = n.saturating_sub(visible_rows);
        state.scroll = i.min(max_scroll);
    }
}

/// Routing de teclas cuando el panel está abierto.
pub fn on_key(_state: &DiffState, event: &KeyEvent) -> Option<DiffMsg> {
    if event.state != KeyState::Pressed {
        return None;
    }
    Some(match &event.key {
        Key::Named(NamedKey::Escape) => DiffMsg::Close,
        Key::Named(NamedKey::ArrowDown) => DiffMsg::Scroll(1),
        Key::Named(NamedKey::ArrowUp) => DiffMsg::Scroll(-1),
        Key::Named(NamedKey::PageDown) => DiffMsg::Scroll(20),
        Key::Named(NamedKey::PageUp) => DiffMsg::Scroll(-20),
        Key::Named(NamedKey::Home) => DiffMsg::Scroll(-(i32::MAX / 4)),
        Key::Named(NamedKey::End) => DiffMsg::Scroll(i32::MAX / 4),
        Key::Character(s) if s == "n" => DiffMsg::NextHunk,
        Key::Character(s) if s == "N" => DiffMsg::PrevHunk,
        _ => return None,
    })
}

/// El atajo recomendado: **Ctrl+Shift+D**, similar al "Compare with
/// Saved" de VS Code (que usa Ctrl+Shift+P + comando).
pub fn open_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && event.modifiers.shift
        && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("d"))
}

/// Paleta visual con colores diff convencionales (verde para insert,
/// rojo apagado para delete).
#[derive(Debug, Clone)]
pub struct DiffPalette {
    pub bg_panel: Color,
    pub bg_header: Color,
    pub bg_insert: Color,
    pub bg_delete: Color,
    pub bg_empty: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_insert: Color,
    pub fg_delete: Color,
}

impl DiffPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        // Verde/rojo apagados — visibles sobre fondo oscuro pero sin
        // saturar. Si el theme expone colores semánticos de diff en
        // el futuro, los usamos; por ahora hardcoded.
        Self {
            bg_panel: t.bg_panel,
            bg_header: t.bg_panel_alt,
            bg_insert: Color::from_rgba8(40, 80, 50, 255),
            bg_delete: Color::from_rgba8(90, 40, 45, 255),
            bg_empty: t.bg_panel_alt,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_insert: Color::from_rgba8(170, 230, 180, 255),
            fg_delete: Color::from_rgba8(240, 180, 185, 255),
        }
    }
}

/// Render del panel side-by-side. `height_px` es la altura total
/// disponible; el módulo divide entre el header de 18 px y la grid.
pub fn view<HostMsg, F>(
    state: &DiffState,
    palette: &DiffPalette,
    height_px: f32,
    to_host: F,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
    F: Fn(DiffMsg) -> HostMsg + Copy + 'static,
{
    let _ = to_host; // v0 no monta eventos puntuales sobre filas

    let header_text = format!(
        "diff · {} ↔ {} · +{} -{} ={} · ↑↓ scroll · n/N hunk · Esc cierra",
        state.before_label,
        state.after_label,
        state.stats.inserts,
        state.stats.deletes,
        state.stats.equals,
    );
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(HEADER_H) },
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
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let grid_h = (height_px - HEADER_H).max(0.0);
    let max_rows = ((grid_h / ROW_H) as usize).max(1);
    let end = (state.scroll + max_rows).min(state.rows.len());

    let mut grid_rows: Vec<View<HostMsg>> = Vec::with_capacity(max_rows);
    for row in &state.rows[state.scroll..end] {
        grid_rows.push(render_row(row, palette));
    }
    while grid_rows.len() < max_rows {
        // Padding visual para mantener altura constante.
        grid_rows.push(empty_row(palette));
    }

    let mut children: Vec<View<HostMsg>> = Vec::with_capacity(1 + grid_rows.len());
    children.push(header);
    children.extend(grid_rows);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(height_px) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .children(children)
}

fn render_row<HostMsg>(row: &DiffRow, palette: &DiffPalette) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
{
    let (left_bg, left_fg, left_mark) = match row.kind {
        DiffKind::Equal => (palette.bg_panel, palette.fg_text, " "),
        DiffKind::Delete => (palette.bg_delete, palette.fg_delete, "-"),
        DiffKind::Insert => (palette.bg_empty, palette.fg_muted, " "),
    };
    let (right_bg, right_fg, right_mark) = match row.kind {
        DiffKind::Equal => (palette.bg_panel, palette.fg_text, " "),
        DiffKind::Insert => (palette.bg_insert, palette.fg_insert, "+"),
        DiffKind::Delete => (palette.bg_empty, palette.fg_muted, " "),
    };

    let left_text = match &row.left {
        Some(c) => format!("{:>4} {}{}", c.line_no, left_mark, c.text),
        None => String::new(),
    };
    let right_text = match &row.right {
        Some(c) => format!("{:>4} {}{}", c.line_no, right_mark, c.text),
        None => String::new(),
    };

    let cell = |bg: Color, fg: Color, text: String| {
        View::new(Style {
            flex_grow: 1.0,
            size: Size { width: percent(0.5_f32), height: length(ROW_H) },
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
        .fill(bg)
        .text_aligned(text, 10.5, fg, Alignment::Start)
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(ROW_H) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![cell(left_bg, left_fg, left_text), cell(right_bg, right_fg, right_text)])
}

fn empty_row<HostMsg>(palette: &DiffPalette) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
{
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(ROW_H) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_panel)
}
