//! `charka-lexer` — tokenizador de COBOL.
//!
//! Primera etapa del transpilador COBOL→Rust: convierte el texto fuente
//! en una secuencia de [`Token`]. El lexer es **deliberadamente tonto**
//! — no conoce keywords ni la cláusula `PICTURE`; emite `Word` para todo
//! identificador y deja la clasificación al parser. COBOL es
//! case-insensitive: el `text` de un `Word` va en su caja original y
//! quien matchee keywords debe normalizar.
//!
//! Soporta los dos formatos de fuente:
//! - **Fijo** (`SourceFormat::Fixed`) — la tarjeta de 80 columnas:
//!   cols 1-6 área de secuencia, col 7 indicadora (`*`/`/` comentario,
//!   `D` debugging), cols 8-72 código, 73-80 identificación.
//! - **Libre** (`SourceFormat::Free`) — la línea entera es código; `*`
//!   o `*>` al inicio (tras espacios) es comentario.
//!
//! Limitación conocida (v1): no soporta continuación de literales entre
//! líneas (indicador `-` en col 7) — esos casos se tratan como código
//! normal. Es un subconjunto COBOL'85; el hito intermedio del plan.

#![forbid(unsafe_code)]

use thiserror::Error;

/// Formato del código fuente COBOL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// Formato fijo tradicional (tarjeta de 80 columnas).
    Fixed,
    /// Formato libre: la línea entera es código.
    Free,
}

/// Clase de un token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Palabra COBOL: keyword o identificador (puede llevar guiones
    /// internos, p. ej. `WORKING-STORAGE`).
    Word,
    /// Literal numérico sin signo (`42`, `3.14`). El signo, si lo hay,
    /// es un `Symbol` aparte.
    Number,
    /// Literal de texto, con las comillas dobladas ya colapsadas.
    String,
    /// El punto `.` — terminador de sentencia/párrafo en COBOL.
    Period,
    /// Cualquier otro símbolo: `( ) , ; :` y los operadores
    /// `+ - * / ** = < > <= >= <>`. El símbolo concreto va en `text`.
    Symbol,
}

/// Un token con su posición en el fuente (línea y columna 1-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Lexema. `Word`: caja original. `String`: valor ya decodificado.
    /// `Number`/`Symbol`/`Period`: los caracteres tal cual.
    pub text: String,
    /// Línea 1-based.
    pub line: u32,
    /// Columna 1-based del primer carácter del token.
    pub col: u32,
}

/// Error de tokenización, con la posición donde ocurrió.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LexError {
    #[error("línea {line}:{col}: literal de texto sin cerrar")]
    UnterminatedString { line: u32, col: u32 },
    #[error("línea {line}:{col}: carácter inesperado {ch:?}")]
    UnexpectedChar { line: u32, col: u32, ch: char },
}

/// Tokeniza un fuente COBOL completo. Falla con el primer [`LexError`].
pub fn lex(source: &str, format: SourceFormat) -> Result<Vec<Token>, LexError> {
    let mut tokens = Vec::new();
    for (idx, raw) in source.lines().enumerate() {
        let line = (idx + 1) as u32;
        if let Some((content, base_col)) = prepare_line(raw, format) {
            lex_line(&content, line, base_col, &mut tokens)?;
        }
    }
    Ok(tokens)
}

/// Extrae el área de código de una línea según el formato. `None` si la
/// línea entera se descarta (comentario, debugging). El `u32` es la
/// columna 1-based del primer carácter del contenido devuelto.
fn prepare_line(raw: &str, format: SourceFormat) -> Option<(String, u32)> {
    match format {
        SourceFormat::Fixed => {
            let chars: Vec<char> = raw.chars().collect();
            // Col 7 (índice 6): área indicadora.
            match chars.get(6).copied().unwrap_or(' ') {
                '*' | '/' => return None, // comentario / salto de página
                'D' | 'd' => return None, // línea de debugging — v1 la omite
                _ => {}
            }
            // Cols 8-72 (índices 7..72) = 65 columnas de código.
            let content: String = chars.iter().skip(7).take(65).collect();
            Some((content, 8))
        }
        SourceFormat::Free => {
            let trimmed = raw.trim_start();
            if trimmed.starts_with('*') {
                return None; // `*` o `*>` al inicio: comentario
            }
            Some((raw.to_string(), 1))
        }
    }
}

