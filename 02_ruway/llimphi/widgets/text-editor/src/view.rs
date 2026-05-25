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
use crate::highlight::{Highlighter, Language, Span, SyntaxPalette, TokenKind};
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
}

/// Render principal sin syntax highlight — todas las líneas en
/// `palette.fg_text`. Útil para texto plano o cuando no se conoce el
/// language. Equivale a `text_editor_view_highlighted(.., Language::Plain)`.
pub fn text_editor_view<Msg: Clone + 'static>(
    state: &EditorState,
    palette: &EditorPalette,
    metrics: EditorMetrics,
    height: f32,
    on_focus: Msg,
) -> View<Msg> {
    text_editor_view_highlighted(
        state,
        palette,
        metrics,
        height,
        Language::Plain,
        on_focus,
    )
}

/// Render con syntax highlight. El highlighter se construye on-the-fly
/// (parseo completo del buffer cada call — aceptable para celdas de
/// notebook). Para edición intensiva en archivos grandes, el caller
/// puede cachear su propio `Highlighter` y llamar a una variante que
/// reciba los `Vec<Vec<Span>>` precomputados (TODO si surge el caso).
pub fn text_editor_view_highlighted<Msg: Clone + 'static>(
    state: &EditorState,
    palette: &EditorPalette,
    metrics: EditorMetrics,
    height: f32,
    language: Language,
    on_focus: Msg,
) -> View<Msg> {
    let line_count = state.line_count();
    let caret = state.cursor.caret;
    let syntax = SyntaxPalette::dark_default(&llimphi_theme::Theme::dark());

    let spans = if matches!(language, Language::Plain) {
        Vec::new()
    } else {
        let mut h = Highlighter::new(language);
        h.highlight(&state.text())
    };

    let gutter = build_gutter(line_count, caret.line, metrics, palette);
    let content = build_content(state, palette, metrics, height, spans, &syntax, on_focus);

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
    line_count: usize,
    active_line: usize,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::with_capacity(line_count);
    for n in 0..line_count {
        let color = if n == active_line {
            palette.fg_line_number_active
        } else {
            palette.fg_line_number
        };
        let label = (n + 1).to_string();
        children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(0.0_f32),
                    top: length(n as f32 * metrics.line_height),
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
    spans_per_line: Vec<Vec<Span>>,
    syntax: &SyntaxPalette,
    on_focus: Msg,
) -> View<Msg> {
    let line_count = state.line_count();
    let caret = state.cursor.caret;
    let mut children: Vec<View<Msg>> = Vec::new();

    // 1) Fondo del renglón activo.
    children.push(line_highlight(caret.line, metrics, palette));

    // 2) Selección — un rect por línea afectada.
    if let Some(sel) = state.cursor.selection() {
        children.extend(selection_rects(state, sel, metrics, palette));
    }

    // 2b) Bracket pair bajo el cursor.
    if let Some((a, b)) = crate::bracket::find_bracket_pair(&state.buffer, &state.cursor) {
        children.push(bracket_highlight(a, metrics, palette));
        children.push(bracket_highlight(b, metrics, palette));
    }

    // 3) Texto — una View por línea. Con spans se usan tokens
    //    coloreados; sin spans, monocolor.
    for n in 0..line_count {
        let text = state.buffer.line(n);
        let text = text.trim_end_matches('\n').to_owned();
        if let Some(line_spans) = spans_per_line.get(n) {
            children.push(line_text_tokens(n, &text, line_spans, metrics, palette, syntax));
        } else {
            children.push(line_text_plain(n, text, metrics, palette));
        }
    }

    // 4) Caret — bloque de 2 px de ancho a la izquierda del char en
    //    `(caret.line, caret.col)`. No parpadea (sin animación).
    children.push(caret_rect(caret, metrics, palette));

    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: length(height) },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .on_click(on_focus)
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

fn selection_rects<Msg: Clone + 'static>(
    state: &EditorState,
    sel: crate::cursor::Selection,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> Vec<View<Msg>> {
    let (start_off, end_off) = state.cursor.selection_range(&state.buffer);
    if start_off == end_off {
        return vec![];
    }
    let _ = sel;
    let (start_line, start_col) = state.buffer.offset_to_pos(start_off);
    let (end_line, end_col) = state.buffer.offset_to_pos(end_off);

    let mut out: Vec<View<Msg>> = Vec::new();
    for line in start_line..=end_line {
        let line_len = state.buffer.line_len_chars(line);
        let col_start = if line == start_line { start_col } else { 0 };
        let col_end = if line == end_line { end_col } else { line_len };
        let x = 4.0 + col_start as f32 * metrics.char_width;
        // Si la selección llega al fin de línea, extendemos visualmente
        // el rect un char extra (efecto de "seleccionó hasta el \n").
        let extra = if line < end_line { 1.0 } else { 0.0 };
        let w = ((col_end - col_start) as f32 + extra) * metrics.char_width;
        if w <= 0.0 {
            continue;
        }
        let y = line as f32 * metrics.line_height;
        out.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(x),
                    top: length(y),
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
