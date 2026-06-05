//! Render virtualizado modo línea (Capas 1–2 del SDD-TERMINAL).
//!
//! La apuesta del SDD: scrollback **ilimitado** a costo de render
//! **constante**. Esto lo logra [`line_surface`] materializando **sólo** las
//! filas que caen dentro del viewport — un `ls -alR` de 1 M de líneas pinta
//! ~40 Views (las visibles) + la barra, no un millón. El scroll vive en el
//! **propio widget** (un `scroll_y` en px que el caller guarda en su Model),
//! NO en un `transform` del panel sobre contenido alto (esa fue la fuente del
//! bug clip+transform que ya costó — ver SDD §"Anti-features").
//!
//! El widget es agnóstico de shuma: no sabe de comandos. El color y el tinte
//! de cada renglón los **inyecta el caller** vía el callback `line_style`
//! (Regla 2: núcleo agnóstico, frontend lo pinta). La numeración del gutter es
//! la **global 1-based** del store ([`Scrollback::line_number`]), estable
//! aunque el frente se recorte.

use std::sync::{Arc, Mutex};

use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, FlexDirection, Position, Rect, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, PaintRect, View};
use llimphi_widget_scroll::{max_offset, thumb_geometry, DEFAULT_LINE_PX};

use crate::store::Scrollback;

/// Ancho de la barra de scroll, en px.
const BAR_WIDTH: f32 = 10.0;
/// Alto mínimo del thumb, en px (para que no desaparezca con scrollback enorme).
const MIN_THUMB: f32 = 28.0;

/// Métricas de la superficie — todo derivado del `font_size`. Asume fuente
/// monoespaciada (la mono embebida de `llimphi-text`): `char_width` es el
/// avance fijo de un carácter, base para columnar y para ubicar la selección
/// (Fase 3).
#[derive(Debug, Clone, Copy)]
pub struct TermMetrics {
    pub font_size: f32,
    /// Alto de cada renglón, en px (`font_size * 1.4`).
    pub line_height: f32,
    /// Avance de un carácter mono, en px (`font_size * 0.6`).
    pub char_width: f32,
}

impl Default for TermMetrics {
    fn default() -> Self {
        Self::for_font_size(13.0)
    }
}

impl TermMetrics {
    pub const fn for_font_size(font_size: f32) -> Self {
        Self {
            font_size,
            line_height: font_size * 1.4,
            char_width: font_size * 0.6,
        }
    }
}

/// Paleta de la superficie. Defaults dark, derivables del [`Theme`].
///
/// [`Theme`]: llimphi_theme::Theme
#[derive(Debug, Clone, Copy)]
pub struct TermPalette {
    /// Fondo del área de texto.
    pub bg: Color,
    /// Fondo del gutter (columna de números).
    pub bg_gutter: Color,
    /// Color del texto por defecto (cuando `LineStyle::fg` no se pisa).
    pub fg_text: Color,
    /// Color de los números de línea del gutter.
    pub fg_line_number: Color,
    /// Track de la barra de scroll.
    pub bar_track: Color,
    /// Thumb de la barra en reposo.
    pub bar_thumb: Color,
    /// Thumb de la barra al pasar el cursor.
    pub bar_thumb_hover: Color,
}

impl Default for TermPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TermPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_input,
            bg_gutter: t.bg_panel,
            fg_text: t.fg_text,
            fg_line_number: t.fg_muted,
            bar_track: t.bg_panel_alt,
            bar_thumb: t.border,
            bar_thumb_hover: t.accent,
        }
    }
}

/// Estilo de un renglón concreto, provisto por el caller. El widget no decide
/// color: shuma tinta `ls`/paths/urls (vía `runs`), marca `stderr` con un
/// `bg`, etc., sin que la superficie sepa de comandos.
#[derive(Debug, Clone, Default)]
pub struct LineStyle {
    /// Color base del texto del renglón. `None` → `palette.fg_text`.
    pub fg: Option<Color>,
    /// Overrides de color por rango de **bytes** del renglón
    /// (`(start, end, color)`) — coloreo semántico (ls, syntax). Se clampean
    /// al largo real del texto.
    pub runs: Vec<(usize, usize, Color)>,
    /// Tinte de fondo del renglón completo (p. ej. `stderr` en rojo tenue).
    /// El caller elige el alpha; el widget lo pinta literal.
    pub bg: Option<Color>,
}

impl LineStyle {
    /// Renglón con un color base y sin runs ni tinte — el caso común.
    pub fn fg(color: Color) -> Self {
        Self {
            fg: Some(color),
            ..Default::default()
        }
    }
}

