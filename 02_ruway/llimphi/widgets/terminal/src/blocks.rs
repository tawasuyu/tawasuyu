//! Modelo de bloques + chrome virtualizado (Capa 1 del SDD-TERMINAL, Fase 2).
//!
//! El stream no es una lista plana de líneas: es una secuencia de **bloques**.
//! Para shuma un bloque = un comando (header `$ …` + cuerpo + badge + filas de
//! etapa + estado colapsado). El widget no sabe de comandos (Regla 2): modela
//! [`Item`]s heterogéneos —
//!
//! - [`Item::Chrome`] — un nodo opaco de **alto fijo** que el caller pinta
//!   (header de card, fila de etapas, badge). El widget sólo lo **ubica**.
//! - [`Item::Lines`] — un rango `[start, end)` de líneas del store en modo
//!   línea (numeradas/coloreadas). **Colapsar** un bloque = no emitir su item
//!   `Lines` (o emitirlo con `start == end`): la virtualización lo respeta gratis.
//!
//! [`block_surface`] virtualiza sobre **alturas mixtas**: localiza por búsqueda
//! binaria los items que tocan el viewport y, dentro de un `Lines` enorme,
//! materializa **sólo** las sub-filas visibles. Costo de render constante aunque
//! un body tenga millones de líneas. El modo línea de la Fase 1
//! ([`crate::view::line_surface`]) es el caso de **un solo** `Item::Lines` que
//! cubre todo el store.

use std::sync::{Arc, Mutex};

use llimphi_ui::llimphi_layout::taffy::prelude::{auto, length, percent, Position, Rect, Size, Style};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, PaintRect, View};
use llimphi_widget_scroll::{max_offset, thumb_geometry, DEFAULT_LINE_PX};

use crate::store::Scrollback;
use crate::select::{selection_rects, SelectionRange};
use crate::view::{LineStyle, TermMetrics, TermPalette};

/// Ancho de la barra de scroll, en px.
const BAR_WIDTH: f32 = 10.0;
/// Alto mínimo del thumb, en px (para que no desaparezca con scrollback enorme).
const MIN_THUMB: f32 = 28.0;

/// Un item del stream virtualizado: chrome opaco (alto fijo, lo pinta el caller)
/// o un rango de líneas del store (modo línea).
pub enum Item<Msg> {
    /// Chrome de alto fijo provisto por el caller — header de card, fila de
    /// etapas, badge. El widget lo ubica en su `top` y recorta lo que sobra.
    Chrome { height: f32, view: View<Msg> },
    /// Rango `[start, end)` de líneas del store (índices 0-based vigentes).
    Lines { start: usize, end: usize },
}

impl<Msg> Item<Msg> {
    /// Chrome de alto fijo (header, etapa, badge…).
    pub fn chrome(height: f32, view: View<Msg>) -> Self {
        Self::Chrome { height, view }
    }

    /// Filas del store `[start, end)`.
    pub fn lines(start: usize, end: usize) -> Self {
        Self::Lines { start, end }
    }

    /// Alto del item en px (chrome: el suyo; lines: filas × alto_fila).
    pub fn height(&self, row_h: f32) -> f32 {
        match self {
            Self::Chrome { height, .. } => *height,
            Self::Lines { start, end } => end.saturating_sub(*start) as f32 * row_h,
        }
    }

    /// Geometría liviana (sin la `View` del chrome) para hit-tests fuera del
    /// render. Es `Copy`, así que se puede stashear en un `Mutex` para que el
    /// `update` resuelva clicks contra el layout del frame anterior.
    pub fn geo(&self) -> ItemGeo {
        match self {
            Self::Chrome { height, .. } => ItemGeo::Chrome(*height),
            Self::Lines { start, end } => ItemGeo::Lines(*start, *end),
        }
    }
}

/// Variante liviana y `Copy` de [`Item`] sin la `View` — sólo lo necesario
/// para resolver hit-tests por (lx, ly). Se obtiene con [`Item::geo`].
#[derive(Debug, Clone, Copy)]
pub enum ItemGeo {
    Chrome(f32),
    Lines(usize, usize),
}

impl ItemGeo {
    /// Alto del item en px, mismo cálculo que [`Item::height`].
    pub fn height(&self, row_h: f32) -> f32 {
        match self {
            Self::Chrome(h) => *h,
            Self::Lines(s, e) => e.saturating_sub(*s) as f32 * row_h,
        }
    }
}