/// Tokeniza una línea ya recortada al área de código.
fn lex_line(content: &str, line: u32, base_col: u32, out: &mut Vec<Token>) -> Result<(), LexError> {
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let col = base_col + i as u32;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            let (value, next) = lex_string(&chars, i, c, line, col)?;
            out.push(Token {
                kind: TokenKind::String,
                text: value,
                line,
                col,
            });
            i = next;
        } else if c.is_ascii_digit() {
            let (text, next) = lex_number(&chars, i);
            out.push(Token {
                kind: TokenKind::Number,
                text,
                line,
                col,
            });
            i = next;
        } else if c.is_ascii_alphabetic() {
            let (text, next) = lex_word(&chars, i);
            out.push(Token {
                kind: TokenKind::Word,
                text,
                line,
                col,
            });
            i = next;
        } else if c == '.' {
            out.push(Token {
                kind: TokenKind::Period,
                text: ".".into(),
                line,
                col,
            });
            i += 1;
        } else if let Some(op) = two_char_op(&chars, i) {
            out.push(Token {
                kind: TokenKind::Symbol,
                text: op.into(),
                line,
                col,
            });
            i += 2;
        } else if "()+-*/=<>,;:".contains(c) {
            out.push(Token {
                kind: TokenKind::Symbol,
                text: c.to_string(),
                line,
                col,
            });
            i += 1;
        } else {
            return Err(LexError::UnexpectedChar { line, col, ch: c });
        }
    }
    Ok(())
}

/// Lee un literal de texto desde la comilla de apertura. Una comilla
/// doblada dentro del literal representa una comilla literal.
fn lex_string(
    chars: &[char],
    start: usize,
    quote: char,
    line: u32,
    col: u32,
) -> Result<(String, usize), LexError> {
    let mut value = String::new();
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == quote {
            if chars.get(i + 1) == Some(&quote) {
                value.push(quote); // comilla doblada → comilla literal
                i += 2;
            } else {
                return Ok((value, i + 1)); // comilla de cierre
            }
        } else {
            value.push(chars[i]);
            i += 1;
        }
    }
    Err(LexError::UnterminatedString { line, col })
}

/// Lee un literal numérico sin signo. El punto decimal sólo cuenta si
/// lo sigue un dígito — sino es el terminador `.`.
fn lex_number(chars: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
        i += 1;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
    }
    (chars[start..i].iter().collect(), i)
}

/// Lee una palabra COBOL: empieza con letra, sigue con alfanuméricos y
/// guiones internos (un guión sólo si lo sigue un alfanumérico).
fn lex_word(chars: &[char], start: usize) -> (String, usize) {
    let mut i = start + 1;
    while i < chars.len() {
        let c = chars[i];
        // Alfanumérico, o un guión interno (sólo si lo sigue otro
        // alfanumérico — `WORKING-STORAGE`, no un `MOVE-` colgante).
        let word_char = c.is_ascii_alphanumeric()
            || (c == '-' && chars.get(i + 1).is_some_and(|n| n.is_ascii_alphanumeric()));
        if !word_char {
            break;
        }
        i += 1;
    }
    (chars[start..i].iter().collect(), i)
}