/// La ventana de filas visibles dado el scroll y el viewport. Resultado puro y
/// testeable: el corazón de la virtualización (qué materializar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleWindow {
    /// Primera fila a materializar (índice 0-based **vigente** en el store).
    pub first: usize,
    /// Una más allá de la última fila a materializar (exclusivo).
    pub last: usize,
    /// Píxeles del scroll que caen **dentro** de la primera fila — el desfase
    /// con el que la columna de filas se sube para que el scroll sea suave por
    /// sub-renglón (no salta de fila en fila).
    pub partial_px: i64,
}

impl VisibleWindow {
    /// Cantidad de filas que esta ventana materializa.
    pub fn count(&self) -> usize {
        self.last.saturating_sub(self.first)
    }
}

/// Alto total del contenido (px) si se pintara entero — `filas * alto_fila`.
/// El scrollbar lo usa para dimensionar el thumb; nunca se materializa así.
pub fn content_height(total_rows: usize, row_h: f32) -> f32 {
    total_rows as f32 * row_h
}

/// El `scroll_y` que ancla el contenido **al fondo** (estilo terminal): el
/// máximo offset posible. El caller lo fija mientras el usuario no scrollee
/// arriba, así el append mantiene el fondo pegado.
pub fn scroll_to_bottom(total_rows: usize, viewport_h: f32, row_h: f32) -> f32 {
    max_offset(content_height(total_rows, row_h), viewport_h)
}

/// Calcula la ventana de filas a materializar. **Puro** — sin GPU, sin Views.
///
/// `scroll_y` se clampea a `[0, max_offset]` acá mismo (defensa en
/// profundidad; el caller igual debería clamparlo en su `update`). La ventana
/// incluye una fila de guarda extra al fondo para cubrir el renglón
/// parcialmente visible del borde inferior.
pub fn visible_window(
    total_rows: usize,
    scroll_y: f32,
    viewport_h: f32,
    row_h: f32,
) -> VisibleWindow {
    if total_rows == 0 || row_h <= 0.0 || viewport_h <= 0.0 {
        return VisibleWindow {
            first: 0,
            last: 0,
            partial_px: 0,
        };
    }
    let content_h = content_height(total_rows, row_h);
    let max_off = (content_h - viewport_h).max(0.0);
    let off = scroll_y.clamp(0.0, max_off);

    let first = ((off / row_h).floor() as usize).min(total_rows.saturating_sub(1));
    // Desfase sub-renglón: cuánto del primer renglón ya pasó por arriba.
    let partial_px = (off - first as f32 * row_h).round() as i64;
    // Filas que entran en el viewport + el desfase + una de guarda al fondo.
    let rows_in_view = ((viewport_h + partial_px as f32) / row_h).ceil() as usize + 1;
    let last = (first + rows_in_view).min(total_rows);

    VisibleWindow {
        first,
        last,
        partial_px,
    }
}

/// Ancho del gutter (px) para acomodar el número global más grande posible
/// (`total_pushed`), con un padding fijo a cada lado. Se fija por el total
/// histórico (no por lo visible) para que el gutter no salte al scrollear.
fn gutter_width(store: &Scrollback, metrics: TermMetrics) -> f32 {
    let max_num = store.total_pushed().max(1);
    let digits = (max_num as f64).log10().floor() as usize + 1;
    // 4 px de padding izquierdo + dígitos + 6 px de respiro a la derecha.
    metrics.char_width * digits as f32 + 10.0
}