/// Tops acumulados (content coords) de cada item dados sus altos, y el alto
/// total. `tops[i]` = `y` del item `i`; el total cierra el contenido. **Puro**.
pub fn item_tops(heights: &[f32]) -> (Vec<f32>, f32) {
    let mut tops = Vec::with_capacity(heights.len());
    let mut acc = 0.0;
    for &h in heights {
        tops.push(acc);
        acc += h;
    }
    (tops, acc)
}

/// Rango `[first, last)` de items que **intersectan** el viewport `[off, off+vp)`
/// bajo `scroll_y` (clampeado a `[0, total-vp]`). `tops` es monótono → búsqueda
/// binaria, O(log n) en la cantidad de bloques. **Puro**.
pub fn visible_items(tops: &[f32], total: f32, scroll_y: f32, viewport_h: f32) -> (usize, usize) {
    let n = tops.len();
    if n == 0 || viewport_h <= 0.0 {
        return (0, 0);
    }
    let off = scroll_y.clamp(0.0, (total - viewport_h).max(0.0));
    // Primer item visible = el que contiene `off` (último top ≤ off). Como
    // `tops[0] == 0 ≤ off`, el prefijo de tops ≤ off es no vacío.
    let first = tops.partition_point(|&t| t <= off).saturating_sub(1);
    // Último visible = primer item cuyo top ya cae fuera del fondo del viewport.
    let last = tops.partition_point(|&t| t < off + viewport_h);
    (first, last.max(first + 1).min(n))
}

/// Sub-filas locales `[k0, k1)` de un item `Lines` de `nrows` filas cuyo `top`
/// (content coords) materializan dentro del viewport `[off, off+vp)`. **Puro**.
fn visible_rows_in_item(top: f32, nrows: usize, off: f32, vp: f32, row_h: f32) -> (usize, usize) {
    if nrows == 0 || row_h <= 0.0 {
        return (0, 0);
    }
    let k0 = (((off - top) / row_h).floor().max(0.0) as usize).min(nrows);
    let k1 = (((off + vp - top) / row_h).ceil().max(0.0) as usize).min(nrows);
    (k0, k1.max(k0))
}

/// Alto total del contenido de una lista de items, en px.
pub fn blocks_height<Msg>(items: &[Item<Msg>], row_h: f32) -> f32 {
    items.iter().map(|it| it.height(row_h)).sum()
}

/// El `scroll_y` que ancla el stream **al fondo** (estilo terminal): el máximo
/// offset posible dada la altura total de los bloques.
pub fn blocks_scroll_to_bottom<Msg>(items: &[Item<Msg>], viewport_h: f32, row_h: f32) -> f32 {
    max_offset(blocks_height(items, row_h), viewport_h)
}

/// Superficie de terminal **por bloques, virtualizada** (Capa 1–2).
///
/// Materializa sólo los items —y dentro de un `Lines`, sólo las sub-filas— que
/// caen en el viewport bajo `scroll_y` (px). Costo de render **constante**
/// respecto del scrollback y del tamaño de cada body. `on_scroll(delta_px)`,
/// `line_style` y `measure` son como en [`crate::view::line_surface`].
///
/// El caller construye **todos** los chrome `View`s (los headers de card); sólo
/// los visibles se pintan (los demás se descartan). Para cientos de bloques —el
/// caso de un shell— es trivial; las **líneas** (los millones) sí se virtualizan
/// de raíz.
#[allow(clippy::too_many_arguments)]
pub fn block_surface<Msg, S, F>(
    store: &Scrollback,
    items: Vec<Item<Msg>>,
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
    block_surface_with_selection(
        store,
        items,
        scroll_y,
        viewport_h,
        metrics,
        palette,
        line_style,
        on_scroll,
        measure,
        SelectionConfig::default(),
    )
}

