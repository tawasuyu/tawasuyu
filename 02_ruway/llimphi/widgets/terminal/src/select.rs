//! Modelo de selección del scrollback — base de la Fase 3 del SDD-TERMINAL.
//!
//! La selección se ancla por **(índice de línea en el store vigente, columna
//! en bytes UTF-8 del texto de esa línea)** — no por id global ni por píxeles
//! —, así sobrevive el append al fondo pero el caller debe descartarla si el
//! frente del store se recortó (los índices se corren). El `Scrollback` ya
//! expone `line_id`/`index_of_id` para que el caller traduzca antes/después
//! del `drain` si quiere persistir la selección a través del recorte.
//!
//! Diseño:
//!
//! - `SelectionRange { anchor, head }`: dos puntos. `anchor` = donde empezó
//!   (press), `head` = donde está ahora (drag). `head == anchor` => selección
//!   vacía (cursor sin alcance).
//! - `normalized()`: devuelve `(start, end)` con `start <= end`, **sin** mover
//!   el modelo (la UI quiere saber dónde está el cursor "vivo" para el caret,
//!   pero la extracción/painting necesita el rango ordenado).
//! - `slice_text(store)`: extrae el texto seleccionado, una línea por
//!   renglón del store, recortado por columnas en la primera/última (clampeado
//!   a límites de char UTF-8).
//!
//! Sin dependencias de UI ni de wgpu — puro, testeable a mano. Las pintadas y
//! el cableado de mouse vienen en commits siguientes (Fase 3 continúa).

use crate::blocks::Item;
use crate::store::Scrollback;
use crate::view::TermMetrics;

/// Un punto en el scrollback — un par `(idx_línea, col_byte)`. El índice es
/// vigente en el store (post-recortes); la columna es offset **en bytes** del
/// texto de esa línea. Se clampea al largo real al usar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    /// Índice 0-based de la línea en el store vigente.
    pub line: usize,
    /// Offset en bytes dentro del texto de la línea (clampeado a límite UTF-8).
    pub col: usize,
}

impl Point {
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// Una selección viva — `anchor` (press) y `head` (drag actual). Convertir
/// a `(start, end)` ordenado con [`Self::normalized`] antes de pintar o
/// extraer texto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionRange {
    pub anchor: Point,
    pub head: Point,
}

impl SelectionRange {
    /// Selección colapsada (cursor sin alcance) en `p`.
    pub const fn collapsed(p: Point) -> Self {
        Self { anchor: p, head: p }
    }

    /// `true` si la selección no cubre ningún byte.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Devuelve `(start, end)` con `start <= end` en orden lexicográfico
    /// `(line, col)`. **No** mueve el modelo — el caller decide si quiere
    /// el ancla por separado del head (para el caret).
    pub fn normalized(&self) -> (Point, Point) {
        let a = self.anchor;
        let b = self.head;
        if (a.line, a.col) <= (b.line, b.col) {
            (a, b)
        } else {
            (b, a)
        }
    }

    /// `true` si la selección toca el renglón `line` (alguna parte del
    /// rango está sobre esa línea). Útil para el painter de la ventana
    /// visible: itera filas y pinta el highlight sólo donde aplica.
    pub fn touches_line(&self, line: usize) -> bool {
        let (s, e) = self.normalized();
        line >= s.line && line <= e.line
    }

    /// Rango de columnas `(start_col, end_col_exclusive)` que la selección
    /// cubre en la línea `line` cuyo texto tiene `text_len` bytes.
    /// Para líneas intermedias: `(0, text_len)`. Para la primera/última:
    /// recorta. Si la selección no toca esta línea: `None`.
    pub fn col_range_on(&self, line: usize, text_len: usize) -> Option<(usize, usize)> {
        let (s, e) = self.normalized();
        if line < s.line || line > e.line {
            return None;
        }
        let start = if line == s.line { s.col.min(text_len) } else { 0 };
        let end = if line == e.line {
            e.col.min(text_len)
        } else {
            text_len
        };
        // Si el rango es vacío (selección colapsada justo en límite) → None,
        // para que el painter no dibuje un highlight de 0 bytes.
        if start >= end {
            return None;
        }
        Some((start, end))
    }

