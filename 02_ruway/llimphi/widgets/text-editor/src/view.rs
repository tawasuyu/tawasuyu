//! Render del editor. Layout: gutter izquierdo (line numbers) + área
//! principal (texto + selección como rects + caret bloque). El scroll
//! vertical es implícito por viewport — el caller decide cuántas líneas
//! caben en el `height` que pasa.
//!
//! Limitaciones del PMV de render:
//! - **Char width fijo** — asume fuente monoespaciada y un ancho de
//!   carácter en píxeles fijo. Para CJK / proportional el caret y la
//!   selección se desalinean. Para texto ASCII monoespaciado es exacto.
//! - **Selección multilínea** se pinta como un rect por línea afectada
//!   (sin "rio" continuo); estilo Sublime Text / antiguo, lectura clara.
//! - **Sin syntax highlight todavía** — eso vive en su propio bloque y
//!   requiere `llimphi-text` rich (Vec<Run>); aquí cada línea va
//!   monocolor `fg_text`.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Rect, Size, Style},
    AlignItems,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

use crate::cursor::Pos;
use crate::diagnostics::{Diagnostic, Severity};
use crate::highlight::{Language, Span, SyntaxPalette, TokenKind};
use crate::state::EditorState;

/// Paleta del editor. Defaults dark.
#[derive(Debug, Clone, Copy)]
pub struct EditorPalette {
    pub bg: Color,
    pub bg_gutter: Color,
    pub bg_selection: Color,
    pub bg_current_line: Color,
    pub fg_text: Color,
    pub fg_line_number: Color,
    pub fg_line_number_active: Color,
    pub caret: Color,
    /// Fondo del bracket bajo el cursor + su par. Un acento sutil.
    pub bg_bracket_pair: Color,
    /// Fondo de cada match del find activo.
    pub bg_match: Color,
    /// Subrayado de diagnostic — Error.
    pub diag_error: Color,
    /// Subrayado de diagnostic — Warning.
    pub diag_warning: Color,
    /// Subrayado de diagnostic — Information.
    pub diag_info: Color,
    /// Subrayado de diagnostic — Hint.
    pub diag_hint: Color,
}

impl Default for EditorPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl EditorPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        // Reutilizamos slots del theme; los que no existen como semánticos
        // se derivan con `mix`/transparencia conceptual.
        Self {
            bg: t.bg_input,
            bg_gutter: t.bg_panel,
            bg_selection: t.bg_selected,
            bg_current_line: t.bg_panel_alt,
            fg_text: t.fg_text,
            fg_line_number: t.fg_muted,
            fg_line_number_active: t.fg_text,
            caret: t.accent,
            bg_bracket_pair: t.bg_button_hover,
            bg_match: t.bg_button_hover,
            diag_error: t.fg_destructive,
            diag_warning: Color::from_rgb8(229, 192, 123),
            diag_info: Color::from_rgb8(97, 175, 239),
            diag_hint: t.fg_muted,
        }
    }
}

/// Métricas del editor — todo derivado del `font_size`. Cambiar la
/// fuente requiere recalcular `char_width` empíricamente para la mono
/// que use llimphi-text; los valores acá son razonables para
/// `font_size = 12` con la mono default de parley.
#[derive(Debug, Clone, Copy)]
pub struct EditorMetrics {
    pub font_size: f32,
    /// Alto de cada línea en píxeles (font_size * line_height_ratio).
    pub line_height: f32,
    /// Ancho promedio de un char (mono). Si la fuente no es mono, esto
    /// es sólo una aproximación.
    pub char_width: f32,
    /// Ancho del gutter (incluye padding interno).
    pub gutter_width: f32,
}

impl Default for EditorMetrics {
    fn default() -> Self {
        Self::for_font_size(12.0)
    }
}

impl EditorMetrics {
    pub const fn for_font_size(font_size: f32) -> Self {
        Self {
            font_size,
            line_height: font_size * 1.4,
            char_width: font_size * 0.6,
            gutter_width: font_size * 3.5,
        }
    }