/// Configura el cableado de selección sobre el `block_surface`. Empaqueta el
/// rango actual (para pintar el overlay) y un handler de drag (para que el
/// caller traduzca cada `(DragPhase, lx0, ly0, dx, dy)` del viewport en
/// `Msg`s que actualicen su estado de selección).
///
/// Diseño en dos partes para mantener el control puro (Regla 2): el widget
/// no toca el modelo, sólo pinta el rango que el caller le pasa y dispara
/// callbacks; el caller arma la `SelectionRange` con [`crate::point_at`] y
/// las acumula en su `Model`.
pub struct SelectionConfig<'a, Msg> {
    /// Rango vigente — si está, se pinta como overlay translúcido.
    pub range: Option<&'a SelectionRange>,
    /// Handler de drag del cuerpo de la superficie. Se ata al viewport con
    /// `draggable_at` (gana sobre el `on_click` global del padre). `None`
    /// = la superficie no es seleccionable por mouse (sólo pinta).
    pub on_drag: Option<Arc<dyn Fn(DragPhase, f32, f32, f32, f32) -> Option<Msg> + Send + Sync>>,
    /// Handler de doble-click. Recibe `(lx, ly, rect_w, rect_h)` del
    /// viewport. El caller lo resuelve a una palabra y la selecciona —
    /// paridad con la UX clásica de terminal (double-click select-word).
    pub on_double_click: Option<Arc<dyn Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync>>,
}

impl<Msg> Default for SelectionConfig<'_, Msg> {
    fn default() -> Self {
        Self {
            range: None,
            on_drag: None,
            on_double_click: None,
        }
    }
}

impl<'a, Msg> SelectionConfig<'a, Msg> {
    /// Sólo pinta el rango (sin cableado de mouse).
    pub fn painted(range: &'a SelectionRange) -> Self {
        Self {
            range: Some(range),
            on_drag: None,
            on_double_click: None,
        }
    }
}