/// Superficie de terminal **modo línea, virtualizada**.
///
/// Materializa sólo las filas de `[v0, v1)` visibles bajo `scroll_y` (px) en un
/// viewport de alto `viewport_h`. Costo de render **constante** respecto del
/// scrollback. El scroll es del propio widget: `on_scroll(delta_px)` se invoca
/// con el delta a sumar a `scroll_y` (rueda y arrastre de la barra); el caller
/// acumula y clampea con [`llimphi_widget_scroll::clamp_offset`] en su `update`.
///
/// `line_style(idx, texto)` da el color/tinte de cada renglón visible (`idx` es
/// el índice 0-based vigente en el store) — así shuma inyecta su semántica sin
/// que la superficie sepa de comandos.
///
/// `measure`, si se provee, es un `Arc<Mutex<f32>>` que el widget **escribe**
/// con el alto real medido del viewport en cada paint — el caller lo guarda en
/// su Model y lo usa como `viewport_h` el próximo frame (el patrón de medición
/// del shell). En el primer frame `viewport_h` puede ser una estimación.
#[allow(clippy::too_many_arguments)]
pub fn line_surface<Msg, S, F>(
    store: &Scrollback,
    scroll_y: f32,
    viewport_h: f32,
    metrics: TermMetrics,
    palette: &TermPalette,
    line_style: S,
    on_scroll: F,
    measure: Option<Arc<Mutex<f32>>>,
) -> View<Msg>
where
    Msg: Clone + 'static,
    S: Fn(usize, &str) -> LineStyle,
    F: Fn(f32) -> Msg + Send + Sync + 'static,
{
    let total_rows = store.len();
    let row_h = metrics.line_height;
    let gw = gutter_width(store, metrics);
    let win = visible_window(total_rows, scroll_y, viewport_h, row_h);

    // Filas visibles: dos columnas (gutter | contenido) con hijos absolutos
    // ubicados por su índice LOCAL dentro de la ventana. Sólo `win.count()`
    // nodos, no `total_rows` — el corazón de la virtualización.
    let mut gutter_children: Vec<View<Msg>> = Vec::with_capacity(win.count());
    let mut body_children: Vec<View<Msg>> = Vec::with_capacity(win.count() * 2);

    for idx in win.first..win.last {
        let local = (idx - win.first) as f32;
        let y = local * row_h;
        let text = store.line(idx).unwrap_or("");
        let style = line_style(idx, text);

        // Número global 1-based, alineado a la derecha del gutter.
        gutter_children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(0.0_f32),
                    top: length(y),
                    right: length(6.0_f32),
                    bottom: auto(),
                },
                size: Size {
                    width: length(gw - 6.0),
                    height: length(row_h),
                },
                ..Default::default()
            })
            .text_aligned(
                store.line_number(idx).to_string(),
                metrics.font_size * 0.85,
                palette.fg_line_number,
                Alignment::End,
            )
            .mono(),
        );

        // Tinte de fondo del renglón (stderr, etc.) — debajo del texto.
        if let Some(bg) = style.bg {
            body_children.push(
                View::new(Style {
                    position: Position::Absolute,
                    inset: Rect {
                        left: length(0.0_f32),
                        top: length(y),
                        right: length(0.0_f32),
                        bottom: auto(),
                    },
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(row_h),
                    },
                    ..Default::default()
                })
                .fill(bg),
            );
        }

        // Texto del renglón, multicolor por runs (clampeados al largo real).
        let fg = style.fg.unwrap_or(palette.fg_text);
        let runs = clamp_runs(style.runs, text.len());
        body_children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(4.0_f32),
                    top: length(y),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size {
                    width: length(4000.0_f32),
                    height: length(row_h),
                },
                ..Default::default()
            })
            .text_runs(text.to_string(), metrics.font_size, fg, runs, Alignment::Start)
            .mono(),
        );
    }

    let gutter = View::new(Style {
        size: Size {
            width: length(gw),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_gutter)
    .children(gutter_children);

    let body = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: auto(),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .children(body_children);

    // La fila [gutter | contenido], de alto = filas materializadas.
    let rows = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(win.count() as f32 * row_h),
        },
        ..Default::default()
    })
    .children(vec![gutter, body]);

    // La columna de filas se sube `partial_px` para el scroll sub-renglón. El
    // viewport recorta el sobrante (la fila de guarda del fondo).
    let rows_wrap = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(-(win.partial_px as f32)),
            left: length(0.0_f32),
            right: length(0.0_f32),
            bottom: auto(),
        },
        ..Default::default()
    })
    .children(vec![rows]);

    let mut children = vec![rows_wrap];
    let on_wheel = Arc::new(on_scroll);

    // Barra de scroll: sólo si hay overflow. Es del widget (no traslada
    // contenido alto), dimensionada con el alto TOTAL virtual.
    let content_h = content_height(total_rows, row_h);
    if max_offset(content_h, viewport_h) > 0.0 {
        children.push(scrollbar(scroll_y, content_h, viewport_h, palette, &on_wheel));
    }

    // Viewport: alto fijo, contenido recortado, rueda local. Position::Relative
    // para contener los hijos absolutos. El painter de medición captura el alto
    // real para el próximo frame (patrón del shell).
    let on_wheel_view = Arc::clone(&on_wheel);
    let mut viewport = View::new(Style {
        position: Position::Relative,
        size: Size {
            width: percent(1.0_f32),
            height: length(viewport_h),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .on_scroll(move |_dx, dy| Some((on_wheel_view)(dy * DEFAULT_LINE_PX)))
    .children(children);

    if let Some(slot) = measure {
        viewport = viewport.paint_with(move |_scene, _ts, rect: PaintRect| {
            if let Ok(mut g) = slot.lock() {
                *g = rect.h;
            }
        });
    }

    viewport
}

/// Barra de scroll vertical del widget: track + thumb arrastrable. Reusa la
/// geometría de `llimphi-widget-scroll` (`thumb_geometry`) pero dimensionada
/// con el alto TOTAL virtual — el thumb refleja la posición dentro del
/// scrollback completo, no del fragmento materializado.
fn scrollbar<Msg, F>(
    scroll_y: f32,
    content_h: f32,
    viewport_h: f32,
    palette: &TermPalette,
    on_scroll: &Arc<F>,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(f32) -> Msg + Send + Sync + 'static,
{
    let (thumb_h, thumb_y, offset_per_px) = thumb_geometry(scroll_y, content_h, viewport_h);
    let thumb_h = thumb_h.max(MIN_THUMB.min(viewport_h));

    let on_thumb = Arc::clone(on_scroll);
    let thumb = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(thumb_y),
            right: length(0.0_f32),
            left: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(BAR_WIDTH),
            height: length(thumb_h),
        },
        ..Default::default()
    })
    .fill(palette.bar_thumb)
    .hover_fill(palette.bar_thumb_hover)
    .radius((BAR_WIDTH * 0.5) as f64)
    .draggable(move |phase, _dx, dy| match phase {
        DragPhase::Move => Some((on_thumb)(dy * offset_per_px)),
        DragPhase::End => None,
    });

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
            left: auto(),
        },
        size: Size {
            width: length(BAR_WIDTH),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(palette.bar_track)
    .children(vec![thumb])
}

