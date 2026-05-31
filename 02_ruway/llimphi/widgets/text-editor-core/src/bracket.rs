//! Matching de paréntesis/corchetes/llaves bajo el cursor.
//!
//! Si el carácter inmediatamente *antes* o *en* el caret es un bracket
//! abridor o cerrador, busca su par contando profundidad y devuelve las
//! dos posiciones. Útil para el visor (resaltar ambas).
//!
//! Restricciones del PMV: no diferencia brackets dentro de strings ni
//! comentarios — el tokenizer del bloque de highlight (tree-sitter) lo
//! resolverá mejor en una pasada futura. Para WAT/JSON/Lisp esto basta.

use crate::buffer::Buffer;
use crate::cursor::{Cursor, Pos};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// Pares reconocidos.
const PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}')];

fn pair_of(c: char) -> Option<(char, char, Direction)> {
    for &(o, cl) in PAIRS {
        if c == o {
            return Some((o, cl, Direction::Forward));
        }
        if c == cl {
            return Some((o, cl, Direction::Backward));
        }
    }
    None
}

/// Si el caret toca un bracket, devuelve `(pos_del_bracket, pos_del_par)`.
pub fn find_bracket_pair(buf: &Buffer, cursor: &Cursor) -> Option<(Pos, Pos)> {
    let caret_off = buf.pos_to_offset(cursor.caret.line, cursor.caret.col);

    // Probamos en `caret` y `caret-1` — un caret "entre" dos chars puede
    // tocar al de la izquierda visualmente.
    let candidates: [Option<usize>; 2] = [
        Some(caret_off).filter(|&o| o < buf.len_chars()),
        caret_off.checked_sub(1),
    ];

    for opt in candidates {
        let Some(off) = opt else { continue };
        let Some(ch) = buf.char_at(off) else { continue };
        let Some((open, close, dir)) = pair_of(ch) else { continue };
        let mate = match dir {
            Direction::Forward => find_forward(buf, off + 1, open, close),
            Direction::Backward => find_backward(buf, off, open, close),
        };
        if let Some(mate_off) = mate {
            let a = buf.offset_to_pos(off);
            let b = buf.offset_to_pos(mate_off);
            return Some((Pos::new(a.0, a.1), Pos::new(b.0, b.1)));
        }
    }
    None
}

fn find_forward(buf: &Buffer, from: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 1usize;
    let mut off = from;
    let len = buf.len_chars();
    while off < len {
        match buf.char_at(off) {
            Some(c) if c == open => depth += 1,
            Some(c) if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(off);
                }
            }
            _ => {}
        }
        off += 1;
    }
    None
}

fn find_backward(buf: &Buffer, before: usize, open: char, close: char) -> Option<usize> {
    if before == 0 {
        return None;
    }
    let mut depth = 1usize;
    let mut off = before;
    while off > 0 {
        off -= 1;
        match buf.char_at(off) {
            Some(c) if c == close => depth += 1,
            Some(c) if c == open => {
                depth -= 1;
                if depth == 0 {
                    return Some(off);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empareja_paren_simple() {
        let b = Buffer::from_str("(a)");
        let c = Cursor::at(0, 0); // caret antes del '('
        let (a, m) = find_bracket_pair(&b, &c).unwrap();
        assert_eq!(a, Pos::new(0, 0));
        assert_eq!(m, Pos::new(0, 2));
    }

    #[test]
    fn empareja_desde_el_lado_derecho() {
        let b = Buffer::from_str("(a)");
        let c = Cursor::at(0, 3); // caret después del ')'
        let (a, m) = find_bracket_pair(&b, &c).unwrap();
        assert_eq!(a, Pos::new(0, 2)); // ')'
        assert_eq!(m, Pos::new(0, 0)); // '('
    }

    #[test]
    fn anidados_respeta_profundidad() {
        let b = Buffer::from_str("((a))");
        let c = Cursor::at(0, 0); // primer '('
        let (_, m) = find_bracket_pair(&b, &c).unwrap();
        assert_eq!(m, Pos::new(0, 4)); // último ')'
    }

    #[test]
    fn empareja_brackets_y_llaves() {
        let b = Buffer::from_str("[a]");
        assert!(find_bracket_pair(&b, &Cursor::at(0, 0)).is_some());

        let b2 = Buffer::from_str("{a}");
        assert!(find_bracket_pair(&b2, &Cursor::at(0, 0)).is_some());
    }

    #[test]
    fn caret_lejos_de_bracket_devuelve_none() {
        let b = Buffer::from_str("hola");
        let c = Cursor::at(0, 2);
        assert!(find_bracket_pair(&b, &c).is_none());
    }

    #[test]
    fn bracket_sin_par_devuelve_none() {
        let b = Buffer::from_str("(a");
        let c = Cursor::at(0, 0);
        assert!(find_bracket_pair(&b, &c).is_none());
    }

    #[test]
    fn multilinea_pasa_saltos() {
        let b = Buffer::from_str("(\n  a\n)");
        let c = Cursor::at(0, 0);
        let (_, m) = find_bracket_pair(&b, &c).unwrap();
        assert_eq!(m, Pos::new(2, 0));
    }
}
