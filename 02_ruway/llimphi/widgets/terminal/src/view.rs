//! Tipos de la superficie + render virtualizado **modo línea** (Capas 1–2).
//!
//! Acá viven los tipos compartidos ([`TermMetrics`], [`TermPalette`],
//! [`LineStyle`]) y la matemática pura de la ventana visible de **filas
//! uniformes** ([`visible_window`]). El render modo línea, [`line_surface`], es
//! el caso particular —un solo bloque de líneas que cubre todo el store— del
//! modelo de bloques general de [`crate::blocks`] (Fase 2): delega en
//! [`crate::blocks::block_surface`] para no duplicar la maquinaria de
//! virtualización ni los builders de fila.
//!
//! La apuesta del SDD: scrollback **ilimitado** a costo de render
//! **constante** — sólo se materializan las filas que caen en el viewport. El
//! scroll vive en el **propio widget** (un `scroll_y` en px que el caller
//! guarda en su Model), NO en un `transform` del panel sobre contenido alto
//! (esa fue la fuente del bug clip+transform que ya costó — SDD §"Anti-features").
//!
//! El widget es agnóstico de shuma: el color/tinte de cada renglón los **inyecta
//! el caller** vía `line_style` (Regla 2). La numeración del gutter es la global
//! 1-based del store, estable aunque el frente se recorte.

use std::sync::{Arc, Mutex};

use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_widget_scroll::max_offset;

use crate::blocks::{block_surface, Item};
use crate::store::Scrollback;

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
/// testeable: el corazón de la virtualización de **filas uniformes**.
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

/// Calcula la ventana de filas a materializar para un stream **de filas
/// uniformes** (sin bloques). **Puro** — sin GPU, sin Views.
///
/// `scroll_y` se clampea a `[0, max_offset]` acá mismo (defensa en
/// profundidad). La ventana incluye una fila de guarda extra al fondo para
/// cubrir el renglón parcialmente visible del borde inferior.
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

/// Superficie de terminal **modo línea, virtualizada** — el caso de **un solo
/// bloque** de líneas que cubre todo el store. Delega en
/// [`crate::blocks::block_surface`].
///
/// `on_scroll(delta_px)` se invoca con el delta a sumar a `scroll_y` (rueda y
/// arrastre de la barra); el caller acumula y clampea con
/// [`llimphi_widget_scroll::clamp_offset`] en su `update`. `line_style(idx,
/// texto)` da el color/tinte de cada renglón visible. `measure`, si se provee,
/// recibe el alto real del viewport en cada paint (patrón de medición del shell).
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
    block_surface(
        store,
        vec![Item::lines(0, store.len())],
        scroll_y,
        viewport_h,
        metrics,
        palette,
        line_style,
        on_scroll,
        measure,
    )
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
}
