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

/// Tope de líneas que la variante embebida (`text_editor_view_colored`)
/// renderiza de una. La virtualización del editor-de-archivos capa a 200 para
/// no generar miles de Views (wgpu rechaza el bind group); pero la variante
/// embebida deja el scroll al contenedor de afuera y necesita pintar TODAS sus
/// líneas (si no, la mitad de abajo queda sin pintar = negro al anclar el panel
/// al fondo). Este tope es sólo la red de seguridad de wgpu — el caller acota el
/// total real (el shell, por su `MAX_VISIBLE = 400`). Probado: ~400 líneas
/// renderizan sin que wgpu rechace nada (el render plano viejo ya lo hacía).
pub const EMBEDDED_LINE_CAP: usize = 512;

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

/// Cómo renderizar la columna izquierda del editor.
///
/// - [`GutterStyle::Numbers`] es el comportamiento clásico de IDE:
///   "1", "2", "3"… alineados a la derecha del gutter.
/// - [`GutterStyle::Phantom`] suprime los números y dibuja en su lugar
///   un tick **muy sutil** por línea (un pequeño segmento horizontal
///   con baja opacidad). Sirve para prosa narrativa donde el número de
///   línea es ruido — la línea sigue estando, pero "fingiendo no
///   estar". El gutter en este modo se acorta a un sliver fino.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GutterStyle {
    #[default]
    Numbers,
    Phantom,
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
    /// Cómo se pinta el gutter. Default [`GutterStyle::Numbers`] — el
    /// comportamiento clásico se conserva para callers existentes.
    pub gutter_style: GutterStyle,
    /// Si `true`, cada línea **guarda** (índices en
    /// `EditorState::guard_lines`) recibe un segmento horizontal con
    /// baja opacidad atravesando su centro — un divisor fantasma que
    /// sugiere "acá termina un bloque" sin gritar. Sin guardas, esto
    /// no hace nada visible. Default `false`: comportamiento IDE
    /// clásico.
    pub phantom_guard_lines: bool,
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
            gutter_style: GutterStyle::Numbers,
            phantom_guard_lines: false,
        }
    }

    /// Variante "prosa": gutter fantasma (ticks sutiles, sin números) +
    /// divisores fantasma en cada guarda. Ancho del gutter reducido a
    /// un sliver porque ya no necesita acomodar dígitos.
    ///
    /// Pensado para editores narrativos tipo `cuerpo_ide` donde el
    /// número de línea es ruido y las junctions están marcadas como
    /// guardas.
    pub const fn prosa(font_size: f32) -> Self {
        Self {
            font_size,
            line_height: font_size * 1.4,
            char_width: font_size * 0.6,
            gutter_width: font_size * 1.0,
            gutter_style: GutterStyle::Phantom,
            phantom_guard_lines: true,
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
        let line_local = ((local_y - PAD_Y).max(0.0) / self.line_height) as usize;
        let col = ((local_x - PAD_X).max(0.0) / self.char_width).round() as usize;
        (scroll_offset + line_local, col)
    }
}

/// Sangría izquierda del área de texto (px): aire entre el gutter y el
/// primer carácter. El caret, la selección, el preedit y `screen_to_pos`
/// usan la misma constante para no desalinearse.
pub(crate) const PAD_X: f32 = 10.0;
/// Aire vertical arriba de la primera línea (px). El gutter lo respeta para
/// que los números queden a la par del texto.
pub(crate) const PAD_Y: f32 = 7.0;
/// Margen izquierdo del número de línea (px) — separa del borde del panel.
const GUTTER_PAD_L: f32 = 6.0;
/// Respiro entre el número de línea y el texto (px).
const GUTTER_PAD_R: f32 = 8.0;