/// Como [`block_surface`], pero acepta una [`SelectionConfig`] para pintar el
/// overlay de selección y/o cablear el drag del mouse al `Msg` del caller.
#[allow(clippy::too_many_arguments)]
pub fn block_surface_with_selection<Msg, S, F>(
    store: &Scrollback,
    items: Vec<Item<Msg>>,
    scroll_y: f32,
    viewport_h: f32,
    metrics: TermMetrics,
    palette: &TermPalette,
    line_style: S,
    on_scroll: F,
    measure: Option<Arc<Mutex<f32>>>,
    selection: SelectionConfig<'_, Msg>,
) -> View<Msg>
where
    Msg: Clone + 'static,
    S: Fn(usize, &str) -> LineStyle,
    F: Fn(f32) -> Msg + Send + Sync + 'static,
{
    let row_h = metrics.line_height;
    let gw = gutter_width(store, metrics);
    let heights: Vec<f32> = items.iter().map(|it| it.height(row_h)).collect();
    let (tops, total) = item_tops(&heights);
    let off = scroll_y.clamp(0.0, max_offset(total, viewport_h));
    let (first, last) = visible_items(&tops, total, off, viewport_h);

    // Highlight de selección: precomputado contra `&items` antes de consumirlos
    // en la iteración. La pintada va DESPUÉS del texto para que se vea encima
    // (alpha de la paleta) y ANTES del scrollbar.
    let sel_rects = match selection.range {
        Some(sel) if !sel.is_empty() => {
            selection_rects(&items, off, viewport_h, metrics, gw, store, sel)
        }
        _ => Vec::new(),
    };

    // Hijos absolutos en coords de viewport (content - off). Sólo los items
    // visibles y, dentro de un Lines, sólo sus sub-filas visibles.
    let mut children: Vec<View<Msg>> = Vec::new();

    for (i, item) in items.into_iter().enumerate() {
        if i < first || i >= last {
            // Fuera de la ventana: el View de chrome se descarta acá (cheap).
            continue;
        }
        let top = tops[i];
        match item {
            Item::Chrome { height, view } => {
                children.push(
                    View::new(Style {
                        position: Position::Absolute,
                        inset: Rect {
                            top: length(top - off),
                            left: length(0.0_f32),
                            right: length(0.0_f32),
                            bottom: auto(),
                        },
                        size: Size {
                            width: percent(1.0_f32),
                            height: length(height),
                        },
                        ..Default::default()
                    })
                    .children(vec![view]),
                );
            }
            Item::Lines { start, end } => {
                let nrows = end.saturating_sub(start);
                if nrows == 0 {
                    continue;
                }
                let item_h = nrows as f32 * row_h;
                // Tira de gutter del bloque, recortada al tramo visible (evita
                // coords gigantes con bodies de millones de px de alto).
                let vis_top = top.max(off);
                let vis_bot = (top + item_h).min(off + viewport_h);
                if vis_bot > vis_top {
                    children.push(gutter_bg(vis_top - off, vis_bot - vis_top, gw, palette));
                }
                let (k0, k1) = visible_rows_in_item(top, nrows, off, viewport_h, row_h);
                for k in k0..k1 {
                    let idx = start + k;
                    let y = top + k as f32 * row_h - off;
                    let text = store.line(idx).unwrap_or("");
                    let style = line_style(idx, text);
                    if let Some(bg) = style.bg {
                        children.push(row_tint(y, row_h, gw, bg));
                    }
                    children.push(gutter_number(store.line_number(idx), y, gw, row_h, metrics, palette));
                    let fg = style.fg.unwrap_or(palette.fg_text);
                    let runs = clamp_runs(style.runs, text.len());
                    children.push(text_row(text, y, gw, row_h, fg, runs, metrics));
                }
            }
        }
    }

    // Overlay del highlight de selección — encima del texto, debajo del
    // scrollbar. Translúcido (alpha en `palette.bg_selection`) para no tapar
    // los glifos.
    for r in &sel_rects {
        children.push(selection_overlay_rect::<Msg>(*r, palette.bg_selection));
    }

    let on_wheel = Arc::new(on_scroll);
    if max_offset(total, viewport_h) > 0.0 {
        children.push(scrollbar(off, total, viewport_h, palette, &on_wheel));
    }

    // Viewport: alto fijo, contenido recortado, rueda local. Relative para
    // contener los hijos absolutos; painter de medición opcional (patrón shell).
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

    // Drag-to-select: forwardea cada `(DragPhase, lx0, ly0, dx, dy)` del
    // viewport al handler del caller. El caller mantiene el `SelectionRange`
    // en su `Model` y usa `crate::point_at` para mapear (lx, ly) → `Point`.
    if let Some(on_drag) = selection.on_drag {
        viewport = viewport.draggable_at(move |phase, dx, dy, lx0, ly0| {
            (on_drag)(phase, lx0, ly0, dx, dy)
        });
    }
    // Doble-click: paridad con terminales clásicas (select-word). El caller
    // resuelve `(lx, ly)` a `Point` con `point_at_geo` + computa los
    // boundaries de palabra y emite un `Msg` que actualiza `surf_selection`.
    if let Some(on_double) = selection.on_double_click {
        viewport = viewport.on_double_tap_at(move |lx, ly, rect_w, rect_h| {
            (on_double)(lx, ly, rect_w, rect_h)
        });
    }

    viewport
}

// ── Builders de nodos por fila (compartidos con la Fase 1) ──────────────────

/// Tira de fondo del gutter de un bloque: rect a la izquierda, ancho `gw`.
fn gutter_bg<Msg: Clone + 'static>(y: f32, h: f32, gw: f32, palette: &TermPalette) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(y),
            left: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(gw),
            height: length(h),
        },
        ..Default::default()
    })
    .fill(palette.bg_gutter)
}

/// Rect translúcido del overlay de selección — coords ya en viewport
/// (scroll descontado por `selection_rects`). Va sobre el texto, sin
/// recolorearlo (alpha del color del caller).
fn selection_overlay_rect<Msg: Clone + 'static>(
    r: crate::select::HighlightRect,
    bg: Color,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(r.y),
            left: length(r.x),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(r.w),
            height: length(r.h),
        },
        ..Default::default()
    })
    .fill(bg)
}

/// Tinte de fondo de un renglón (stderr, etc.), del gutter hacia la derecha.
fn row_tint<Msg: Clone + 'static>(y: f32, row_h: f32, gw: f32, bg: Color) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(y),
            left: length(gw),
            right: length(0.0_f32),
            bottom: auto(),
        },
        size: Size {
            width: auto(),
            height: length(row_h),
        },
        ..Default::default()
    })
    .fill(bg)
}