    /// Extrae el texto seleccionado del `store`. Multi-línea: las líneas
    /// intermedias enteras, la primera/última recortadas por columna.
    /// Columnas se clampean al límite de char UTF-8 más cercano hacia abajo
    /// (no panic si caen a media codepoint). Líneas fuera del store
    /// vigente se ignoran. Selección vacía → string vacío.
    pub fn slice_text(&self, store: &Scrollback) -> String {
        if self.is_empty() {
            return String::new();
        }
        let (s, e) = self.normalized();
        if store.len() == 0 || s.line >= store.len() {
            return String::new();
        }
        let last_line = e.line.min(store.len().saturating_sub(1));
        let mut out = String::new();
        for line in s.line..=last_line {
            let Some(text) = store.line(line) else {
                continue;
            };
            let (a, b) = if line == s.line && line == e.line {
                (clamp_char_boundary(text, s.col), clamp_char_boundary(text, e.col))
            } else if line == s.line {
                (clamp_char_boundary(text, s.col), text.len())
            } else if line == last_line {
                (0, clamp_char_boundary(text, e.col))
            } else {
                (0, text.len())
            };
            if a < b {
                out.push_str(&text[a..b]);
            }
            if line != last_line {
                out.push('\n');
            }
        }
        out
    }
}

/// Clampea `col` hacia abajo hasta el primer límite de char UTF-8 ≤ `col`.
/// Si `col >= text.len()` devuelve `text.len()`. Garantiza que `text[..ret]`
/// sea un slice válido.
fn clamp_char_boundary(text: &str, col: usize) -> usize {
    if col >= text.len() {
        return text.len();
    }
    let mut c = col;
    while c > 0 && !text.is_char_boundary(c) {
        c -= 1;
    }
    c
}