/// Clampa los runs de color al `[0, len]` del texto, descartando los vacíos o
/// fuera de rango — defensa contra runs stale del caller (el texto de un id
/// pudo cambiar de largo).
fn clamp_runs(runs: Vec<(usize, usize, Color)>, len: usize) -> Vec<(usize, usize, Color)> {
    runs.into_iter()
        .filter_map(|(s, e, c)| {
            let s = s.min(len);
            let e = e.min(len);
            (s < e).then_some((s, e, c))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f32 = 18.0;

    #[test]
    fn empty_store_no_window() {
        let w = visible_window(0, 0.0, 600.0, ROW);
        assert_eq!(w.count(), 0);
        assert_eq!(w, VisibleWindow { first: 0, last: 0, partial_px: 0 });
    }

    #[test]
    fn window_at_top_is_constant_cost() {
        // 1 M de filas, viewport de 600 px → ~34 filas + guarda, NO un millón.
        let total = 1_000_000;
        let w = visible_window(total, 0.0, 600.0, ROW);
        assert_eq!(w.first, 0);
        assert_eq!(w.partial_px, 0);
        // ceil(600/18)+1 = 34+1 = 35.
        assert_eq!(w.count(), 35);
        assert!(w.count() < 50, "el costo debe ser constante, no {total}");
    }

    #[test]
    fn window_in_the_middle_has_partial_offset() {
        // Scroll a 1000 px: la primera fila visible es floor(1000/18)=55,
        // y el desfase sub-renglón es 1000 - 55*18 = 1000 - 990 = 10.
        let w = visible_window(1_000_000, 1000.0, 600.0, ROW);
        assert_eq!(w.first, 55);
        assert_eq!(w.partial_px, 10);
        assert!(w.count() < 50);
        assert!(w.last <= 1_000_000);
    }

    #[test]
    fn scroll_clamps_to_bottom() {
        // Scroll exagerado → se clampa al máximo; la última fila visible es la
        // última del store, sin pasarse.
        let total = 500;
        let w = visible_window(total, 1e9, 600.0, ROW);
        assert_eq!(w.last, total);
        // El máximo offset deja exactamente las últimas filas a la vista.
        let bottom = scroll_to_bottom(total, 600.0, ROW);
        let w2 = visible_window(total, bottom, 600.0, ROW);
        assert_eq!(w2.last, total);
    }

    #[test]
    fn content_smaller_than_viewport_shows_all() {
        // 10 filas en 600 px de viewport: entran todas, sin scroll.
        let w = visible_window(10, 0.0, 600.0, ROW);
        assert_eq!(w.first, 0);
        assert_eq!(w.last, 10);
        assert_eq!(scroll_to_bottom(10, 600.0, ROW), 0.0);
    }

    #[test]
    fn window_count_independent_of_scrollback_size() {
        // La invariante central del SDD: el costo no depende del total.
        let a = visible_window(1_000, 9000.0, 600.0, ROW).count();
        let b = visible_window(10_000_000, 9000.0, 600.0, ROW).count();
        assert_eq!(a, b);
    }

    #[test]
    fn clamp_runs_drops_out_of_range() {
        let c = Color::from_rgb8(1, 2, 3);
        let runs = vec![(0, 5, c), (3, 100, c), (50, 60, c), (4, 4, c)];
        let out = clamp_runs(runs, 10);
        // (0,5) queda; (3,100)→(3,10); (50,60) fuera → descartado; (4,4) vacío.
        assert_eq!(out, vec![(0, 5, c), (3, 10, c)]);
    }

    #[test]
    fn gutter_grows_with_line_count() {
        let m = TermMetrics::for_font_size(13.0);
        let mut s = Scrollback::new(0);
        s.push_line("a");
        let narrow = gutter_width(&s, m);
        for _ in 0..100_000 {
            s.push_line("x");
        }
        let wide = gutter_width(&s, m);
        assert!(wide > narrow, "el gutter debe crecer con números más largos");
    }
}
