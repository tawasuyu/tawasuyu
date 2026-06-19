//! Búsqueda en el buffer. PMV: case-insensitive opcional, sin regex,
//! sin replace. La UI del prompt vive en el caller (típicamente una
//! barra arriba del editor); este módulo sólo provee:
//!
//! - [`FindState`] con el query actual + dirección + flag case-sensitive.
//! - [`find_next`] / [`find_prev`] que devuelven la próxima/anterior
//!   match desde el caret del editor.
//! - [`all_matches`] para que el render resalte cada ocurrencia.

use crate::buffer::Buffer;
use crate::cursor::{Cursor, Pos};

/// Configuración de búsqueda del editor.
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub query: String,
    pub case_sensitive: bool,
}

impl FindState {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_query(query: impl Into<String>) -> Self {
        Self { query: query.into(), case_sensitive: false }
    }
    pub fn is_active(&self) -> bool {
        !self.query.is_empty()
    }
}

/// Devuelve todas las ocurrencias del query en el buffer como
/// `(start_offset, end_offset)` en char offsets. Vacío si query vacío.
pub fn all_matches(buf: &Buffer, find: &FindState) -> Vec<(usize, usize)> {
    if find.query.is_empty() {
        return Vec::new();
    }
    let hay = buf.text();
    let (hay_search, needle_search) = if find.case_sensitive {
        (hay.clone(), find.query.clone())
    } else {
        (hay.to_lowercase(), find.query.to_lowercase())
    };

    // Buscamos en bytes; convertimos a char_offsets al devolver.
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut byte_start = 0;
    while let Some(pos) = hay_search[byte_start..].find(&needle_search) {
        let byte_match = byte_start + pos;
        let char_start = hay[..byte_match].chars().count();
        let char_end = char_start + find.query.chars().count();
        out.push((char_start, char_end));
        byte_start = byte_match + needle_search.len().max(1);
    }
    out
}

/// Encuentra la próxima ocurrencia con `start >= caret_off` (la match
/// **en** el caret cuenta, no la saltea). Para avanzar a la siguiente
/// real, el caller mueve el caret al `end` de la match anterior y
/// vuelve a llamar. Wrap-around al fin del buffer → primera match.
pub fn find_next(buf: &Buffer, find: &FindState, cursor: &Cursor) -> Option<(Pos, Pos)> {
    let matches = all_matches(buf, find);
    if matches.is_empty() {
        return None;
    }
    let caret_off = buf.pos_to_offset(cursor.caret.line, cursor.caret.col);
    let next = matches
        .iter()
        .find(|(s, _)| *s >= caret_off)
        .copied()
        .or_else(|| matches.first().copied())?;
    Some(positions_of(buf, next))
}

/// Como [`find_next`] pero en reverso.
pub fn find_prev(buf: &Buffer, find: &FindState, cursor: &Cursor) -> Option<(Pos, Pos)> {
    let matches = all_matches(buf, find);
    if matches.is_empty() {
        return None;
    }
    let caret_off = buf.pos_to_offset(cursor.caret.line, cursor.caret.col);
    let prev = matches
        .iter()
        .rev()
        .find(|(_, e)| *e < caret_off)
        .copied()
        .or_else(|| matches.last().copied())?;
    Some(positions_of(buf, prev))
}

fn positions_of(buf: &Buffer, (start, end): (usize, usize)) -> (Pos, Pos) {
    let (sl, sc) = buf.offset_to_pos(start);
    let (el, ec) = buf.offset_to_pos(end);
    (Pos::new(sl, sc), Pos::new(el, ec))
}

/// Reemplaza la **próxima** ocurrencia desde el caret por `replacement` y
/// deja el caret al final del texto insertado. Devuelve el [`EditDelta`]
/// para la pila de undo, o `None` si no hay ninguna ocurrencia. El
/// wrap-around es el de [`find_next`].
pub fn replace_next(
    buf: &mut Buffer,
    find: &FindState,
    cursor: &mut Cursor,
    replacement: &str,
) -> Option<crate::ops::EditDelta> {
    let (start_pos, end_pos) = find_next(buf, find, cursor)?;
    cursor.anchor = Some(start_pos);
    cursor.caret = end_pos;
    Some(crate::ops::replace_selection(buf, cursor, replacement))
}