    /// Convierte coords locales del **área de contenido** (no del gutter)
    /// a `(line, col)` absolutas en el buffer. `local_x` se mide desde el
    /// borde izquierdo del área de texto (sin el padding interno de 4 px);
    /// `local_y` desde la primera línea visible.
    ///
    /// Devuelve coordenadas siempre dentro del buffer — el caller
    /// generalmente las pasa a `EditorState::set_caret_at` que clampea
    /// `col` al ancho real de la línea.
    pub fn screen_to_pos(self, local_x: f32, local_y: f32, scroll_offset: usize) -> (usize, usize) {
        let line_local = (local_y / self.line_height).max(0.0) as usize;
        let col = ((local_x - 4.0).max(0.0) / self.char_width).round() as usize;
        (scroll_offset + line_local, col)
    }
}

/// Render principal sin syntax highlight — todas las líneas visibles
/// en `palette.fg_text`. `visible_lines` es cuántas líneas mostrar como
/// máximo en el viewport.
///
/// `on_pointer` se invoca con el evento del mouse dentro del área de
/// texto (no del gutter): el caller decide cómo mover el caret /
/// extender selección. Ver [`PointerEvent`].
pub fn text_editor_view<Msg: Clone + 'static>(
    state: &EditorState,
    palette: &EditorPalette,
    metrics: EditorMetrics,
    visible_lines: usize,
    on_pointer: impl Fn(PointerEvent) -> Option<Msg> + Send + Sync + Clone + 'static,
) -> View<Msg> {
    text_editor_view_highlighted(
        state,
        palette,
        metrics,
        visible_lines,
        Language::Plain,
        on_pointer,
    )
}

/// Evento de mouse que el view envía al caller dentro del área de texto.
/// El caller convierte `(x, y)` con [`EditorMetrics::screen_to_pos`] y
/// aplica `set_caret_at` (Click) o `extend_selection_to` (Drag).
///
/// `Drag` entrega `initial` (pos del press inicial, constante durante el
/// drag) + `delta` (delta desde el evento anterior). El caller debe
/// acumular el delta — el view no mantiene state. Patrón típico:
/// `accum += (dx, dy); actual = (initial_x + accum.0, initial_y + accum.1)`.
#[derive(Debug, Clone, Copy)]
pub enum PointerEvent {
    Click { x: f32, y: f32 },
    Drag { initial_x: f32, initial_y: f32, dx: f32, dy: f32 },
}

/// Render con syntax highlight + **viewport scrolling**: sólo se renderizan
/// las líneas en `[state.scroll_offset, scroll_offset + visible_lines)`.
///
/// `visible_lines` es cuántas líneas máximo dibujamos por frame; el caller
/// se asegura de tener un container con altura ≥ `visible_lines * line_height`
/// o aplica clip propio. Para archivos grandes (1000+ líneas), el cap es
/// crítico — sin él generaríamos miles de Views y wgpu rechazaría el bind
/// group por `max_*_buffer_binding_size`.
///
/// Recomendación para el caller: tras cada edición, llamar a
/// [`EditorState::ensure_caret_visible`] con el mismo `visible_lines` para
/// que el viewport siga al caret.
pub fn text_editor_view_highlighted<Msg: Clone + 'static>(
    state: &EditorState,
    palette: &EditorPalette,
    metrics: EditorMetrics,
    visible_lines: usize,
    language: Language,
    on_pointer: impl Fn(PointerEvent) -> Option<Msg> + Send + Sync + Clone + 'static,
) -> View<Msg> {
    text_editor_view_full(
        state,
        palette,
        metrics,
        visible_lines,
        language,
        &[],
        on_pointer,
    )
}