/// X (px, dentro del área de contenido) del carácter en la columna `col`.
#[inline]
fn text_x(col: usize, m: EditorMetrics) -> f32 {
    PAD_X + col as f32 * m.char_width
}
/// Y (px) del tope de la línea `local_line` (relativa al viewport).
#[inline]
fn text_y(local_line: usize, m: EditorMetrics) -> f32 {
    PAD_Y + local_line as f32 * m.line_height
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

/// Override de estilo de texto sobre un rango de **columnas-char**
/// `[start_col, end_col)` de una línea. A diferencia de [`Span`] (que sólo
/// lleva una `TokenKind` → color), un `StyledSpan` lleva el set completo de
/// propiedades rich-text que el caller quiera pisar: color de glifo, color de
/// fondo (resaltado), familia, tamaño, peso, itálica, subrayado y tachado.
/// Cada campo es opcional — `None` hereda del default del editor.
///
/// El widget traduce internamente las columnas-char a offsets de byte (que es
/// lo que consume `text_spans`/parley) y pinta el `bg` como rect por debajo
/// del texto (parley no lleva fondo por span). Pensado para editores de prosa
/// que estilan zonas o la selección (pluma multilienzo); no se combina con el
/// syntax highlight por `Language` — es una vía de pintado paralela.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyledSpan {
    pub start_col: usize,
    pub end_col: usize,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub font_family: Option<String>,
    pub size_px: Option<f32>,
    pub weight: Option<f32>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
}

/// Como [`text_editor_view_highlighted`] pero el caller provee, por línea, un
/// conjunto de [`StyledSpan`] con estilo rich-text completo (color/fondo/
/// fuente/tamaño/peso/itálica/subrayado/tachado). `styled_per_line[n]` son los
/// spans de la línea `n` del buffer (índice absoluto). Las líneas sin spans se
/// pintan planas en `palette.fg_text`. Va por encima del syntax highlight: si
/// pasás spans, mandan ellos (el caso de pluma, que es prosa sin lenguaje).
pub fn text_editor_view_styled<Msg: Clone + 'static>(
    state: &EditorState,
    palette: &EditorPalette,
    metrics: EditorMetrics,
    visible_lines: usize,
    styled_per_line: &[Vec<StyledSpan>],
    match_ranges: &[(usize, usize)],
    on_pointer: impl Fn(PointerEvent) -> Option<Msg> + Send + Sync + Clone + 'static,
) -> View<Msg> {
    let caret = state.cursor.caret;
    let syntax = crate::syntax_palette_dark(&llimphi_theme::Theme::dark());

    let visible = visible_lines.max(1).min(200);
    let line_count = state.line_count();
    let scroll = state.scroll_offset.min(line_count.saturating_sub(1));
    let end_line = (scroll + visible).min(line_count);
    let height = (end_line - scroll) as f32 * metrics.line_height;

    let gutter = build_gutter(state, scroll, end_line, caret.line, metrics, palette);
    let content = build_content(
        state,
        palette,
        metrics,
        height,
        scroll,
        end_line,
        Vec::new(),
        &syntax,
        match_ranges,
        None,
        Some(styled_per_line),
        on_pointer,
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        min_size: Size { width: auto(), height: length(height) },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![gutter, content])
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
    let syntax = crate::syntax_palette_dark(&llimphi_theme::Theme::dark());

    let visible = visible_lines.max(1).min(200);
    let line_count = state.line_count();
    let scroll = state.scroll_offset.min(line_count.saturating_sub(1));
    let end_line = (scroll + visible).min(line_count);
    let height = (end_line - scroll) as f32 * metrics.line_height;

    // Memoizado por `edit_seq` — sólo reparseamos cuando el buffer
    // realmente cambió o cambia el `Language`.
    let spans = state.highlighted_spans(language);

    let gutter = build_gutter(state, scroll, end_line, caret.line, metrics, palette);
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
        None,
        None,
        on_pointer,
    );

    // Llena el alto disponible (para verse como UN campo continuo: se puede
    // clickear/tipear debajo de la última línea), pero nunca colapsa por debajo
    // del alto del contenido — `min_size` es el piso si el contenedor es
    // chico/indefinido. El caret y los clicks debajo del texto aterrizan en la
    // última línea (los clampea `set_caret_at`).
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        min_size: Size { width: auto(), height: length(height) },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![gutter, content])
}