/// Número global 1-based del renglón, alineado a la derecha del gutter.
fn gutter_number<Msg: Clone + 'static>(
    number: u64,
    y: f32,
    gw: f32,
    row_h: f32,
    metrics: TermMetrics,
    palette: &TermPalette,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(gw - 6.0),
            height: length(row_h),
        },
        ..Default::default()
    })
    .text_aligned(
        number.to_string(),
        metrics.font_size * 0.85,
        palette.fg_line_number,
        Alignment::End,
    )
    .mono()
}

/// Texto de un renglón, multicolor por runs, a la derecha del gutter.
fn text_row<Msg: Clone + 'static>(
    text: &str,
    y: f32,
    gw: f32,
    row_h: f32,
    fg: Color,
    runs: Vec<(usize, usize, Color)>,
    metrics: TermMetrics,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(gw + 4.0),
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
    .mono()
}

/// Barra de scroll vertical del widget: track + thumb arrastrable. Reusa la
/// geometría de `llimphi-widget-scroll` (`thumb_geometry`) dimensionada con el
/// alto TOTAL virtual — el thumb refleja la posición en el scrollback completo.
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

/// Ancho del gutter (px) para acomodar el número global más grande posible
/// (`total_pushed`), con un padding fijo. Se fija por el total histórico (no por
/// lo visible) para que el gutter no salte al scrollear, y es **el mismo** para
/// todos los bloques (los números alinean entre cards).
/// `y` (content coords) del renglón global `target_line` recorrido en el
/// stream de `items`. Devuelve `None` si la línea no cae en ningún
/// `Item::Lines`. **Puro** — base del auto-scroll al match de find: el
/// caller compone `scroll_y = top - margin` y lo clampa al overflow.
pub fn line_top_in_content(items_geo: &[ItemGeo], row_h: f32, target_line: usize) -> Option<f32> {
    let mut top = 0.0_f32;
    for it in items_geo {
        match it {
            ItemGeo::Chrome(h) => top += *h,
            ItemGeo::Lines(start, end) => {
                if target_line >= *start && target_line < *end {
                    return Some(top + (target_line - start) as f32 * row_h);
                }
                top += (end.saturating_sub(*start)) as f32 * row_h;
            }
        }
    }
    None
}

/// Padding extra entre el borde derecho del gutter y el primer carácter del
/// texto del renglón. Lo respeta `text_row` (línea ~495) y DEBE incluirse en
/// los offsets de hit-test (`point_at`) y selección visual para que el rect
/// pintado y el byte_col copiado coincidan con donde cayó el mouse.
pub const TEXT_LEFT_PADDING_PX: f32 = 4.0;

pub fn gutter_width(store: &Scrollback, metrics: TermMetrics) -> f32 {
    let max_num = store.total_pushed().max(1);
    let digits = (max_num as f64).log10().floor() as usize + 1;
    metrics.char_width * digits as f32 + 10.0
}