/// Como [`text_editor_view_highlighted`] + `match_ranges` para pintar
/// las ocurrencias de un find activo. Cada par `(char_start, char_end)`
/// es un rango de chars globales del buffer.
pub fn text_editor_view_full<Msg: Clone + 'static>(
    state: &EditorState,
    palette: &EditorPalette,
    metrics: EditorMetrics,
    visible_lines: usize,
    language: Language,
    match_ranges: &[(usize, usize)],
    on_pointer: impl Fn(PointerEvent) -> Option<Msg> + Send + Sync + Clone + 'static,
) -> View<Msg> {
    let caret = state.cursor.caret;
    let syntax = SyntaxPalette::dark_default(&llimphi_theme::Theme::dark());

    let visible = visible_lines.max(1).min(200);
    let line_count = state.line_count();
    let scroll = state.scroll_offset.min(line_count.saturating_sub(1));
    let end_line = (scroll + visible).min(line_count);
    let height = (end_line - scroll) as f32 * metrics.line_height;

    // Memoizado por `edit_seq` — sólo reparseamos cuando el buffer
    // realmente cambió o cambia el `Language`.
    let spans = state.highlighted_spans(language);

    let gutter = build_gutter(scroll, end_line, caret.line, metrics, palette);
    let content = build_content(
        state,
        palette,
        metrics,
        height,
        scroll,
        end_line,
        spans,
        &syntax,
        match_ranges,
        on_pointer,
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(height) },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![gutter, content])
}

fn build_gutter<Msg: Clone + 'static>(
    scroll: usize,
    end_line: usize,
    active_line: usize,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    let count = end_line.saturating_sub(scroll);
    let mut children: Vec<View<Msg>> = Vec::with_capacity(count);
    for n in scroll..end_line {
        let color = if n == active_line {
            palette.fg_line_number_active
        } else {
            palette.fg_line_number
        };
        let label = (n + 1).to_string();
        let y = (n - scroll) as f32 * metrics.line_height;
        children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(0.0_f32),
                    top: length(y),
                    right: length(4.0_f32),
                    bottom: auto(),
                },
                size: Size {
                    width: length(metrics.gutter_width - 4.0),
                    height: length(metrics.line_height),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(label, metrics.font_size * 0.85, color, Alignment::End),
        );
    }

    View::new(Style {
        size: Size {
            width: length(metrics.gutter_width),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_gutter)
    .clip(true)
    .children(children)
}

fn build_content<Msg: Clone + 'static>(
    state: &EditorState,
    palette: &EditorPalette,
    metrics: EditorMetrics,
    height: f32,
    scroll: usize,
    end_line: usize,
    spans_per_line: Vec<Vec<Span>>,
    syntax: &SyntaxPalette,
    match_ranges: &[(usize, usize)],
    on_pointer: impl Fn(PointerEvent) -> Option<Msg> + Send + Sync + Clone + 'static,
) -> View<Msg> {
    let caret = state.cursor.caret;
    let mut children: Vec<View<Msg>> = Vec::new();

    // 1) Fondo del renglón activo — sólo el del primary cursor.
    if caret.line >= scroll && caret.line < end_line {
        children.push(line_highlight(caret.line - scroll, metrics, palette));
    }

    // 1b) Highlight de matches del find.
    for (s, e) in match_ranges {
        children.extend(match_rects(state, *s, *e, scroll, end_line, metrics, palette));
    }

    // 2) Selección — por cada cursor que tenga selección.
    for c in state.all_cursors() {
        if c.has_selection() {
            children.extend(selection_rects_for_cursor(
                state, c, scroll, end_line, metrics, palette,
            ));
        }
    }

    // 2b) Bracket pair bajo el primary cursor — si visible.
    if let Some((a, b)) = crate::bracket::find_bracket_pair(&state.buffer, &state.cursor) {
        if a.line >= scroll && a.line < end_line {
            children.push(bracket_highlight(crate::cursor::Pos::new(a.line - scroll, a.col), metrics, palette));
        }
        if b.line >= scroll && b.line < end_line {
            children.push(bracket_highlight(crate::cursor::Pos::new(b.line - scroll, b.col), metrics, palette));
        }
    }

    // 3) Texto — sólo las líneas en viewport.
    for n in scroll..end_line {
        let text = state.buffer.line(n);
        let text = text.trim_end_matches('\n').to_owned();
        let local_line = n - scroll;
        if let Some(line_spans) = spans_per_line.get(n) {
            children.push(line_text_tokens(local_line, &text, line_spans, metrics, palette, syntax));
        } else {
            children.push(line_text_plain(local_line, text, metrics, palette));
        }
    }

    // 3b) Diagnostics — subrayado bajo el rango, color por severity.
    for d in &state.diagnostics {
        children.extend(diagnostic_underline(d, scroll, end_line, metrics, palette));
    }

    // 4) Caret — uno por cursor, sólo si visible.
    for c in state.all_cursors() {
        let p = c.caret;
        if p.line >= scroll && p.line < end_line {
            let local = crate::cursor::Pos::new(p.line - scroll, p.col);
            children.push(caret_rect(local, metrics, palette));
        }
    }

    let click_cb = on_pointer.clone();
    let drag_cb = on_pointer;
    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: length(height) },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .on_click_at(move |x, y, _w, _h| click_cb(PointerEvent::Click { x, y }))
    .draggable_at(move |phase, dx, dy, lx, ly| match phase {
        llimphi_ui::DragPhase::Move => drag_cb(PointerEvent::Drag {
            initial_x: lx,
            initial_y: ly,
            dx,
            dy,
        }),
        llimphi_ui::DragPhase::End => None,
    })
    .children(children)
}