/// Como [`text_editor_view`] pero el caller provee el color de cada tramo de
/// cada línea (`line_color_runs[n]` = `(byte_start, byte_end, Color)` de la
/// línea `n`), en vez de derivarlo de un `Language`. Para outputs con coloreo
/// semántico propio (un shell que tinta `ls`, paths, urls, números…) sobre el
/// mismo editor read-only (numeración + selección + copiar).
pub fn text_editor_view_colored<Msg: Clone + 'static>(
    state: &EditorState,
    palette: &EditorPalette,
    metrics: EditorMetrics,
    visible_lines: usize,
    line_color_runs: &[Vec<(usize, usize, Color)>],
    on_pointer: impl Fn(PointerEvent) -> Option<Msg> + Send + Sync + Clone + 'static,
) -> View<Msg> {
    let caret = state.cursor.caret;
    let syntax = crate::syntax_palette_dark(&llimphi_theme::Theme::dark());
    // Variante embebida: el contenedor de afuera (el panel de output del shell)
    // hace el scroll y reserva alto para TODAS las líneas, así que las pintamos
    // completas (cap alto = red de seguridad de wgpu, ver `EMBEDDED_LINE_CAP`).
    let visible = visible_lines.max(1).min(EMBEDDED_LINE_CAP);
    let line_count = state.line_count();
    let scroll = state.scroll_offset.min(line_count.saturating_sub(1));
    let end_line = (scroll + visible).min(line_count);
    let height = (end_line - scroll) as f32 * metrics.line_height;
    let gutter = build_gutter(state, scroll, end_line, caret.line, metrics, palette);
    let content = build_content(
        state,
        palette,
        metrics,
        height,
        scroll,
        end_line,
        Vec::new(),
        &syntax,
        &[],
        Some(line_color_runs),
        None,
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
    state: &EditorState,
    scroll: usize,
    end_line: usize,
    active_line: usize,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    let count = end_line.saturating_sub(scroll);
    let mut children: Vec<View<Msg>> = Vec::with_capacity(count);
    for n in scroll..end_line {
        // Las líneas-guarda son separadores estructurales entre zonas
        // de texto: ni se numeran ni se pueden escribir. El espacio
        // se preserva (la línea sigue existiendo), pero el gutter las
        // saltea — visualmente la numeración "rompe" en cada zona.
        // Si `guard_lines` está vacío, este check es siempre `false`
        // y la numeración cubre todas las líneas (modo IDE clásico).
        if state.is_guard_line(n) {
            continue;
        }
        let color = if n == active_line {
            palette.fg_line_number_active
        } else {
            palette.fg_line_number
        };
        let y = text_y(n - scroll, metrics);
        match metrics.gutter_style {
            GutterStyle::Numbers => {
                let label = (n + 1).to_string();
                // Número alineado a la derecha con un respiro de `GUTTER_PAD_R`
                // hasta el texto (antes 4 px → se pegaba y el clip lo cortaba) y
                // un margen izquierdo para que no toque el borde de la pantalla.
                children.push(
                    View::new(Style {
                        position: Position::Absolute,
                        inset: Rect {
                            left: length(GUTTER_PAD_L),
                            top: length(y),
                            right: length(GUTTER_PAD_R),
                            bottom: auto(),
                        },
                        size: Size {
                            width: length(
                                (metrics.gutter_width - GUTTER_PAD_L - GUTTER_PAD_R).max(1.0),
                            ),
                            height: length(metrics.line_height),
                        },
                        align_items: Some(AlignItems::Center),
                        ..Default::default()
                    })
                    .text_aligned(label, metrics.font_size * 0.85, color, Alignment::End)
                    .mono(),
                );
            }
            GutterStyle::Phantom => {
                // Tick fantasma — un segmento horizontal corto centrado
                // verticalmente en la línea, con la opacidad bajada.
                // La línea activa queda un pelín más visible.
                let alpha = if n == active_line { 0.35 } else { 0.12 };
                let tick_w = (metrics.gutter_width * 0.5).max(3.0);
                let tick_h = 1.0_f32;
                let tick_y = y + (metrics.line_height - tick_h) * 0.5;
                let tick_x = (metrics.gutter_width - tick_w) * 0.5;
                children.push(
                    View::new(Style {
                        position: Position::Absolute,
                        inset: Rect {
                            left: length(tick_x),
                            top: length(tick_y),
                            right: auto(),
                            bottom: auto(),
                        },
                        size: Size {
                            width: length(tick_w),
                            height: length(tick_h),
                        },
                        ..Default::default()
                    })
                    .fill(with_alpha(color, alpha)),
                );
            }
        }
    }

    // En modo Phantom el gutter es un sliver: no aplicamos `fill` —
    // se mezcla con el fondo del editor. El gutter "está sin estar".
    let bg = match metrics.gutter_style {
        GutterStyle::Numbers => palette.bg_gutter,
        GutterStyle::Phantom => palette.bg,
    };
    View::new(Style {
        size: Size {
            width: length(metrics.gutter_width),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .clip(true)
    .children(children)
}

/// Devuelve `c` con la opacidad multiplicada por `alpha` (clamp 0..1).
fn with_alpha(c: Color, alpha: f32) -> Color {
    let rgba = c.to_rgba8();
    let a = ((alpha.clamp(0.0, 1.0)) * (rgba.a as f32)) as u8;
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, a)
}

#[allow(clippy::too_many_arguments)]
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
    // Override de color por línea: `color_runs[n]` son `(byte_start, byte_end,
    // Color)` para la línea `n` del buffer (índice absoluto). Cuando es `Some`,
    // gana sobre el syntax highlight — para callers que colorean por semántica
    // propia (un shell que tinta `ls`, paths, urls…). `None` = highlight normal.
    color_runs: Option<&[Vec<(usize, usize, Color)>]>,
    // Spans rich-text por línea (color/fondo/fuente/tamaño/peso/itálica/…).
    // Cuando es `Some`, ganan sobre el syntax highlight — la vía de pluma para
    // estilar zonas y selección de prosa. `None` = comportamiento previo.
    styled_per_line: Option<&[Vec<StyledSpan>]>,
    on_pointer: impl Fn(PointerEvent) -> Option<Msg> + Send + Sync + Clone + 'static,
) -> View<Msg> {
    let caret = state.cursor.caret;
    let mut children: Vec<View<Msg>> = Vec::new();

    // 0) Tintes por línea — la capa más baja, debajo de todo el resto.
    //    Pinta un rect del ancho completo del área de contenido por
    //    cada línea con tinte asignado. El caller elige el alpha — el
    //    widget no lo modula. Si la línea cae fuera de viewport o no
    //    tiene tinte, no se pinta nada.
    for n in scroll..end_line {
        if let Some(Some(c)) = state.line_tints.get(n) {
            children.push(line_tint(n - scroll, *c, metrics));
        }
    }

    // 1) Fondo del renglón activo — sólo el del primary cursor.
    if caret.line >= scroll && caret.line < end_line {
        children.push(line_highlight(caret.line - scroll, metrics, palette));
    }

    // 1b) Highlight de matches del find.
    for (s, e) in match_ranges {
        children.extend(match_rects(state, *s, *e, scroll, end_line, metrics, palette));
    }

    // 1c) Fondos de los spans estilados (resaltado). Por debajo de la
    //     selección y del texto, igual que los tintes — un rect por span
    //     con `bg`. Char-cols → x/w vía `char_width` (mono).
    if let Some(styled) = styled_per_line {
        for n in scroll..end_line {
            if let Some(line_spans) = styled.get(n) {
                children.extend(styled_bg_rects(n - scroll, line_spans, metrics));
            }
        }
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
    //    Si `phantom_guard_lines` está activo, cada guarda recibe un
    //    divisor fantasma (segmento horizontal con baja opacidad)
    //    atravesando su centro — sin texto, sólo un susurro visual.
    for n in scroll..end_line {
        let text = state.buffer.line(n);
        let text = text.trim_end_matches('\n').to_owned();
        let local_line = n - scroll;
        if metrics.phantom_guard_lines && state.is_guard_line(n) {
            children.push(phantom_guard_divider(local_line, metrics, palette));
            continue;
        }
        if let Some(line_spans) = styled_per_line.and_then(|s| s.get(n)).filter(|s| !s.is_empty()) {
            children.push(line_text_styled(local_line, &text, line_spans, metrics, palette));
        } else if let Some(runs) = color_runs.and_then(|cr| cr.get(n)) {
            children.push(line_text_color_runs(local_line, &text, runs, metrics, palette));
        } else if let Some(line_spans) = spans_per_line.get(n) {
            children.push(line_text_tokens(local_line, &text, line_spans, metrics, palette, syntax));
        } else {
            children.push(line_text_plain(local_line, text, metrics, palette));
        }
    }

    // 3b) Diagnostics — subrayado bajo el rango, color por severity.
    for d in &state.diagnostics {
        children.extend(diagnostic_underline(d, scroll, end_line, metrics, palette));
    }

    // 4) Caret — uno por cursor, sólo si visible. El caret del cursor
    //    primario se corre detrás del preedit del IME en composición (el
    //    texto compuesto se pinta desde `p.col`), para que quede al final
    //    de lo que el usuario está tecleando. En la fase "apagada" del
    //    parpadeo (`caret_on == false`) no se dibuja ningún caret.
    let preedit_cols = state.preedit.as_ref().map_or(0, |p| p.text.chars().count());
    for c in state.all_cursors() {
        let p = c.caret;
        if state.caret_on && p.line >= scroll && p.line < end_line {
            let is_primary = std::ptr::eq(c, &state.cursor);
            let col = if is_primary { p.col + preedit_cols } else { p.col };
            let local = crate::cursor::Pos::new(p.line - scroll, col);
            children.push(caret_rect(local, metrics, palette));
        }
    }

    // 5) Preedit del IME — texto en composición pintado en el caret con
    //    subrayado, todavía fuera del buffer. Sólo en el cursor primario y
    //    si su línea está en viewport. (En mono el ancho es exacto; el
    //    texto que sigue al caret puede solaparse mientras se compone —
    //    transitorio y, en el caso típico de acentos, de un solo char.)
    if let Some(pre) = state.preedit.as_ref() {
        let p = state.cursor.caret;
        if p.line >= scroll && p.line < end_line {
            children.extend(preedit_views(p.line - scroll, p.col, &pre.text, metrics, palette));
        }
    }

    let click_cb = on_pointer.clone();
    let drag_cb = on_pointer;
    View::new(Style {
        flex_grow: 1.0,
        // Llena el alto del editor (campo continuo), con piso en el alto del
        // contenido para el modo embebido que scrollea (`_colored`).
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        min_size: Size { width: auto(), height: length(height) },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    // Cursor de texto (I-beam) sobre el área editable; fuera vuelve al normal.
    .cursor(llimphi_ui::Cursor::Text)
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

/// Rect de tinte para una línea. Cubre el ancho completo y el alto
/// exacto de la línea, pintado al color literal pasado (el caller
/// elige el alpha). Posición absoluta dentro del área de contenido.
fn line_tint<Msg: Clone + 'static>(
    line: usize,
    color: Color,
    metrics: EditorMetrics,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(text_y(line, metrics)),
            right: length(0.0_f32),
            bottom: auto(),
        },
        size: Size {
            width: percent(1.0_f32),
            height: length(metrics.line_height),
        },
        ..Default::default()
    })
    .fill(color)
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
            top: length(text_y(line, metrics)),
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

/// Línea-fantasma para una guarda: un segmento horizontal con baja
/// opacidad atravesando el centro vertical de la línea. Ancho
/// limitado para que parezca un susurro y no una regla. Color derivado
/// de `fg_line_number` que ya está pensado como "muted".
fn phantom_guard_divider<Msg: Clone + 'static>(
    line: usize,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    let h = 1.0_f32;
    let y = text_y(line, metrics) + (metrics.line_height - h) * 0.5;
    // Largo visual del divisor — generoso pero no infinito.
    let w = 320.0_f32;
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(8.0_f32),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(w),
            height: length(h),
        },
        ..Default::default()
    })
    .fill(with_alpha(palette.fg_line_number, 0.18))
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
            left: length(PAD_X),
            top: length(text_y(line, metrics)),
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
    .mono()
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
    // char-col → byte-offset: parley rangea por bytes, los spans por chars.
    let mut byte_at: Vec<usize> = Vec::with_capacity(text.len() + 1);
    let mut acc = 0usize;
    byte_at.push(0);
    for ch in text.chars() {
        acc += ch.len_utf8();
        byte_at.push(acc);
    }
    let nchars = byte_at.len() - 1;

    // Un run de color por span no-Other (el default_color cubre el resto).
    let mut runs: Vec<(usize, usize, Color)> = Vec::with_capacity(spans.len());
    for span in spans {
        if span.start_col >= nchars || matches!(span.kind, TokenKind::Other) {
            continue;
        }
        let end = span.end_col.min(nchars);
        if end <= span.start_col {
            continue;
        }
        runs.push((byte_at[span.start_col], byte_at[end], syntax.color(span.kind)));
    }

    // Una sola línea shapeada de una vez, multicolor, en lugar de un nodo
    // (+ layout parley) por token. El `+4` de gutter va en el inset del nodo.
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(PAD_X),
            top: length(text_y(line, metrics)),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(2000.0_f32), height: length(metrics.line_height) },
        ..Default::default()
    })
    .text_runs(
        text.to_string(),
        metrics.font_size,
        palette.fg_text,
        runs,
        Alignment::Start,
    )
    .mono()
}