/// Clampa los runs de color al `[0, len]` del texto, descartando vacíos o fuera
/// de rango — defensa contra runs stale del caller (el texto pudo cambiar).
pub(crate) fn clamp_runs(
    runs: Vec<(usize, usize, Color)>,
    len: usize,
) -> Vec<(usize, usize, Color)> {
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

    fn lines<Msg>(n: usize) -> Item<Msg> {
        Item::lines(0, n)
    }

    #[test]
    fn item_tops_accumulate() {
        let (tops, total) = item_tops(&[10.0, 20.0, 5.0]);
        assert_eq!(tops, vec![0.0, 10.0, 30.0]);
        assert_eq!(total, 35.0);
        let (t, tot) = item_tops(&[]);
        assert!(t.is_empty());
        assert_eq!(tot, 0.0);
    }

    #[test]
    fn visible_items_picks_intersecting_blocks() {
        // 4 bloques de 100px = 400px total; viewport 150px.
        let (tops, total) = item_tops(&[100.0, 100.0, 100.0, 100.0]);
        // Scroll 0 → items 0,1 (y un toque del 2 si entrara, pero 150<200).
        let (a, b) = visible_items(&tops, total, 0.0, 150.0);
        assert_eq!((a, b), (0, 2));
        // Scroll 120 → ventana [120,270): items 1 y 2.
        let (a, b) = visible_items(&tops, total, 120.0, 150.0);
        assert_eq!((a, b), (1, 3));
        // Scroll al fondo → últimos items, sin pasarse.
        let (a, b) = visible_items(&tops, total, 1e9, 150.0);
        assert!(b == 4 && a >= 2);
    }

    #[test]
    fn visible_items_empty() {
        assert_eq!(visible_items(&[], 0.0, 0.0, 100.0), (0, 0));
    }

    #[test]
    fn rows_in_item_is_constant_cost() {
        // Un body de 1 M filas que arranca en top=40; viewport 600, scroll tal
        // que estamos en el medio del body. Materializa ~viewport/row filas.
        let (k0, k1) = visible_rows_in_item(40.0, 1_000_000, 9000.0, 600.0, ROW);
        assert!(k1 - k0 < 40, "costo constante, no {}", k1 - k0);
        // Anclado arriba del item.
        let (k0, k1) = visible_rows_in_item(40.0, 1_000_000, 0.0, 600.0, ROW);
        assert_eq!(k0, 0);
        assert!(k1 <= 34);
    }

    #[test]
    fn rows_in_item_clamps_to_nrows() {
        // Item chico totalmente visible.
        let (k0, k1) = visible_rows_in_item(0.0, 5, 0.0, 600.0, ROW);
        assert_eq!((k0, k1), (0, 5));
    }

    #[test]
    fn blocks_height_and_bottom() {
        let items: Vec<Item<()>> = vec![
            Item::chrome(30.0, View::new(Style::default())),
            lines(10),
            Item::chrome(30.0, View::new(Style::default())),
            lines(100),
        ];
        let h = blocks_height(&items, ROW);
        assert_eq!(h, 30.0 + 10.0 * ROW + 30.0 + 100.0 * ROW);
        // Cabe? No (h grande), así que scroll_to_bottom > 0.
        assert!(blocks_scroll_to_bottom(&items, 200.0, ROW) > 0.0);
        // Si el viewport es enorme, no hay scroll.
        assert_eq!(blocks_scroll_to_bottom(&items, h + 100.0, ROW), 0.0);
    }

    #[test]
    fn item_height_chrome_vs_lines() {
        let c: Item<()> = Item::chrome(42.0, View::new(Style::default()));
        assert_eq!(c.height(ROW), 42.0);
        let l: Item<()> = Item::lines(5, 25);
        assert_eq!(l.height(ROW), 20.0 * ROW);
    }

    #[test]
    fn clamp_runs_drops_out_of_range() {
        let c = Color::from_rgb8(1, 2, 3);
        let runs = vec![(0, 5, c), (3, 100, c), (50, 60, c), (4, 4, c)];
        let out = clamp_runs(runs, 10);
        assert_eq!(out, vec![(0, 5, c), (3, 10, c)]);
    }

    #[test]
    fn line_top_camina_chrome_y_lineas() {
        // [Chrome(22), Lines(0..3), Chrome(10), Lines(0..2)]: target=0 → top=22;
        // target=2 → top=22+2*16=54; target=3 (en el segundo bloque, offset
        // chrome 10 + 3 filas anteriores * 16) → no aplica porque target=3 está
        // FUERA del primer Lines y el segundo es de 0..2 (otro rango). Test
        // ajustado: target=0 cae en el PRIMER Lines (que es 0..3).
        let items: Vec<ItemGeo> = vec![
            ItemGeo::Chrome(22.0),
            ItemGeo::Lines(0, 3),
            ItemGeo::Chrome(10.0),
            ItemGeo::Lines(10, 12),
        ];
        // target_line=0 → primer Lines, k=0 → top = 22 + 0*16 = 22.
        assert_eq!(line_top_in_content(&items, 16.0, 0), Some(22.0));
        // target_line=2 → primer Lines, k=2 → top = 22 + 2*16 = 54.
        assert_eq!(line_top_in_content(&items, 16.0, 2), Some(54.0));
        // target_line=10 → segundo Lines, k=0 → top = 22 + 48 + 10 + 0 = 80.
        assert_eq!(line_top_in_content(&items, 16.0, 10), Some(80.0));
        // target_line=11 → segundo Lines, k=1 → top = 80 + 16 = 96.
        assert_eq!(line_top_in_content(&items, 16.0, 11), Some(96.0));
        // target fuera del store → None.
        assert_eq!(line_top_in_content(&items, 16.0, 99), None);
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
        assert!(gutter_width(&s, m) > narrow);
    }
}