fn line_highlight<Msg: Clone + 'static>(
    line: usize,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(line as f32 * metrics.line_height),
            right: length(0.0_f32),
            bottom: auto(),
        },
        size: Size {
            width: percent(1.0_f32),
            height: length(metrics.line_height),
        },
        ..Default::default()
    })
    .fill(palette.bg_current_line)
}

fn line_text_plain<Msg: Clone + 'static>(
    line: usize,
    text: String,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(4.0_f32),
            top: length(line as f32 * metrics.line_height),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(2000.0_f32),
            height: length(metrics.line_height),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text, metrics.font_size, palette.fg_text, Alignment::Start)
}

/// Renderiza una línea como secuencia de Views absolutos posicionados,
/// cada uno con el color de su span. El posicionamiento horizontal usa
/// `char_width` (mono); para fuentes proporcionales habría que medir
/// cada token con parley (TODO).
fn line_text_tokens<Msg: Clone + 'static>(
    line: usize,
    text: &str,
    spans: &[Span],
    metrics: EditorMetrics,
    palette: &EditorPalette,
    syntax: &SyntaxPalette,
) -> View<Msg> {
    let chars: Vec<char> = text.chars().collect();
    let mut children: Vec<View<Msg>> = Vec::with_capacity(spans.len());

    for span in spans {
        if span.start_col >= chars.len() {
            continue;
        }
        let end = span.end_col.min(chars.len());
        if end <= span.start_col {
            continue;
        }
        let token: String = chars[span.start_col..end].iter().collect();
        if token.chars().all(|c| c.is_whitespace()) && !matches!(span.kind, TokenKind::String) {
            continue;
        }
        let x = 4.0 + span.start_col as f32 * metrics.char_width;
        let w = (end - span.start_col) as f32 * metrics.char_width + metrics.char_width;
        let color = if matches!(span.kind, TokenKind::Other) {
            palette.fg_text
        } else {
            syntax.color(span.kind)
        };
        children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(x),
                    top: length(0.0_f32),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size { width: length(w), height: length(metrics.line_height) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(token, metrics.font_size, color, Alignment::Start),
        );
    }

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(line as f32 * metrics.line_height),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(2000.0_f32), height: length(metrics.line_height) },
        ..Default::default()
    })
    .children(children)
}

fn caret_rect<Msg: Clone + 'static>(
    caret: Pos,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    let x = 4.0 + caret.col as f32 * metrics.char_width;
    let y = caret.line as f32 * metrics.line_height;
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y + 2.0),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(2.0_f32), height: length(metrics.line_height - 4.0) },
        ..Default::default()
    })
    .fill(palette.caret)
}

fn bracket_highlight<Msg: Clone + 'static>(
    pos: Pos,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    let x = 4.0 + pos.col as f32 * metrics.char_width;
    let y = pos.line as f32 * metrics.line_height;
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(metrics.char_width), height: length(metrics.line_height) },
        ..Default::default()
    })
    .fill(palette.bg_bracket_pair)
}