/// Como [`line_text_tokens`] pero con `(byte_start, byte_end, Color)`
/// explícitos provistos por el caller (coloreo semántico propio, p. ej. un
/// shell que tinta `ls`/paths/urls). El resto del texto va en `fg_text`.
fn line_text_color_runs<Msg: Clone + 'static>(
    line: usize,
    text: &str,
    runs: &[(usize, usize, Color)],
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(PAD_X),
            top: length(text_y(line, metrics)),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(2000.0_f32), height: length(metrics.line_height) },
        ..Default::default()
    })
    .text_runs(
        text.to_string(),
        metrics.font_size,
        palette.fg_text,
        runs.to_vec(),
        Alignment::Start,
    )
    .mono()
}

/// Renderiza una línea con spans rich-text ([`StyledSpan`]): construye los
/// [`llimphi_text::TextSpan`] (offsets en byte, vía el mapa char→byte) y los
/// pasa a `text_spans`. El fondo de cada span lo pinta [`styled_bg_rects`]
/// aparte. El texto fuera de todo span va en `palette.fg_text` /
/// `metrics.font_size`. Mantiene `.mono()` para que el posicionamiento por
/// `char_width` (caret/selección) siga cuadrando.
fn line_text_styled<Msg: Clone + 'static>(
    line: usize,
    text: &str,
    spans: &[StyledSpan],
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    // char-col → byte-offset (parley rangea por bytes; los spans, por chars).
    let mut byte_at: Vec<usize> = Vec::with_capacity(text.len() + 1);
    let mut acc = 0usize;
    byte_at.push(0);
    for ch in text.chars() {
        acc += ch.len_utf8();
        byte_at.push(acc);
    }
    let nchars = byte_at.len() - 1;

    let mut ts: Vec<llimphi_ui::llimphi_text::TextSpan> = Vec::with_capacity(spans.len());
    for s in spans {
        if s.start_col >= nchars {
            continue;
        }
        let end = s.end_col.min(nchars);
        if end <= s.start_col {
            continue;
        }
        let style = llimphi_ui::llimphi_text::TextSpanStyle {
            size_px: s.size_px,
            weight: s.weight,
            italic: s.italic,
            font_family: s.font_family.clone(),
            color: s.fg,
            underline: s.underline,
            strikethrough: s.strikethrough,
        };
        ts.push(llimphi_ui::llimphi_text::TextSpan::new(
            byte_at[s.start_col],
            byte_at[end],
            style,
        ));
    }

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(PAD_X),
            top: length(text_y(line, metrics)),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(2000.0_f32), height: length(metrics.line_height) },
        ..Default::default()
    })
    .text_spans(
        text.to_string(),
        metrics.font_size,
        palette.fg_text,
        ts,
        Alignment::Start,
    )
    .mono()
}