/// Reconoce un operador de dos caracteres en la posición `i`.
fn two_char_op(chars: &[char], i: usize) -> Option<&'static str> {
    let a = *chars.get(i)?;
    let b = *chars.get(i + 1)?;
    match (a, b) {
        ('*', '*') => Some("**"),
        ('<', '=') => Some("<="),
        ('>', '=') => Some(">="),
        ('<', '>') => Some("<>"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: lexa y devuelve `(kind, text)` por token.
    fn kinds(src: &str, fmt: SourceFormat) -> Vec<(TokenKind, String)> {
        lex(src, fmt)
            .expect("lex OK")
            .into_iter()
            .map(|t| (t.kind, t.text))
            .collect()
    }

    #[test]
    fn simple_sentence_free() {
        let toks = kinds("MOVE 5 TO WS-COUNT.", SourceFormat::Free);
        assert_eq!(
            toks,
            vec![
                (TokenKind::Word, "MOVE".into()),
                (TokenKind::Number, "5".into()),
                (TokenKind::Word, "TO".into()),
                (TokenKind::Word, "WS-COUNT".into()),
                (TokenKind::Period, ".".into()),
            ]
        );
    }

    #[test]
    fn hyphenated_word_kept_whole() {
        let toks = kinds("WORKING-STORAGE SECTION.", SourceFormat::Free);
        assert_eq!(toks[0], (TokenKind::Word, "WORKING-STORAGE".into()));
        assert_eq!(toks[1], (TokenKind::Word, "SECTION".into()));
    }

    #[test]
    fn word_case_is_preserved() {
        let toks = kinds("Move x", SourceFormat::Free);
        assert_eq!(toks[0].1, "Move");
        assert_eq!(toks[1].1, "x");
    }

    #[test]
    fn string_literal_value_decoded() {
        let toks = kinds("'hola mundo'", SourceFormat::Free);
        assert_eq!(toks, vec![(TokenKind::String, "hola mundo".into())]);
    }

    #[test]
    fn doubled_quote_is_literal_quote() {
        let toks = kinds("'it''s ok'", SourceFormat::Free);
        assert_eq!(toks, vec![(TokenKind::String, "it's ok".into())]);
    }

    #[test]
    fn double_quoted_string() {
        let toks = kinds("\"abc\"", SourceFormat::Free);
        assert_eq!(toks, vec![(TokenKind::String, "abc".into())]);
    }

    #[test]
    fn number_with_decimal_vs_trailing_period() {
        // `3.14` es un número; `5.` es número 5 + terminador.
        assert_eq!(
            kinds("3.14", SourceFormat::Free),
            vec![(TokenKind::Number, "3.14".into())]
        );
        assert_eq!(
            kinds("5.", SourceFormat::Free),
            vec![
                (TokenKind::Number, "5".into()),
                (TokenKind::Period, ".".into()),
            ]
        );
    }

    #[test]
    fn two_and_one_char_operators() {
        let toks = kinds("A <= B >= C <> D ** E + F", SourceFormat::Free);
        let syms: Vec<&str> = toks
            .iter()
            .filter(|(k, _)| *k == TokenKind::Symbol)
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(syms, vec!["<=", ">=", "<>", "**", "+"]);
    }

    #[test]
    fn parens_and_separators() {
        let toks = kinds("PIC X(20), Y;", SourceFormat::Free);
        let syms: Vec<&str> = toks
            .iter()
            .filter(|(k, _)| *k == TokenKind::Symbol)
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(syms, vec!["(", ")", ",", ";"]);
    }

    #[test]
    fn fixed_format_ignores_sequence_and_id_areas() {
        // Cols 1-6 secuencia, col 7 espacio, cols 8.. código, 73+ id.
        let mut line = String::new();
        line.push_str("000100"); // cols 1-6: secuencia
        line.push(' '); // col 7: indicador
        line.push_str("    MOVE 1 TO X."); // código desde col 8
                                           // Rellenar hasta col 73 y agregar área de identificación.
        while line.chars().count() < 72 {
            line.push(' ');
        }
        line.push_str("PROG0001"); // cols 73-80: ignoradas
        let toks = kinds(&line, SourceFormat::Fixed);
        assert_eq!(
            toks,
            vec![
                (TokenKind::Word, "MOVE".into()),
                (TokenKind::Number, "1".into()),
                (TokenKind::Word, "TO".into()),
                (TokenKind::Word, "X".into()),
                (TokenKind::Period, ".".into()),
            ]
        );
    }

    #[test]
    fn fixed_format_comment_line_skipped() {
        let comment = "000100*  esto es un comentario y no debe tokenizar";
        let code = "000200     DISPLAY 'HI'.";
        let src = format!("{comment}\n{code}");
        let toks = kinds(&src, SourceFormat::Fixed);
        assert_eq!(
            toks,
            vec![
                (TokenKind::Word, "DISPLAY".into()),
                (TokenKind::String, "HI".into()),
                (TokenKind::Period, ".".into()),
            ]
        );
    }

    #[test]
    fn free_format_comment_line_skipped() {
        let src = "* un comentario\n*> otro comentario\nDISPLAY 1.";
        let toks = kinds(src, SourceFormat::Free);
        assert_eq!(toks.len(), 3); // DISPLAY 1 .
        assert_eq!(toks[0], (TokenKind::Word, "DISPLAY".into()));
    }

    #[test]
    fn line_and_column_are_tracked() {
        let src = "MOVE 1\n  TO X.";
        let toks = lex(src, SourceFormat::Free).unwrap();
        assert_eq!((toks[0].line, toks[0].col), (1, 1)); // MOVE
        assert_eq!((toks[1].line, toks[1].col), (1, 6)); // 1
        assert_eq!((toks[2].line, toks[2].col), (2, 3)); // TO
        assert_eq!((toks[3].line, toks[3].col), (2, 6)); // X
    }

    #[test]
    fn unterminated_string_is_an_error() {
        let err = lex("MOVE 'sin cerrar", SourceFormat::Free).unwrap_err();
        assert!(matches!(err, LexError::UnterminatedString { line: 1, .. }));
    }

    #[test]
    fn unexpected_char_is_an_error() {
        let err = lex("MOVE 5 ! X", SourceFormat::Free).unwrap_err();
        assert!(matches!(
            err,
            LexError::UnexpectedChar {
                ch: '!',
                line: 1,
                ..
            }
        ));
    }

    #[test]
    fn empty_source_yields_no_tokens() {
        assert!(lex("", SourceFormat::Free).unwrap().is_empty());
        assert!(lex("\n\n   \n", SourceFormat::Free).unwrap().is_empty());
    }

    #[test]
    fn realistic_paragraph() {
        let src = "\
ADD-TOTALS.
    COMPUTE WS-TOTAL = WS-A + WS-B.
    IF WS-TOTAL > 100
        DISPLAY 'GRANDE'
    END-IF.";
        let toks = lex(src, SourceFormat::Free).unwrap();
        // Arranca con el nombre del párrafo y su punto.
        assert_eq!(toks[0].text, "ADD-TOTALS");
        assert_eq!(toks[1].kind, TokenKind::Period);
        // Hay un literal y el operador `>` y `=` en el medio.
        assert!(toks
            .iter()
            .any(|t| t.kind == TokenKind::String && t.text == "GRANDE"));
        assert!(toks
            .iter()
            .any(|t| t.kind == TokenKind::Symbol && t.text == ">"));
        assert!(toks
            .iter()
            .any(|t| t.kind == TokenKind::Symbol && t.text == "="));
        assert!(toks.iter().any(|t| t.text == "END-IF"));
    }
}