fn diagnostic_underline<Msg: Clone + 'static>(
    d: &Diagnostic,
    scroll: usize,
    end_viewport: usize,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> Vec<View<Msg>> {
    let color = match d.severity {
        Severity::Error => palette.diag_error,
        Severity::Warning => palette.diag_warning,
        Severity::Information => palette.diag_info,
        Severity::Hint => palette.diag_hint,
    };
    let mut out: Vec<View<Msg>> = Vec::new();
    let first = d.range.start.line.max(scroll);
    let last = d.range.end.line.min(end_viewport.saturating_sub(1));
    if first > last {
        return out;
    }
    for line in first..=last {
        let col_start = if line == d.range.start.line { d.range.start.col } else { 0 };
        let col_end = if line == d.range.end.line {
            d.range.end.col
        } else {
            // Fin de línea — extendemos 1 char extra para visualizar el wrap.
            col_start + 1
        };
        if col_end <= col_start {
            continue;
        }
        let x = 4.0 + col_start as f32 * metrics.char_width;
        let w = (col_end - col_start) as f32 * metrics.char_width;
        // Subrayado de 1.5 px al final de la línea.
        let y = (line - scroll) as f32 * metrics.line_height + metrics.line_height - 2.0;
        out.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(x),
                    top: length(y),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size { width: length(w), height: length(1.5_f32) },
                ..Default::default()
            })
            .fill(color),
        );
    }
    out
}

fn match_rects<Msg: Clone + 'static>(
    state: &EditorState,
    start_off: usize,
    end_off: usize,
    scroll: usize,
    end_viewport: usize,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> Vec<View<Msg>> {
    if start_off == end_off {
        return vec![];
    }
    let (start_line, start_col) = state.buffer.offset_to_pos(start_off);
    let (end_line, end_col) = state.buffer.offset_to_pos(end_off);
    let mut out: Vec<View<Msg>> = Vec::new();
    let first = start_line.max(scroll);
    let last = end_line.min(end_viewport.saturating_sub(1));
    if first > last {
        return out;
    }
    for line in first..=last {
        let line_len = state.buffer.line_len_chars(line);
        let col_start = if line == start_line { start_col } else { 0 };
        let col_end = if line == end_line { end_col } else { line_len };
        if col_end <= col_start {
            continue;
        }
        let x = 4.0 + col_start as f32 * metrics.char_width;
        let w = (col_end - col_start) as f32 * metrics.char_width;
        let local_y = (line - scroll) as f32 * metrics.line_height;
        out.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(x),
                    top: length(local_y),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size { width: length(w), height: length(metrics.line_height) },
                ..Default::default()
            })
            .fill(palette.bg_match),
        );
    }
    out
}

fn selection_rects_for_cursor<Msg: Clone + 'static>(
    state: &EditorState,
    cursor: &crate::cursor::Cursor,
    scroll: usize,
    end_viewport: usize,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> Vec<View<Msg>> {
    let (start_off, end_off) = cursor.selection_range(&state.buffer);
    if start_off == end_off {
        return vec![];
    }
    let (start_line, start_col) = state.buffer.offset_to_pos(start_off);
    let (end_line, end_col) = state.buffer.offset_to_pos(end_off);

    let mut out: Vec<View<Msg>> = Vec::new();
    let first = start_line.max(scroll);
    let last = end_line.min(end_viewport.saturating_sub(1));
    if first > last {
        return out;
    }
    for line in first..=last {
        let line_len = state.buffer.line_len_chars(line);
        let col_start = if line == start_line { start_col } else { 0 };
        let col_end = if line == end_line { end_col } else { line_len };
        let x = 4.0 + col_start as f32 * metrics.char_width;
        let extra = if line < end_line { 1.0 } else { 0.0 };
        let w = ((col_end - col_start) as f32 + extra) * metrics.char_width;
        if w <= 0.0 {
            continue;
        }
        let local_y = (line - scroll) as f32 * metrics.line_height;
        out.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(x),
                    top: length(local_y),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size { width: length(w), height: length(metrics.line_height) },
                ..Default::default()
            })
            .fill(palette.bg_selection),
        );
    }
    out
}