/// Rects de fondo (resaltado) de los spans de una línea con `bg` definido.
/// Char-cols → x/w por `char_width` (mono), mismo origen que el texto.
fn styled_bg_rects<Msg: Clone + 'static>(
    local_line: usize,
    spans: &[StyledSpan],
    metrics: EditorMetrics,
) -> Vec<View<Msg>> {
    let mut out: Vec<View<Msg>> = Vec::new();
    for s in spans {
        let Some(bg) = s.bg else { continue };
        if s.end_col <= s.start_col {
            continue;
        }
        let x = text_x(s.start_col, metrics);
        let w = (s.end_col - s.start_col) as f32 * metrics.char_width;
        out.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(x),
                    top: length(text_y(local_line, metrics)),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size { width: length(w), height: length(metrics.line_height) },
                ..Default::default()
            })
            .fill(bg),
        );
    }
    out
}

fn caret_rect<Msg: Clone + 'static>(
    caret: Pos,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    let x = text_x(caret.col, metrics);
    let y = text_y(caret.line, metrics);
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

/// Pinta el texto en composición del IME en `(local_line, col)`: el texto
/// provisional + un subrayado debajo que lo marca como no-confirmado.
/// Devuelve los dos Views (texto y subrayado). Posición en coords del
/// área de contenido (mismo origen que [`line_text_plain`]).
fn preedit_views<Msg: Clone + 'static>(
    local_line: usize,
    col: usize,
    text: &str,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> Vec<View<Msg>> {
    let x = text_x(col, metrics);
    let y = text_y(local_line, metrics);
    let w = (text.chars().count() as f32 * metrics.char_width).max(metrics.char_width);
    vec![
        // Texto provisional, en el color de texto normal.
        View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(x),
                top: length(y),
                right: auto(),
                bottom: auto(),
            },
            size: Size { width: length(w), height: length(metrics.line_height) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(text.to_string(), metrics.font_size, palette.fg_text, Alignment::Start)
        .mono(),
        // Subrayado: una línea fina en el color del caret bajo el texto.
        View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(x),
                top: length(y + metrics.line_height - 2.0),
                right: auto(),
                bottom: auto(),
            },
            size: Size { width: length(w), height: length(1.5_f32) },
            ..Default::default()
        })
        .fill(palette.caret),
    ]
}

fn bracket_highlight<Msg: Clone + 'static>(
    pos: Pos,
    metrics: EditorMetrics,
    palette: &EditorPalette,
) -> View<Msg> {
    let x = text_x(pos.col, metrics);
    let y = text_y(pos.line, metrics);
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
        let x = text_x(col_start, metrics);
        let w = (col_end - col_start) as f32 * metrics.char_width;
        // Subrayado de 1.5 px al final de la línea.
        let y = text_y(line - scroll, metrics) + metrics.line_height - 2.0;
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
        let x = text_x(col_start, metrics);
        let w = (col_end - col_start) as f32 * metrics.char_width;
        let local_y = text_y(line - scroll, metrics);
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
        let x = text_x(col_start, metrics);
        let extra = if line < end_line { 1.0 } else { 0.0 };
        let w = ((col_end - col_start) as f32 + extra) * metrics.char_width;
        if w <= 0.0 {
            continue;
        }
        let local_y = text_y(line - scroll, metrics);
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
