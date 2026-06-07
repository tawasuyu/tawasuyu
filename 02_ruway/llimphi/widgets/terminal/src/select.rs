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

use crate::store::Scrollback;

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
