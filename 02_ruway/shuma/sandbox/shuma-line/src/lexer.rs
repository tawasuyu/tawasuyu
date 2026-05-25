//! El lexer — convierte una línea de texto en tokens clasificados.
//!
//! Dos pasadas: un escaneo léxico que reconoce comillas, variables,
//! tuberías, redirecciones, operadores y palabras; y una pasada de
//! clasificación que distingue el *comando* (la primera palabra de cada
//! etapa) de sus *argumentos*.

use crate::dialect::Dialect;
use crate::token::{Token, TokenKind};

/// Analiza `input` según `dialect` y devuelve los tokens, contiguos y
/// clasificados, cubriendo toda la línea.
pub fn tokenize(input: &str, dialect: Dialect) -> Vec<Token> {
    let raw = match dialect {
        Dialect::Bash => scan_bash(input),
    };
    classify(raw)
}

/// `true` si `c` corta una palabra suelta.
fn is_word_break(c: char) -> bool {
    c.is_whitespace() || matches!(c, '|' | '&' | ';' | '<' | '>' | '"' | '\'' | '$')
}

/// Detecta una redirección a partir de `p`: un dígito opcional, luego
/// `>`/`<`, y un segundo `>`/`<` opcional (`>>`, `<<`). Devuelve la
/// posición final, o `None`.
fn try_redirect(chars: &[(usize, char)], p: usize) -> Option<usize> {
    let n = chars.len();
    let mut q = p;
    if q < n && chars[q].1.is_ascii_digit() {
        q += 1;
    }
    if q < n && (chars[q].1 == '>' || chars[q].1 == '<') {
        let r = chars[q].1;
        q += 1;
        if q < n && chars[q].1 == r {
            q += 1;
        }
        Some(q)
    } else {
        None
    }
}