/// Un rectángulo de highlight para pintar — coords **relativas al viewport**
/// del `block_surface` (origen = esquina superior-izquierda del rect del
/// widget, ya descontado `scroll_y`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Calcula los rectángulos de highlight que pinta una selección sobre la
/// ventana visible de un `block_surface`. **Puro**: no depende de wgpu ni
/// de Views — devuelve geometría que el painter del widget consume con
/// `scene.fill`. El caller pasa `gutter_w` (típicamente vía
/// [`crate::blocks::gutter_width`]) y las métricas de la superficie.
///
/// Sólo emite rects para líneas que (a) caen dentro de un `Item::Lines` del
/// stream y (b) intersectan el viewport. Items `Chrome` no entran (el chrome
/// es opaco y el caller decide su propio highlight si lo necesita).
///
/// Las columnas en `SelectionRange` son **bytes UTF-8**; el rect se calcula
/// en **columnas visuales** (chars contados, mono = 1 cell por char). CJK
/// ancho doble queda fuera del MVP — emite rects de 1 cell por char.
pub fn selection_rects<Msg>(
    items: &[Item<Msg>],
    scroll_y: f32,
    viewport_h: f32,
    metrics: TermMetrics,
    gutter_w: f32,
    store: &Scrollback,
    sel: &SelectionRange,
) -> Vec<HighlightRect> {
    if sel.is_empty() || viewport_h <= 0.0 || metrics.line_height <= 0.0 {
        return Vec::new();
    }
    let (s, e) = sel.normalized();
    let row_h = metrics.line_height;
    let char_w = metrics.char_width.max(0.5);
    let mut out: Vec<HighlightRect> = Vec::new();

    let mut item_top = 0.0_f32;
    for it in items {
        let item_h = it.height(row_h);
        let item_bottom = item_top + item_h;
        // Skip items totalmente fuera del viewport.
        if item_bottom <= scroll_y || item_top >= scroll_y + viewport_h {
            item_top = item_bottom;
            continue;
        }
        if let Item::Lines { start, end } = it {
            let nrows = end.saturating_sub(*start);
            if nrows == 0 {
                item_top = item_bottom;
                continue;
            }
            // Sub-filas dentro del item que tocan el viewport (locales 0-based).
            let off = scroll_y;
            let k0 = (((off - item_top) / row_h).floor().max(0.0) as usize).min(nrows);
            let k1 = (((off + viewport_h - item_top) / row_h).ceil().max(0.0) as usize).min(nrows);
            for k in k0..k1 {
                let idx = start + k;
                if !sel.touches_line(idx) {
                    continue;
                }
                let Some(text) = store.line(idx) else { continue };
                let Some((a, b)) = sel.col_range_on(idx, text.len()) else { continue };
                // Snap a límites UTF-8 (defensa; col_range_on ya clampa a len).
                let a_safe = clamp_char_boundary(text, a);
                let b_safe = clamp_char_boundary(text, b);
                if a_safe >= b_safe {
                    continue;
                }
                let vis_a = text[..a_safe].chars().count() as f32;
                let vis_b = text[..b_safe].chars().count() as f32;
                let row_y = item_top + k as f32 * row_h - scroll_y;
                out.push(HighlightRect {
                    x: gutter_w + vis_a * char_w,
                    y: row_y,
                    w: (vis_b - vis_a) * char_w,
                    h: row_h,
                });
            }
        }
        item_top = item_bottom;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_of(lines: &[&str]) -> Scrollback {
        let mut s = Scrollback::new(0);
        for l in lines {
            s.push_line(l);
        }
        s
    }

    #[test]
    fn collapsed_is_empty_and_yields_empty_slice() {
        let sel = SelectionRange::collapsed(Point::new(0, 0));
        assert!(sel.is_empty());
        let store = store_of(&["hola", "mundo"]);
        assert_eq!(sel.slice_text(&store), "");
    }

    #[test]
    fn normalized_swaps_when_head_before_anchor() {
        let sel = SelectionRange {
            anchor: Point::new(3, 7),
            head: Point::new(1, 2),
        };
        let (s, e) = sel.normalized();
        assert_eq!(s, Point::new(1, 2));
        assert_eq!(e, Point::new(3, 7));
    }

    #[test]
    fn single_line_slice_recorta_por_columnas() {
        let store = store_of(&["the quick brown fox"]);
        let sel = SelectionRange {
            anchor: Point::new(0, 4),
            head: Point::new(0, 9),
        };
        assert_eq!(sel.slice_text(&store), "quick");
    }

    #[test]
    fn multi_line_slice_incluye_lineas_intermedias_completas() {
        let store = store_of(&["uno dos", "tres cuatro", "cinco seis"]);
        let sel = SelectionRange {
            anchor: Point::new(0, 4),
            head: Point::new(2, 5),
        };
        // De "dos" en línea 0 (col 4..7), TODA línea 1, hasta "cinco" en línea 2.
        assert_eq!(sel.slice_text(&store), "dos\ntres cuatro\ncinco");
    }

    #[test]
    fn col_range_on_recorta_solo_primera_y_ultima() {
        let sel = SelectionRange {
            anchor: Point::new(0, 4),
            head: Point::new(2, 5),
        };
        assert_eq!(sel.col_range_on(0, 7), Some((4, 7))); // primera: recorta start
        assert_eq!(sel.col_range_on(1, 11), Some((0, 11))); // intermedia: línea entera
        assert_eq!(sel.col_range_on(2, 10), Some((0, 5))); // última: recorta end
        assert_eq!(sel.col_range_on(3, 10), None); // fuera
    }

    #[test]
    fn col_range_on_descarta_rango_vacio() {
        // Si la selección termina en col 0 de una línea, su contribución a esa
        // línea es 0 bytes → no se debe pintar nada.
        let sel = SelectionRange {
            anchor: Point::new(0, 4),
            head: Point::new(1, 0),
        };
        assert_eq!(sel.col_range_on(1, 5), None);
    }

    #[test]
    fn touches_line_chequea_rango_inclusivo() {
        let sel = SelectionRange {
            anchor: Point::new(2, 0),
            head: Point::new(4, 0),
        };
        assert!(!sel.touches_line(1));
        assert!(sel.touches_line(2));
        assert!(sel.touches_line(3));
        assert!(sel.touches_line(4));
        assert!(!sel.touches_line(5));
    }

    #[test]
    fn slice_text_clampa_col_fuera_de_texto() {
        // col más allá del largo del texto → recorta al fin del texto, sin panic.
        let store = store_of(&["hi"]);
        let sel = SelectionRange {
            anchor: Point::new(0, 0),
            head: Point::new(0, 999),
        };
        assert_eq!(sel.slice_text(&store), "hi");
    }

    #[test]
    fn slice_text_respeta_limites_utf8() {
        // "héllo" — la 'é' es 2 bytes (0xC3 0xA9). Col 2 cae a mitad de char;
        // debe redondear hacia abajo a col 1 (después de 'h'), no panic.
        let store = store_of(&["héllo"]);
        let sel = SelectionRange {
            anchor: Point::new(0, 0),
            head: Point::new(0, 2),
        };
        // col 2 → boundary 1 (después de 'h'); slice "h".
        assert_eq!(sel.slice_text(&store), "h");
    }

    #[test]
    fn slice_text_clampa_lineas_fuera_del_store() {
        // El store tiene 2 líneas; la selección termina en la 5 → recorta a la 1.
        let store = store_of(&["uno", "dos"]);
        let sel = SelectionRange {
            anchor: Point::new(0, 0),
            head: Point::new(5, 999),
        };
        assert_eq!(sel.slice_text(&store), "uno\ndos");
    }

    #[test]
    fn slice_text_de_seleccion_vacia_es_vacio_aun_con_anchor_no_nulo() {
        // anchor == head → vacío, aún si están en (3, 5).
        let store = store_of(&["abcd", "efgh", "ijkl", "mnop"]);
        let sel = SelectionRange::collapsed(Point::new(3, 2));
        assert_eq!(sel.slice_text(&store), "");
    }

    #[test]
    fn slice_text_sobre_store_vacio_es_vacio() {
        let store = Scrollback::new(0);
        let sel = SelectionRange {
            anchor: Point::new(0, 0),
            head: Point::new(2, 5),
        };
        assert_eq!(sel.slice_text(&store), "");
    }

    fn rects<Msg>(
        items: &[Item<Msg>],
        scroll_y: f32,
        viewport_h: f32,
        gutter_w: f32,
        store: &Scrollback,
        sel: &SelectionRange,
    ) -> Vec<HighlightRect> {
        let metrics = TermMetrics {
            font_size: 12.0,
            line_height: 16.0,
            char_width: 8.0,
        };
        selection_rects(items, scroll_y, viewport_h, metrics, gutter_w, store, sel)
    }

    #[test]
    fn rects_de_seleccion_vacia_son_vacio() {
        let store = store_of(&["abc"]);
        let items: Vec<Item<()>> = vec![Item::lines(0, 1)];
        let sel = SelectionRange::collapsed(Point::new(0, 1));
        assert_eq!(rects(&items, 0.0, 100.0, 30.0, &store, &sel), Vec::new());
    }

    #[test]
    fn rect_single_line_ubica_x_y_w_correctos() {
        // Línea 0 entera (3 chars). chars 0..3 → x = gutter + 0, w = 3 * char_w.
        let store = store_of(&["abc"]);
        let items: Vec<Item<()>> = vec![Item::lines(0, 1)];
        let sel = SelectionRange {
            anchor: Point::new(0, 0),
            head: Point::new(0, 3),
        };
        let r = rects(&items, 0.0, 100.0, 30.0, &store, &sel);
        assert_eq!(r.len(), 1);
        let h = r[0];
        assert_eq!(h.x, 30.0);
        assert_eq!(h.y, 0.0);
        assert_eq!(h.w, 24.0); // 3 * 8.0
        assert_eq!(h.h, 16.0);
    }

    #[test]
    fn rect_multi_line_emite_uno_por_renglon() {
        // 3 líneas, selección abarca las 3 (primera/última recortadas).
        let store = store_of(&["alpha", "beta", "gamma"]);
        let items: Vec<Item<()>> = vec![Item::lines(0, 3)];
        let sel = SelectionRange {
            anchor: Point::new(0, 2), // "pha"
            head: Point::new(2, 3),   // "gam"
        };
        let r = rects(&items, 0.0, 100.0, 30.0, &store, &sel);
        assert_eq!(r.len(), 3);
        // Línea 0: chars 2..5 → x = 30 + 2*8 = 46, w = 3*8 = 24
        assert_eq!(r[0].x, 46.0);
        assert_eq!(r[0].w, 24.0);
        // Línea 1 entera: "beta" (4 chars).
        assert_eq!(r[1].x, 30.0);
        assert_eq!(r[1].w, 32.0);
        // Línea 2: chars 0..3 → "gam".
        assert_eq!(r[2].x, 30.0);
        assert_eq!(r[2].w, 24.0);
    }

    #[test]
    fn rects_descartan_lineas_fuera_del_viewport() {
        // 100 líneas, viewport 32 px (=2 filas), scroll a la mitad → sólo 2-3 rects.
        let lines: Vec<&str> = (0..100).map(|_| "row").collect();
        let store = store_of(&lines);
        let items: Vec<Item<()>> = vec![Item::lines(0, 100)];
        // Selección sobre TODAS las líneas, pero sólo 2-3 entran al viewport.
        let sel = SelectionRange {
            anchor: Point::new(0, 0),
            head: Point::new(99, 3),
        };
        // scroll a la fila 50 (50 * 16 = 800 px). Viewport de 32 px → filas
        // 50, 51 (+ guarda).
        let r = rects(&items, 800.0, 32.0, 30.0, &store, &sel);
        assert!(r.len() <= 3 && !r.is_empty(),
            "esperado ~2-3 rects, no {} (todas las líneas)", r.len());
    }

    #[test]
    fn rects_saltan_items_chrome() {
        // Item 0 = chrome (alto 20), item 1 = 2 líneas. Selección sobre las dos
        // líneas. El chrome no debe aportar rects.
        let store = store_of(&["aa", "bb"]);
        let chrome_view: llimphi_ui::View<()> = llimphi_ui::View::new(Default::default());
        let items: Vec<Item<()>> = vec![Item::chrome(20.0, chrome_view), Item::lines(0, 2)];
        let sel = SelectionRange {
            anchor: Point::new(0, 0),
            head: Point::new(1, 2),
        };
        let r = rects(&items, 0.0, 100.0, 30.0, &store, &sel);
        assert_eq!(r.len(), 2);
        // El primer rect arranca DESPUÉS del chrome (y = 20).
        assert_eq!(r[0].y, 20.0);
        // El segundo está una fila más abajo (20 + 16 = 36).
        assert_eq!(r[1].y, 36.0);
    }

    #[test]
    fn rects_usan_visual_cols_no_bytes_para_utf8() {
        // "héllo" — 'é' es 2 bytes, pero 1 char visual. Selección de col 0 a
        // col 3 (byte) → snap a 3 (después de "hé"), 2 chars visuales.
        let store = store_of(&["héllo"]);
        let items: Vec<Item<()>> = vec![Item::lines(0, 1)];
        let sel = SelectionRange {
            anchor: Point::new(0, 0),
            head: Point::new(0, 3),
        };
        let r = rects(&items, 0.0, 100.0, 30.0, &store, &sel);
        assert_eq!(r.len(), 1);
        // 2 chars visuales × 8 px = 16.
        assert_eq!(r[0].w, 16.0);
    }

    #[test]
    fn slice_text_a_linea_intermedia_omite_las_que_no_existen() {
        // Si una línea intermedia desaparece (no debería pasar acá pero el
        // store sólo expone `line()`), se omite — no se inserta `\n` extra.
        // Acá lo cubrimos indirectamente con un store contiguo.
        let store = store_of(&["aa", "bb", "cc"]);
        let sel = SelectionRange {
            anchor: Point::new(0, 1),
            head: Point::new(2, 1),
        };
        assert_eq!(sel.slice_text(&store), "a\nbb\nc");
    }
}