/// Reemplaza **todas** las ocurrencias del query por `replacement` en una
/// sola operación reversible. Devuelve `(delta, n)` con el delta para undo
/// y la cantidad reemplazada, o `None` si no había ninguna. El reemplazo
/// es literal (sin grupos/regex).
pub fn replace_all(
    buf: &mut Buffer,
    find: &FindState,
    cursor: &mut Cursor,
    replacement: &str,
) -> Option<(crate::ops::EditDelta, usize)> {
    let matches = all_matches(buf, find);
    if matches.is_empty() {
        return None;
    }
    let n = matches.len();
    // Región mínima que cubre de la primera a la última match: un único
    // delta atómico (un solo undo deshace todo el reemplazo).
    let region_start = matches.first().unwrap().0;
    let region_end = matches.last().unwrap().1;
    let removed = buf.slice(region_start, region_end);

    // Reconstruimos la región reemplazando cada match, copiando lo de entre
    // medio tal cual.
    let mut inserted = String::with_capacity(removed.len());
    let mut prev_end = region_start;
    for (s, e) in &matches {
        inserted.push_str(&buf.slice(prev_end, *s));
        inserted.push_str(replacement);
        prev_end = *e;
    }
    inserted.push_str(&buf.slice(prev_end, region_end));

    let before = *cursor;
    buf.delete(region_start, region_end);
    buf.insert(region_start, &inserted);
    let new_off = region_start + inserted.chars().count();
    let (line, col) = buf.offset_to_pos(new_off);
    cursor.caret = Pos::new(line, col);
    cursor.desired_col = col;
    cursor.anchor = None;
    let delta = crate::ops::EditDelta {
        start: region_start,
        removed,
        inserted,
        cursor_before: before,
        cursor_after: *cursor,
    };
    Some((delta, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_matches_vacio_devuelve_vacio() {
        let b = Buffer::from_str("hola hola");
        let f = FindState::new();
        assert!(all_matches(&b, &f).is_empty());
    }

    #[test]
    fn all_matches_encuentra_todas() {
        let b = Buffer::from_str("ab cd ab ef ab");
        let f = FindState::with_query("ab");
        let m = all_matches(&b, &f);
        assert_eq!(m, vec![(0, 2), (6, 8), (12, 14)]);
    }

    #[test]
    fn case_insensitive_por_default() {
        let b = Buffer::from_str("Hola HOLA hola");
        let f = FindState::with_query("hola");
        assert_eq!(all_matches(&b, &f).len(), 3);
    }

    #[test]
    fn case_sensitive_filtra() {
        let b = Buffer::from_str("Hola HOLA hola");
        let f = FindState { query: "hola".into(), case_sensitive: true };
        assert_eq!(all_matches(&b, &f).len(), 1);
    }

    #[test]
    fn find_next_wrap_al_final() {
        let b = Buffer::from_str("ab cd ab");
        let f = FindState::with_query("ab");
        let c = Cursor::at(0, 8); // al final
        let (a, _) = find_next(&b, &f, &c).unwrap();
        assert_eq!(a, Pos::new(0, 0)); // wrap al primero
    }

    #[test]
    fn find_prev_wrap_al_principio() {
        let b = Buffer::from_str("ab cd ab");
        let f = FindState::with_query("ab");
        let c = Cursor::at(0, 0);
        let (a, _) = find_prev(&b, &f, &c).unwrap();
        assert_eq!(a, Pos::new(0, 6)); // wrap al último
    }

    #[test]
    fn find_next_devuelve_match_en_el_caret() {
        let b = Buffer::from_str("ab ab ab");
        let f = FindState::with_query("ab");
        let c = Cursor::at(0, 0);
        let (a, _) = find_next(&b, &f, &c).unwrap();
        assert_eq!(a, Pos::new(0, 0));
    }

    #[test]
    fn replace_next_reemplaza_una() {
        let mut b = Buffer::from_str("ab cd ab");
        let f = FindState::with_query("ab");
        let mut c = Cursor::at(0, 0);
        replace_next(&mut b, &f, &mut c, "XY");
        assert_eq!(b.text(), "XY cd ab");
    }

    #[test]
    fn replace_all_reemplaza_todas() {
        let mut b = Buffer::from_str("ab cd ab ef ab");
        let f = FindState::with_query("ab");
        let mut c = Cursor::at(0, 0);
        let (_, n) = replace_all(&mut b, &f, &mut c, "Z").unwrap();
        assert_eq!(n, 3);
        assert_eq!(b.text(), "Z cd Z ef Z");
    }

    #[test]
    fn replace_all_distinto_largo_y_undo() {
        let mut b = Buffer::from_str("foo foo");
        let f = FindState::with_query("foo");
        let mut c = Cursor::at(0, 0);
        let (delta, _) = replace_all(&mut b, &f, &mut c, "barbar").unwrap();
        assert_eq!(b.text(), "barbar barbar");
        delta.undo(&mut b, &mut c);
        assert_eq!(b.text(), "foo foo");
    }

    #[test]
    fn find_next_avanza_si_caret_va_al_fin_de_match_anterior() {
        let b = Buffer::from_str("ab ab ab");
        let f = FindState::with_query("ab");
        let mut c = Cursor::at(0, 0);
        let (_, end1) = find_next(&b, &f, &c).unwrap();
        c.caret = end1; // (0, 2) — fin de la primera
        let (a2, _) = find_next(&b, &f, &c).unwrap();
        assert_eq!(a2, Pos::new(0, 3));
    }
}