/// Escaneo léxico de Bash.
fn scan_bash(input: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = input.char_indices().collect();
    let n = chars.len();
    let byte_at = |p: usize| if p < n { chars[p].0 } else { input.len() };
    let mut tokens: Vec<Token> = Vec::new();
    let push = |tokens: &mut Vec<Token>, kind: TokenKind, sp: usize, ep: usize| {
        let (sb, eb) = (byte_at(sp), byte_at(ep));
        tokens.push(Token::new(kind, sb, eb, &input[sb..eb]));
    };

    let mut p = 0;
    while p < n {
        let c = chars[p].1;

        // Espacio en blanco.
        if c.is_whitespace() {
            let mut q = p;
            while q < n && chars[q].1.is_whitespace() {
                q += 1;
            }
            push(&mut tokens, TokenKind::Whitespace, p, q);
            p = q;
            continue;
        }

        // Comentario hasta fin de línea.
        if c == '#' {
            let mut q = p;
            while q < n && chars[q].1 != '\n' {
                q += 1;
            }
            push(&mut tokens, TokenKind::Comment, p, q);
            p = q;
            continue;
        }

        // Cadena entre comillas simples — literal.
        if c == '\'' {
            let mut q = p + 1;
            while q < n && chars[q].1 != '\'' {
                q += 1;
            }
            if q < n {
                q += 1; // incluye la comilla de cierre
            }
            push(&mut tokens, TokenKind::StringLit, p, q);
            p = q;
            continue;
        }

        // Cadena entre comillas dobles — admite `\"`.
        if c == '"' {
            let mut q = p + 1;
            while q < n {
                if chars[q].1 == '\\' && q + 1 < n {
                    q += 2;
                    continue;
                }
                if chars[q].1 == '"' {
                    break;
                }
                q += 1;
            }
            if q < n {
                q += 1;
            }
            push(&mut tokens, TokenKind::StringLit, p, q);
            p = q;
            continue;
        }

        // Variable / sustitución.
        if c == '$' {
            let mut q = p + 1;
            if q < n && chars[q].1 == '{' {
                while q < n && chars[q].1 != '}' {
                    q += 1;
                }
                if q < n {
                    q += 1;
                }
            } else if q < n && chars[q].1 == '(' {
                let mut depth = 0;
                while q < n {
                    match chars[q].1 {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                q += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    q += 1;
                }
            } else {
                while q < n && (chars[q].1.is_alphanumeric() || chars[q].1 == '_') {
                    q += 1;
                }
            }
            push(&mut tokens, TokenKind::Variable, p, q);
            p = q;
            continue;
        }

        // Tubería vs. OR lógico.
        if c == '|' {
            if p + 1 < n && chars[p + 1].1 == '|' {
                push(&mut tokens, TokenKind::Operator, p, p + 2);
                p += 2;
            } else {
                push(&mut tokens, TokenKind::Pipe, p, p + 1);
                p += 1;
            }
            continue;
        }

        // `&&`, `&>`, `&`.
        if c == '&' {
            if p + 1 < n && chars[p + 1].1 == '&' {
                push(&mut tokens, TokenKind::Operator, p, p + 2);
                p += 2;
            } else if p + 1 < n && chars[p + 1].1 == '>' {
                push(&mut tokens, TokenKind::Redirect, p, p + 2);
                p += 2;
            } else {
                push(&mut tokens, TokenKind::Operator, p, p + 1);
                p += 1;
            }
            continue;
        }

        // Separador de comandos.
        if c == ';' {
            push(&mut tokens, TokenKind::Operator, p, p + 1);
            p += 1;
            continue;
        }

        // Redirección (con dígito de descriptor opcional).
        if let Some(q) = try_redirect(&chars, p) {
            push(&mut tokens, TokenKind::Redirect, p, q);
            p = q;
            continue;
        }

        // Palabra suelta — argumento o flag.
        let mut q = p;
        while q < n && !is_word_break(chars[q].1) {
            q += 1;
        }
        if q == p {
            // Carácter aislado no reconocido: no estancar el bucle.
            push(&mut tokens, TokenKind::Unknown, p, p + 1);
            p += 1;
        } else {
            let kind = if chars[p].1 == '-' {
                TokenKind::Flag
            } else {
                TokenKind::Argument
            };
            push(&mut tokens, kind, p, q);
            p = q;
        }
    }
    tokens
}

/// Segunda pasada: la primera palabra de cada etapa es el comando.
fn classify(mut tokens: Vec<Token>) -> Vec<Token> {
    let mut expect_command = true;
    for t in &mut tokens {
        match t.kind {
            TokenKind::Whitespace | TokenKind::Comment | TokenKind::Redirect => {}
            TokenKind::Pipe | TokenKind::Operator => expect_command = true,
            TokenKind::Argument => {
                if expect_command {
                    t.kind = TokenKind::Command;
                }
                expect_command = false;
            }
            _ => expect_command = false,
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(input: &str) -> Vec<TokenKind> {
        tokenize(input, Dialect::Bash)
            .into_iter()
            .filter(|t| t.kind != TokenKind::Whitespace)
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn tokens_cover_the_whole_line() {
        let input = "ls -la /home";
        let toks = tokenize(input, Dialect::Bash);
        assert_eq!(toks.first().unwrap().start, 0);
        assert_eq!(toks.last().unwrap().end, input.len());
        for w in toks.windows(2) {
            assert_eq!(w[0].end, w[1].start, "los tokens son contiguos");
        }
    }

    #[test]
    fn first_word_is_the_command() {
        assert_eq!(
            kinds("ls -la /home"),
            vec![TokenKind::Command, TokenKind::Flag, TokenKind::Argument]
        );
    }

    #[test]
    fn word_after_pipe_is_a_command_again() {
        let k = kinds("cat file | grep error");
        assert_eq!(
            k,
            vec![
                TokenKind::Command,
                TokenKind::Argument,
                TokenKind::Pipe,
                TokenKind::Command,
                TokenKind::Argument,
            ]
        );
    }

    #[test]
    fn operators_reset_the_command_position() {
        let k = kinds("make && ./run ; echo done");
        assert_eq!(k[0], TokenKind::Command); // make
        assert_eq!(k[2], TokenKind::Command); // ./run, tras &&
        assert_eq!(k[4], TokenKind::Command); // echo, tras ;
        assert_eq!(k[5], TokenKind::Argument); // done
    }

    #[test]
    fn quotes_are_single_string_tokens() {
        assert_eq!(
            kinds("echo \"hola mundo\" 'literal'"),
            vec![TokenKind::Command, TokenKind::StringLit, TokenKind::StringLit]
        );
    }

    #[test]
    fn variables_are_recognized() {
        assert_eq!(
            kinds("echo $HOME ${PATH} $(date)"),
            vec![
                TokenKind::Command,
                TokenKind::Variable,
                TokenKind::Variable,
                TokenKind::Variable,
            ]
        );
    }

    #[test]
    fn redirects_with_descriptors() {
        let k = kinds("cmd 2> err.log >> out.log");
        assert_eq!(k[1], TokenKind::Redirect);
        assert_eq!(k[3], TokenKind::Redirect);
    }

    #[test]
    fn pipe_distinct_from_logical_or() {
        assert_eq!(kinds("a | b")[1], TokenKind::Pipe);
        assert_eq!(kinds("a || b")[1], TokenKind::Operator);
    }

    #[test]
    fn comment_runs_to_end_of_line() {
        let k = kinds("ls # esto es un comentario");
        assert_eq!(k, vec![TokenKind::Command, TokenKind::Comment]);
    }

    #[test]
    fn handles_unicode_without_panicking() {
        let toks = tokenize("echo 'añoño café' ☕", Dialect::Bash);
        assert_eq!(toks.last().unwrap().end, "echo 'añoño café' ☕".len());
    }

    #[test]
    fn empty_line_yields_no_tokens() {
        assert!(tokenize("", Dialect::Bash).is_empty());
    }
}
